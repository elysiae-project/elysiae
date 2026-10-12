//! A journal records intent, not deletion authority. Cleanup additionally needs
//! a derived temporary name and the matching owner marker. The registry is the
//! commit point: recovery rolls back before it changes and finishes cleanup after.
use std::{
    fs::{self as disk, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Installation, OWNER_MARKER, Store, check_conflicts, verify_owner, write_owner};
use crate::core::{
    fs,
    game::Game,
    operations::{Operation, Phase},
    sys,
};

const JOURNALS: &str = "installation-transactions";
const MAX_JOURNAL: u64 = 64 * 1024;
const BUFFER: usize = 64 * 1024;
const MAX_DEPTH: usize = 128;
const GAMES: [Game; 6] = [
    Game::Bh3,
    Game::Hk4e,
    Game::Hkrpg,
    Game::Nap,
    Game::Abc,
    Game::Hyg,
];

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    schema_version: u32,
    game: String,
    id: String,
    original: Installation,
    source_device: u64,
    source_inode: u64,
    staging_device: u64,
    staging_inode: u64,
    action: Action,
    step: Step,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Action {
    Relocate {
        destination: PathBuf,
        remove_source: bool,
        destination_existed: bool,
    },
    Uninstall,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
enum Step {
    Copying,
    Ready,
    Cleaning,
}

impl Journal {
    fn new(game: Game, original: Installation, action: Action) -> Result<Self> {
        let metadata = disk::symlink_metadata(&original.path)?;
        Ok(Self {
            schema_version: 1,
            game: game.code().to_owned(),
            id: Uuid::new_v4().to_string(),
            original,
            source_device: metadata.dev(),
            source_inode: metadata.ino(),
            staging_device: 0,
            staging_inode: 0,
            action,
            step: Step::Ready,
        })
    }

    fn destination(&self) -> Result<&Path> {
        match &self.action {
            Action::Relocate { destination, .. } => Ok(destination),
            Action::Uninstall => bail!("Uninstall has no destination"),
        }
    }

    fn generated(&self, parent: &Path, kind: &str) -> PathBuf {
        parent.join(format!(".elysiae-{kind}-{}-{}", self.game, self.id))
    }

    fn staging(&self) -> Result<PathBuf> {
        Ok(self.generated(
            self.destination()?
                .parent()
                .context("Destination has no parent")?,
            "stage",
        ))
    }

    fn quarantine(&self) -> Result<PathBuf> {
        Ok(self.generated(
            self.original
                .path
                .parent()
                .context("Source has no parent")?,
            "trash",
        ))
    }

    fn target(&self, path: PathBuf) -> Installation {
        Installation {
            path,
            managed: true,
            allow_modify: true,
            ownership_token: self.id.clone(),
        }
    }

    fn check_staging(&self, game: Game, path: &Path) -> Result<()> {
        let metadata = disk::symlink_metadata(path)?;
        ensure!(
            metadata.is_dir()
                && metadata.dev() == self.staging_device
                && metadata.ino() == self.staging_inode,
            "Relocation staging/destination directory was replaced"
        );
        verify_owner(game, &self.target(path.to_owned()))
    }

    fn check_source(&self, game: Game, path: &Path) -> Result<()> {
        let metadata = disk::symlink_metadata(path)?;
        ensure!(
            metadata.is_dir()
                && metadata.dev() == self.source_device
                && metadata.ino() == self.source_inode,
            "Confirmed installation directory was replaced"
        );
        if self.original.managed {
            let mut original = self.original.clone();
            original.path = path.to_owned();
            verify_owner(game, &original)?;
        }
        Ok(())
    }
}

impl Store {
    fn journal_path(&self, game: Game) -> Result<PathBuf> {
        self.resolve(&PathBuf::from(JOURNALS).join(format!("{}.json", game.code())))
    }

    pub(super) fn journal(&self, game: Game) -> Result<Option<Journal>> {
        let path = self.journal_path(game)?;
        match disk::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(metadata) => ensure!(
                metadata.is_file(),
                "Maintenance journal must be a regular file"
            ),
        }
        let mut bytes = Vec::new();
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?
            .take(MAX_JOURNAL + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_JOURNAL,
            "Maintenance journal is too large"
        );
        let journal: Journal = serde_json::from_slice(&bytes)
            .context("Invalid maintenance journal; retained for inspection")?;
        self.validate_journal(game, &journal)?;
        ensure!(
            !matches!(journal.action, Action::Relocate { .. }) || journal.staging_inode != 0,
            "Maintenance journal has no staging identity"
        );
        Ok(Some(journal))
    }

    fn validate_journal(&self, game: Game, journal: &Journal) -> Result<()> {
        ensure!(
            journal.schema_version == 1 && journal.game == game.code(),
            "Unsupported or mismatched maintenance journal"
        );
        let id = Uuid::parse_str(&journal.id)?;
        ensure!(
            id.to_string() == journal.id && id.get_version_num() == 4,
            "Invalid transaction identifier"
        );
        Uuid::parse_str(&journal.original.ownership_token)?;
        ensure!(
            !journal.original.managed || journal.original.allow_modify,
            "Inconsistent original installation consent"
        );
        self.validate_root(&journal.original.path)?;
        let quarantine = journal.quarantine()?;
        self.validate_root(&quarantine)?;
        ensure!(
            !overlap(&quarantine, &journal.original.path),
            "Quarantine aliases the original installation"
        );
        match &journal.action {
            Action::Relocate {
                destination,
                remove_source,
                ..
            } => {
                ensure!(
                    !remove_source || journal.original.managed,
                    "Imported sources cannot be deleted"
                );
                self.validate_root(destination)?;
                ensure!(
                    !overlap(destination, &journal.original.path),
                    "Relocation source and destination overlap"
                );
                let staging = journal.staging()?;
                self.validate_root(&staging)?;
                ensure!(
                    !overlap(&staging, &journal.original.path)
                        && !overlap(&staging, destination)
                        && !overlap(&staging, &quarantine)
                        && !overlap(&quarantine, destination),
                    "Generated transaction paths overlap an installation"
                );
            }
            Action::Uninstall => ensure!(
                journal.original.managed && journal.step != Step::Copying,
                "Invalid uninstall authority"
            ),
        }
        Ok(())
    }

    fn save_journal(&self, game: Game, journal: &Journal) -> Result<()> {
        self.validate_journal(game, journal)?;
        ensure!(
            !matches!(journal.action, Action::Relocate { .. }) || journal.staging_inode != 0,
            "Maintenance journal has no staging identity"
        );
        let bytes = serde_json::to_vec(journal)?;
        ensure!(
            bytes.len() as u64 <= MAX_JOURNAL,
            "Maintenance journal is too large"
        );
        fs::write_file(
            self.0.join(JOURNALS).join(format!("{}.json", game.code())),
            &bytes,
            None,
        )?;
        sync_file_and_parent(&self.journal_path(game)?)?;
        // Persist the journal directory's creation as well as its entry.
        sync_directory(&self.root()?)
    }

    fn clear_journal(&self, game: Game) -> Result<()> {
        let path = self.journal_path(game)?;
        disk::remove_file(&path)?;
        sync_directory(path.parent().context("Journal has no parent")?)
    }

    pub(super) fn ensure_no_pending(&self) -> Result<()> {
        for game in GAMES {
            ensure!(
                self.journal(game)?.is_none(),
                "Recover pending maintenance for {} first",
                game.code()
            );
        }
        Ok(())
    }

    fn save_registry(&self, registry: &super::Registry) -> Result<()> {
        self.save(registry)?;
        sync_file_and_parent(&self.resolve(Path::new(super::REGISTRY))?)
    }

    pub(super) fn relocate(
        &self,
        game: Game,
        destination: &Path,
        confirmed_source: &Path,
        remove_source: bool,
        operation: &Operation,
    ) -> Result<()> {
        operation.check_game(game)?;
        operation.check_cancelled()?;
        self.ensure_no_pending()?;
        let mut registry = self.load()?;
        let original = registry
            .installations
            .get(game.code())
            .context("Game is not registered")?
            .clone();
        ensure!(
            confirmed_source == original.path,
            "Confirmation does not match the registered source"
        );
        ensure!(
            !remove_source || original.managed,
            "Imported sources may be copied but never deleted"
        );
        if original.managed {
            verify_owner(game, &original)?;
        }
        self.validate_root(destination)?;
        ensure!(
            !overlap(destination, &original.path),
            "Relocation source and destination overlap"
        );
        check_conflicts(&registry, game, destination)?;
        let destination_parent = destination.parent().context("Destination has no parent")?;
        let existed = empty_destination(destination)?;
        let mut journal = Journal::new(
            game,
            original,
            Action::Relocate {
                destination: destination.to_owned(),
                remove_source,
                destination_existed: existed,
            },
        )?;
        journal.step = Step::Copying;
        self.validate_journal(game, &journal)?;
        check_conflicts(&registry, game, &journal.staging()?)?;
        check_conflicts(&registry, game, &journal.quarantine()?)?;
        let bytes = tree_size(
            &journal.original.path,
            &journal.original.path,
            journal.source_device,
            0,
            Some(operation),
            true,
        )?;
        ensure!(
            sys::required_storage_available(
                destination.to_str().context("Destination is not UTF-8")?,
                bytes.saturating_add(16 * 1024 * 1024) as f64,
                fs::Sizes::Bytes
            )?,
            "Insufficient space on destination filesystem"
        );
        operation.check_cancelled()?;
        create_destination_parents(destination_parent)?;
        self.validate_root(destination)?;
        let staging = journal.staging()?;
        {
            use std::os::unix::fs::DirBuilderExt;
            disk::DirBuilder::new()
                .mode(0o700)
                .create(&staging)
                .context("Could not create unique relocation staging directory")?;
        }
        let identity = disk::symlink_metadata(&staging)?;
        journal.staging_device = identity.dev();
        journal.staging_inode = identity.ino();
        // No file data is copied before both ownership and durable intent exist.
        // A crash before journaling leaves an unconfirmed stray, never auto-adopted.
        write_owner(game, &journal.target(staging.clone()))?;
        sync_directory(&staging)?;
        sync_directory(destination_parent)?;
        self.save_journal(game, &journal)?;
        let result = (|| -> Result<()> {
            copy_tree(
                &journal.original.path,
                &journal.original.path,
                &staging,
                journal.source_device,
                0,
                operation,
            )?;
            verify_tree(
                (&journal.original.path, &staging),
                &journal.original.path,
                &staging,
                journal.source_device,
                disk::symlink_metadata(&staging)?.dev(),
                0,
                Some(operation),
            )?;
            journal.check_staging(game, &staging)?;
            journal.step = Step::Ready;
            self.save_journal(game, &journal)?;
            operation.check_cancelled()?;
            operation.set_phase(Phase::Committing)?;
            operation.check_cancelled()?; // Close the cancellation/phase-transition race before committing.
            self.commit_relocation(game, &journal, &mut registry)?;
            self.recover_transaction(game, &mut journal)
        })();
        self.recover_on_error(game, operation, result)
    }

    fn commit_relocation(
        &self,
        game: Game,
        journal: &Journal,
        registry: &mut super::Registry,
    ) -> Result<()> {
        ensure!(
            journal.step == Step::Ready,
            "Relocation copy has not been verified"
        );
        journal.check_source(game, &journal.original.path)?;
        let destination = journal.destination()?;
        self.validate_root(destination)?;
        empty_destination(destination)?;
        let staging = journal.staging()?;
        self.validate_root(&staging)?;
        journal.check_staging(game, &staging)?;
        // This rename is always on the destination filesystem, including when
        // the original installation is on a different device.
        rename_directory(&staging, destination, 0)?;
        sync_directory(destination.parent().context("Destination has no parent")?)?;
        registry.installations.insert(
            game.code().to_owned(),
            journal.target(destination.to_owned()),
        );
        self.save_registry(registry)
    }

    pub(super) fn uninstall(
        &self,
        game: Game,
        confirmed_root: &Path,
        delete_files: bool,
        operation: &Operation,
    ) -> Result<()> {
        operation.check_game(game)?;
        operation.check_cancelled()?;
        self.ensure_no_pending()?;
        let mut registry = self.load()?;
        let original = registry
            .installations
            .get(game.code())
            .context("Game is not registered")?
            .clone();
        ensure!(
            confirmed_root == original.path,
            "Confirmation does not match the registered root"
        );
        if !delete_files {
            operation.set_phase(Phase::Committing)?;
            operation.check_cancelled()?;
            registry.installations.remove(game.code());
            return self.save_registry(&registry);
        }
        ensure!(original.managed, "Imported installations cannot be deleted");
        verify_owner(game, &original)?;
        let mut journal = Journal::new(game, original, Action::Uninstall)?;
        self.validate_journal(game, &journal)?;
        check_conflicts(&registry, game, &journal.quarantine()?)?;
        tree_size(
            &journal.original.path,
            &journal.original.path,
            journal.source_device,
            0,
            Some(operation),
            false,
        )?;
        self.save_journal(game, &journal)?;
        let result = (|| -> Result<()> {
            operation.set_phase(Phase::Committing)?;
            operation.check_cancelled()?;
            self.quarantine(game, &journal)?;
            registry.installations.remove(game.code());
            self.save_registry(&registry)?;
            self.recover_transaction(game, &mut journal)
        })();
        self.recover_on_error(game, operation, result)
    }

    fn recover_on_error(
        &self,
        game: Game,
        operation: &Operation,
        result: Result<()>,
    ) -> Result<()> {
        if let Err(error) = result {
            operation.check_game(game)?;
            if let Some(mut journal) = self.journal(game)? {
                if let Err(transition) = operation.set_phase(Phase::Committing) {
                    // A cancelled lease cannot enter Committing. It can still
                    // discard its copied staging while retaining the original;
                    // any already committed work requires a fresh recovery lease.
                    let registry = self.load()?;
                    ensure!(
                        operation.check_cancelled().is_err()
                            && registry.installations.get(game.code()) == Some(&journal.original)
                            && !journal.quarantine()?.try_exists()?,
                        "{transition:#}; recovery requires a fresh operation"
                    );
                }
                self.recover_transaction(game, &mut journal)
                    .with_context(|| {
                        format!("{error:#}; maintenance recovery is still required")
                    })?;
            }
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn recover(&self, game: Game, operation: &Operation) -> Result<()> {
        operation.check_game(game)?;
        operation.check_cancelled()?;
        let Some(mut journal) = self.journal(game)? else {
            return Ok(());
        };
        operation.set_phase(Phase::Committing)?;
        operation.check_cancelled()?;
        self.recover_transaction(game, &mut journal)
    }

    fn recover_transaction(&self, game: Game, journal: &mut Journal) -> Result<()> {
        self.validate_journal(game, journal)?;
        let mut registry = self.load()?;
        for path in [&journal.original.path, &journal.quarantine()?] {
            check_conflicts(&registry, game, path)?;
        }
        let current = registry.installations.get(game.code());
        match &journal.action {
            Action::Relocate {
                destination,
                remove_source,
                destination_existed,
            } => {
                let destination = destination.clone();
                let remove_source = *remove_source;
                let destination_existed = *destination_existed;
                let staging = journal.staging()?;
                check_conflicts(&registry, game, &destination)?;
                check_conflicts(&registry, game, &staging)?;
                if current == Some(&journal.original) {
                    journal.check_source(game, &journal.original.path)?;
                    ensure!(
                        !journal.quarantine()?.try_exists()?,
                        "Unexpected source quarantine before registry commit"
                    );
                    if destination.try_exists()? && empty_destination(&destination).is_err() {
                        ensure!(
                            journal.step != Step::Copying,
                            "Unverified relocation destination must not be adopted"
                        );
                        journal.check_staging(game, &destination)?;
                        ensure!(
                            !staging.try_exists()?,
                            "Both relocation staging and destination exist"
                        );
                        rename_directory(&destination, &staging, libc::RENAME_NOREPLACE)?;
                        sync_directory(destination.parent().context("Destination has no parent")?)?;
                    }
                    self.cleanup(game, journal, &staging, &journal.id.clone())?;
                    if destination_existed && !destination.try_exists()? {
                        disk::create_dir(&destination)?;
                    }
                } else {
                    ensure!(
                        current == Some(&journal.target(destination.clone())),
                        "Registry no longer matches this relocation transaction"
                    );
                    ensure!(
                        journal.step != Step::Copying,
                        "Registry references an unverified relocation"
                    );
                    journal.check_staging(game, &destination)?;
                    // Never discard the backup on a possibly undurable registry switch.
                    sync_file_and_parent(&self.resolve(Path::new(super::REGISTRY))?)?;
                    ensure!(
                        !staging.try_exists()?,
                        "Unexpected staging directory after relocation commit"
                    );
                    if remove_source {
                        let quarantine = journal.quarantine()?;
                        if journal.original.path.try_exists()? {
                            journal.check_source(game, &journal.original.path)?;
                            self.quarantine(game, journal)?;
                        }
                        // Quarantine cuts off ordinary writes through the old
                        // pathname before the last verification and any deletion.
                        if quarantine.try_exists()? && journal.step != Step::Cleaning {
                            journal.check_source(game, &quarantine)?;
                            if let Err(error) = verify_tree(
                                (&quarantine, &destination),
                                &quarantine,
                                &destination,
                                journal.source_device,
                                journal.staging_device,
                                0,
                                None,
                            ) {
                                rename_directory(
                                    &quarantine,
                                    &journal.original.path,
                                    libc::RENAME_NOREPLACE,
                                )?;
                                sync_directory(
                                    journal
                                        .original
                                        .path
                                        .parent()
                                        .context("Source has no parent")?,
                                )?;
                                registry
                                    .installations
                                    .insert(game.code().to_owned(), journal.original.clone());
                                self.save_registry(&registry)?;
                                self.recover_transaction(game, journal)?;
                                return Err(error.context(
                                    "Source changed before cleanup; restored original installation",
                                ));
                            }
                        }
                        self.cleanup(
                            game,
                            journal,
                            &quarantine,
                            &journal.original.ownership_token.clone(),
                        )?;
                    }
                }
            }
            Action::Uninstall => {
                let quarantine = journal.quarantine()?;
                if current == Some(&journal.original) {
                    ensure!(
                        journal.step != Step::Cleaning,
                        "Cannot restore a partially cleaned uninstall"
                    );
                    if quarantine.try_exists()? {
                        ensure!(
                            !journal.original.path.try_exists()?,
                            "Both installation and quarantine exist"
                        );
                        journal.check_source(game, &quarantine)?;
                        rename_directory(
                            &quarantine,
                            &journal.original.path,
                            libc::RENAME_NOREPLACE,
                        )?;
                        sync_directory(
                            journal
                                .original
                                .path
                                .parent()
                                .context("Source has no parent")?,
                        )?;
                    } else {
                        journal.check_source(game, &journal.original.path)?;
                    }
                } else {
                    ensure!(
                        current.is_none(),
                        "Registry no longer matches this uninstall transaction"
                    );
                    sync_file_and_parent(&self.resolve(Path::new(super::REGISTRY))?)?;
                    ensure!(
                        !journal.original.path.try_exists()?,
                        "Source reappeared after uninstall commit; refusing cleanup"
                    );
                    self.cleanup(
                        game,
                        journal,
                        &quarantine,
                        &journal.original.ownership_token.clone(),
                    )?;
                }
            }
        }
        self.clear_journal(game)
    }

    fn quarantine(&self, game: Game, journal: &Journal) -> Result<()> {
        self.validate_root(&journal.original.path)?;
        journal.check_source(game, &journal.original.path)?;
        let quarantine = journal.quarantine()?;
        self.validate_root(&quarantine)?;
        ensure!(
            !quarantine.try_exists()?,
            "Quarantine already exists; refusing replacement"
        );
        rename_directory(&journal.original.path, &quarantine, libc::RENAME_NOREPLACE)?;
        sync_directory(quarantine.parent().context("Quarantine has no parent")?)
    }

    fn cleanup(&self, game: Game, journal: &mut Journal, path: &Path, token: &str) -> Result<()> {
        ensure!(
            path == journal.quarantine()?
                || matches!(journal.action, Action::Relocate { .. })
                    && path == journal.staging()?,
            "Cleanup is restricted to this transaction's generated paths"
        );
        self.validate_root(path)?;
        if !path.try_exists()? {
            return Ok(());
        }
        let metadata = disk::symlink_metadata(path)?;
        let expected_identity = if path == journal.quarantine()? {
            (journal.source_device, journal.source_inode)
        } else {
            (journal.staging_device, journal.staging_inode)
        };
        ensure!(
            (metadata.dev(), metadata.ino()) == expected_identity,
            "Transaction cleanup directory was replaced"
        );
        let installation = Installation {
            path: path.to_owned(),
            managed: true,
            allow_modify: true,
            ownership_token: token.to_owned(),
        };
        // Empty is accepted only after a durable cleaning intent: a crash may
        // occur between deleting the last marker and removing the root itself.
        if disk::read_dir(path)?.next().transpose()?.is_none() {
            ensure!(
                journal.step == Step::Cleaning,
                "Unconfirmed empty staging/quarantine directory"
            );
        } else {
            verify_owner(game, &installation)?;
            let device = metadata.dev();
            tree_size(path, path, device, 0, None, false)?;
            journal.step = Step::Cleaning;
            self.save_journal(game, journal)?;
            let directory = open_without_links(path, true)?;
            let pinned = directory.metadata()?;
            ensure!(
                (pinned.dev(), pinned.ino()) == expected_identity,
                "Cleanup directory changed while opening it"
            );
            verify_owner(
                game,
                &Installation {
                    path: fd_path(&directory),
                    ..installation.clone()
                },
            )?;
            remove_contents(&directory, device, 0)?;
            // Persist content removal before dropping the last ownership proof.
            directory.sync_all()?;
            disk::remove_file(fd_path(&directory).join(OWNER_MARKER))?;
            directory.sync_all()?;
        }
        disk::remove_dir(path)?;
        sync_directory(path.parent().context("Cleanup directory has no parent")?)
    }
}

fn overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn create_destination_parents(path: &Path) -> Result<()> {
    let mut missing = Vec::new();
    let mut ancestor = path;
    loop {
        match disk::symlink_metadata(ancestor) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir() && ancestor.canonicalize()? == ancestor,
                    "Destination ancestor is not a canonical directory"
                );
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(ancestor.to_owned());
                ancestor = ancestor
                    .parent()
                    .context("Destination has no existing ancestor")?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    for directory in missing.into_iter().rev() {
        disk::create_dir(&directory)?;
        sync_directory(&directory)?;
        // fsync of the leaf alone cannot persist its entry in the parent.
        sync_directory(
            directory
                .parent()
                .context("Created directory has no parent")?,
        )?;
    }
    Ok(())
}

fn empty_destination(path: &Path) -> Result<bool> {
    match disk::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
        Ok(metadata) => {
            ensure!(
                metadata.is_dir() && path.canonicalize()? == path,
                "Destination must be a canonical real directory"
            );
            ensure!(
                disk::read_dir(path)?.next().transpose()?.is_none(),
                "Destination is not empty"
            );
            Ok(true)
        }
    }
}

fn sync_directory(path: &Path) -> Result<()> {
    Ok(File::open(path)?.sync_all()?)
}
fn sync_file_and_parent(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    sync_directory(path.parent().context("Metadata has no parent directory")?)
}

fn rename_directory(source: &Path, destination: &Path, flags: libc::c_uint) -> Result<()> {
    use std::os::fd::AsRawFd;
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source_parent = open_without_links(source.parent().context("Source has no parent")?, true)?;
    let destination_parent = open_without_links(
        destination.parent().context("Destination has no parent")?,
        true,
    )?;
    let source = CString::new(
        source
            .file_name()
            .context("Source has no filename")?
            .as_bytes(),
    )?;
    let destination = CString::new(
        destination
            .file_name()
            .context("Destination has no filename")?
            .as_bytes(),
    )?;
    // Pin real parents. Quarantine/rollback use NOREPLACE; only committing to
    // the explicitly selected, checked-empty destination permits replacement.
    let result = unsafe {
        libc::renameat2(
            source_parent.as_raw_fd(),
            source.as_ptr(),
            destination_parent.as_raw_fd(),
            destination.as_ptr(),
            flags,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn cancelled(operation: Option<&Operation>) -> Result<()> {
    if let Some(operation) = operation {
        operation.check_cancelled()?;
    }
    Ok(())
}

fn metadata(root: &Path, path: &Path, device: u64) -> Result<disk::Metadata> {
    let metadata = open_tree_path(root, path, libc::O_PATH | libc::O_NOFOLLOW)?.metadata()?;
    ensure!(
        metadata.dev() == device,
        "Installation contains another filesystem: {}",
        path.display()
    );
    ensure!(
        metadata.is_file() || metadata.is_dir() || metadata.is_symlink(),
        "Installation contains a special filesystem entry: {}",
        path.display()
    );
    if metadata.is_dir() {
        ensure!(
            path.canonicalize()? == path,
            "Installation directory was redirected"
        );
    }
    Ok(metadata)
}

fn tree_size(
    root: &Path,
    path: &Path,
    device: u64,
    depth: usize,
    operation: Option<&Operation>,
    inspect_links: bool,
) -> Result<u64> {
    ensure!(
        depth <= MAX_DEPTH,
        "Installation directory nesting is too deep"
    );
    cancelled(operation)?;
    ensure!(
        metadata(root, path, device)?.is_dir(),
        "Installation root is not a directory"
    );
    let mut size = 0u64;
    for entry in disk::read_dir(path)? {
        cancelled(operation)?;
        let entry = entry?;
        if path == root && entry.file_name() == OWNER_MARKER {
            continue;
        }
        let metadata = metadata(root, &entry.path(), device)?;
        let bytes = if metadata.is_dir() {
            tree_size(
                root,
                &entry.path(),
                device,
                depth + 1,
                operation,
                inspect_links,
            )?
        } else if metadata.is_symlink() {
            if inspect_links {
                safe_link(root, &entry.path())?;
            }
            0
        } else {
            metadata.len()
        };
        size = size
            .checked_add(bytes)
            .context("Installation size overflow")?;
    }
    Ok(size)
}

fn safe_link(root: &Path, path: &Path) -> Result<PathBuf> {
    let target = disk::read_link(path)?;
    ensure!(
        !target.is_absolute() && !target.as_os_str().is_empty(),
        "Installation symlinks must have internal relative targets"
    );
    resolve_link(
        root,
        &path
            .parent()
            .context("Link has no parent")?
            .strip_prefix(root)?
            .join(&target),
        &mut 40,
    )?;
    Ok(target)
}

fn resolve_link(root: &Path, relative: &Path, remaining: &mut usize) -> Result<PathBuf> {
    let mut path = root.to_owned();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                ensure!(path != root, "Installation link escapes its root");
                path.pop();
            }
            Component::Normal(name) => {
                path.push(name);
                match disk::symlink_metadata(&path) {
                    Ok(metadata) if metadata.is_symlink() => {
                        ensure!(
                            *remaining > 0,
                            "Installation symlink cycle or excessive depth"
                        );
                        *remaining -= 1;
                        let target = disk::read_link(&path)?;
                        path = resolve_link(
                            root,
                            &path
                                .parent()
                                .context("Link has no parent")?
                                .strip_prefix(root)?
                                .join(target),
                            remaining,
                        )?;
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            _ => bail!("Installation links must remain relative"),
        }
    }
    Ok(path)
}

fn open_regular(root: &Path, path: &Path, device: u64) -> Result<File> {
    let expected = metadata(root, path, device)?;
    ensure!(expected.is_file(), "Expected a regular installation file");
    let file = open_tree_path(root, path, libc::O_RDONLY | libc::O_NONBLOCK)?;
    let actual = file.metadata()?;
    ensure!(
        actual.is_file() && actual.dev() == device && actual.ino() == expected.ino(),
        "Installation file was replaced while opening it"
    );
    Ok(file)
}

fn copy_stream(
    reader: &mut impl Read,
    writer: &mut impl Write,
    operation: &Operation,
) -> Result<()> {
    let mut buffer = [0u8; BUFFER];
    loop {
        operation.check_cancelled()?;
        let count = reader.read(&mut buffer)?;
        operation.check_cancelled()?;
        if count == 0 {
            return Ok(());
        }
        writer.write_all(&buffer[..count])?;
    }
}

fn copy_tree(
    root: &Path,
    source: &Path,
    destination: &Path,
    device: u64,
    depth: usize,
    operation: &Operation,
) -> Result<()> {
    ensure!(
        depth <= MAX_DEPTH,
        "Installation directory nesting is too deep"
    );
    let destination = open_without_links(destination, true)?;
    copy_tree_into(root, source, &destination, device, depth, operation)
}

fn copy_tree_into(
    root: &Path,
    source: &Path,
    destination: &File,
    device: u64,
    depth: usize,
    operation: &Operation,
) -> Result<()> {
    ensure!(
        depth <= MAX_DEPTH,
        "Installation directory nesting is too deep"
    );
    let directory = metadata(root, source, device)?;
    ensure!(directory.is_dir(), "Copy source is not a real directory");
    // Pin every output parent. Following our own live fd is safe even if a
    // staging ancestor is renamed; no child symlink or mount is traversed.
    for entry in disk::read_dir(source)? {
        operation.check_cancelled()?;
        let entry = entry?;
        if source == root && entry.file_name() == OWNER_MARKER {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name();
        let target = fd_path(destination).join(&name);
        let metadata = metadata(root, &path, device)?;
        if metadata.is_dir() {
            disk::create_dir(&target)?;
            let child = open_beneath(
                destination,
                Path::new(&name),
                libc::O_RDONLY | libc::O_DIRECTORY,
            )?;
            copy_tree_into(root, &path, &child, device, depth + 1, operation)?;
        } else if metadata.is_symlink() {
            symlink(safe_link(root, &path)?, target)?;
        } else {
            let mut source = open_regular(root, &path, device)?;
            let mut output = open_beneath(
                destination,
                Path::new(&name),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            )?;
            copy_stream(&mut source, &mut output, operation)?;
            output.set_permissions(disk::Permissions::from_mode(metadata.mode() & 0o777))?;
            output.sync_all()?;
        }
    }
    destination.set_permissions(disk::Permissions::from_mode(directory.mode() & 0o777))?;
    Ok(destination.sync_all()?)
}

fn verify_tree(
    roots: (&Path, &Path),
    source: &Path,
    destination: &Path,
    source_device: u64,
    destination_device: u64,
    depth: usize,
    operation: Option<&Operation>,
) -> Result<()> {
    ensure!(
        depth <= MAX_DEPTH,
        "Installation directory nesting is too deep"
    );
    cancelled(operation)?;
    let (root, destination_root) = roots;
    let original = metadata(root, source, source_device)?;
    let copied = metadata(destination_root, destination, destination_device)?;
    ensure!(
        original.is_dir() && copied.is_dir() && original.mode() & 0o777 == copied.mode() & 0o777,
        "Copied directory permissions differ"
    );
    let mut expected_count = 0usize;
    for entry in disk::read_dir(source)? {
        cancelled(operation)?;
        let entry = entry?;
        if source == root && entry.file_name() == OWNER_MARKER {
            continue;
        }
        expected_count += 1;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        let original = metadata(root, &path, source_device)?;
        let copied = metadata(destination_root, &target, destination_device)?;
        ensure!(
            original.file_type() == copied.file_type(),
            "Copied entry type differs"
        );
        if original.is_dir() {
            verify_tree(
                roots,
                &path,
                &target,
                source_device,
                destination_device,
                depth + 1,
                operation,
            )?;
        } else if original.is_symlink() {
            ensure!(
                safe_link(root, &path)? == disk::read_link(&target)?,
                "Copied symlink differs"
            );
        } else {
            ensure!(
                original.len() == copied.len() && original.mode() & 0o777 == copied.mode() & 0o777,
                "Copied file metadata differs"
            );
            let mut original = open_regular(root, &path, source_device)?;
            let mut copied = open_regular(destination_root, &target, destination_device)?;
            // Heap buffers keep recursive directory frames small even in debug builds.
            let mut left = vec![0u8; BUFFER];
            let mut right = vec![0u8; BUFFER];
            loop {
                cancelled(operation)?;
                let count = original.read(&mut left)?;
                if count == 0 {
                    ensure!(
                        copied.read(&mut right[..1])? == 0,
                        "Copied file grew during verification"
                    );
                    break;
                }
                copied.read_exact(&mut right[..count])?;
                ensure!(
                    left[..count] == right[..count],
                    "Copied file contents differ"
                );
            }
        }
    }
    let mut actual_count = 0usize;
    for entry in disk::read_dir(destination)? {
        let entry = entry?;
        if source != root || entry.file_name() != OWNER_MARKER {
            actual_count += 1;
        }
    }
    ensure!(
        expected_count == actual_count,
        "Copied directory entry count differs"
    );
    Ok(())
}

// Trusted canonical roots may themselves live on a different mount. Descendant
// access must instead use open_beneath: NO_XDEV on an absolute path would reject
// legitimate installations mounted below /, not just unexpected child mounts.
fn open_without_links(path: &Path, directory: bool) -> Result<File> {
    open_at(
        libc::AT_FDCWD,
        path,
        libc::O_RDONLY | libc::O_NONBLOCK | if directory { libc::O_DIRECTORY } else { 0 },
        0x04,
    ) // RESOLVE_NO_SYMLINKS
}

fn open_beneath(directory: &File, path: &Path, flags: libc::c_int) -> Result<File> {
    use std::os::fd::AsRawFd;
    open_at(directory.as_raw_fd(), path, flags, 0x01 | 0x04 | 0x08) // RESOLVE_NO_XDEV | NO_SYMLINKS | BENEATH
}

fn open_tree_path(root: &Path, path: &Path, flags: libc::c_int) -> Result<File> {
    let directory = open_without_links(root, true)?;
    let relative = path.strip_prefix(root)?;
    open_beneath(
        &directory,
        if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        },
        flags,
    )
}

// openat2 is available on the supported Linux kernels. Fail closed if absent;
// st_dev comparisons alone do not detect same-filesystem bind mounts.
fn open_at(
    directory: std::os::fd::RawFd,
    path: &Path,
    flags: libc::c_int,
    resolve: u64,
) -> Result<File> {
    use std::{
        ffi::CString,
        os::{fd::FromRawFd, unix::ffi::OsStrExt},
    };
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    let path = CString::new(path.as_os_str().as_bytes())?;
    let how = OpenHow {
        flags: (flags | libc::O_CLOEXEC) as u64,
        mode: if flags & libc::O_CREAT != 0 { 0o600 } else { 0 },
        resolve,
    };
    // SAFETY: NUL-terminated path and correctly sized Linux open_how. A returned
    // descriptor is newly owned and immediately wrapped in File.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            directory,
            path.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd as std::os::fd::RawFd) })
}

fn fd_path(directory: &File) -> PathBuf {
    use std::os::fd::AsRawFd;
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
}

fn remove_contents(directory: &File, device: u64, depth: usize) -> Result<()> {
    ensure!(depth <= MAX_DEPTH, "Cleanup directory nesting is too deep");
    let permissions = directory.metadata()?;
    ensure!(
        permissions.is_dir() && permissions.dev() == device,
        "Cleanup directory was replaced or mounted"
    );
    directory.set_permissions(disk::Permissions::from_mode(
        (permissions.mode() & 0o777) | 0o700,
    ))?;
    // The only followed link is our own live descriptor. Child directories are
    // pinned with O_NOFOLLOW, so a racing source symlink cannot redirect cleanup.
    for entry in disk::read_dir(fd_path(directory))? {
        let entry = entry?;
        if depth == 0 && entry.file_name() == OWNER_MARKER {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name();
        let metadata = open_beneath(directory, Path::new(&name), libc::O_PATH | libc::O_NOFOLLOW)?
            .metadata()?;
        ensure!(
            metadata.dev() == device
                && (metadata.is_file() || metadata.is_dir() || metadata.is_symlink()),
            "Unsafe cleanup entry"
        );
        if metadata.is_dir() {
            let child = open_beneath(
                directory,
                Path::new(&name),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            )?;
            remove_contents(&child, device, depth + 1)?;
            disk::remove_dir(path)?;
        } else {
            disk::remove_file(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        installations::tests::Fixture,
        operations::{OperationKind, Operations},
    };

    const GAME: Game = Game::Bh3;

    fn operation(kind: OperationKind) -> Operation {
        Operations::default().begin(kind, Some(GAME)).unwrap()
    }

    fn managed(fixture: &Fixture) -> PathBuf {
        let root = fixture
            .store
            .prepare_install(GAME, &operation(OperationKind::Install))
            .unwrap();
        disk::write(root.join(GAME.executable().unwrap()), b"test game data").unwrap();
        disk::create_dir(root.join("nested")).unwrap();
        disk::write(root.join("nested/data"), vec![0x5a; BUFFER * 2 + 17]).unwrap();
        root
    }

    #[test]
    fn copying_uses_pinned_staging_not_replacement_symlink() {
        let fixture = Fixture::new();
        let root = managed(&fixture);
        let stage = fixture.directory.join("stage");
        let moved = fixture.directory.join("moved-stage");
        let outside = fixture.directory.join("outside");
        disk::create_dir(&stage).unwrap();
        disk::create_dir(&outside).unwrap();
        disk::set_permissions(&outside, disk::Permissions::from_mode(0o750)).unwrap();
        let pinned = open_without_links(&stage, true).unwrap();
        disk::rename(&stage, &moved).unwrap();
        symlink(&outside, &stage).unwrap();
        copy_tree_into(
            &root,
            &root,
            &pinned,
            disk::metadata(&root).unwrap().dev(),
            0,
            &operation(OperationKind::Relocate),
        )
        .unwrap();
        assert!(moved.join("nested/data").is_file());
        assert_eq!(disk::read_dir(&outside).unwrap().count(), 0);
        assert_eq!(disk::metadata(&outside).unwrap().mode() & 0o777, 0o750);
        assert!(
            copy_tree(
                &root,
                &root,
                &stage,
                disk::metadata(&root).unwrap().dev(),
                0,
                &operation(OperationKind::Relocate)
            )
            .is_err()
        );
    }

    fn staged(fixture: &Fixture, ready: bool, remove_source: bool) -> Journal {
        let root = managed(fixture);
        let original = fixture.store.registered(GAME).unwrap().unwrap();
        let mut journal = Journal::new(
            GAME,
            original,
            Action::Relocate {
                destination: fixture.directory.join("destination"),
                remove_source,
                destination_existed: false,
            },
        )
        .unwrap();
        journal.step = Step::Copying;
        let stage = journal.staging().unwrap();
        disk::create_dir(&stage).unwrap();
        let identity = disk::symlink_metadata(&stage).unwrap();
        journal.staging_device = identity.dev();
        journal.staging_inode = identity.ino();
        write_owner(GAME, &journal.target(stage.clone())).unwrap();
        fixture.store.save_journal(GAME, &journal).unwrap();
        if ready {
            let operation = operation(OperationKind::Relocate);
            copy_tree(&root, &root, &stage, journal.source_device, 0, &operation).unwrap();
            verify_tree(
                (&root, &stage),
                &root,
                &stage,
                journal.source_device,
                disk::metadata(&stage).unwrap().dev(),
                0,
                Some(&operation),
            )
            .unwrap();
            journal.step = Step::Ready;
            fixture.store.save_journal(GAME, &journal).unwrap();
        }
        journal
    }

    #[test]
    fn relocation_copies_contents_permissions_and_internal_links_without_moving_source() {
        let fixture = Fixture::new();
        let source = managed(&fixture);
        disk::set_permissions(source.join("nested"), disk::Permissions::from_mode(0o750)).unwrap();
        disk::set_permissions(
            source.join("nested/data"),
            disk::Permissions::from_mode(0o640),
        )
        .unwrap();
        disk::set_permissions(
            source.join(GAME.executable().unwrap()),
            disk::Permissions::from_mode(0o751),
        )
        .unwrap();
        symlink("nested/data", source.join("file-link")).unwrap();
        symlink("nested", source.join("dir-link")).unwrap();
        symlink("missing/file", source.join("dangling-internal")).unwrap();
        let old_token = fixture
            .store
            .registered(GAME)
            .unwrap()
            .unwrap()
            .ownership_token;
        let destination = fixture.directory.join("destination");
        disk::create_dir(&destination).unwrap();
        fixture
            .store
            .relocate(
                GAME,
                &destination,
                &source,
                false,
                &operation(OperationKind::Relocate),
            )
            .unwrap();
        assert_eq!(fixture.store.path(GAME).unwrap(), destination);
        assert!(source.is_dir());
        assert_ne!(
            disk::metadata(&source).unwrap().ino(),
            disk::metadata(&destination).unwrap().ino()
        );
        assert_eq!(
            disk::read(destination.join("nested/data")).unwrap(),
            disk::read(source.join("nested/data")).unwrap()
        );
        assert_eq!(
            disk::metadata(destination.join("nested")).unwrap().mode() & 0o777,
            0o750
        );
        assert_eq!(
            disk::metadata(destination.join("nested/data"))
                .unwrap()
                .mode()
                & 0o777,
            0o640
        );
        assert_eq!(
            disk::metadata(destination.join(GAME.executable().unwrap()))
                .unwrap()
                .mode()
                & 0o777,
            0o751
        );
        for (name, target) in [
            ("file-link", "nested/data"),
            ("dir-link", "nested"),
            ("dangling-internal", "missing/file"),
        ] {
            assert_eq!(
                disk::read_link(destination.join(name)).unwrap(),
                PathBuf::from(target)
            );
        }
        let record = fixture.store.registered(GAME).unwrap().unwrap();
        assert!(record.managed && record.allow_modify);
        assert_ne!(record.ownership_token, old_token);
        verify_owner(GAME, &record).unwrap();
        assert!(fixture.store.journal(GAME).unwrap().is_none());
    }

    #[test]
    fn imported_relocation_retains_original_and_never_grants_source_deletion() {
        let fixture = Fixture::new();
        let source = fixture.game("external", GAME);
        fixture
            .store
            .import(GAME, &source, true, &operation(OperationKind::Import))
            .unwrap();
        let destination = fixture.directory.join("destination");
        assert!(
            fixture
                .store
                .relocate(
                    GAME,
                    &destination,
                    &source,
                    true,
                    &operation(OperationKind::Relocate)
                )
                .is_err()
        );
        assert!(
            fixture
                .store
                .uninstall(GAME, &source, true, &operation(OperationKind::Uninstall))
                .is_err()
        );
        fixture
            .store
            .relocate(
                GAME,
                &destination,
                &source,
                false,
                &operation(OperationKind::Relocate),
            )
            .unwrap();
        assert_eq!(disk::read_dir(&source).unwrap().count(), 1);
        assert!(!source.join(OWNER_MARKER).exists());
        assert!(fixture.store.registered(GAME).unwrap().unwrap().managed);
    }

    #[test]
    fn unsafe_destinations_wrong_confirmation_and_other_games_are_rejected() {
        let fixture = Fixture::new();
        let source = managed(&fixture);
        let before = disk::read(fixture.registry()).unwrap();
        let destination = fixture.directory.join("destination");
        assert!(
            fixture
                .store
                .relocate(
                    GAME,
                    &destination,
                    &fixture.directory,
                    true,
                    &operation(OperationKind::Relocate)
                )
                .is_err()
        );
        assert!(
            fixture
                .store
                .uninstall(
                    GAME,
                    &fixture.directory,
                    true,
                    &operation(OperationKind::Uninstall)
                )
                .is_err()
        );
        for path in [
            &source,
            &source.join("child"),
            source.parent().unwrap(),
            &fixture.store.root().unwrap().join("proton-data"),
            &fixture.store.root().unwrap().join("cache"),
        ] {
            assert!(
                fixture
                    .store
                    .relocate(
                        GAME,
                        path,
                        &source,
                        false,
                        &operation(OperationKind::Relocate)
                    )
                    .is_err()
            );
        }
        disk::create_dir(&destination).unwrap();
        disk::write(destination.join("keep"), b"untouched").unwrap();
        assert!(
            fixture
                .store
                .relocate(
                    GAME,
                    &destination,
                    &source,
                    false,
                    &operation(OperationKind::Relocate)
                )
                .is_err()
        );
        let alias = fixture.directory.join("alias");
        symlink(&destination, &alias).unwrap();
        assert!(
            fixture
                .store
                .relocate(
                    GAME,
                    &alias,
                    &source,
                    false,
                    &operation(OperationKind::Relocate)
                )
                .is_err()
        );
        let other = fixture
            .store
            .prepare_install(
                Game::Nap,
                &Operations::default()
                    .begin(OperationKind::Install, Some(Game::Nap))
                    .unwrap(),
            )
            .unwrap();
        assert!(
            fixture
                .store
                .relocate(
                    GAME,
                    &other.join("child"),
                    &source,
                    false,
                    &operation(OperationKind::Relocate)
                )
                .is_err()
        );
        assert_eq!(disk::read(destination.join("keep")).unwrap(), b"untouched");
        let mut expected: serde_json::Value = serde_json::from_slice(&before).unwrap();
        let mut actual: serde_json::Value =
            serde_json::from_slice(&disk::read(fixture.registry()).unwrap()).unwrap();
        actual["installations"]
            .as_object_mut()
            .unwrap()
            .remove(Game::Nap.code());
        assert_eq!(actual, expected.take());
        assert!(fixture.store.journal(GAME).unwrap().is_none());
    }

    #[test]
    fn unregister_only_never_writes_inside_managed_or_imported_roots() {
        for imported in [false, true] {
            let fixture = Fixture::new();
            let root = if imported {
                let root = fixture.game("external", GAME);
                fixture
                    .store
                    .import(GAME, &root, false, &operation(OperationKind::Import))
                    .unwrap();
                root
            } else {
                managed(&fixture)
            };
            let count = disk::read_dir(&root).unwrap().count();
            let executable = disk::read(root.join(GAME.executable().unwrap())).unwrap();
            let marker = disk::read(root.join(OWNER_MARKER)).ok();
            fixture
                .store
                .uninstall(GAME, &root, false, &operation(OperationKind::Uninstall))
                .unwrap();
            assert!(fixture.store.registered(GAME).unwrap().is_none());
            assert_eq!(disk::read_dir(&root).unwrap().count(), count);
            assert_eq!(
                disk::read(root.join(GAME.executable().unwrap())).unwrap(),
                executable
            );
            assert_eq!(disk::read(root.join(OWNER_MARKER)).ok(), marker);
        }
    }

    #[test]
    fn managed_uninstall_and_relocation_cleanup_leave_shared_data_untouched() {
        for relocate in [false, true] {
            let fixture = Fixture::new();
            let source = managed(&fixture);
            let shared = fixture.store.root().unwrap().join("proton-data");
            disk::create_dir(&shared).unwrap();
            disk::write(shared.join("saves"), b"preserve saves").unwrap();
            if relocate {
                let destination = fixture.directory.join("destination");
                fixture
                    .store
                    .relocate(
                        GAME,
                        &destination,
                        &source,
                        true,
                        &operation(OperationKind::Relocate),
                    )
                    .unwrap();
                assert_eq!(fixture.store.path(GAME).unwrap(), destination);
                assert_eq!(
                    disk::read(destination.join(GAME.executable().unwrap())).unwrap(),
                    b"test game data"
                );
            } else {
                fixture
                    .store
                    .uninstall(GAME, &source, true, &operation(OperationKind::Uninstall))
                    .unwrap();
                assert!(fixture.store.registered(GAME).unwrap().is_none());
            }
            assert!(!source.exists());
            assert_eq!(disk::read(shared.join("saves")).unwrap(), b"preserve saves");
            assert!(fixture.store.journal(GAME).unwrap().is_none());
            fixture
                .store
                .recover(GAME, &operation(OperationKind::Uninstall))
                .unwrap();
        }
    }

    #[test]
    fn copy_cancellation_and_verification_failure_retain_original_and_clean_owned_stage() {
        for cancel in [false, true] {
            let fixture = Fixture::new();
            let journal = staged(&fixture, !cancel, false);
            let staging = journal.staging().unwrap();
            let operation = operation(OperationKind::Relocate);
            let failure = if cancel {
                struct CancelReader(irmin::DownloadHandle);
                impl Read for CancelReader {
                    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                        output[0] = 1;
                        self.0.cancel();
                        Ok(1)
                    }
                }
                let mut output = File::create(staging.join("partial")).unwrap();
                copy_stream(
                    &mut CancelReader(operation.handle().clone()),
                    &mut output,
                    &operation,
                )
            } else {
                disk::write(staging.join(GAME.executable().unwrap()), b"corrupt game!").unwrap();
                verify_tree(
                    (&journal.original.path, &staging),
                    &journal.original.path,
                    &staging,
                    journal.source_device,
                    disk::metadata(&staging).unwrap().dev(),
                    0,
                    Some(&operation),
                )
            };
            assert!(failure.is_err());
            let error = fixture
                .store
                .recover_on_error(GAME, &operation, failure)
                .unwrap_err();
            assert!(!staging.exists(), "cancel={cancel}: {error:#}");
            assert_eq!(fixture.store.path(GAME).unwrap(), journal.original.path);
            assert_eq!(
                disk::read(journal.original.path.join(GAME.executable().unwrap())).unwrap(),
                b"test game data"
            );
            assert!(fixture.store.journal(GAME).unwrap().is_none());
        }
    }

    #[test]
    fn wrong_game_leases_cannot_mutate_or_recover_installations() {
        let fixture = Fixture::new();
        let source = managed(&fixture);
        let operation = Operations::default()
            .begin(OperationKind::Relocate, Some(Game::Nap))
            .unwrap();
        assert!(
            fixture
                .store
                .relocate(
                    GAME,
                    &fixture.directory.join("destination"),
                    &source,
                    true,
                    &operation
                )
                .is_err()
        );
        assert!(
            fixture
                .store
                .uninstall(GAME, &source, true, &operation)
                .is_err()
        );
        assert!(fixture.store.recover(GAME, &operation).is_err());
        assert!(source.join(GAME.executable().unwrap()).exists());
        assert!(fixture.store.journal(GAME).unwrap().is_none());
    }

    #[test]
    fn nested_tree_copy_verification_and_cleanup_use_bounded_stack_space() {
        let fixture = Fixture::new();
        let source = managed(&fixture);
        let mut directory = source.clone();
        for _ in 0..40 {
            directory.push("d");
        }
        disk::create_dir_all(&directory).unwrap();
        disk::write(directory.join("data"), b"nested data").unwrap();
        let destination = fixture.directory.join("destination");
        fixture
            .store
            .relocate(
                GAME,
                &destination,
                &source,
                true,
                &operation(OperationKind::Relocate),
            )
            .unwrap();
        assert!(!source.exists());
        assert_eq!(
            disk::read(
                destination
                    .join(directory.strip_prefix(&source).unwrap())
                    .join("data")
            )
            .unwrap(),
            b"nested data"
        );
    }

    #[test]
    fn relocation_stages_on_destination_side_even_when_parents_do_not_exist() {
        let fixture = Fixture::new();
        let source = managed(&fixture);
        let destination = fixture.directory.join("new-volume/games/destination");
        fixture
            .store
            .relocate(
                GAME,
                &destination,
                &source,
                false,
                &operation(OperationKind::Relocate),
            )
            .unwrap();
        let record = fixture.store.registered(GAME).unwrap().unwrap();
        // The always-copy path exercises the same algorithm on one or two
        // filesystems; source-to-destination rename (and EXDEV fallback) is absent.
        assert_ne!(
            disk::metadata(&source).unwrap().ino(),
            disk::metadata(&destination).unwrap().ino()
        );
        assert_eq!(record.path, destination);
        let staging = destination
            .parent()
            .unwrap()
            .join(format!(".elysiae-stage-bh3-{}", record.ownership_token));
        assert!(!staging.exists());
        assert_eq!(
            disk::read(source.join("nested/data")).unwrap(),
            disk::read(destination.join("nested/data")).unwrap()
        );
    }

    #[test]
    fn cleanup_never_removes_a_replacement_empty_directory_or_aliased_root() {
        let fixture = Fixture::new();
        let mut journal = staged(&fixture, false, false);
        let stage = journal.staging().unwrap();
        journal.step = Step::Cleaning;
        fixture.store.save_journal(GAME, &journal).unwrap();
        let preserved = fixture.directory.join("preserved-stage");
        disk::rename(&stage, &preserved).unwrap();
        disk::create_dir(&stage).unwrap();
        assert!(
            fixture
                .store
                .recover(GAME, &operation(OperationKind::Relocate))
                .is_err()
        );
        assert!(stage.exists());
        assert!(preserved.join(OWNER_MARKER).exists());
        journal.original.path = stage.clone();
        assert!(fixture.store.validate_journal(GAME, &journal).is_err());
        assert!(stage.exists());
    }

    #[test]
    fn cancelled_operation_does_not_start_maintenance() {
        let fixture = Fixture::new();
        let source = managed(&fixture);
        let operation = operation(OperationKind::Relocate);
        operation.handle().cancel();
        let destination = fixture.directory.join("destination");
        assert!(
            fixture
                .store
                .relocate(GAME, &destination, &source, true, &operation)
                .is_err()
        );
        assert!(
            fixture
                .store
                .uninstall(GAME, &source, true, &operation)
                .is_err()
        );
        assert!(!destination.exists());
        assert!(source.exists());
        assert!(fixture.store.journal(GAME).unwrap().is_none());
    }

    #[test]
    fn failed_registry_switch_keeps_source_and_rolls_back_after_metadata_is_available() {
        let fixture = Fixture::new();
        let journal = staged(&fixture, true, true);
        let original = disk::read(fixture.registry()).unwrap();
        let mut registry = fixture.store.load().unwrap();
        disk::remove_file(fixture.registry()).unwrap();
        disk::create_dir(fixture.registry()).unwrap();
        assert!(
            fixture
                .store
                .commit_relocation(GAME, &journal, &mut registry)
                .is_err()
        );
        assert!(journal.original.path.is_dir());
        assert!(journal.destination().unwrap().is_dir());
        disk::remove_dir(fixture.registry()).unwrap();
        disk::write(fixture.registry(), &original).unwrap();
        fixture
            .store
            .recover(GAME, &operation(OperationKind::Relocate))
            .unwrap();
        assert!(!journal.destination().unwrap().exists());
        assert!(!journal.staging().unwrap().exists());
        assert_eq!(disk::read(fixture.registry()).unwrap(), original);
        fixture
            .store
            .recover(GAME, &operation(OperationKind::Relocate))
            .unwrap();
    }

    #[test]
    fn committed_relocation_recovers_before_or_during_source_cleanup() {
        for partial_cleanup in [false, true] {
            let fixture = Fixture::new();
            let mut journal = staged(&fixture, true, true);
            let mut registry = fixture.store.load().unwrap();
            fixture
                .store
                .commit_relocation(GAME, &journal, &mut registry)
                .unwrap();
            if partial_cleanup {
                fixture.store.quarantine(GAME, &journal).unwrap();
                journal.step = Step::Cleaning;
                fixture.store.save_journal(GAME, &journal).unwrap();
                disk::remove_file(
                    journal
                        .quarantine()
                        .unwrap()
                        .join(GAME.executable().unwrap()),
                )
                .unwrap();
            }
            fixture
                .store
                .recover(GAME, &operation(OperationKind::Relocate))
                .unwrap();
            assert_eq!(
                fixture.store.path(GAME).unwrap(),
                journal.destination().unwrap()
            );
            assert!(!journal.original.path.exists());
            assert!(!journal.quarantine().unwrap().exists());
            assert_eq!(
                disk::read(
                    journal
                        .destination()
                        .unwrap()
                        .join(GAME.executable().unwrap())
                )
                .unwrap(),
                b"test game data"
            );
            fixture
                .store
                .recover(GAME, &operation(OperationKind::Relocate))
                .unwrap();
        }
    }

    #[test]
    fn interrupted_uninstall_restores_before_commit_and_finishes_after_commit() {
        for state in 0..4 {
            let fixture = Fixture::new();
            let source = managed(&fixture);
            let mut registry = fixture.store.load().unwrap();
            let mut journal = Journal::new(
                GAME,
                registry.installations[GAME.code()].clone(),
                Action::Uninstall,
            )
            .unwrap();
            fixture.store.save_journal(GAME, &journal).unwrap();
            fixture.store.quarantine(GAME, &journal).unwrap();
            if state == 0 {
                let original_bytes = disk::read(fixture.registry()).unwrap();
                disk::remove_file(fixture.registry()).unwrap();
                disk::create_dir(fixture.registry()).unwrap();
                registry.installations.remove(GAME.code());
                assert!(fixture.store.save_registry(&registry).is_err());
                assert!(
                    journal
                        .quarantine()
                        .unwrap()
                        .join(GAME.executable().unwrap())
                        .exists()
                );
                disk::remove_dir(fixture.registry()).unwrap();
                disk::write(fixture.registry(), original_bytes).unwrap();
            }
            if state > 0 {
                registry.installations.remove(GAME.code());
                fixture.store.save_registry(&registry).unwrap();
                if state > 1 {
                    journal.step = Step::Cleaning;
                    fixture.store.save_journal(GAME, &journal).unwrap();
                    if state == 2 {
                        disk::remove_file(
                            journal
                                .quarantine()
                                .unwrap()
                                .join(GAME.executable().unwrap()),
                        )
                        .unwrap();
                    } else {
                        let quarantine = journal.quarantine().unwrap();
                        let directory = open_without_links(&quarantine, true).unwrap();
                        remove_contents(&directory, journal.source_device, 0).unwrap();
                        disk::remove_file(quarantine.join(OWNER_MARKER)).unwrap();
                    }
                }
            }
            fixture
                .store
                .recover(GAME, &operation(OperationKind::Uninstall))
                .unwrap();
            assert_eq!(source.exists(), state == 0);
            assert_eq!(
                fixture.store.registered(GAME).unwrap().is_some(),
                state == 0
            );
            assert!(!journal.quarantine().unwrap().exists());
            assert!(fixture.store.journal(GAME).unwrap().is_none());
            fixture
                .store
                .recover(GAME, &operation(OperationKind::Uninstall))
                .unwrap();
        }
    }

    #[test]
    fn journals_block_new_mutations_and_unconfirmed_strays_are_not_adopted_or_deleted() {
        let fixture = Fixture::new();
        let journal = staged(&fixture, false, false);
        let operation = operation(OperationKind::Install);
        assert!(fixture.store.prepare_install(GAME, &operation).is_err());
        assert!(fixture.store.writable_path(GAME, &operation).is_err());
        assert!(
            fixture
                .store
                .import(GAME, &journal.original.path, true, &operation)
                .is_err()
        );
        let stranger = fixture
            .directory
            .join(format!(".elysiae-stage-bh3-{}", Uuid::new_v4()));
        disk::create_dir(&stranger).unwrap();
        disk::write(stranger.join("keep"), b"unconfirmed").unwrap();
        fixture.store.recover(GAME, &operation).unwrap();
        fixture.store.recover(GAME, &operation).unwrap();
        assert_eq!(disk::read(stranger.join("keep")).unwrap(), b"unconfirmed");
    }

    #[test]
    fn corrupted_or_oversized_journals_never_authorize_cleanup() {
        let fixture = Fixture::new();
        let journal = staged(&fixture, false, false);
        let journal_path = fixture.store.journal_path(GAME).unwrap();
        let original = disk::read(&journal_path).unwrap();
        let mut variants = vec![vec![b' '; MAX_JOURNAL as usize + 1], b"{".to_vec()];
        for (key, value) in [
            ("schema_version", serde_json::json!(2)),
            ("id", serde_json::json!("../unrelated")),
            ("game", serde_json::json!("unknown")),
            ("quarantine", serde_json::json!(fixture.directory)),
        ] {
            let mut bad: serde_json::Value = serde_json::from_slice(&original).unwrap();
            bad[key] = value;
            variants.push(serde_json::to_vec(&bad).unwrap());
        }
        let mut bad: serde_json::Value = serde_json::from_slice(&original).unwrap();
        bad["original"]["path"] = serde_json::json!("/");
        variants.push(serde_json::to_vec(&bad).unwrap());
        for bytes in variants {
            disk::write(&journal_path, &bytes).unwrap();
            assert!(
                fixture
                    .store
                    .recover(GAME, &operation(OperationKind::Relocate))
                    .is_err()
            );
            assert_eq!(disk::read(&journal_path).unwrap(), bytes);
            assert!(journal.staging().unwrap().is_dir());
            assert!(journal.original.path.is_dir());
        }
        disk::write(&journal_path, original).unwrap();
        disk::write(
            journal.staging().unwrap().join(OWNER_MARKER),
            b"wrong owner",
        )
        .unwrap();
        assert!(
            fixture
                .store
                .recover(GAME, &operation(OperationKind::Relocate))
                .is_err()
        );
        assert!(journal.staging().unwrap().exists());
        assert!(journal.original.path.exists());
    }

    #[test]
    fn cleanup_unlinks_escaping_links_but_refuses_replaced_source_roots() {
        let fixture = Fixture::new();
        let journal = staged(&fixture, false, false);
        let outside = fixture.game("unrelated", GAME);
        let stage = journal.staging().unwrap();
        symlink(&outside, stage.join("escape")).unwrap();
        fixture
            .store
            .recover(GAME, &operation(OperationKind::Relocate))
            .unwrap();
        assert!(!stage.exists());
        assert_eq!(
            disk::read(outside.join(GAME.executable().unwrap())).unwrap(),
            b"fixture, not executable"
        );
        let fixture = Fixture::new();
        let journal = staged(&fixture, false, false);
        let stage = journal.staging().unwrap();
        let moved = fixture.directory.join("moved-source");
        disk::rename(&journal.original.path, &moved).unwrap();
        disk::create_dir(&journal.original.path).unwrap();
        disk::copy(
            moved.join(OWNER_MARKER),
            journal.original.path.join(OWNER_MARKER),
        )
        .unwrap();
        assert!(
            fixture
                .store
                .recover(GAME, &operation(OperationKind::Relocate))
                .is_err()
        );
        assert!(stage.exists());
        assert!(moved.join(GAME.executable().unwrap()).exists());
    }

    #[test]
    fn cleanup_recovers_a_link_chain_after_its_alias_has_already_been_removed() {
        let fixture = Fixture::new();
        let source = managed(&fixture);
        disk::create_dir_all(source.join("a/b")).unwrap();
        disk::write(source.join("data"), b"owned data").unwrap();
        disk::write(fixture.directory.join("data"), b"unrelated data").unwrap();
        symlink("a/b", source.join("alias")).unwrap();
        symlink("alias/../../data", source.join("link")).unwrap();
        assert!(safe_link(&source, &source.join("link")).is_ok());
        let mut registry = fixture.store.load().unwrap();
        let mut journal = Journal::new(
            GAME,
            registry.installations[GAME.code()].clone(),
            Action::Uninstall,
        )
        .unwrap();
        fixture.store.save_journal(GAME, &journal).unwrap();
        fixture.store.quarantine(GAME, &journal).unwrap();
        registry.installations.remove(GAME.code());
        fixture.store.save_registry(&registry).unwrap();
        journal.step = Step::Cleaning;
        fixture.store.save_journal(GAME, &journal).unwrap();
        let quarantine = journal.quarantine().unwrap();
        disk::remove_file(quarantine.join("alias")).unwrap();
        assert!(safe_link(&quarantine, &quarantine.join("link")).is_err());
        fixture
            .store
            .recover(GAME, &operation(OperationKind::Uninstall))
            .unwrap();
        fixture
            .store
            .recover(GAME, &operation(OperationKind::Uninstall))
            .unwrap();
        assert!(!quarantine.exists());
        assert_eq!(
            disk::read(fixture.directory.join("data")).unwrap(),
            b"unrelated data"
        );
    }

    #[test]
    fn source_changes_after_initial_verification_restore_original_instead_of_deleting_it() {
        let fixture = Fixture::new();
        let journal = staged(&fixture, true, true);
        let mut registry = fixture.store.load().unwrap();
        fixture
            .store
            .commit_relocation(GAME, &journal, &mut registry)
            .unwrap();
        disk::write(
            journal.original.path.join("nested/late-save"),
            b"must not be lost",
        )
        .unwrap();
        disk::write(
            journal.original.path.join("nested/data"),
            b"changed after verification",
        )
        .unwrap();
        let error = fixture
            .store
            .recover(GAME, &operation(OperationKind::Relocate))
            .unwrap_err();
        assert!(format!("{error:#}").contains("restored original installation"));
        assert_eq!(fixture.store.path(GAME).unwrap(), journal.original.path);
        assert_eq!(
            disk::read(journal.original.path.join("nested/late-save")).unwrap(),
            b"must not be lost"
        );
        assert_eq!(
            disk::read(journal.original.path.join("nested/data")).unwrap(),
            b"changed after verification"
        );
        assert!(!journal.destination().unwrap().exists());
        assert!(!journal.quarantine().unwrap().exists());
        assert!(fixture.store.journal(GAME).unwrap().is_none());
    }

    #[test]
    fn root_relative_openat2_refuses_mount_crossings_but_accepts_the_selected_mount_root() {
        // No mounts or user files are created. /proc provides a Linux mount
        // boundary; NO_XDEV applies equally to bind mounts with identical st_dev.
        let root = open_without_links(Path::new("/"), true).unwrap();
        let error =
            open_beneath(&root, Path::new("proc"), libc::O_PATH | libc::O_DIRECTORY).unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(libc::EXDEV)
        );
        let proc_root = open_without_links(Path::new("/proc"), true).unwrap();
        assert!(open_beneath(&proc_root, Path::new("."), libc::O_PATH | libc::O_DIRECTORY).is_ok());
    }

    #[test]
    fn copy_never_reads_through_links_and_rejects_special_or_different_device_entries() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let fixture = Fixture::new();
        let source = managed(&fixture);
        let device = disk::metadata(&source).unwrap().dev();
        let alias = source.join("alias");
        symlink("nested", &alias).unwrap();
        assert!(open_regular(&source, &alias.join("data"), device).is_err());
        assert!(metadata(&source, &source.join("nested/data"), device.wrapping_add(1)).is_err());
        let fifo = source.join("fifo");
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let destination = fixture.directory.join("destination");
        assert!(
            fixture
                .store
                .relocate(
                    GAME,
                    &destination,
                    &source,
                    false,
                    &operation(OperationKind::Relocate)
                )
                .is_err()
        );
        assert!(!destination.exists());
        disk::remove_file(fifo).unwrap();
        symlink("../../unrelated", source.join("escape")).unwrap();
        assert!(
            fixture
                .store
                .relocate(
                    GAME,
                    &destination,
                    &source,
                    false,
                    &operation(OperationKind::Relocate)
                )
                .is_err()
        );
        assert!(source.exists());
        assert!(fixture.store.journal(GAME).unwrap().is_none());
    }
}
