//! One owner for mutations and the shared Wine prefix. The lease stays alive
//! until all work, including blocking workers and launched processes, has ended.
use std::{
    fs::File,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{Context, Result, bail, ensure};
use irmin::DownloadHandle;
use uuid::Uuid;

use super::game::Game;

/// Backend actions that require exclusive installation or runtime ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Install,
    Update,
    Preinstall,
    Repair,
    Components,
    Launch,
    Import,
    Relocate,
    Uninstall,
    /// Re-establish quiescence after an interrupted or uncertain Wine session.
    Recovery,
}

impl OperationKind {
    /// Checks whether this action supports download pause and resume controls.
    fn supports_download_controls(self) -> bool {
        matches!(
            self,
            Self::Install | Self::Update | Self::Preinstall | Self::Repair
        )
    }

    /// Checks whether this action supports cancellation in an interruptible phase.
    fn supports_cancellation(self) -> bool {
        self.supports_download_controls() || matches!(self, Self::Relocate | Self::Components)
    }
}

/// Current stage or final outcome of an owned backend operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Running,
    Paused,
    Verifying,
    Committing,
    Completed,
    Cancelled,
    Failed,
}

/// Snapshot of an operation's identity, target game, phase, and error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationStatus {
    pub id: Uuid,
    pub kind: OperationKind,
    pub game: Option<Game>,
    pub phase: Phase,
    pub error: Option<String>,
}

/// Active operation state together with its controls and optional profile lock.
struct ActiveOperation {
    status: OperationStatus,
    handle: DownloadHandle,
    _profile_lock: Option<File>,
}

/// Tracks the active operation, last result, and profile-locking policy.
#[derive(Default)]
struct State {
    persistent: bool,
    active: Option<ActiveOperation>,
    last: Option<OperationStatus>,
}

/// Shared coordinator that allows only one backend operation at a time.
#[derive(Clone, Default)]
pub struct Operations(Arc<Mutex<State>>);

/// Returns the process-wide coordinator with cross-process profile locking enabled.
pub fn operations() -> &'static Operations {
    static OPERATIONS: OnceLock<Operations> = OnceLock::new();
    OPERATIONS.get_or_init(|| {
        Operations(Arc::new(Mutex::new(State {
            persistent: true,
            ..State::default()
        })))
    })
}

/// Acquires an exclusive profile lock using an unshared, non-symlink file.
fn lock_profile(path: &Path) -> Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    std::fs::create_dir_all(path.parent().context("Operation lock has no parent")?)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.nlink() == 1,
        "Operation lock must be an unshared regular file"
    );
    file.try_lock()
        .context("Another Elysiae process owns this profile, or the filesystem cannot lock it")?;
    Ok(file)
}

impl Operations {
    /// Acquires ownership for a new operation, refusing conflicting work or sessions.
    pub fn begin(&self, kind: OperationKind, game: Option<Game>) -> Result<Operation> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        if let Some(active) = &state.active {
            bail!("An operation is already active: {:?}", active.status.kind);
        }
        let profile_lock = if state.persistent {
            Some(lock_profile(&super::fs::full_path(
                Some("operations.lock".into()),
                None,
            )?)?)
        } else {
            None
        };
        if state.persistent && kind != OperationKind::Recovery {
            super::proton_manager::ensure_no_session()?;
        }
        let status = OperationStatus {
            id: Uuid::new_v4(),
            kind,
            game,
            phase: Phase::Running,
            error: None,
        };
        let handle = DownloadHandle::new();
        state.active = Some(ActiveOperation {
            status: status.clone(),
            handle: handle.clone(),
            _profile_lock: profile_lock,
        });
        Ok(Operation {
            owner: self.clone(),
            id: status.id,
            handle,
            finished: false,
        })
    }

    /// Returns the active operation's status, or the most recent final status.
    pub fn status(&self) -> Result<Option<OperationStatus>> {
        let state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        Ok(state
            .active
            .as_ref()
            .map(|active| active.status.clone())
            .or_else(|| state.last.clone()))
    }

    /// Checks whether an operation currently holds ownership.
    pub fn is_busy(&self) -> Result<bool> {
        Ok(self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?
            .active
            .is_some())
    }

    /// Pauses or resumes an active download when its phase allows it.
    pub fn toggle_pause(&self) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        let active = state
            .active
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No active operation"))?;
        ensure!(
            active.status.kind.supports_download_controls()
                && matches!(active.status.phase, Phase::Running | Phase::Paused),
            "This phase cannot be paused safely"
        );
        ensure!(
            !active.handle.is_cancelled(),
            "Cancellation has already been requested"
        );
        if active.handle.is_paused() {
            active.handle.resume();
            active.status.phase = Phase::Running;
        } else {
            active.handle.pause();
            active.status.phase = Phase::Paused;
        }
        Ok(())
    }

    /// Requests safe cancellation while retaining ownership until the worker exits.
    pub fn cancel(&self) -> Result<()> {
        let state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        let active = state
            .active
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No active operation"))?;
        ensure!(
            active.status.kind.supports_cancellation()
                && matches!(active.status.phase, Phase::Running | Phase::Paused),
            "This phase must finish before cancellation is safe"
        );
        active.handle.cancel();
        // Do not release the lease: the worker still owns files until it exits.
        Ok(())
    }
}

/// Exclusive operation lease held until its worker finishes or drops it.
pub struct Operation {
    owner: Operations,
    id: Uuid,
    handle: DownloadHandle,
    finished: bool,
}

impl Operation {
    /// Returns the download control handle associated with this lease.
    pub fn handle(&self) -> &DownloadHandle {
        &self.handle
    }

    /// Checks that this lease still owns the requested game's installation.
    pub fn check_game(&self, game: Game) -> Result<()> {
        let state = self
            .owner
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        ensure!(
            state.active.as_ref().is_some_and(
                |active| active.status.id == self.id && active.status.game == Some(game)
            ),
            "Operation does not own this game's installation"
        );
        Ok(())
    }

    /// Fails if cancellation has been requested for this operation.
    pub fn check_cancelled(&self) -> Result<()> {
        ensure!(!self.handle.is_cancelled(), "Operation cancelled");
        Ok(())
    }

    /// Updates a nonterminal phase after checking ownership and cancellation.
    pub fn set_phase(&self, phase: Phase) -> Result<()> {
        ensure!(
            matches!(phase, Phase::Running | Phase::Verifying | Phase::Committing),
            "Terminal states must be recorded by finish"
        );
        let mut state = self
            .owner
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        let active = state
            .active
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Operation is no longer active"))?;
        ensure!(active.status.id == self.id, "Operation ownership changed");
        ensure!(
            !active.handle.is_cancelled(),
            "Operation cancelled before phase transition"
        );
        active.status.phase = phase;
        Ok(())
    }

    /// Records the final result and releases ownership, returning the result unchanged.
    pub fn finish<T>(mut self, result: Result<T>) -> Result<T> {
        let phase = if result.is_ok() {
            Phase::Completed
        } else if self.handle.is_cancelled() {
            Phase::Cancelled
        } else {
            Phase::Failed
        };
        self.release(
            phase,
            result.as_ref().err().map(|error| format!("{error:#}")),
        );
        self.finished = true;
        result
    }

    /// Stores the terminal status and releases ownership if this lease is still active.
    fn release(&self, phase: Phase, error: Option<String>) {
        let Ok(mut state) = self.owner.0.lock() else {
            log::error!("Operation lock poisoned during completion");
            return;
        };
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.status.id == self.id)
        {
            let mut status = state.active.take().unwrap().status;
            status.phase = phase;
            status.error = error;
            state.last = Some(status);
        }
    }
}

impl Drop for Operation {
    /// Releases an unfinished lease, requesting cancellation and recording its status.
    fn drop(&mut self) {
        if !self.finished {
            let phase = if self.handle.is_cancelled() {
                Phase::Cancelled
            } else {
                Phase::Failed
            };
            self.handle.cancel();
            self.release(phase, Some("Operation ended without completion".into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks pause, cancellation and completion retain and release exclusive ownership.
    #[test]
    fn operation_lifecycle_keeps_exclusion_and_resets_controls() {
        let owner = Operations::default();
        let operation = owner
            .begin(OperationKind::Install, Some(Game::Bh3))
            .unwrap();
        owner.toggle_pause().unwrap();
        assert!(operation.handle().is_paused());
        owner.toggle_pause().unwrap();
        assert!(!operation.handle().is_paused());
        owner.cancel().unwrap();
        assert!(operation.check_cancelled().is_err());
        assert!(operation.set_phase(Phase::Committing).is_err());
        assert!(owner.is_busy().unwrap());
        assert!(
            owner
                .begin(OperationKind::Install, Some(Game::Nap))
                .is_err()
        );
        assert!(
            operation
                .finish::<()>(Err(anyhow::anyhow!("cancelled")))
                .is_err()
        );
        assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Cancelled);
        let next = owner
            .begin(OperationKind::Install, Some(Game::Nap))
            .unwrap();
        assert!(!next.handle().is_cancelled());
        next.finish(Ok(())).unwrap();
        assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Completed);
        drop(owner.begin(OperationKind::Update, Some(Game::Bh3)).unwrap());
        assert!(!owner.is_busy().unwrap());
    }

    /// Checks verification and commit reject controls and retain failure details.
    #[test]
    fn noncancellable_phases_keep_ownership_and_retain_errors() {
        for (kind, phase) in [
            (OperationKind::Repair, Phase::Verifying),
            (OperationKind::Relocate, Phase::Committing),
        ] {
            let owner = Operations::default();
            let operation = owner.begin(kind, Some(Game::Bh3)).unwrap();
            operation.set_phase(phase).unwrap();
            assert!(owner.cancel().is_err());
            assert!(owner.toggle_pause().is_err());
            assert!(operation.check_game(Game::Nap).is_err());
            assert!(
                operation
                    .finish::<()>(Err(anyhow::anyhow!("failed")))
                    .is_err()
            );
            let status = owner.status().unwrap().unwrap();
            assert_eq!(status.phase, Phase::Failed);
            assert_eq!(status.error.as_deref(), Some("failed"));
        }
    }

    /// Checks exclusive profile locking and refusal to follow lock-file symlinks.
    #[test]
    fn independent_process_handles_cannot_share_a_profile_lock() {
        let root = std::env::temp_dir().join(format!("elysiae-lock-{}", Uuid::new_v4()));
        let path = root.join("operations.lock");
        let first = lock_profile(&path).unwrap();
        assert!(lock_profile(&path).is_err());
        drop(first);
        drop(lock_profile(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("outside", &path).unwrap();
        assert!(lock_profile(&path).is_err());
        assert!(!root.join("outside").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
