//! Own game mutations and supervised Wine sessions independently. Compatible
//! work shares one profile lock until the last lease (including workers) ends.
use std::{
    fs::File,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{Context, Result, ensure};
use irmin::DownloadHandle;
use uuid::Uuid;

use super::game::Game;
use crate::util::progress::{self, ProgressEvent, ProgressReporter, ProgressUpdate};

/// Backend actions with game-file or shared-runtime ownership requirements.
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
    Recovery,
}

impl OperationKind {
    fn supports_download_controls(self) -> bool {
        matches!(
            self,
            Self::Install | Self::Update | Self::Preinstall | Self::Repair
        )
    }

    fn supports_cancellation(self) -> bool {
        self.supports_download_controls() || matches!(self, Self::Relocate | Self::Components)
    }
}

/// Only a supervised launch and an independent transfer may coexist. Preinstall
/// writes future-version chunks, not the installed executable or live assets.
fn compatible(a: OperationKind, ag: Option<Game>, b: OperationKind, bg: Option<Game>) -> bool {
    let (transfer, transfer_game, launch_game) = match (a, b) {
        (OperationKind::Launch, transfer) => (transfer, bg, ag),
        (transfer, OperationKind::Launch) => (transfer, ag, bg),
        _ => return false,
    };
    transfer_game.is_some()
        && launch_game.is_some()
        && match transfer {
            OperationKind::Preinstall => true,
            OperationKind::Install | OperationKind::Update => transfer_game != launch_game,
            _ => false,
        }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationStatus {
    pub id: Uuid,
    pub kind: OperationKind,
    pub game: Option<Game>,
    pub phase: Phase,
    pub error: Option<String>,
}

struct ActiveOperation {
    status: OperationStatus,
    handle: DownloadHandle,
}

#[derive(Default)]
struct State {
    persistent: bool,
    active: Vec<ActiveOperation>,
    #[cfg(test)]
    last: Option<OperationStatus>,
    // Owned by the group, never by whichever operation happens to finish first.
    profile_lock: Option<File>,
}

#[derive(Clone, Default)]
pub struct Operations(Arc<Mutex<State>>);

pub fn operations() -> &'static Operations {
    static OPERATIONS: OnceLock<Operations> = OnceLock::new();
    OPERATIONS.get_or_init(|| {
        Operations(Arc::new(Mutex::new(State {
            persistent: true,
            ..State::default()
        })))
    })
}

/// Acquires a cross-process profile lock without following links or sharing files.
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
    /// Acquires a lease after checking every live owner, retaining the existing
    /// profile lock when joining a compatible in-process operation.
    pub fn begin(&self, kind: OperationKind, game: Option<Game>) -> Result<Operation> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        for active in &state.active {
            ensure!(
                compatible(kind, game, active.status.kind, active.status.game),
                "Conflicting operation: {:?}",
                active.status.kind
            );
        }
        let new_lock = if state.persistent && state.profile_lock.is_none() {
            Some(lock_profile(&super::fs::full_path(
                Some("operations.lock".into()),
                None,
            )?)?)
        } else {
            None
        };
        // Only our own retained Launch lease authorizes transfers alongside a
        // session fence. Unknown/orphaned fences still require exclusive recovery.
        let supervised = state
            .active
            .iter()
            .any(|a| a.status.kind == OperationKind::Launch);
        if state.persistent && kind != OperationKind::Recovery && !supervised {
            super::proton_manager::ensure_no_session()?;
        }
        if let Some(lock) = new_lock {
            state.profile_lock = Some(lock);
        }
        let status = OperationStatus {
            id: Uuid::new_v4(),
            kind,
            game,
            phase: Phase::Running,
            error: None,
        };
        let handle = DownloadHandle::new();
        state.active.push(ActiveOperation {
            status: status.clone(),
            handle: handle.clone(),
        });
        progress::publish(ProgressUpdate {
            operation: status.clone(),
            event: ProgressEvent::Started,
        });
        Ok(Operation {
            owner: self.clone(),
            id: status.id,
            handle,
            finished: false,
        })
    }

    /// Read-only UI eligibility hint. begin() rechecks ownership and disk fences.
    pub fn can_begin(&self, kind: OperationKind, game: Option<Game>) -> Result<bool> {
        let state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        Ok(state
            .active
            .iter()
            .all(|a| compatible(kind, game, a.status.kind, a.status.game)))
    }

    pub fn active(&self) -> Result<Vec<OperationStatus>> {
        let state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        Ok(state.active.iter().map(|a| a.status.clone()).collect())
    }

    /// Inspect active or last-completed status in tests, preferring a transfer.
    #[cfg(test)]
    pub fn status(&self) -> Result<Option<OperationStatus>> {
        let state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        Ok(state
            .active
            .iter()
            .find(|a| a.status.kind != OperationKind::Launch)
            .or_else(|| state.active.first())
            .map(|a| a.status.clone())
            .or_else(|| state.last.clone()))
    }

    pub fn is_busy(&self) -> Result<bool> {
        Ok(!self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?
            .active
            .is_empty())
    }

    pub fn toggle_pause(&self) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        let active = state
            .active
            .iter_mut()
            .find(|a| a.status.kind.supports_download_controls())
            .context("No active download")?;
        ensure!(
            matches!(active.status.phase, Phase::Running | Phase::Paused),
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

    pub fn cancel(&self) -> Result<()> {
        let state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        let active = state
            .active
            .iter()
            .find(|a| a.status.kind.supports_cancellation())
            .context("No cancellable operation")?;
        ensure!(
            matches!(active.status.phase, Phase::Running | Phase::Paused),
            "This phase must finish before cancellation is safe"
        );
        active.handle.cancel();
        Ok(())
    }
}

/// Worker-owned lease; dropping an async waiter must not drop this worker's lease.
pub struct Operation {
    owner: Operations,
    id: Uuid,
    handle: DownloadHandle,
    finished: bool,
}

impl Operation {
    pub fn handle(&self) -> &DownloadHandle {
        &self.handle
    }

    pub fn reporter(&self) -> Result<ProgressReporter> {
        let state = self
            .owner
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        let active = state
            .active
            .iter()
            .find(|a| a.status.id == self.id)
            .context("Operation is no longer active")?;
        Ok(ProgressReporter(active.status.clone()))
    }

    pub fn check_game(&self, game: Game) -> Result<()> {
        let state = self
            .owner
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        ensure!(
            state.active.iter().any(|a| a.status.id == self.id
                && a.status.game == Some(game)
                && a.status.kind != OperationKind::Launch),
            "Operation does not own this game's installation"
        );
        Ok(())
    }

    /// Downgrade an update lease once remote metadata confirms a staging-only
    /// preinstall. It can never be promoted back to a live-file mutation.
    pub fn mark_preinstall(&self) -> Result<()> {
        let mut state = self
            .owner
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        let active = state
            .active
            .iter_mut()
            .find(|a| a.status.id == self.id)
            .context("Operation is no longer active")?;
        ensure!(
            matches!(
                active.status.kind,
                OperationKind::Update | OperationKind::Preinstall
            ),
            "Only updates can become preinstalls"
        );
        active.status.kind = OperationKind::Preinstall;
        Ok(())
    }

    pub fn check_cancelled(&self) -> Result<()> {
        ensure!(!self.handle.is_cancelled(), "Operation cancelled");
        Ok(())
    }

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
            .iter_mut()
            .find(|a| a.status.id == self.id)
            .context("Operation is no longer active")?;
        ensure!(
            !active.handle.is_cancelled(),
            "Operation cancelled before phase transition"
        );
        active.status.phase = phase;
        Ok(())
    }

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

    fn release(&self, phase: Phase, error: Option<String>) {
        let Ok(mut state) = self.owner.0.lock() else {
            log::error!("Operation lock poisoned during completion");
            return;
        };
        if let Some(index) = state.active.iter().position(|a| a.status.id == self.id) {
            let mut status = state.active.remove(index).status;
            status.phase = phase;
            status.error = error.clone();
            progress::publish(ProgressUpdate {
                operation: status.clone(),
                event: ProgressEvent::Finished { error },
            });
            #[cfg(test)]
            {
                state.last = Some(status);
            }
            if state.active.is_empty() {
                state.profile_lock = None;
            }
        }
    }
}

impl Drop for Operation {
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

    #[test]
    fn transfers_and_launches_coexist_in_either_order() {
        for kind in [
            OperationKind::Install,
            OperationKind::Update,
            OperationKind::Preinstall,
        ] {
            for launch_first in [false, true] {
                let owner = Operations::default();
                let (first_kind, first_game, second_kind, second_game) = if launch_first {
                    (OperationKind::Launch, Game::Hk4e, kind, Game::Hkrpg)
                } else {
                    (kind, Game::Hkrpg, OperationKind::Launch, Game::Hk4e)
                };
                let first = owner.begin(first_kind, Some(first_game)).unwrap();
                let second = owner.begin(second_kind, Some(second_game)).unwrap();
                assert_eq!(owner.active().unwrap().len(), 2);
                assert!(
                    owner
                        .begin(OperationKind::Install, Some(Game::Nap))
                        .is_err()
                );
                assert!(owner.begin(OperationKind::Launch, Some(Game::Nap)).is_err());
                assert!(owner.begin(OperationKind::Components, None).is_err());
                assert!(owner.begin(OperationKind::Recovery, None).is_err());
                first.finish(Ok(())).unwrap();
                assert!(owner.is_busy().unwrap());
                second.finish(Ok(())).unwrap();
                assert!(!owner.is_busy().unwrap());
            }
        }
    }

    #[test]
    fn same_game_requires_staging_only_preinstall() {
        let owner = Operations::default();
        let update = owner.begin(OperationKind::Update, Some(Game::Bh3)).unwrap();
        assert!(
            !owner
                .can_begin(OperationKind::Launch, Some(Game::Bh3))
                .unwrap()
        );
        update.mark_preinstall().unwrap();
        let launch = owner.begin(OperationKind::Launch, Some(Game::Bh3)).unwrap();
        assert!(launch.check_game(Game::Bh3).is_err());
        assert!(launch.mark_preinstall().is_err());
        assert!(owner.begin(OperationKind::Repair, Some(Game::Nap)).is_err());
        owner.cancel().unwrap();
        assert!(update.check_cancelled().is_err());
        assert!(launch.check_cancelled().is_ok());
        drop(update);
        assert!(owner.is_busy().unwrap());
        launch.finish(Ok(())).unwrap();
    }

    #[test]
    fn controls_and_completion_remain_operation_local() {
        let owner = Operations::default();
        let operation = owner
            .begin(OperationKind::Install, Some(Game::Bh3))
            .unwrap();
        owner.toggle_pause().unwrap();
        assert!(operation.handle().is_paused());
        owner.toggle_pause().unwrap();
        operation.set_phase(Phase::Verifying).unwrap();
        assert!(owner.cancel().is_err());
        assert!(owner.toggle_pause().is_err());
        assert!(operation.check_game(Game::Nap).is_err());
        assert!(
            operation
                .finish::<()>(Err(anyhow::anyhow!("verification failed")))
                .is_err()
        );
        assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Failed);
        let next = owner
            .begin(OperationKind::Install, Some(Game::Nap))
            .unwrap();
        assert!(!next.handle().is_cancelled());
        owner.cancel().unwrap();
        assert!(next.set_phase(Phase::Committing).is_err());
        drop(next);
        assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Cancelled);
    }

    #[test]
    fn profile_lock_lives_until_last_compatible_lease() {
        let root = std::env::temp_dir().join(format!("elysiae-lock-{}", Uuid::new_v4()));
        let path = root.join("operations.lock");
        let owner = Operations::default();
        owner.0.lock().unwrap().profile_lock = Some(lock_profile(&path).unwrap());
        let transfer = owner
            .begin(OperationKind::Install, Some(Game::Bh3))
            .unwrap();
        let launch = owner.begin(OperationKind::Launch, Some(Game::Nap)).unwrap();
        drop(transfer);
        assert!(lock_profile(&path).is_err());
        drop(launch);
        drop(lock_profile(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("outside", &path).unwrap();
        assert!(lock_profile(&path).is_err());
        assert!(!root.join("outside").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
