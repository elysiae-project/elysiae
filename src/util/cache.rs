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

/// Kind of cached artwork to retrieve.
#[derive(Debug, Clone, Copy)]
pub enum AssetType {
    Image,
    Video,
    Icon,
    Shortcut,
    Overlay,
}

/// Asset catalog returned by Aedes for a game and locale.
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

/// Background image with its optional video and matching overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AedesBackgroundAssets {
    image: String,
    video: Option<String>,
    overlay: Option<String>,
}

/// Available background and matching overlay selected from the cache.
#[derive(Debug, Default)]
pub struct ResolvedMedia {
    pub background: Option<PathBuf>,
    pub overlay: Option<PathBuf>,
    pub is_video: bool,
}

/// Per-game results of refreshing the requested locale's assets.
#[derive(Debug, Default)]
pub struct CacheRefresh {
    /// Games whose requested-locale catalog was refreshed successfully.
    pub successes: Vec<Game>,
    /// Requested-locale failures, even when English fallback is still usable.
    pub failures: Vec<(Game, String)>,
}

/// Bounded asset cache with catalogs, active-media pins and a refresh guard.
struct Store {
    relative: PathBuf,
    root: PathBuf,
    budget: u64,
    ttl: Duration,
    active: Mutex<HashSet<PathBuf>>,
    refreshing: AtomicBool,
}

/// Holds exclusive access to a cache refresh until dropped.
struct RefreshPermit<'a>(&'a AtomicBool);
impl Drop for RefreshPermit<'_> {
    /// Release the refresh permit.
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Removes an unpublished staging file when dropped.
struct TemporaryFile(PathBuf);
impl Drop for TemporaryFile {
    /// Remove any staging file left at the guarded path.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Initialize and return the shared asset cache.
fn store() -> Result<&'static Store> {
    static STORE: OnceLock<Store> = OnceLock::new();
    if STORE.get().is_none() {
        let _ = STORE.set(Store::new(PathBuf::from("cache"), ASSET_BUDGET, ASSET_TTL)?);
    }
    STORE.get().context("Asset cache is unavailable")
}

/// Refresh all games for the requested locale with English fallback.
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

/// Refresh the display locale's assets and report any per-game failures.
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

/// Check whether a game has usable cached assets, allowing English fallback.
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

/// Select a cached background and matching overlay for the requested locale.
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

/// Return the selected background and overlay paths.
pub fn media_paths(
    game: Game,
    locale: &str,
    use_video: bool,
) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    let media = resolve_media(game, locale, use_video)?;
    Ok((media.background, media.overlay))
}

/// List available assets of a kind, falling back to the English catalog.
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
    /// Iterate background assets followed by the global icon and shortcut.
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

    /// Iterate all listed asset paths, including localized artwork.
    fn all_paths(&self) -> impl Iterator<Item = &str> {
        self.asset_paths()
            .flatten()
            .chain([self.icon_cn.as_str(), self.shortcut_cn.as_str()])
    }

    /// Order icon or shortcut candidates by locale preference.
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

/// Build the cache key for a game's localized catalog.
fn metadata_path(game: Game, locale: &str) -> PathBuf {
    PathBuf::from(format!("cache/catalogs/{}/{}.json", game.code(), locale))
}

/// Validate a server asset path and map its full key into the cache.
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

/// Try the requested locale before English, without duplicates.
fn locales(locale: &str) -> impl Iterator<Item = &str> {
    std::iter::once(locale).chain((locale != "en-us").then_some("en-us"))
}

impl Store {
    /// Create a cache store with a validated root, byte budget and asset lifetime.
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

    /// Resolve a server path to an existing, nonempty regular cache file.
    fn existing(&self, server_path: &str) -> Option<PathBuf> {
        let path = self.path(&cache_path(Path::new(server_path)).ok()?).ok()?;
        std::fs::symlink_metadata(&path)
            .ok()
            .filter(|m| m.is_file() && m.len() > 0)
            .map(|_| path)
    }

    /// Read a cached catalog while checking its file type and size.
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

    /// Read a catalog, treating missing or invalid metadata as unavailable.
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

    /// Select an available background and its own overlay, preferring video if enabled.
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

    /// Resolve cached media from the requested locale or English fallback.
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

    /// Resolve existing assets of a kind from a catalog in preference order.
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

    /// Find assets of a kind in the requested locale or English fallback.
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

    /// Check that a catalog provides a cached background, icon and shortcut.
    fn usable(&self, response: &AedesResponse, locale: &str, use_video: bool) -> bool {
        self.selected(response, use_video).background.is_some()
            && !self.paths(response, locale, AssetType::Icon).is_empty()
            && !self.paths(response, locale, AssetType::Shortcut).is_empty()
    }

    /// Check for a usable requested-locale or English catalog.
    fn ready(&self, game: Game, locale: &str, use_video: bool) -> bool {
        locales(locale).any(|source| {
            self.catalog(game, source)
                .is_some_and(|response| self.usable(&response, locale, use_video))
        })
    }

    /// Replace active-media pins after validating every supplied cache file.
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

    /// Acquire exclusive refresh access without waiting for another refresh.
    fn permit(&self) -> Result<RefreshPermit<'_>> {
        ensure!(
            self.refreshing
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok(),
            "An asset cache refresh is already running"
        );
        Ok(RefreshPermit(&self.refreshing))
    }

    /// Collect asset paths referenced by supported game and locale catalogs.
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

    /// Remove expired catalog staging files without touching published catalogs.
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

    /// Reuse or refresh an asset within budget, retaining usable stale files on failure.
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

    /// Publish a size-bounded catalog through a synced staging file.
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

    /// Cache required artwork and selected media before publishing a catalog.
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

    /// Refresh game catalogs with fallback using the supplied network operations.
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

    /// Rejects escaping asset keys and keeps game/locale catalogs distinct.
    #[test]
    fn cache_keys_stay_contained_and_distinct() {
        assert_eq!(
            cache_path(Path::new("/images/a.png")).unwrap(),
            PathBuf::from("cache/assets/images/a.png")
        );
        for path in [
            "relative.png",
            "/../outside",
            "/%2e%2e/outside",
            "//host/file",
        ] {
            assert!(cache_path(Path::new(path)).is_err());
        }
        assert_ne!(
            metadata_path(Game::Bh3, "en-us"),
            metadata_path(Game::Hk4e, "en-us")
        );
        assert_ne!(
            metadata_path(Game::Bh3, "en-us"),
            metadata_path(Game::Bh3, "fr-fr")
        );
    }
}
