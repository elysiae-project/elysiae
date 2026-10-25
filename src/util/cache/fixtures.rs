use super::*;
use futures_util::FutureExt;
use std::{sync::atomic::AtomicUsize, task::Poll};

struct Fixture(Store);

impl Fixture {
    fn new(budget: u64) -> Self {
        Self(
            Store::new(
                PathBuf::from(format!("cache-fixtures/{}", uuid::Uuid::new_v4())),
                budget,
                ASSET_TTL,
            )
            .unwrap(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0.root);
    }
}

fn run(future: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future);
}

fn catalog(game: Game, version: &str, optional: bool) -> AedesResponse {
    let prefix = format!("/{}/{version}", game.code());

    AedesResponse {
        backgrounds: vec![AedesBackgroundAssets {
            image: format!("{prefix}/image.jpg"),
            video: optional.then(|| format!("{prefix}/video.webm")),
            overlay: optional.then(|| format!("{prefix}/overlay.png")),
        }],
        icon: format!("{prefix}/icon.png"),
        icon_cn: String::new(),
        shortcut: format!("{prefix}/shortcut.png"),
        shortcut_cn: String::new(),
    }
}

async fn download(_url: String, destination: PathBuf, limit: u64) -> Result<()> {
    ensure!(limit >= 4, "fixture exceeds remaining budget");

    let path = full_path(Some(destination), None)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, b"art!")?;

    Ok(())
}

async fn yield_once() {
    let mut yielded = false;

    std::future::poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

async fn seed(store: &Store, game: Game, response: AedesResponse) {
    store
        .prepare(
            game,
            "en-us",
            "en-us",
            false,
            response,
            &|_| {},
            &mut download,
        )
        .await
        .unwrap();
}

#[test]
fn progress_api_future_can_run_on_the_background_runtime() {
    fn assert_send(_: impl Send) {}

    // Constructing (but never polling) the future performs no network or I/O.
    assert_send(refresh_cache_with_preferences(
        "en-us",
        Game::Nap,
        true,
        |_| {},
    ));
}

#[test]
fn priority_static_is_published_before_optional_assets_and_unrelated_failures() {
    run(async {
        let fixture = Fixture::new(4096);
        let store = &fixture.0;
        let priority = Game::Nap;
        let response = catalog(priority, "v1", true);
        let notified = AtomicBool::new(false);
        let calls = Mutex::new(Vec::new());
        let notifications = Mutex::new(Vec::new());

        let report = store
            .refresh_with_progress(
                "en-us",
                true,
                priority,
                |game| {
                    assert_eq!(game, priority);
                    let media = store.resolve(game, "en-us", true);
                    assert!(media.background.is_some());

                    if !notified.swap(true, Ordering::SeqCst) {
                        assert!(!media.is_video);
                        assert!(media.overlay.is_none());
                        assert!(store.existing(&response.icon).is_none());
                        assert!(store.existing(&response.shortcut).is_none());
                        assert!(
                            store
                                .existing(response.backgrounds[0].video.as_deref().unwrap())
                                .is_none()
                        );
                        assert_eq!(
                            calls.lock().unwrap().as_slice(),
                            [response.backgrounds[0].image.clone()]
                        );
                    }

                    notifications.lock().unwrap().push(media);
                },
                |game, _| {
                    let notified = &notified;
                    async move {
                        if game != priority {
                            // These deliberately yielding failures cannot hold up
                            // the selected game's first visible background.
                            assert!(notified.load(Ordering::SeqCst));
                            yield_once().await;
                            anyhow::bail!("unrelated service failure");
                        }

                        Ok(catalog(game, "v1", true))
                    }
                },
                |url, destination, limit| {
                    calls
                        .lock()
                        .unwrap()
                        .push(url::Url::parse(&url).unwrap().path().to_owned());
                    download(url, destination, limit)
                },
            )
            .await
            .unwrap();

        assert_eq!(report.successes, [priority]);
        assert_eq!(report.failures.len(), Game::all().len() - 1);
        assert!(notifications.lock().unwrap().last().unwrap().is_video);
        assert!(store.ready(priority, "en-us", true));
    });
}

#[test]
fn warm_cache_reuses_assets_and_never_downloads_disabled_videos() {
    run(async {
        let fixture = Fixture::new(4096);
        let store = &fixture.0;
        let transfers = AtomicUsize::new(0);

        for pass in 0..2 {
            let report = store
                .refresh_with(
                    "en-us",
                    false,
                    |game, _| async move { Ok(catalog(game, "v1", true)) },
                    |url, destination, limit| {
                        assert!(!url.ends_with(".webm"));
                        assert_eq!(pass, 0, "warm cache must not transfer artwork again");
                        transfers.fetch_add(1, Ordering::SeqCst);
                        download(url, destination, limit)
                    },
                )
                .await
                .unwrap();

            assert!(report.failures.is_empty());
            assert_eq!(report.successes.len(), Game::all().len());

            let old_time = std::time::UNIX_EPOCH + Duration::from_secs(1);
            for game in Game::all() {
                let path = store.path(&metadata_path(game, "en-us")).unwrap();
                if pass == 0 {
                    std::fs::File::open(path)
                        .unwrap()
                        .set_modified(old_time)
                        .unwrap();
                } else {
                    assert_eq!(
                        std::fs::metadata(path).unwrap().modified().unwrap(),
                        old_time,
                        "unchanged warm catalogs must not be rewritten"
                    );
                }
            }
        }

        assert_eq!(transfers.load(Ordering::SeqCst), Game::all().len() * 4);
    });
}

#[test]
fn catalog_prefetch_is_bounded_and_progresses_during_serial_downloads() {
    run(async {
        let fixture = Fixture::new(4096);
        let fetching = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let downloading = AtomicUsize::new(0);
        let overlapped = AtomicBool::new(false);

        let report = fixture
            .0
            .refresh_with(
                "en-us",
                false,
                |game, _| {
                    let (fetching, peak, downloading, overlapped) =
                        (&fetching, &peak, &downloading, &overlapped);
                    async move {
                        let count = fetching.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(count, Ordering::SeqCst);
                        assert!(count <= CATALOG_CONCURRENCY);
                        yield_once().await;
                        yield_once().await;
                        if downloading.load(Ordering::SeqCst) != 0 {
                            overlapped.store(true, Ordering::SeqCst);
                        }

                        fetching.fetch_sub(1, Ordering::SeqCst);

                        Ok(catalog(game, "v1", false))
                    }
                },
                |url, destination, limit| {
                    let downloading = &downloading;
                    async move {
                        assert_eq!(downloading.fetch_add(1, Ordering::SeqCst), 0);
                        yield_once().await;
                        let result = download(url, destination, limit).await;
                        assert_eq!(downloading.fetch_sub(1, Ordering::SeqCst), 1);

                        result
                    }
                },
            )
            .await
            .unwrap();

        assert!(report.failures.is_empty());
        assert_eq!(peak.load(Ordering::SeqCst), CATALOG_CONCURRENCY);
        assert!(overlapped.load(Ordering::SeqCst));
    });
}

#[test]
fn budget_includes_staging_and_retains_old_catalog_when_full() {
    run(async {
        let fixture = Fixture::new(16);
        let store = &fixture.0;
        let staging = store
            .path(Path::new("cache/assets/.download-cache-live"))
            .unwrap();
        std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
        std::fs::write(&staging, b"live").unwrap();

        let limits = Mutex::new(Vec::new());

        store
            .prepare(
                Game::Bh3,
                "en-us",
                "en-us",
                false,
                catalog(Game::Bh3, "v1", false),
                &|_| {},
                &mut |url, destination, limit| {
                    limits.lock().unwrap().push(limit);
                    download(url, destination, limit)
                },
            )
            .await
            .unwrap();

        assert_eq!(*limits.lock().unwrap(), [12, 8, 4]);
        assert_eq!(store.collect(&HashSet::new()).unwrap(), 16);
        assert!(staging.exists());

        let path = store.path(&metadata_path(Game::Bh3, "en-us")).unwrap();
        let original = std::fs::read(&path).unwrap();
        let error = store
            .prepare(
                Game::Bh3,
                "en-us",
                "en-us",
                false,
                catalog(Game::Bh3, "v2", false),
                &|_| {},
                &mut |_, _, _| async { panic!("full cache must not start a transfer") },
            )
            .await
            .unwrap_err();

        assert!(format!("{error:#}").contains("exhausted"));
        assert_eq!(std::fs::read(path).unwrap(), original);
        assert!(store.ready(Game::Bh3, "en-us", false));
    });
}

#[test]
fn failed_required_asset_restores_old_catalog_and_preserves_active_new_image() {
    run(async {
        let fixture = Fixture::new(4096);
        let store = &fixture.0;
        let game = Game::Hyg;
        let old = catalog(game, "v1", false);
        seed(store, game, old.clone()).await;

        let metadata = store.path(&metadata_path(game, "en-us")).unwrap();
        let original = std::fs::read(&metadata).unwrap();
        let new = catalog(game, "v2", false);
        let notifications = AtomicUsize::new(0);

        let result = store
            .prepare(
                game,
                "en-us",
                "en-us",
                false,
                new.clone(),
                &|_| {
                    notifications.fetch_add(1, Ordering::SeqCst);
                    let image = store.resolve(game, "en-us", false).background.unwrap();
                    assert_eq!(image, store.existing(&new.backgrounds[0].image).unwrap());
                    assert_eq!(
                        store.cached_paths(game, "en-us", AssetType::Icon),
                        [store.existing(&old.icon).unwrap()]
                    );
                    store.set_active(&[image]).unwrap();
                },
                &mut |url, destination, limit| async move {
                    ensure!(!url.ends_with("icon.png"), "icon service failed");
                    download(url, destination, limit).await
                },
            )
            .await;

        assert!(result.is_err());
        assert_eq!(notifications.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(metadata).unwrap(), original);

        store.collect(&HashSet::new()).unwrap();
        for path in old.all_paths().filter(|path| !path.is_empty()) {
            assert!(store.existing(path).is_some());
        }

        assert!(store.existing(&new.backgrounds[0].image).is_some());

        store.set_active(&[]).unwrap();
        store.collect(&HashSet::new()).unwrap();
        assert!(store.existing(&new.backgrounds[0].image).is_none());
    });
}

#[test]
fn requested_locale_failure_still_refreshes_and_reports_english_fallback() {
    run(async {
        let fixture = Fixture::new(4096);
        let game = Game::Abc;
        let requests = Mutex::new(Vec::new());
        let notified = AtomicBool::new(false);

        let report = fixture
            .0
            .refresh_with_progress(
                "fr-fr",
                false,
                game,
                |ready| {
                    assert_eq!(ready, game);
                    assert!(fixture.0.resolve(game, "fr-fr", false).background.is_some());
                    notified.store(true, Ordering::SeqCst);
                },
                |requested, source| {
                    requests.lock().unwrap().push((requested, source.clone()));
                    async move {
                        ensure!(
                            requested == game && source == "en-us",
                            "localized service unavailable"
                        );
                        Ok(catalog(game, "english", false))
                    }
                },
                download,
            )
            .await
            .unwrap();

        assert!(notified.load(Ordering::SeqCst));
        assert!(report.successes.is_empty());
        assert!(
            report
                .failures
                .iter()
                .any(|(failed, message)| *failed == game && message.starts_with("fr-fr:"))
        );
        assert_eq!(
            &requests.lock().unwrap()[..2],
            &[(game, "fr-fr".to_owned()), (game, "en-us".to_owned())]
        );
        assert!(fixture.0.ready(game, "fr-fr", false));
        assert!(fixture.0.catalog(game, "fr-fr").is_none());
        assert!(fixture.0.catalog(game, "en-us").is_some());
    });
}

#[test]
fn chinese_icon_failures_use_global_variants() {
    run(async {
        let fixture = Fixture::new(4096);
        let game = Game::Hk4e;
        let mut response = catalog(game, "v1", true);
        response.icon_cn = "/cn/icon.png".to_owned();
        response.shortcut_cn = "/cn/shortcut.png".to_owned();

        fixture
            .0
            .prepare(
                game,
                "zh-cn",
                "zh-cn",
                false,
                response.clone(),
                &|_| {},
                &mut |url, destination, limit| async move {
                    assert!(!url.ends_with(".webm"));
                    ensure!(!url.contains("/cn/"), "localized artwork unavailable");
                    download(url, destination, limit).await
                },
            )
            .await
            .unwrap();

        assert!(fixture.0.ready(game, "zh-cn", false));
        assert_eq!(
            fixture.0.cached_paths(game, "zh-cn", AssetType::Icon),
            [fixture.0.existing(&response.icon).unwrap()]
        );
    });
}

#[test]
fn cancelling_partial_refresh_restores_previous_catalog() {
    run(async {
        let fixture = Fixture::new(4096);
        let store = &fixture.0;
        let game = Game::Bh3;
        seed(store, game, catalog(game, "v1", false)).await;

        let metadata = store.path(&metadata_path(game, "en-us")).unwrap();
        let original = std::fs::read(&metadata).unwrap();
        let notified = AtomicBool::new(false);
        let on_ready = |_| {
            notified.store(true, Ordering::SeqCst);
        };
        let mut transfer = |url: String, destination, limit| async move {
            if url.ends_with("icon.png") {
                std::future::pending::<()>().await;
            }

            download(url, destination, limit).await
        };
        let pending = store.prepare(
            game,
            "en-us",
            "en-us",
            false,
            catalog(game, "v2", false),
            &on_ready,
            &mut transfer,
        );

        assert!(pending.now_or_never().is_none());
        assert!(notified.load(Ordering::SeqCst));
        assert_eq!(std::fs::read(metadata).unwrap(), original);
        assert!(store.ready(game, "en-us", false));
    });
}

#[test]
fn stale_replacements_account_for_old_and_staged_bytes_and_keep_pins() {
    run(async {
        let mut fixture = Fixture::new(16);
        let game = Game::Bh3;
        let response = catalog(game, "v1", false);
        seed(&fixture.0, game, response.clone()).await;

        fixture.0.ttl = Duration::ZERO;
        let icon = fixture.0.existing(&response.icon).unwrap();
        fixture.0.set_active(std::slice::from_ref(&icon)).unwrap();
        let limits = Mutex::new(Vec::new());
        let notified = AtomicBool::new(false);

        fixture
            .0
            .prepare(
                game,
                "en-us",
                "en-us",
                false,
                response,
                &|_| {
                    notified.store(true, Ordering::SeqCst);
                },
                &mut |url, destination, limit| {
                    assert!(
                        notified.load(Ordering::SeqCst),
                        "stale media must be ready before revalidation"
                    );
                    assert!(
                        !url.ends_with("icon.png"),
                        "pinned file must not be replaced"
                    );
                    limits.lock().unwrap().push(limit);
                    download(url, destination, limit)
                },
            )
            .await
            .unwrap();

        assert_eq!(*limits.lock().unwrap(), [4, 4]);
        assert_eq!(std::fs::read(icon).unwrap(), b"art!");
        assert_eq!(fixture.0.collect(&HashSet::new()).unwrap(), 12);
    });
}

#[test]
fn cold_cache_keeps_displayable_static_image_when_required_icon_fails() {
    run(async {
        let fixture = Fixture::new(4096);
        let game = Game::Nap;
        let response = catalog(game, "v1", true);
        let notified = AtomicBool::new(false);

        let result = fixture
            .0
            .prepare(
                game,
                "en-us",
                "en-us",
                false,
                response,
                &|_| {
                    notified.store(true, Ordering::SeqCst);
                    assert!(fixture.0.resolve(game, "en-us", false).background.is_some());
                },
                &mut |url, destination, limit| async move {
                    ensure!(!url.ends_with("icon.png"), "icon unavailable");
                    download(url, destination, limit).await
                },
            )
            .await;

        assert!(result.is_err());
        assert!(notified.load(Ordering::SeqCst));

        fixture.0.collect(&HashSet::new()).unwrap();
        assert!(fixture.0.resolve(game, "en-us", false).background.is_some());
        assert!(!fixture.0.ready(game, "en-us", false));
    });
}

#[cfg(unix)]
#[test]
fn symlink_assets_are_neither_downloaded_nor_collected_through() {
    run(async {
        let fixture = Fixture::new(4096);
        let store = &fixture.0;
        let outside = store.root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let sentinel = outside.join("image.jpg");
        std::fs::write(&sentinel, b"private").unwrap();

        let link = store.path(Path::new("cache/assets/linked")).unwrap();
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        let mut response = catalog(Game::Bh3, "v1", false);
        response.backgrounds[0].image = "/linked/image.jpg".to_owned();

        assert!(
            store
                .prepare(
                    Game::Bh3,
                    "en-us",
                    "en-us",
                    false,
                    response,
                    &|_| {},
                    &mut |_, _, _| async { panic!("symlink must fail before transfer") },
                )
                .await
                .is_err()
        );
        assert!(store.existing("/linked/image.jpg").is_none());
        assert!(store.set_active(&[link.join("image.jpg")]).is_err());

        store.collect(&HashSet::new()).unwrap();
        assert_eq!(std::fs::read(sentinel).unwrap(), b"private");
    });
}
