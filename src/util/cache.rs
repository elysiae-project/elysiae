use std::{
    collections::HashSet,
    future::Future,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    core::{fs::full_path, game::Game},
    util::{
        settings,
        web::{download_file_limited, fetch_data},
    },
};

const GAMES: [Game; 6] = [
    Game::Bh3,
    Game::Hk4e,
    Game::Hkrpg,
    Game::Nap,
    Game::Abc,
    Game::Hyg,
];
// The asset budget includes download staging files. Catalogs are separately
// bounded to 4 MiB each and the supported game/locale pairs. Referenced or active
// files are never evicted to make room: an oversized working set keeps its last
// usable catalog instead. URLs are not assumed immutable; recheck after 7 days
// when unpinned. The cap is on asset bytes, not filesystem allocation overhead.
const ASSET_BUDGET: u64 = 512 * 1024 * 1024;
const ASSET_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MAX_CATALOG_SIZE: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub enum AssetType {
    Image,
    Video,
    Icon,
    Shortcut,
    Overlay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AedesResponse {
    backgrounds: Vec<AedesBackgroundAssets>,
    icon: String,
    #[serde(default)]
    icon_cn: String,
    shortcut: String,
    #[serde(default)]
    shortcut_cn: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AedesBackgroundAssets {
    image: String,
    video: Option<String>,
    overlay: Option<String>,
}

#[derive(Debug, Default)]
pub struct ResolvedMedia {
    pub background: Option<PathBuf>,
    pub overlay: Option<PathBuf>,
    pub is_video: bool,
}

#[derive(Debug, Default)]
pub struct CacheRefresh {
    /// Games whose requested-locale catalog was refreshed successfully.
    pub successes: Vec<Game>,
    /// Requested-locale failures, even when English fallback is still usable.
    pub failures: Vec<(Game, String)>,
}

struct Store {
    relative: PathBuf,
    root: PathBuf,
    budget: u64,
    ttl: Duration,
    active: Mutex<HashSet<PathBuf>>,
    refreshing: AtomicBool,
}

struct RefreshPermit<'a>(&'a AtomicBool);
impl Drop for RefreshPermit<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct TemporaryFile(PathBuf);
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn store() -> Result<&'static Store> {
    static STORE: OnceLock<Store> = OnceLock::new();
    if STORE.get().is_none() {
        let _ = STORE.set(Store::new(PathBuf::from("cache"), ASSET_BUDGET, ASSET_TTL)?);
    }
    STORE.get().context("Asset cache is unavailable")
}

pub async fn refresh_cache(locale: &str) -> Result<CacheRefresh> {
    let use_video = bool::try_from(settings::get_option("use-video-background")?)?;
    store()?
        .refresh_with(
            locale,
            use_video,
            |game, locale| async move {
                let mut url = url::Url::parse("https://aedes.elysiae.app/getAssets")?;
                url.query_pairs_mut()
                    .append_pair("lang", &locale)
                    .append_pair("game", game.code());
                fetch_data::<AedesResponse>(url.as_str()).await
            },
            |url, destination, limit| download_file_limited(url, destination, None, None, limit),
        )
        .await
}

pub async fn update_cache() -> Result<()> {
    let report = refresh_cache(&settings::display_locale()?).await?;
    ensure!(
        report.failures.is_empty(),
        "Asset cache refresh failed: {}",
        report
            .failures
            .iter()
            .map(|(game, error)| format!("{}: {error}", game.code()))
            .collect::<Vec<_>>()
            .join("; ")
    );
    Ok(())
}

/// All games have usable media, an icon and a shortcut (possibly in English).
pub fn cache_available(locale: &str) -> Result<bool> {
    let locale = settings::normalize_locale(locale)?;
    let use_video = bool::try_from(settings::get_option("use-video-background")?)?;
    let store = store()?;
    Ok(GAMES
        .into_iter()
        .all(|game| store.ready(game, &locale, use_video)))
}

pub fn game_cache_available(game: Game, locale: &str) -> Result<bool> {
    let locale = settings::normalize_locale(locale)?;
    let use_video = bool::try_from(settings::get_option("use-video-background")?)?;
    Ok(store()?.ready(game, &locale, use_video))
}

/// Replace the displayed-media pins. Passing an empty slice clears them.
/// Only existing, regular files inside this cache's asset tree can be pinned.
pub fn set_active_assets(paths: &[PathBuf]) -> Result<()> {
    store()?.set_active(paths)
}

pub fn resolve_media(game: Game, locale: &str, use_video: bool) -> Result<ResolvedMedia> {
    let locale = settings::normalize_locale(locale)?;
    log::info!(
        "Resolving media paths: game={}, locale={locale}, use_video={use_video}",
        game.code()
    );
    let media = store()?.resolve(game, &locale, use_video);
    log::info!(
        "Resolved media: background={:?}, overlay={:?}, is_video={}",
        media.background,
        media.overlay,
        media.is_video
    );
    Ok(media)
}

pub fn media_paths(
    game: Game,
    locale: &str,
    use_video: bool,
) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    let media = resolve_media(game, locale, use_video)?;
    Ok((media.background, media.overlay))
}

pub fn get_cached_asset_paths(
    game: Game,
    locale: &str,
    asset_type: AssetType,
) -> Result<Vec<PathBuf>> {
    let locale = settings::normalize_locale(locale)?;
    log::info!(
        "Reading cached assets: game={}, locale={locale}, type={asset_type:?}",
        game.code()
    );
    Ok(store()?.cached_paths(game, &locale, asset_type))
}

impl AedesResponse {
    fn asset_paths(&self) -> impl Iterator<Item = Option<&str>> {
        self.backgrounds
            .iter()
            .flat_map(|background| {
                [
                    Some(background.image.as_str()),
                    background.video.as_deref(),
                    background.overlay.as_deref(),
                ]
            })
            .chain([Some(self.icon.as_str()), Some(self.shortcut.as_str())])
    }

    fn all_paths(&self) -> impl Iterator<Item = &str> {
        self.asset_paths()
            .flatten()
            .chain([self.icon_cn.as_str(), self.shortcut_cn.as_str()])
    }

    fn variants(&self, locale: &str, shortcut: bool) -> Vec<&str> {
        let (global, cn) = if shortcut {
            (&self.shortcut, &self.shortcut_cn)
        } else {
            (&self.icon, &self.icon_cn)
        };
        // This selects localized artwork, not a game-server region.
        if locale == "zh-cn" && !cn.is_empty() {
            vec![cn, global]
        } else {
            vec![global]
        }
    }
}

fn metadata_path(game: Game, locale: &str) -> PathBuf {
    PathBuf::from(format!("cache/catalogs/{}/{}.json", game.code(), locale))
}

fn cache_path(path: &Path) -> Result<PathBuf> {
    let text = path.to_str().context("Asset path is not UTF-8")?;
    let relative = text
        .strip_prefix('/')
        .context("Asset path must be absolute on the Aedes server")?;
    // Reject ambiguous URL syntax before Url can normalize it. Preserve every
    // server path component, including the server's own `assets` directory and
    // percent-encoded filenames, but never encoded separators or dot segments.
    let lowercase = relative.to_ascii_lowercase();
    let bytes = relative.as_bytes();
    ensure!(
        !relative.is_empty()
            && !relative
                .chars()
                .any(|c| c.is_control() || matches!(c, '\\' | '?' | '#'))
            && !lowercase.contains("%2f")
            && !lowercase.contains("%5c")
            && bytes
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'%')
                .all(|(i, _)| bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                    && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
            && lowercase.split('/').all(|part| !part.is_empty()
                && !matches!(part.replace("%2e", ".").as_str(), "." | "..")
                && !part.starts_with(".download-")),
        "Asset path contains unsafe components"
    );
    Ok(PathBuf::from("cache/assets").join(relative))
}

fn locales(locale: &str) -> impl Iterator<Item = &str> {
    std::iter::once(locale).chain((locale != "en-us").then_some("en-us"))
}

impl Store {
    fn new(relative: PathBuf, budget: u64, ttl: Duration) -> Result<Self> {
        let root = full_path(Some(relative.clone()), None)?;
        let store = Self {
            relative,
            root,
            budget,
            ttl,
            active: Mutex::new(HashSet::new()),
            refreshing: AtomicBool::new(false),
        };
        store.path(Path::new("cache/assets"))?;
        Ok(store)
    }

    /// core::fs accepts relative inputs; native I/O below only uses these
    /// validated absolute paths. Reject even internal symlinks, so GC cannot
    /// reach catalogs or a different application's tree via a directory link.
    fn path(&self, key: &Path) -> Result<PathBuf> {
        let tail = key
            .strip_prefix("cache")
            .context("Path is not in the asset cache")?;
        ensure!(
            !tail.as_os_str().is_empty()
                && tail.components().all(|c| matches!(c, Component::Normal(_))),
            "Unsafe cache path"
        );
        let destination = full_path(Some(self.relative.join(tail)), None)?;
        let mut current = self.root.clone();
        for component in std::iter::once(None).chain(tail.components().map(Some)) {
            if let Some(component) = component {
                current.push(component);
            }
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) => ensure!(
                    !metadata.file_type().is_symlink(),
                    "Symlinks are not cache entries: {}",
                    current.display()
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(destination)
    }

    fn existing(&self, server_path: &str) -> Option<PathBuf> {
        let path = self.path(&cache_path(Path::new(server_path)).ok()?).ok()?;
        std::fs::symlink_metadata(&path)
            .ok()
            .filter(|m| m.is_file() && m.len() > 0)
            .map(|_| path)
    }

    fn read_catalog(&self, game: Game, locale: &str) -> Result<AedesResponse> {
        let path = self.path(&metadata_path(game, locale))?;
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_file() && metadata.len() <= MAX_CATALOG_SIZE,
            "Invalid cached catalog size or type"
        );
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_CATALOG_SIZE + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_CATALOG_SIZE,
            "Cached catalog is oversized"
        );
        serde_json::from_slice(&bytes).context("Invalid cached catalog")
    }

    fn catalog(&self, game: Game, locale: &str) -> Option<AedesResponse> {
        match self.read_catalog(game, locale) {
            Ok(response) => Some(response),
            Err(error) => {
                log::debug!(
                    "No usable asset catalog for {}/{locale}: {error:#}",
                    game.code()
                );
                None
            }
        }
    }

    fn selected(&self, response: &AedesResponse, use_video: bool) -> ResolvedMedia {
        for video in [true, false] {
            if video && !use_video {
                continue;
            }
            for background in &response.backgrounds {
                let candidate = if video {
                    background.video.as_deref()
                } else {
                    Some(background.image.as_str())
                };
                if let Some(path) = candidate.and_then(|path| self.existing(path)) {
                    return ResolvedMedia {
                        background: Some(path),
                        overlay: background
                            .overlay
                            .as_deref()
                            .and_then(|path| self.existing(path)),
                        is_video: video,
                    };
                }
            }
        }
        ResolvedMedia::default()
    }

    fn resolve(&self, game: Game, locale: &str, use_video: bool) -> ResolvedMedia {
        for source in locales(locale) {
            if let Some(response) = self.catalog(game, source) {
                let media = self.selected(&response, use_video);
                if media.background.is_some() {
                    return media;
                }
            }
        }
        ResolvedMedia::default()
    }

    fn paths(&self, response: &AedesResponse, locale: &str, kind: AssetType) -> Vec<PathBuf> {
        let paths: Vec<&str> = match kind {
            AssetType::Image => response
                .backgrounds
                .iter()
                .map(|b| b.image.as_str())
                .collect(),
            AssetType::Video => response
                .backgrounds
                .iter()
                .filter_map(|b| b.video.as_deref())
                .collect(),
            AssetType::Overlay => response
                .backgrounds
                .iter()
                .filter_map(|b| b.overlay.as_deref())
                .collect(),
            AssetType::Icon => response.variants(locale, false),
            AssetType::Shortcut => response.variants(locale, true),
        };
        paths
            .into_iter()
            .filter_map(|path| self.existing(path))
            .collect()
    }

    fn cached_paths(&self, game: Game, locale: &str, kind: AssetType) -> Vec<PathBuf> {
        for source in locales(locale) {
            if let Some(response) = self.catalog(game, source) {
                let paths = self.paths(&response, locale, kind);
                if !paths.is_empty() {
                    return paths;
                }
            }
        }
        Vec::new()
    }

    fn usable(&self, response: &AedesResponse, locale: &str, use_video: bool) -> bool {
        self.selected(response, use_video).background.is_some()
            && !self.paths(response, locale, AssetType::Icon).is_empty()
            && !self.paths(response, locale, AssetType::Shortcut).is_empty()
    }

    fn ready(&self, game: Game, locale: &str, use_video: bool) -> bool {
        locales(locale).any(|source| {
            self.catalog(game, source)
                .is_some_and(|response| self.usable(&response, locale, use_video))
        })
    }

    fn set_active(&self, paths: &[PathBuf]) -> Result<()> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("Cache pin lock poisoned"))?;
        let mut pins = HashSet::new();
        for path in paths {
            let relative = path
                .strip_prefix(self.root.join("assets"))
                .context("Active asset is outside the cache")?;
            let validated = self.path(&Path::new("cache/assets").join(relative))?;
            ensure!(
                &validated == path && std::fs::symlink_metadata(&validated)?.is_file(),
                "Active asset is not a regular cache file"
            );
            pins.insert(validated);
        }
        *active = pins;
        Ok(())
    }

    fn permit(&self) -> Result<RefreshPermit<'_>> {
        ensure!(
            self.refreshing
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok(),
            "An asset cache refresh is already running"
        );
        Ok(RefreshPermit(&self.refreshing))
    }

    fn references(&self) -> HashSet<PathBuf> {
        let mut paths = HashSet::new();
        for game in GAMES {
            for locale in settings::SUPPORTED_LOCALES {
                if let Ok(response) = self.read_catalog(game, locale) {
                    paths.extend(
                        response
                            .all_paths()
                            .filter_map(|path| cache_path(Path::new(path)).ok())
                            .filter_map(|key| self.path(&key).ok()),
                    );
                }
            }
        }
        paths
    }

    /// Scan only owned asset files, without following links. Temporary download
    /// files count against the budget until the TTL expires. Their transfer
    /// deadline is shorter than that TTL, so abandoned downloads can be reclaimed.
    /// Published catalogs and unrelated cache roots are untouched.
    fn collect(&self, prospective: &HashSet<PathBuf>) -> Result<u64> {
        self.collect_catalog_temporaries();
        let mut protected = self.references();
        protected.extend(prospective.iter().cloned());
        let root = self.path(Path::new("cache/assets"))?;
        if !root.try_exists()? {
            return Ok(0);
        }
        let mut pending = vec![root];
        let mut directories = Vec::new();
        let mut files = Vec::new();
        while let Some(directory) = pending.pop() {
            let key = Path::new("cache").join(directory.strip_prefix(&self.root)?);
            self.path(&key)?;
            directories.push(directory.clone());
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    pending.push(entry.path());
                } else if kind.is_file() {
                    files.push((entry.path(), entry.metadata()?));
                }
            }
        }
        let mut total = 0u64;
        for (path, metadata) in files {
            let size = metadata.len();
            let active = self
                .active
                .lock()
                .map_err(|_| anyhow::anyhow!("Cache pin lock poisoned"))?;
            let temporary = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(".download-"));
            let recent_temporary = temporary
                && metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.elapsed().ok())
                    .is_none_or(|age| age < self.ttl);
            if recent_temporary || protected.contains(&path) || active.contains(&path) {
                total = total.checked_add(size).context("Cache size overflow")?;
            } else {
                let key = Path::new("cache").join(path.strip_prefix(&self.root)?);
                std::fs::remove_file(self.path(&key)?)?;
            }
        }
        // Removed versions must not leave an ever-growing tree of empty folders.
        for directory in directories.into_iter().rev() {
            let key = Path::new("cache").join(directory.strip_prefix(&self.root)?);
            match std::fs::remove_dir(self.path(&key)?) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(total)
    }

    fn collect_catalog_temporaries(&self) {
        for game in GAMES {
            let Ok(directory) = self.path(&Path::new("cache/catalogs").join(game.code())) else {
                continue;
            };
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(id) = name
                    .to_str()
                    .and_then(|name| name.strip_prefix(".catalog-"))
                else {
                    continue;
                };
                if uuid::Uuid::parse_str(id).is_err() {
                    continue;
                }
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if metadata.is_file()
                    && metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.elapsed().ok())
                        .is_some_and(|age| age >= self.ttl)
                    && let Err(error) = std::fs::remove_file(entry.path())
                {
                    log::warn!("Could not reclaim abandoned catalog: {error}");
                }
            }
        }
    }

    async fn asset<D, DF>(
        &self,
        server_path: &str,
        prospective: &HashSet<PathBuf>,
        download: &mut D,
    ) -> Result<PathBuf>
    where
        D: FnMut(String, PathBuf, u64) -> DF,
        DF: Future<Output = Result<()>>,
    {
        let key = cache_path(Path::new(server_path))?;
        let destination = self.path(&key)?;
        let existing = self.existing(server_path);
        if existing.is_some() {
            let active = self
                .active
                .lock()
                .map_err(|_| anyhow::anyhow!("Cache pin lock poisoned"))?;
            if active.contains(&destination)
                || std::fs::metadata(&destination)?
                    .modified()?
                    .elapsed()
                    .is_ok_and(|age| age < self.ttl)
            {
                return Ok(destination);
            }
        }
        let result = async {
            let used = self.collect(prospective)?;
            let limit = self.budget.saturating_sub(used);
            ensure!(
                limit > 0,
                "Asset cache budget of {} bytes is exhausted",
                self.budget
            );
            let temporary_key = PathBuf::from(format!(
                "cache/assets/.download-cache-{}",
                uuid::Uuid::new_v4()
            ));
            let temporary = TemporaryFile(self.path(&temporary_key)?);
            let relative = self.relative.join(temporary_key.strip_prefix("cache")?);
            let mut url = url::Url::parse("https://aedes.elysiae.app")?;
            url.set_path(server_path);
            download(url.into(), relative, limit).await?;
            let metadata = std::fs::symlink_metadata(&temporary.0)?;
            ensure!(
                metadata.is_file() && metadata.len() > 0 && metadata.len() <= limit,
                "Downloaded asset is empty or exceeds the cache budget"
            );
            // A display change may pin the old file while the download awaits.
            // Commit under the same short lock used by pinning and collection.
            let active = self
                .active
                .lock()
                .map_err(|_| anyhow::anyhow!("Cache pin lock poisoned"))?;
            if !(active.contains(&destination) && existing.is_some()) {
                let destination = self.path(&key)?;
                std::fs::create_dir_all(destination.parent().context("Asset has no parent")?)?;
                std::fs::rename(&temporary.0, destination)?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            if existing.is_none() {
                return Err(error).with_context(|| format!("Could not cache {server_path}"));
            }
            log::warn!("Retaining stale cached asset {server_path}: {error:#}");
        }
        Ok(destination)
    }

    fn write_catalog(&self, game: Game, locale: &str, response: &AedesResponse) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(response)?;
        ensure!(
            bytes.len() as u64 <= MAX_CATALOG_SIZE,
            "Asset catalog is oversized"
        );
        let destination = self.path(&metadata_path(game, locale))?;
        let parent = destination.parent().context("Catalog has no parent")?;
        std::fs::create_dir_all(parent)?;
        let temporary_path = parent.join(format!(".catalog-{}", uuid::Uuid::new_v4()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)?;
        let temporary = TemporaryFile(temporary_path);
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary.0, destination)?;
        Ok(())
    }

    async fn prepare<D, DF>(
        &self,
        game: Game,
        source: &str,
        locale: &str,
        use_video: bool,
        response: AedesResponse,
        download: &mut D,
    ) -> Result<()>
    where
        D: FnMut(String, PathBuf, u64) -> DF,
        DF: Future<Output = Result<()>>,
    {
        let prospective = response
            .all_paths()
            .filter_map(|path| cache_path(Path::new(path)).ok())
            .filter_map(|key| self.path(&key).ok())
            .collect();
        for shortcut in [false, true] {
            let mut errors = Vec::new();
            for candidate in response.variants(locale, shortcut) {
                match self.asset(candidate, &prospective, download).await {
                    Ok(_) => {
                        errors.clear();
                        break;
                    }
                    Err(error) => errors.push(format!("{error:#}")),
                }
            }
            ensure!(
                errors.is_empty(),
                "Required {} is unavailable: {}",
                if shortcut { "shortcut" } else { "icon" },
                errors.join("; ")
            );
        }
        let mut selected = None;
        let mut errors = Vec::new();
        for video in [true, false] {
            if video && !use_video {
                continue;
            }
            let mut candidates: Vec<_> = response
                .backgrounds
                .iter()
                .filter_map(|background| {
                    let path = if video {
                        background.video.as_deref()
                    } else {
                        Some(background.image.as_str())
                    }?;
                    Some((background, path))
                })
                .collect();
            // Keep the already-usable alternative, rather than repeatedly
            // transferring an unused, unavailable earlier entry at startup.
            candidates.sort_by_key(|(_, path)| self.existing(path).is_none());
            for (background, path) in candidates {
                match self.asset(path, &prospective, download).await {
                    Ok(_) => {
                        selected = Some((background, video));
                        break;
                    }
                    Err(error) => errors.push(format!("{error:#}")),
                }
            }
            if selected.is_some() {
                break;
            }
        }
        let (background, video) =
            selected.with_context(|| format!("No usable background: {}", errors.join("; ")))?;
        if video && let Err(error) = self.asset(&background.image, &prospective, download).await {
            log::debug!("Static background fallback unavailable: {error:#}");
        }
        if let Some(overlay) = background.overlay.as_deref()
            && let Err(error) = self.asset(overlay, &prospective, download).await
        {
            log::debug!("Optional background overlay unavailable: {error:#}");
        }
        ensure!(
            self.usable(&response, locale, use_video),
            "Required cached assets disappeared during refresh"
        );
        self.write_catalog(game, source, &response)
    }

    async fn refresh_with<F, FF, D, DF>(
        &self,
        locale: &str,
        use_video: bool,
        mut fetch: F,
        mut download: D,
    ) -> Result<CacheRefresh>
    where
        F: FnMut(Game, String) -> FF,
        FF: Future<Output = Result<AedesResponse>>,
        D: FnMut(String, PathBuf, u64) -> DF,
        DF: Future<Output = Result<()>>,
    {
        let locale = settings::normalize_locale(locale)?;
        // ponytail: one refresh per process; a nonblocking permit avoids holding
        // a blocking mutex across network awaits or building a task queue.
        let _permit = self.permit()?;
        let mut report = CacheRefresh::default();
        for game in GAMES {
            let mut errors = Vec::new();
            for source in locales(&locale) {
                let result = match fetch(game, source.to_owned()).await {
                    Ok(response) => {
                        self.prepare(game, source, &locale, use_video, response, &mut download)
                            .await
                    }
                    Err(error) => Err(error),
                };
                match result {
                    Ok(()) => {
                        if errors.is_empty() {
                            report.successes.push(game);
                        }
                        break;
                    }
                    Err(error) => {
                        log::warn!(
                            "Cache refresh failed for {}/{source}: {error:#}",
                            game.code()
                        );
                        errors.push(format!("{source}: {error:#}"));
                    }
                }
            }
            if !errors.is_empty() {
                report.failures.push((game, errors.join("; ")));
            }
        }
        if let Err(error) = self.collect(&HashSet::new()) {
            log::warn!("Could not collect unused cached assets: {error:#}");
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use futures_util::FutureExt;

    fn response() -> AedesResponse {
        AedesResponse {
            backgrounds: vec![AedesBackgroundAssets {
                image: "/images/a.png".into(),
                video: Some("/videos/a.mp4".into()),
                overlay: None,
            }],
            icon: "/icons/a.png".into(),
            icon_cn: "/icons/a-cn.png".into(),
            shortcut: "/shortcuts/a.png".into(),
            shortcut_cn: "/shortcuts/a-cn.png".into(),
        }
    }

    struct Fixture(Store);
    impl Fixture {
        fn new(budget: u64) -> Self {
            Self(
                Store::new(
                    PathBuf::from(format!("cache-fixture-{}", uuid::Uuid::new_v4())),
                    budget,
                    ASSET_TTL,
                )
                .unwrap(),
            )
        }
        fn put(&self, key: &Path, bytes: &[u8]) -> PathBuf {
            let path = self.0.path(key).unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            path
        }
        fn asset(&self, path: &str) -> PathBuf {
            self.put(&cache_path(Path::new(path)).unwrap(), b"asset")
        }
        fn catalog(&self, game: Game, locale: &str, response: &AedesResponse) {
            self.0.write_catalog(game, locale, response).unwrap();
        }
        fn populate(&self, response: &AedesResponse) {
            for path in response.all_paths() {
                self.asset(path);
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0.root);
        }
    }

    fn run(test: impl Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(test);
    }

    async fn download(_url: String, destination: PathBuf, limit: u64) -> Result<()> {
        ensure!(limit >= 5, "fixture body exceeds {limit} byte limit");
        let path = full_path(Some(destination), None)?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, b"asset")?;
        Ok(())
    }

    #[test]
    fn abandoned_downloads_and_catalogs_expire_without_removing_fresh_ones() {
        let fixture = Fixture::new(100);
        let old = fixture.put(Path::new("cache/assets/.download-old"), b"old");
        let fresh = fixture.put(Path::new("cache/assets/.download-fresh"), b"fresh");
        let catalog = fixture.put(
            &PathBuf::from(format!(
                "cache/catalogs/bh3/.catalog-{}",
                uuid::Uuid::new_v4()
            )),
            b"old",
        );
        let published = fixture.put(Path::new("cache/catalogs/bh3/en-us.json"), b"keep");
        let stale = std::time::SystemTime::now() - ASSET_TTL - Duration::from_secs(1);
        for path in [&old, &catalog, &published] {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(stale))
                .unwrap();
        }
        assert_eq!(fixture.0.collect(&HashSet::new()).unwrap(), 5);
        assert!(!old.exists());
        assert!(!catalog.exists());
        assert!(fresh.exists());
        assert_eq!(std::fs::read(published).unwrap(), b"keep");
    }

    #[test]
    fn asset_paths_preserve_order_and_skip_optional_assets() {
        let response = response();
        let paths: Vec<_> = response.asset_paths().flatten().collect();
        assert_eq!(
            paths,
            vec![
                "/images/a.png",
                "/videos/a.mp4",
                "/icons/a.png",
                "/shortcuts/a.png"
            ]
        );
    }

    #[test]
    fn cache_paths_require_safe_absolute_server_paths() {
        assert_eq!(
            cache_path(Path::new("/assets/a.png")).unwrap(),
            PathBuf::from("cache/assets/assets/a.png")
        );
        for path in [
            "assets/a.png",
            "/../secret",
            "/",
            "//host/a",
            "/a//b",
            "/a/./b",
            "/a/../b",
            "/a/",
            "/a?b",
            "/a#b",
            "/%2e%2e/b",
            "/.%2E/b",
            "/a%2Fb",
            "/a%5Cb",
            "/bad%",
            "/bad%2G",
            "/a\\b",
            "/.download-fake",
        ] {
            assert!(cache_path(Path::new(path)).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn encoded_asset_filenames_preserve_the_full_server_key() {
        run(async {
            let fixture = Fixture::new(1024);
            let server_path = "/assets/folder/my%20image.png";
            let key = cache_path(Path::new(server_path)).unwrap();
            assert_eq!(
                key,
                PathBuf::from("cache/assets/assets/folder/my%20image.png")
            );
            let protected = HashSet::from([fixture.0.path(&key).unwrap()]);
            fixture
                .0
                .asset(server_path, &protected, &mut |url: String, dest, limit| {
                    assert_eq!(
                        url,
                        "https://aedes.elysiae.app/assets/folder/my%20image.png"
                    );
                    download(url, dest, limit)
                })
                .await
                .unwrap();
            assert!(fixture.0.existing(server_path).is_some());
        });
    }

    #[test]
    fn metadata_paths_use_game_and_locale() {
        assert_eq!(
            metadata_path(Game::Bh3, "en-us"),
            PathBuf::from("cache/catalogs/bh3/en-us.json")
        );
    }

    #[test]
    fn full_server_paths_do_not_collide() {
        let fixture = Fixture::new(1024);
        let paths = [
            "/assets/one/a.png",
            "/assets/two/a.png",
            "/icons/a.png",
            "/images/a.png",
            "/one/a.png",
        ];
        let files: HashSet<_> = paths.into_iter().map(|path| fixture.asset(path)).collect();
        assert_eq!(files.len(), paths.len());
        assert!(files.iter().all(|file| file.is_file()));
    }

    #[test]
    fn media_uses_one_entry_and_missing_options_are_harmless() {
        let fixture = Fixture::new(1024);
        let mut response = response();
        response.backgrounds[0].overlay = Some("/overlays/wrong.png".into());
        response.backgrounds.push(AedesBackgroundAssets {
            image: "/images/b.png".into(),
            video: None,
            overlay: Some("/overlays/right.png".into()),
        });
        fixture.asset("/overlays/wrong.png");
        let image = fixture.asset("/images/b.png");
        fixture.catalog(Game::Bh3, "en-us", &response);
        let resolved = fixture.0.resolve(Game::Bh3, "en-us", true);
        assert_eq!(resolved.background, Some(image));
        assert!(!resolved.is_video);
        assert!(resolved.overlay.is_none());
        let overlay = fixture.asset("/overlays/right.png");
        assert_eq!(
            fixture.0.resolve(Game::Bh3, "en-us", false).overlay,
            Some(overlay)
        );
        assert!(
            fixture
                .0
                .cached_paths(Game::Bh3, "en-us", AssetType::Video)
                .is_empty()
        );
        let video = fixture.asset("/videos/a.mp4");
        let resolved = fixture.0.resolve(Game::Bh3, "en-us", true);
        assert_eq!(resolved.background, Some(video));
        assert!(resolved.is_video);
        assert_eq!(resolved.overlay, fixture.0.existing("/overlays/wrong.png"));
    }

    #[test]
    fn corrupt_missing_and_unusable_catalogs_fall_back_to_english() {
        let fixture = Fixture::new(1024);
        assert!(!fixture.0.ready(Game::Bh3, "fr-fr", false));
        assert!(
            fixture
                .0
                .resolve(Game::Bh3, "fr-fr", false)
                .background
                .is_none()
        );
        let response = response();
        fixture.catalog(Game::Bh3, "en-us", &response);
        assert!(!fixture.0.ready(Game::Bh3, "en-us", false));
        fixture.populate(&response);
        fixture.put(&metadata_path(Game::Bh3, "fr-fr"), b"broken JSON");
        assert!(fixture.0.ready(Game::Bh3, "fr-fr", false));
        assert!(fixture.0.resolve(Game::Bh3, "fr-fr", true).is_video);
        let mut unusable = response.clone();
        unusable.backgrounds.clear();
        fixture.catalog(Game::Bh3, "fr-fr", &unusable);
        assert!(
            fixture
                .0
                .resolve(Game::Bh3, "fr-fr", false)
                .background
                .is_some()
        );
        assert!(!fixture.0.ready(Game::Hk4e, "fr-fr", false));
    }

    #[test]
    fn locale_variants_use_cn_artwork_without_changing_server_region() {
        let fixture = Fixture::new(1024);
        let response = response();
        fixture.catalog(Game::Bh3, "en-us", &response);
        fixture.populate(&response);
        assert_eq!(
            fixture.0.cached_paths(Game::Bh3, "zh-cn", AssetType::Icon)[0],
            fixture.0.existing(&response.icon_cn).unwrap()
        );
        assert_eq!(
            fixture.0.cached_paths(Game::Bh3, "zh-tw", AssetType::Icon)[0],
            fixture.0.existing(&response.icon).unwrap()
        );
        std::fs::remove_file(fixture.0.existing(&response.shortcut_cn).unwrap()).unwrap();
        assert_eq!(
            fixture
                .0
                .cached_paths(Game::Bh3, "zh-cn", AssetType::Shortcut),
            vec![fixture.0.existing(&response.shortcut).unwrap()]
        );
    }

    #[test]
    fn warm_refresh_skips_bodies_and_static_preference_skips_videos() {
        run(async {
            let fixture = Fixture::new(1024);
            let mut transferred = Vec::new();
            let report = fixture
                .0
                .refresh_with(
                    "EN_US",
                    false,
                    |_, _| std::future::ready(Ok(response())),
                    |url: String, path, limit| {
                        transferred.push(url.clone());
                        download(url, path, limit)
                    },
                )
                .await
                .unwrap();
            assert!(report.failures.is_empty());
            assert_eq!(report.successes.len(), 6);
            assert_eq!(transferred.len(), 3);
            assert!(transferred.iter().all(|url| !url.contains("/videos/")));
            let report = fixture
                .0
                .refresh_with(
                    "en-us",
                    false,
                    |_, _| std::future::ready(Ok(response())),
                    |_, _, _| async {
                        panic!("warm refresh transferred a body");
                        #[allow(unreachable_code)]
                        Ok(())
                    },
                )
                .await
                .unwrap();
            assert!(report.failures.is_empty());
            let report = fixture
                .0
                .refresh_with(
                    "en-us",
                    true,
                    |_, _| std::future::ready(Ok(response())),
                    |url: String, path, limit| {
                        transferred.push(url.clone());
                        download(url, path, limit)
                    },
                )
                .await
                .unwrap();
            assert!(report.failures.is_empty());
            assert_eq!(transferred.len(), 4);
            assert!(fixture.0.resolve(Game::Bh3, "en-us", true).is_video);
        });
    }

    #[test]
    fn per_game_errors_continue_and_locale_fallback_remains_usable() {
        run(async {
            let fixture = Fixture::new(1024);
            let report = fixture
                .0
                .refresh_with(
                    "fr-fr",
                    false,
                    |game, locale| {
                        std::future::ready(if game == Game::Bh3 || locale == "fr-fr" {
                            bail_result("fixture metadata unavailable")
                        } else {
                            Ok(response())
                        })
                    },
                    download,
                )
                .await
                .unwrap();
            assert!(report.successes.is_empty());
            assert_eq!(report.failures.len(), 6);
            assert!(!fixture.0.ready(Game::Bh3, "fr-fr", false));
            assert!(fixture.0.ready(Game::Hk4e, "fr-fr", false));
            assert!(
                !fixture
                    .0
                    .path(&metadata_path(Game::Hk4e, "fr-fr"))
                    .unwrap()
                    .exists()
            );
            let report = fixture
                .0
                .refresh_with(
                    "en-us",
                    false,
                    |game, _| {
                        std::future::ready(if game == Game::Bh3 {
                            bail_result("offline")
                        } else {
                            Ok(response())
                        })
                    },
                    download,
                )
                .await
                .unwrap();
            assert_eq!(report.successes.len(), 5);
            assert_eq!(report.failures.len(), 1);
            assert_eq!(report.failures[0].0, Game::Bh3);
        });
    }

    fn bail_result(message: &str) -> Result<AedesResponse> {
        bail!("{message}")
    }

    #[test]
    fn failed_required_download_keeps_last_catalog_and_assets() {
        run(async {
            let fixture = Fixture::new(1024);
            let previous = response();
            fixture.populate(&previous);
            fixture.catalog(Game::Bh3, "en-us", &previous);
            let before =
                std::fs::read(fixture.0.path(&metadata_path(Game::Bh3, "en-us")).unwrap()).unwrap();
            let mut next = previous.clone();
            next.backgrounds[0].image = "/new/missing.png".into();
            let report = fixture
                .0
                .refresh_with(
                    "en-us",
                    false,
                    |_, _| std::future::ready(Ok(next.clone())),
                    |_, _, _| async { bail!("fixture 404") },
                )
                .await
                .unwrap();
            assert_eq!(report.failures.len(), 6);
            assert_eq!(
                std::fs::read(fixture.0.path(&metadata_path(Game::Bh3, "en-us")).unwrap()).unwrap(),
                before
            );
            assert!(fixture.0.existing(&previous.backgrounds[0].image).is_some());
        });
    }

    #[test]
    fn optional_failures_and_unused_alternatives_do_not_block_commit() {
        run(async {
            let fixture = Fixture::new(1024);
            let mut response = response();
            response.backgrounds[0].overlay = Some("/optional/overlay.png".into());
            response.backgrounds.push(AedesBackgroundAssets {
                image: "/unused/alternative.png".into(),
                video: Some("/unused/video.mp4".into()),
                overlay: None,
            });
            let report = fixture
                .0
                .refresh_with(
                    "en-us",
                    true,
                    |_, _| std::future::ready(Ok(response.clone())),
                    |url: String, path, limit| async move {
                        if url.contains("/videos/")
                            || url.contains("/optional/")
                            || url.contains("/unused/")
                        {
                            bail!("optional unavailable");
                        }
                        download(url, path, limit).await
                    },
                )
                .await
                .unwrap();
            assert!(report.failures.is_empty());
            let media = fixture.0.resolve(Game::Bh3, "en-us", true);
            assert!(media.background.is_some());
            assert!(!media.is_video);
            assert!(media.overlay.is_none());
        });
    }

    #[test]
    fn budget_limits_downloads_and_protects_active_and_referenced_assets() {
        run(async {
            let fixture = Fixture::new(20);
            let active = fixture.asset("/active.png");
            fixture.0.set_active(std::slice::from_ref(&active)).unwrap();
            let orphan = fixture.asset("/unused.png");
            let catalog = response();
            fixture.catalog(Game::Bh3, "en-us", &catalog);
            let referenced = fixture.asset(&catalog.icon);
            let temporary = fixture.put(Path::new("cache/assets/.download-in-flight"), b"busy!");
            let unrelated = fixture.put(Path::new("cache/unrelated/keep"), b"keep");
            assert_eq!(fixture.0.collect(&HashSet::new()).unwrap(), 15);
            assert!(!orphan.exists());
            assert!(
                active.exists() && referenced.exists() && temporary.exists() && unrelated.exists()
            );
            let candidate = "/new.png";
            let protected = HashSet::from([fixture
                .0
                .path(&cache_path(Path::new(candidate)).unwrap())
                .unwrap()]);
            fixture
                .0
                .asset(candidate, &protected, &mut |url, path, limit| {
                    assert_eq!(limit, 5);
                    download(url, path, limit)
                })
                .await
                .unwrap();
            assert_eq!(fixture.0.collect(&protected).unwrap(), 20);
            assert!(
                fixture
                    .0
                    .asset("/too-big.png", &protected, &mut download)
                    .await
                    .is_err()
            );
            assert!(
                fixture
                    .0
                    .path(&metadata_path(Game::Bh3, "en-us"))
                    .unwrap()
                    .exists()
            );
            fixture.0.set_active(&[]).unwrap();
            fixture.0.collect(&protected).unwrap();
            assert!(!active.exists());
        });
    }

    #[test]
    fn oversized_working_set_fails_soft_without_publishing_catalogs() {
        run(async {
            let fixture = Fixture::new(14);
            let report = fixture
                .0
                .refresh_with(
                    "en-us",
                    false,
                    |_, _| std::future::ready(Ok(response())),
                    download,
                )
                .await
                .unwrap();
            assert_eq!(report.failures.len(), 6);
            assert!(report.successes.is_empty());
            assert!(!fixture.0.ready(Game::Bh3, "en-us", false));
            assert!(fixture.0.collect(&HashSet::new()).unwrap() <= 14);
        });
    }

    #[test]
    fn expired_assets_revalidate_but_active_files_and_failed_rechecks_survive() {
        run(async {
            let mut fixture = Fixture::new(1024);
            fixture.0.ttl = Duration::ZERO;
            let path = fixture.asset("/old.png");
            let protected = HashSet::from([path.clone()]);
            fixture
                .0
                .asset("/old.png", &protected, &mut |_, _, _| async {
                    bail!("offline")
                })
                .await
                .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"asset");
            fixture
                .0
                .asset("/old.png", &protected, &mut |url, dest, limit| async move {
                    download(url, dest.clone(), limit).await?;
                    std::fs::write(full_path(Some(dest), None)?, b"new")?;
                    Ok(())
                })
                .await
                .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"new");
            fixture.0.set_active(std::slice::from_ref(&path)).unwrap();
            fixture
                .0
                .asset("/old.png", &protected, &mut |_, _, _| async {
                    panic!("active asset rechecked");
                    #[allow(unreachable_code)]
                    Ok(())
                })
                .await
                .unwrap();
            assert_eq!(std::fs::read(path).unwrap(), b"new");
        });
    }

    #[test]
    fn pins_added_during_download_prevent_replacement() {
        run(async {
            let mut fixture = Fixture::new(1024);
            fixture.0.ttl = Duration::ZERO;
            let path = fixture.asset("/old.png");
            let protected = HashSet::from([path.clone()]);
            fixture
                .0
                .asset("/old.png", &protected, &mut |url, dest, limit| {
                    fixture.0.set_active(std::slice::from_ref(&path)).unwrap();
                    async move {
                        download(url, dest.clone(), limit).await?;
                        std::fs::write(full_path(Some(dest), None)?, b"new")?;
                        Ok(())
                    }
                })
                .await
                .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"asset");
            assert_eq!(fixture.0.collect(&protected).unwrap(), 5);
        });
    }

    #[test]
    fn collector_never_follows_symlinks_or_accepts_unowned_pins() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new(1024);
        let outside = fixture.put(Path::new("cache/unrelated/keep"), b"keep");
        let assets = fixture.0.path(Path::new("cache/assets")).unwrap();
        std::fs::create_dir_all(&assets).unwrap();
        symlink(outside.parent().unwrap(), assets.join("linked-directory")).unwrap();
        symlink(&outside, assets.join("linked-file")).unwrap();
        assert!(fixture.0.existing("/linked-file").is_none());
        assert!(
            fixture
                .0
                .set_active(std::slice::from_ref(&outside))
                .is_err()
        );
        assert!(fixture.0.set_active(&[assets.join("linked-file")]).is_err());
        assert_eq!(fixture.0.collect(&HashSet::new()).unwrap(), 0);
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
        assert!(assets.join("linked-directory").is_symlink());
        assert!(assets.join("linked-file").is_symlink());
    }

    #[test]
    fn replacing_pins_releases_old_files_and_preserves_new_selection() {
        let fixture = Fixture::new(1024);
        let previous = fixture.asset("/previous/image.png");
        let latest = fixture.asset("/latest/image.png");
        fixture
            .0
            .set_active(std::slice::from_ref(&previous))
            .unwrap();
        fixture.0.set_active(std::slice::from_ref(&latest)).unwrap();
        assert_eq!(fixture.0.collect(&HashSet::new()).unwrap(), 5);
        assert!(!previous.exists());
        assert!(!previous.parent().unwrap().exists());
        assert!(latest.exists());
        assert!(fixture.0.set_active(&[previous]).is_err());
        assert_eq!(fixture.0.collect(&HashSet::new()).unwrap(), 5);
        assert!(
            latest.exists(),
            "invalid replacement must not discard existing pins"
        );
    }

    #[test]
    fn corrupt_catalogs_with_no_backgrounds_never_report_success() {
        run(async {
            let fixture = Fixture::new(1024);
            for game in GAMES {
                fixture.put(&metadata_path(game, "en-us"), b"corrupt");
            }
            let mut empty = response();
            empty.backgrounds.clear();
            let report = fixture
                .0
                .refresh_with(
                    "en-us",
                    false,
                    |_, _| std::future::ready(Ok(empty.clone())),
                    download,
                )
                .await
                .unwrap();
            assert!(report.successes.is_empty());
            assert_eq!(report.failures.len(), GAMES.len());
            assert!(
                GAMES
                    .into_iter()
                    .all(|game| !fixture.0.ready(game, "en-us", false))
            );
            assert!(
                fixture
                    .0
                    .resolve(Game::Bh3, "en-us", false)
                    .background
                    .is_none()
            );
        });
    }

    #[test]
    fn cancelling_refresh_releases_permit_and_staging_files() {
        let fixture = Fixture::new(1024);
        let refresh = fixture.0.refresh_with(
            "en-us",
            false,
            |_, _| std::future::ready(Ok(response())),
            |url, path, limit| async move {
                download(url, path, limit).await?;
                std::future::pending::<Result<()>>().await
            },
        );
        assert!(refresh.now_or_never().is_none());
        assert!(fixture.0.permit().is_ok());
        assert_eq!(fixture.0.collect(&HashSet::new()).unwrap(), 0);
    }

    #[test]
    fn localized_refresh_downloads_cn_variants_into_distinct_catalogs() {
        run(async {
            let fixture = Fixture::new(1024);
            let mut urls = Vec::new();
            let report = fixture
                .0
                .refresh_with(
                    "zh-cn",
                    false,
                    |_, locale| {
                        assert_eq!(locale, "zh-cn");
                        std::future::ready(Ok(response()))
                    },
                    |url: String, dest, limit| {
                        urls.push(url.clone());
                        download(url, dest, limit)
                    },
                )
                .await
                .unwrap();
            assert!(report.failures.is_empty());
            assert!(urls.iter().any(|url| url.ends_with("/icons/a-cn.png")));
            assert!(urls.iter().any(|url| url.ends_with("/shortcuts/a-cn.png")));
            assert!(urls.iter().all(|url| !url.ends_with("/icons/a.png") && !url.ends_with("/shortcuts/a.png")));
            let report = fixture
                .0
                .refresh_with(
                    "en-us",
                    false,
                    |_, _| std::future::ready(Ok(response())),
                    download,
                )
                .await
                .unwrap();
            assert!(report.failures.is_empty());
            for game in GAMES {
                assert!(fixture.0.ready(game, "zh-cn", false));
                assert!(fixture.0.ready(game, "en-us", false));
                assert!(fixture.0.read_catalog(game, "zh-cn").is_ok());
                assert!(fixture.0.read_catalog(game, "en-us").is_ok());
            }
        });
    }

    #[test]
    fn empty_or_oversized_bodies_are_not_published_and_staging_is_cleaned() {
        run(async {
            for bytes in [b"".as_slice(), b"too large".as_slice()] {
                let fixture = Fixture::new(5);
                let path = fixture
                    .0
                    .path(&cache_path(Path::new("/bad.png")).unwrap())
                    .unwrap();
                let protected = HashSet::from([path.clone()]);
                assert!(
                    fixture
                        .0
                        .asset("/bad.png", &protected, &mut |_, dest, limit| async move {
                            assert_eq!(limit, 5);
                            let dest = full_path(Some(dest), None)?;
                            std::fs::create_dir_all(dest.parent().unwrap())?;
                            std::fs::write(dest, bytes)?;
                            Ok(())
                        })
                        .await
                        .is_err()
                );
                assert!(!path.exists());
                assert_eq!(fixture.0.collect(&HashSet::new()).unwrap(), 0);
            }
        });
    }

    #[test]
    fn symlinked_asset_root_cannot_turn_collection_into_unrelated_deletion() {
        let fixture = Fixture::new(1024);
        let outside = fixture.put(Path::new("cache/unrelated/keep"), b"keep");
        std::os::unix::fs::symlink(outside.parent().unwrap(), fixture.0.root.join("assets"))
            .unwrap();
        assert!(fixture.0.collect(&HashSet::new()).is_err());
        assert!(fixture.0.existing("/keep").is_none());
        assert_eq!(std::fs::read(outside).unwrap(), b"keep");
    }

    #[test]
    fn refresh_permit_rejects_overlap_and_releases_on_drop() {
        let fixture = Fixture::new(1024);
        let permit = fixture.0.permit().unwrap();
        assert!(fixture.0.permit().is_err());
        drop(permit);
        assert!(fixture.0.permit().is_ok());
    }
}
