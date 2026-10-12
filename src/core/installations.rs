//! Installation locations and write consent, not game-version metadata.
//! Irmin's version marker remains authoritative, including for imported games.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{fs, game::Game, operations::Operation};

const REGISTRY: &str = "installations.json";
const OWNER_MARKER: &str = ".elysiae-owner.json";
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Installation {
    pub(crate) path: PathBuf,
    pub(crate) managed: bool,
    pub(crate) allow_modify: bool,
    // Imported records have an identity, but no ownership marker or deletion rights.
    pub(crate) ownership_token: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema_version: u32,
    #[serde(deserialize_with = "unique_installations")]
    installations: BTreeMap<String, Installation>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnerMarker {
    game: String,
    token: String,
}

/// Return a validated canonical registered root, or the default absolute path
/// for an unregistered game. A missing registered root is allowed so status can
/// report a deleted/incomplete installation. This does not grant write consent.
pub fn path(game: Game) -> Result<PathBuf> {
    Store(PathBuf::new()).path(game)
}

/// Create/register an empty default directory, or resume a verified managed
/// installation. Existing nonempty unregistered directories require import.
pub fn prepare_install(game: Game, operation: &Operation) -> Result<PathBuf> {
    Store(PathBuf::new()).prepare_install(game, operation)
}

/// Require a matching operation and either verified managed ownership or
/// explicit import write consent. An unregistered default path is never writable.
pub fn writable_path(game: Game, operation: &Operation) -> Result<PathBuf> {
    Store(PathBuf::new()).writable_path(game, operation)
}

/// Register an existing game directory without changing any files inside it.
/// The known executable must be present; an Irmin version marker need not be.
/// Re-importing the same imported root can change write consent. Changing a
/// registered root or converting a managed installation requires maintenance.
pub fn import(game: Game, root: &Path, allow_modify: bool, operation: &Operation) -> Result<()> {
    Store(PathBuf::new()).import(game, root, allow_modify, operation)
}

/// Read a record with validated schema and canonical path (possibly missing).
/// This does not verify a managed ownership marker; callers must hold an
/// Operation and use writable_path before modifying an existing installation.
pub(crate) fn registered(game: Game) -> Result<Option<Installation>> {
    Store(PathBuf::new()).registered(game)
}

// Relative to AppData. Tests use a unique subdirectory, never a shared registry.
struct Store(PathBuf);

impl Store {
    fn root(&self) -> Result<PathBuf> {
        if self.0.as_os_str().is_empty() {
            fs::full_path(None, None)
        } else {
            fs::full_path(Some(self.0.clone()), None)
        }
    }

    fn resolve(&self, relative: &Path) -> Result<PathBuf> {
        fs::full_path(Some(self.0.join(relative)), None)
    }

    fn load(&self) -> Result<Registry> {
        let file = self.resolve(Path::new(REGISTRY))?;
        match std::fs::symlink_metadata(&file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Registry {
                    schema_version: SCHEMA_VERSION,
                    installations: BTreeMap::new(),
                });
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) => ensure!(
                metadata.is_file(),
                "Installation registry must be a regular file"
            ),
        }
        let registry: Registry = serde_json::from_slice(&std::fs::read(file)?)
            .context("Invalid installation registry; existing data was not changed")?;
        ensure!(
            registry.schema_version == SCHEMA_VERSION,
            "Unsupported installation registry schema: {}",
            registry.schema_version
        );
        for (code, installation) in &registry.installations {
            let game = Game::try_from(code.as_str())?;
            Uuid::parse_str(&installation.ownership_token).context("Invalid installation token")?;
            ensure!(
                !installation.managed || installation.allow_modify,
                "Managed installation has inconsistent write consent"
            );
            self.validate_root(&installation.path)?;
            check_conflicts(&registry, game, &installation.path)?;
        }
        Ok(registry)
    }

    fn save(&self, registry: &Registry) -> Result<()> {
        fs::write_file(
            self.0.join(REGISTRY),
            &serde_json::to_vec_pretty(registry)?,
            None,
        )
        .context("Could not save installation registry")
    }

    fn registered(&self, game: Game) -> Result<Option<Installation>> {
        Ok(self.load()?.installations.remove(game.code()))
    }

    fn path(&self, game: Game) -> Result<PathBuf> {
        match self.registered(game)? {
            Some(installation) => Ok(installation.path),
            None => self.resolve(&game.install_path()),
        }
    }

    fn writable_path(&self, game: Game, operation: &Operation) -> Result<PathBuf> {
        operation.check_game(game)?;
        operation.check_cancelled()?;
        let installation = self
            .registered(game)?
            .context("Installation is not registered; prepare or import it first")?;
        ensure!(
            installation.path.is_dir(),
            "Installation directory is missing"
        );
        if installation.managed {
            verify_owner(game, &installation)?;
        } else {
            ensure!(
                installation.allow_modify,
                "Imported installation does not have permission to modify files"
            );
        }
        Ok(installation.path)
    }

    fn prepare_install(&self, game: Game, operation: &Operation) -> Result<PathBuf> {
        operation.check_game(game)?;
        operation.check_cancelled()?;
        let mut registry = self.load()?;
        if let Some(installation) = registry.installations.get(game.code()) {
            ensure!(
                installation.managed,
                "Imported installations must use explicit write consent"
            );
            if installation.path.try_exists()? {
                verify_owner(game, installation)?;
            } else {
                std::fs::create_dir_all(&installation.path)?;
                let result = self
                    .validate_root(&installation.path)
                    .and_then(|()| operation.check_cancelled())
                    .and_then(|()| write_owner(game, installation));
                if let Err(error) = result {
                    // Only remove the empty root just created here, never its parents.
                    std::fs::remove_dir(&installation.path).with_context(|| {
                        format!("{error:#}; could not remove recreated empty directory")
                    })?;
                    return Err(error);
                }
            }
            return Ok(installation.path.clone());
        }

        let destination = self.resolve(&game.install_path())?;
        let expected = canonicalize_with_missing(&self.root()?)?.join(game.install_path());
        // An XDG base may be a symlink, but a game directory or its games/
        // ancestor must not silently redirect a newly managed installation.
        ensure!(
            canonicalize_with_missing(&destination)? == expected,
            "Default installation path contains a redirected directory"
        );
        self.check_safe_root(&expected)?;
        check_conflicts(&registry, game, &expected)?;
        std::fs::create_dir_all(&destination).context("Could not create installation directory")?;
        self.validate_root(&expected)?;
        ensure!(
            std::fs::read_dir(&expected)?.next().transpose()?.is_none(),
            "Default installation directory is not empty; import it explicitly"
        );
        let installation = Installation {
            path: expected.clone(),
            managed: true,
            allow_modify: true,
            ownership_token: Uuid::new_v4().to_string(),
        };
        registry
            .installations
            .insert(game.code().to_owned(), installation.clone());
        self.commit_managed(game, &installation, &registry, operation)?;
        Ok(expected)
    }

    fn commit_managed(
        &self,
        game: Game,
        installation: &Installation,
        registry: &Registry,
        operation: &Operation,
    ) -> Result<()> {
        operation.check_cancelled()?;
        write_owner(game, installation)?;
        let result = operation
            .check_cancelled()
            .and_then(|()| self.save(registry));
        if let Err(error) = result {
            // Roll back only our marker, never a directory or arbitrary content.
            // A crash can leave an orphan marker; it is not sufficient to adopt
            // an unregistered nonempty directory on the next attempt.
            verify_owner(game, installation)
                .and_then(|()| Ok(std::fs::remove_file(installation.path.join(OWNER_MARKER))?))
                .with_context(|| {
                    format!("{error:#}; could not remove unregistered ownership marker")
                })?;
            return Err(error);
        }
        Ok(())
    }

    fn import(
        &self,
        game: Game,
        root: &Path,
        allow_modify: bool,
        operation: &Operation,
    ) -> Result<()> {
        operation.check_game(game)?;
        operation.check_cancelled()?;
        let mut registry = self.load()?;
        let root = root
            .canonicalize()
            .context("Import directory does not exist")?;
        self.validate_root(&root)?;
        let executable = root.join(game.executable()?);
        ensure!(
            executable.is_file(),
            "Import directory is missing the known game executable"
        );
        ensure!(
            executable.canonicalize()?.starts_with(&root),
            "Game executable points outside the import directory"
        );
        check_conflicts(&registry, game, &root)?;
        let token = if let Some(existing) = registry.installations.get(game.code()) {
            ensure!(
                !existing.managed,
                "Cannot replace managed ownership through import"
            );
            ensure!(
                existing.path == root,
                "Game is already registered at another location"
            );
            existing.ownership_token.clone()
        } else {
            Uuid::new_v4().to_string()
        };
        registry.installations.insert(
            game.code().to_owned(),
            Installation {
                path: root,
                managed: false,
                allow_modify,
                ownership_token: token,
            },
        );
        operation.check_cancelled()?;
        self.save(&registry)
    }

    fn validate_root(&self, root: &Path) -> Result<()> {
        ensure!(
            root.is_absolute(),
            "Registered installation path must be absolute"
        );
        ensure!(
            canonicalize_with_missing(root)? == root,
            "Installation root is no longer canonical: {}",
            root.display()
        );
        match std::fs::symlink_metadata(root) {
            Ok(metadata) => ensure!(
                metadata.is_dir(),
                "Installation root must be a real directory"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.check_safe_root(root)
    }

    fn check_safe_root(&self, root: &Path) -> Result<()> {
        let home = canonicalize_with_missing(&fs::full_path(None, Some(fs::BaseDirectory::Home))?)?;
        let data = canonicalize_with_missing(&self.root()?)?;
        ensure!(
            !home.starts_with(root) && !data.starts_with(root),
            "Installation root must not contain home or application data"
        );
        for directory in ["proton-data", "proton", "phlogiston", "components", "cache"] {
            let shared = canonicalize_with_missing(&data.join(directory))?;
            ensure!(
                !root.starts_with(&shared) && !shared.starts_with(root),
                "Installation root overlaps shared runtime data"
            );
        }
        Ok(())
    }
}

// Managed roots can eventually be relocated outside AppData. Use native I/O
// on the validated absolute root; fs wrappers accept only base-relative paths.
fn write_owner(game: Game, installation: &Installation) -> Result<()> {
    use std::io::Write;

    ensure!(
        std::fs::read_dir(&installation.path)?
            .next()
            .transpose()?
            .is_none(),
        "Ownership markers can only be created in an empty directory"
    );
    let marker = OwnerMarker {
        game: game.code().to_owned(),
        token: installation.ownership_token.clone(),
    };
    let temporary = installation
        .path
        .join(format!(".elysiae-owner-{}.tmp", Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(&serde_json::to_vec(&marker)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, installation.path.join(OWNER_MARKER))?;
        Ok(())
    })();
    if result.is_err() {
        std::fs::remove_file(&temporary)
            .context("Could not clean up temporary ownership marker")?;
    }
    result
}

fn verify_owner(game: Game, installation: &Installation) -> Result<()> {
    let marker = installation.path.join(OWNER_MARKER);
    ensure!(
        std::fs::symlink_metadata(&marker)?.is_file(),
        "Ownership marker must be a regular file"
    );
    let marker: OwnerMarker = serde_json::from_slice(&std::fs::read(marker)?)
        .context("Invalid installation ownership marker")?;
    ensure!(
        marker.game == game.code() && marker.token == installation.ownership_token,
        "Installation ownership marker does not match its registration"
    );
    Ok(())
}

fn check_conflicts(registry: &Registry, game: Game, root: &Path) -> Result<()> {
    for (code, installation) in &registry.installations {
        ensure!(
            code == game.code()
                || (!root.starts_with(&installation.path) && !installation.path.starts_with(root)),
            "Installation root overlaps the registered game {code}"
        );
    }
    Ok(())
}

// Resolve configured bases even before AppData or shared runtime directories
// exist. A dangling symlink is an error, not a missing directory to recreate.
fn canonicalize_with_missing(path: &Path) -> Result<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(path.canonicalize()?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().context("Path has no existing ancestor")?;
            Ok(canonicalize_with_missing(parent)?
                .join(path.file_name().context("Path has no name")?))
        }
        Err(error) => Err(error.into()),
    }
}

// serde's map default accepts duplicate keys and silently discards an entry.
fn unique_installations<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, Installation>, D::Error> {
    struct Entries;
    impl<'de> serde::de::Visitor<'de> for Entries {
        type Value = BTreeMap<String, Installation>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map of unique game codes to installations")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut entries = BTreeMap::new();
            while let Some((code, installation)) = map.next_entry::<String, Installation>()? {
                if entries.insert(code, installation).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate game code in installation registry",
                    ));
                }
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_map(Entries)
}

#[cfg(test)]
mod tests {
    use super::super::operations::{OperationKind, Operations};
    use super::*;
    use std::os::unix::fs::symlink;

    struct Fixture {
        directory: PathBuf,
        store: Store,
    }

    impl Fixture {
        fn new() -> Self {
            let relative = PathBuf::from(format!("installation-tests-{}", Uuid::new_v4()));
            let directory = fs::full_path(Some(relative.clone()), None).unwrap();
            assert!(directory.starts_with(std::env::temp_dir()));
            let store = Store(relative.join("appdata"));
            std::fs::create_dir_all(store.root().unwrap()).unwrap();
            Self { directory, store }
        }

        fn game(&self, name: &str, game: Game) -> PathBuf {
            let root = self.directory.join(name);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join(game.executable().unwrap()),
                b"fixture, not executable",
            )
            .unwrap();
            root.canonicalize().unwrap()
        }

        fn registry(&self) -> PathBuf {
            self.store.resolve(Path::new(REGISTRY)).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    fn operation(game: Game) -> Operation {
        Operations::default()
            .begin(OperationKind::Install, Some(game))
            .unwrap()
    }

    #[test]
    fn fallback_is_absolute_but_unregistered_files_are_never_adopted() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let operation = operation(game);
        let root = fixture.store.path(game).unwrap();
        assert_eq!(
            root,
            fixture.store.root().unwrap().join(game.install_path())
        );
        assert!(root.is_absolute());
        assert!(fixture.store.writable_path(game, &operation).is_err());
        assert!(!fixture.registry().exists());
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("keep"), b"existing data").unwrap();
        assert!(fixture.store.prepare_install(game, &operation).is_err());
        assert_eq!(std::fs::read(root.join("keep")).unwrap(), b"existing data");
        assert!(!root.join(OWNER_MARKER).exists());
        assert!(!fixture.registry().exists());
    }

    #[test]
    fn managed_installations_resume_and_recreate_with_the_same_token() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let operation = operation(game);
        let root = fixture.store.prepare_install(game, &operation).unwrap();
        assert_eq!(fixture.store.writable_path(game, &operation).unwrap(), root);
        let installation = fixture.store.registered(game).unwrap().unwrap();
        assert!(installation.managed);
        assert!(Uuid::parse_str(&installation.ownership_token).is_ok());
        let marker = std::fs::read(root.join(OWNER_MARKER)).unwrap();
        let registry = std::fs::read(fixture.registry()).unwrap();
        std::fs::write(root.join("partial-download"), b"partial").unwrap();
        assert_eq!(
            fixture.store.prepare_install(game, &operation).unwrap(),
            root
        );
        assert_eq!(std::fs::read(root.join(OWNER_MARKER)).unwrap(), marker);
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir(root.parent().unwrap()).unwrap();
        assert_eq!(fixture.store.path(game).unwrap(), root);
        assert!(fixture.store.registered(game).unwrap().is_some());
        assert!(fixture.store.writable_path(game, &operation).is_err());
        assert_eq!(
            fixture.store.prepare_install(game, &operation).unwrap(),
            root
        );
        assert_eq!(std::fs::read(root.join(OWNER_MARKER)).unwrap(), marker);
        assert_eq!(std::fs::read(fixture.registry()).unwrap(), registry);
        assert_eq!(fixture.store.writable_path(game, &operation).unwrap(), root);
    }

    #[test]
    fn empty_existing_default_can_be_managed_but_orphan_markers_cannot() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let operation = operation(game);
        let root = fixture.store.path(game).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(OWNER_MARKER), b"orphan").unwrap();
        assert!(fixture.store.prepare_install(game, &operation).is_err());
        assert_eq!(std::fs::read(root.join(OWNER_MARKER)).unwrap(), b"orphan");
        std::fs::remove_file(root.join(OWNER_MARKER)).unwrap();
        assert_eq!(
            fixture.store.prepare_install(game, &operation).unwrap(),
            root
        );
    }

    #[test]
    fn managed_writes_require_a_matching_regular_owner_marker() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let operation = operation(game);
        let root = fixture.store.prepare_install(game, &operation).unwrap();
        let marker_path = root.join(OWNER_MARKER);
        let original = std::fs::read(&marker_path).unwrap();
        let registry = std::fs::read(fixture.registry()).unwrap();
        let mut wrong_game: OwnerMarker = serde_json::from_slice(&original).unwrap();
        wrong_game.game = Game::Nap.code().to_owned();
        let mut wrong_token: OwnerMarker = serde_json::from_slice(&original).unwrap();
        wrong_token.token = Uuid::new_v4().to_string();
        for bytes in [
            b"{".to_vec(),
            serde_json::to_vec(&wrong_game).unwrap(),
            serde_json::to_vec(&wrong_token).unwrap(),
        ] {
            std::fs::write(&marker_path, bytes).unwrap();
            assert_eq!(fixture.store.path(game).unwrap(), root);
            assert!(fixture.store.writable_path(game, &operation).is_err());
            assert!(fixture.store.prepare_install(game, &operation).is_err());
        }
        std::fs::remove_file(&marker_path).unwrap();
        assert!(fixture.store.writable_path(game, &operation).is_err());
        assert!(fixture.store.prepare_install(game, &operation).is_err());
        let copy = fixture.directory.join("copied-marker");
        std::fs::write(&copy, &original).unwrap();
        symlink(copy, &marker_path).unwrap();
        assert!(fixture.store.writable_path(game, &operation).is_err());
        assert!(fixture.store.prepare_install(game, &operation).is_err());
        assert_eq!(std::fs::read(fixture.registry()).unwrap(), registry);
    }

    #[test]
    fn managed_creation_rejects_redirected_default_directory_and_ancestor() {
        for redirect_parent in [false, true] {
            let fixture = Fixture::new();
            let game = Game::Bh3;
            let operation = operation(game);
            let root = fixture.store.path(game).unwrap();
            let other = fixture.store.root().unwrap().join("other");
            std::fs::create_dir(&other).unwrap();
            if redirect_parent {
                symlink(&other, root.parent().unwrap()).unwrap();
            } else {
                std::fs::create_dir_all(root.parent().unwrap()).unwrap();
                symlink(&other, &root).unwrap();
            }
            assert!(fixture.store.prepare_install(game, &operation).is_err());
            assert!(std::fs::read_dir(other).unwrap().next().is_none());
            assert!(!fixture.registry().exists());
        }
    }

    #[test]
    fn imports_are_non_destructive_and_write_consent_is_explicit_and_revocable() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let operation = operation(game);
        let root = fixture.game("external", game);
        std::fs::write(root.join("keep"), b"user data").unwrap();
        let alias = fixture.directory.join("alias");
        symlink(&root, &alias).unwrap();
        fixture
            .store
            .import(game, &alias, false, &operation)
            .unwrap();
        assert_eq!(fixture.store.path(game).unwrap(), root);
        let record = fixture.store.registered(game).unwrap().unwrap();
        assert!(!record.managed);
        assert!(!record.allow_modify);
        assert!(fixture.store.writable_path(game, &operation).is_err());
        assert!(fixture.store.prepare_install(game, &operation).is_err());
        fixture.store.import(game, &root, true, &operation).unwrap();
        assert_eq!(fixture.store.writable_path(game, &operation).unwrap(), root);
        assert!(fixture.store.prepare_install(game, &operation).is_err());
        assert_eq!(
            fixture
                .store
                .registered(game)
                .unwrap()
                .unwrap()
                .ownership_token,
            record.ownership_token
        );
        fixture
            .store
            .import(game, &root, false, &operation)
            .unwrap();
        assert!(fixture.store.writable_path(game, &operation).is_err());
        assert_eq!(std::fs::read(root.join("keep")).unwrap(), b"user data");
        assert_eq!(
            std::fs::read(root.join(game.executable().unwrap())).unwrap(),
            b"fixture, not executable"
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        assert!(!root.join(OWNER_MARKER).exists());
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(fixture.store.path(game).unwrap(), root);
        assert!(fixture.store.writable_path(game, &operation).is_err());
        assert!(fixture.store.prepare_install(game, &operation).is_err());
        assert!(!root.exists());
    }

    #[test]
    fn import_requires_a_known_executable_contained_in_a_real_game_directory() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let operation = operation(game);
        let root = fixture.directory.join("empty");
        std::fs::create_dir(&root).unwrap();
        assert!(
            fixture
                .store
                .import(game, &root, false, &operation)
                .is_err()
        );
        assert!(
            fixture
                .store
                .import(game, &root.join("missing"), false, &operation)
                .is_err()
        );
        std::fs::create_dir(root.join(game.executable().unwrap())).unwrap();
        assert!(
            fixture
                .store
                .import(game, &root, false, &operation)
                .is_err()
        );
        std::fs::remove_dir(root.join(game.executable().unwrap())).unwrap();
        let outside = fixture.game("outside", game);
        symlink(
            outside.join(game.executable().unwrap()),
            root.join(game.executable().unwrap()),
        )
        .unwrap();
        assert!(
            fixture
                .store
                .import(game, &root, false, &operation)
                .is_err()
        );
        assert!(
            fixture
                .store
                .import(
                    game,
                    &outside.join(game.executable().unwrap()),
                    false,
                    &operation
                )
                .is_err()
        );
        let unknown = Operations::default()
            .begin(OperationKind::Import, Some(Game::Abc))
            .unwrap();
        assert!(
            fixture
                .store
                .import(Game::Abc, &outside, false, &unknown)
                .is_err()
        );
        assert!(!fixture.registry().exists());
    }

    #[test]
    fn roots_cannot_contain_home_appdata_or_shared_runtime_data() {
        let fixture = Fixture::new();
        let data = fixture.store.root().unwrap();
        let home = fs::full_path(None, Some(fs::BaseDirectory::Home)).unwrap();
        for root in [
            PathBuf::from("/"),
            home.clone(),
            home.parent().unwrap().to_owned(),
            data.clone(),
            fixture.directory.clone(),
        ] {
            assert!(
                fixture.store.validate_root(&root).is_err(),
                "{}",
                root.display()
            );
        }
        for shared in ["proton-data", "proton", "phlogiston", "components", "cache"] {
            let root = data.join(shared);
            std::fs::create_dir_all(root.join("nested")).unwrap();
            assert!(fixture.store.validate_root(&root).is_err());
            assert!(fixture.store.validate_root(&root.join("nested")).is_err());
        }
        std::fs::remove_dir_all(data.join("proton-data")).unwrap();
        let runtime = fixture.directory.join("external-runtime");
        std::fs::create_dir_all(runtime.join("prefix")).unwrap();
        symlink(&runtime, data.join("proton-data")).unwrap();
        assert!(
            fixture
                .store
                .validate_root(&runtime.join("prefix"))
                .is_err()
        );
        assert!(
            fixture
                .store
                .validate_root(&fixture.game("safe", Game::Bh3))
                .is_ok()
        );
    }

    #[test]
    fn conflicting_roots_and_implicit_replacement_are_rejected_without_rewriting_registry() {
        let fixture = Fixture::new();
        let root = fixture.game("external/first", Game::Bh3);
        fixture
            .store
            .import(Game::Bh3, &root, false, &operation(Game::Bh3))
            .unwrap();
        let registry = std::fs::read(fixture.registry()).unwrap();
        std::fs::write(root.join(Game::Hk4e.executable().unwrap()), b"fixture").unwrap();
        let child = fixture.game("external/first/child", Game::Hk4e);
        let parent = fixture.game("external", Game::Hk4e);
        for other in [&root, &child, &parent] {
            assert!(
                fixture
                    .store
                    .import(Game::Hk4e, other, true, &operation(Game::Hk4e))
                    .is_err()
            );
            assert_eq!(std::fs::read(fixture.registry()).unwrap(), registry);
        }
        let replacement = fixture.game("replacement", Game::Bh3);
        assert!(
            fixture
                .store
                .import(Game::Bh3, &replacement, true, &operation(Game::Bh3))
                .is_err()
        );
        assert_eq!(std::fs::read(fixture.registry()).unwrap(), registry);
        let sibling = fixture.game("sibling", Game::Hk4e);
        fixture
            .store
            .import(Game::Hk4e, &sibling, false, &operation(Game::Hk4e))
            .unwrap();
        let managed = fixture
            .store
            .prepare_install(Game::Nap, &operation(Game::Nap))
            .unwrap();
        std::fs::write(managed.join(Game::Nap.executable().unwrap()), b"fixture").unwrap();
        assert!(
            fixture
                .store
                .import(Game::Nap, &managed, true, &operation(Game::Nap))
                .is_err()
        );
        assert!(
            fixture
                .store
                .registered(Game::Nap)
                .unwrap()
                .unwrap()
                .managed
        );
    }

    #[test]
    fn invalid_or_future_registry_is_never_reset() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let operation = operation(game);
        let root = fixture.game("external", game);
        fixture
            .store
            .import(game, &root, false, &operation)
            .unwrap();
        let original: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture.registry()).unwrap()).unwrap();
        let entry = original["installations"][game.code()].clone();
        let mut variants = vec![
            b"{".to_vec(),
            format!(
                "{{\"schema_version\":1,\"installations\":{{\"bh3\":{entry},\"bh3\":{entry}}}}}"
            )
            .into_bytes(),
        ];
        for (key, value) in [
            ("schema_version", serde_json::json!(2)),
            ("schema_version", serde_json::json!(0)),
            ("extra", serde_json::json!(true)),
            ("installations", serde_json::json!([])),
        ] {
            let mut bad = original.clone();
            bad[key] = value;
            variants.push(serde_json::to_vec(&bad).unwrap());
        }
        let mut bad = original.clone();
        bad.as_object_mut().unwrap().remove("schema_version");
        variants.push(serde_json::to_vec(&bad).unwrap());
        for code in ["unknown", "../bh3", "hk4e"] {
            let mut bad = original.clone();
            bad["installations"][code] = entry.clone();
            variants.push(serde_json::to_vec(&bad).unwrap());
        }
        for (key, value) in [
            ("path", serde_json::json!("relative")),
            ("ownership_token", serde_json::json!("invalid")),
            ("managed", serde_json::json!(true)),
            ("extra", serde_json::json!(true)),
        ] {
            let mut bad = original.clone();
            bad["installations"][game.code()][key] = value;
            variants.push(serde_json::to_vec(&bad).unwrap());
        }
        let mut bad = original.clone();
        bad["installations"][game.code()]
            .as_object_mut()
            .unwrap()
            .remove("allow_modify");
        variants.push(serde_json::to_vec(&bad).unwrap());
        for bytes in variants {
            std::fs::write(fixture.registry(), &bytes).unwrap();
            assert!(fixture.store.path(game).is_err());
            assert!(fixture.store.prepare_install(game, &operation).is_err());
            assert!(fixture.store.writable_path(game, &operation).is_err());
            assert!(fixture.store.import(game, &root, true, &operation).is_err());
            assert_eq!(std::fs::read(fixture.registry()).unwrap(), bytes);
            assert!(!root.join(OWNER_MARKER).exists());
        }
    }

    #[test]
    fn registered_root_and_parent_symlink_replacement_are_rejected() {
        for replace_parent in [false, true] {
            let fixture = Fixture::new();
            let game = Game::Bh3;
            let root = fixture.game("external/game", game);
            fixture
                .store
                .import(game, &root, true, &operation(game))
                .unwrap();
            let registry = std::fs::read(fixture.registry()).unwrap();
            let target = if replace_parent {
                root.parent().unwrap()
            } else {
                &root
            };
            let moved = fixture.directory.join("moved");
            std::fs::rename(target, &moved).unwrap();
            symlink(&moved, target).unwrap();
            assert!(fixture.store.path(game).is_err());
            assert!(fixture.store.writable_path(game, &operation(game)).is_err());
            assert!(
                fixture
                    .store
                    .import(game, &root, false, &operation(game))
                    .is_err()
            );
            assert_eq!(std::fs::read(fixture.registry()).unwrap(), registry);
        }
    }

    #[test]
    fn missing_managed_roots_cannot_be_recreated_through_redirected_ancestors() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let operation = operation(game);
        let root = fixture.store.prepare_install(game, &operation).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let moved = fixture.directory.join("moved-games");
        std::fs::rename(root.parent().unwrap(), &moved).unwrap();
        symlink(&moved, root.parent().unwrap()).unwrap();
        assert!(fixture.store.path(game).is_err());
        assert!(fixture.store.prepare_install(game, &operation).is_err());
        assert!(std::fs::read_dir(moved).unwrap().next().is_none());
    }

    #[test]
    fn registry_symlinks_are_not_read_or_overwritten() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let root = fixture.game("external", game);
        fixture
            .store
            .import(game, &root, false, &operation(game))
            .unwrap();
        let registry_path = fixture.registry();
        let backup = fixture.directory.join("registry-backup");
        let bytes = std::fs::read(&registry_path).unwrap();
        std::fs::rename(&registry_path, &backup).unwrap();
        symlink(&backup, &registry_path).unwrap();
        assert!(fixture.store.path(game).is_err());
        assert!(
            fixture
                .store
                .import(game, &root, true, &operation(game))
                .is_err()
        );
        assert_eq!(std::fs::read(backup).unwrap(), bytes);
        assert!(
            std::fs::symlink_metadata(registry_path)
                .unwrap()
                .is_symlink()
        );
    }

    #[test]
    fn failed_registry_commit_removes_only_the_new_ownership_marker() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let operation = operation(game);
        let root = fixture.store.path(game).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let installation = Installation {
            path: root.clone(),
            managed: true,
            allow_modify: true,
            ownership_token: Uuid::new_v4().to_string(),
        };
        let registry = Registry {
            schema_version: SCHEMA_VERSION,
            installations: BTreeMap::from([(game.code().to_owned(), installation.clone())]),
        };
        // Force the commit stage to fail after writing the marker, without
        // permission-dependent tests or changes to another process's files.
        std::fs::create_dir(fixture.registry()).unwrap();
        assert!(
            fixture
                .store
                .commit_managed(game, &installation, &registry, &operation)
                .is_err()
        );
        assert!(std::fs::read_dir(&root).unwrap().next().is_none());
        assert!(fixture.registry().is_dir());
        std::fs::remove_dir(fixture.registry()).unwrap();
        assert_eq!(
            fixture.store.prepare_install(game, &operation).unwrap(),
            root
        );
    }

    #[test]
    fn mutations_reject_wrong_game_or_cancelled_operation_before_writing() {
        let fixture = Fixture::new();
        let game = Game::Bh3;
        let root = fixture.game("external", game);
        let cancelled = operation(game);
        cancelled.handle().cancel();
        for operation in [
            operation(Game::Nap),
            cancelled,
            Operations::default()
                .begin(OperationKind::Components, None)
                .unwrap(),
        ] {
            assert!(fixture.store.prepare_install(game, &operation).is_err());
            assert!(fixture.store.import(game, &root, true, &operation).is_err());
            assert!(fixture.store.writable_path(game, &operation).is_err());
            assert!(!fixture.registry().exists());
            assert!(!root.join(OWNER_MARKER).exists());
        }
    }
}
