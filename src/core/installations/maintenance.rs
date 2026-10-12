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

/// Recorded maintenance intent and directory identities used to guide recovery.
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

/// Relocation or managed-deletion intent, distinct from unregister-only operations.
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

/// Copy and cleanup progress; registry contents determine whether commit occurred.
#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
enum Step {
    Copying,
    Ready,
    Cleaning,
}

impl Journal {
    /// Capture source identity and transaction intent before staging or quarantine.
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

    /// Return the relocation destination, rejecting uninstall transactions.
    fn destination(&self) -> Result<&Path> {
        match &self.action {
            Action::Relocate { destination, .. } => Ok(destination),
            Action::Uninstall => bail!("Uninstall has no destination"),
        }
    }

    /// Derive a transaction path from its parent, role, game, and transaction ID.
    fn generated(&self, parent: &Path, kind: &str) -> PathBuf {
        parent.join(format!(".elysiae-{kind}-{}-{}", self.game, self.id))
    }

    /// Derive the staging directory beside the relocation destination.
    fn staging(&self) -> Result<PathBuf> {
        Ok(self.generated(
            self.destination()?
                .parent()
                .context("Destination has no parent")?,
            "stage",
        ))
    }

    /// Derive the quarantine directory beside the original installation.
    fn quarantine(&self) -> Result<PathBuf> {
        Ok(self.generated(
            self.original
                .path
                .parent()
                .context("Source has no parent")?,
            "trash",
        ))
    }

    /// Describe a managed relocation root using the transaction ID as its token.
    fn target(&self, path: PathBuf) -> Installation {
        Installation {
            path,
            managed: true,
            allow_modify: true,
            ownership_token: self.id.clone(),
        }
    }

    /// Check the path's recorded staging identity and transaction ownership marker.
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

    /// Check a source path's recorded identity and, when managed, ownership marker.
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
    /// Resolve the game's maintenance journal within this store.
    fn journal_path(&self, game: Game) -> Result<PathBuf> {
        self.resolve(&PathBuf::from(JOURNALS).join(format!("{}.json", game.code())))
    }

    /// Read a size-bounded journal and validate its schema and recorded intent.
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

    /// Check journal intent and derived paths without granting cleanup authority.
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

    /// Validate and persist bounded maintenance intent with its directory entries.
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

    /// Remove a completed journal and persist its parent directory entry change.
    fn clear_journal(&self, game: Game) -> Result<()> {
        let path = self.journal_path(game)?;
        disk::remove_file(&path)?;
        sync_directory(path.parent().context("Journal has no parent")?)
    }

    /// Require all game maintenance journals to be cleared before new mutations.
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

    /// Save the transaction's registry state and sync the file and its parent.
    fn save_registry(&self, registry: &super::Registry) -> Result<()> {
        self.save(registry)?;
        sync_file_and_parent(&self.resolve(Path::new(super::REGISTRY))?)
    }

    /// Copy and verify into destination-side staging before committing registration.
    /// Delete the confirmed source only when managed and explicitly requested.
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

    /// Promote verified staging to the destination, then commit its registration.
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

    /// Unregister only, or journal quarantine and deletion of a confirmed managed root.
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

    /// Attempt rollback or cleanup after failure while retaining the original error.
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

    /// Recover any pending journal under the game's matching operation.
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

    /// Use registration as commit evidence to roll back or finish journaled maintenance.
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

    /// Move the checked source to quarantine without replacing existing entries.
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

    /// Clean an owned staging or quarantine tree, or finish an empty journaled cleanup.
    /// Content deletion uses pinned descriptors; final root removal uses its path.
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

/// Check whether either path contains the other, including equality.
fn overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

/// Create missing canonical destination parents and persist their directory entries.
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

/// Require an absent or empty canonical directory, returning whether it exists.
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

/// Open a directory by path and sync its metadata to disk.
fn sync_directory(path: &Path) -> Result<()> {
    Ok(File::open(path)?.sync_all()?)
}
/// Sync a file opened by path and the directory containing its entry.
fn sync_file_and_parent(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    sync_directory(path.parent().context("Metadata has no parent directory")?)
}

/// Rename between pinned parent descriptors using the supplied rename flags.
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

/// Check cancellation when an operation accompanies the traversal.
fn cancelled(operation: Option<&Operation>) -> Result<()> {
    if let Some(operation) = operation {
        operation.check_cancelled()?;
    }
    Ok(())
}

/// Inspect a root-relative entry, rejecting mount crossings and special files.
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

/// Measure a bounded tree excluding its owner marker, optionally checking link targets.
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

/// Read a relative symlink target and check that its chain stays within the root.
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

/// Resolve a bounded symlink chain within a root, allowing missing internal targets.
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

/// Open a root-relative regular file and confirm its previously observed identity.
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

/// Copy buffered bytes while checking cancellation between reads and writes.
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

/// Open and pin a destination path before copying the source tree into it.
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

/// Copy into a pinned destination directory, preserving permissions and internal links.
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

/// Compare tree contents, permissions, and links while excluding root ownership markers.
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
/// Open a selected path without symlink traversal, allowing mount boundaries.
fn open_without_links(path: &Path, directory: bool) -> Result<File> {
    open_at(
        libc::AT_FDCWD,
        path,
        libc::O_RDONLY | libc::O_NONBLOCK | if directory { libc::O_DIRECTORY } else { 0 },
        0x04,
    ) // RESOLVE_NO_SYMLINKS
}

/// Open beneath a pinned directory without crossing mounts or following symlinks.
fn open_beneath(directory: &File, path: &Path, flags: libc::c_int) -> Result<File> {
    use std::os::fd::AsRawFd;
    open_at(directory.as_raw_fd(), path, flags, 0x01 | 0x04 | 0x08) // RESOLVE_NO_XDEV | NO_SYMLINKS | BENEATH
}

/// Pin a selected root and open a path relative to it with descendant restrictions.
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
/// Apply Linux openat2 flags and resolution restrictions, returning an owned descriptor.
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
    /// Linux openat2 options in the kernel's expected C layout.
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

/// Build a procfs path referring to a live directory descriptor.
fn fd_path(directory: &File) -> PathBuf {
    use std::os::fd::AsRawFd;
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
}

/// Delete entries relative to pinned directories, retaining the root ownership marker.
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
