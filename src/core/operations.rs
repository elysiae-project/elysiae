//! One owner for mutations and the shared Wine prefix. The lease stays alive
//! until all work, including blocking workers and launched processes, has ended.
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Result, bail, ensure};
use irmin::DownloadHandle;
use uuid::Uuid;

use super::game::Game;

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
    active: Option<ActiveOperation>,
    last: Option<OperationStatus>,
}

#[derive(Clone, Default)]
pub struct Operations(Arc<Mutex<State>>);

pub fn operations() -> &'static Operations {
    static OPERATIONS: OnceLock<Operations> = OnceLock::new();
    OPERATIONS.get_or_init(Operations::default)
}

impl Operations {
    pub fn begin(&self, kind: OperationKind, game: Option<Game>) -> Result<Operation> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?;
        if let Some(active) = &state.active {
            bail!("An operation is already active: {:?}", active.status.kind);
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
        });
        Ok(Operation {
            owner: self.clone(),
            id: status.id,
            handle,
            finished: false,
        })
    }

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

    pub fn is_busy(&self) -> Result<bool> {
        Ok(self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Operation lock poisoned"))?
            .active
            .is_some())
    }

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
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Operation is no longer active"))?;
        ensure!(active.status.id == self.id, "Operation ownership changed");
        ensure!(!active.handle.is_cancelled(), "Operation cancelled before phase transition");
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
    fn first_operation_starts_and_completion_releases_ownership() {
        let owner = Operations::default();
        let operation = owner
            .begin(OperationKind::Install, Some(Game::Bh3))
            .unwrap();
        assert!(!operation.handle().is_cancelled());
        assert!(owner.begin(OperationKind::Launch, Some(Game::Bh3)).is_err());
        operation.finish(Ok(())).unwrap();
        assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Completed);
        assert!(!owner.is_busy().unwrap());
    }

    #[test]
    fn cancellation_keeps_ownership_until_worker_exits_then_uses_fresh_handle() {
        let owner = Operations::default();
        let operation = owner
            .begin(OperationKind::Install, Some(Game::Bh3))
            .unwrap();
        owner.cancel().unwrap();
        assert!(operation.check_cancelled().is_err());
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
    }

    #[test]
    fn noncancellable_phases_keep_ownership_and_retain_errors() {
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Repair, Some(Game::Bh3)).unwrap();
        operation.set_phase(Phase::Verifying).unwrap();
        assert!(owner.cancel().is_err());
        assert!(owner.toggle_pause().is_err());
        assert!(
            operation
                .finish::<()>(Err(anyhow::anyhow!("repair failed")))
                .is_err()
        );
        let status = owner.status().unwrap().unwrap();
        assert_eq!(status.phase, Phase::Failed);
        assert_eq!(status.error.as_deref(), Some("repair failed"));
    }

    #[test]
    fn relocation_cancels_before_commit_but_commit_cannot_be_interrupted() {
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Relocate, Some(Game::Bh3)).unwrap();
        assert!(owner.toggle_pause().is_err());
        owner.cancel().unwrap();
        assert!(operation.set_phase(Phase::Committing).is_err());
        drop(operation);
        let operation = owner.begin(OperationKind::Relocate, Some(Game::Bh3)).unwrap();
        operation.set_phase(Phase::Committing).unwrap();
        assert!(owner.cancel().is_err());
        assert!(operation.check_game(Game::Nap).is_err());
        operation.finish(Ok(())).unwrap();
    }

    #[test]
    fn pause_and_drop_do_not_poison_subsequent_operations() {
        let owner = Operations::default();
        let operation = owner.begin(OperationKind::Update, Some(Game::Bh3)).unwrap();
        owner.toggle_pause().unwrap();
        assert!(operation.handle().is_paused());
        owner.toggle_pause().unwrap();
        assert!(!operation.handle().is_paused());
        drop(operation);
        assert!(!owner.is_busy().unwrap());
        let launch = owner.begin(OperationKind::Launch, Some(Game::Bh3)).unwrap();
        assert!(owner.cancel().is_err());
        launch.finish(Ok(())).unwrap();
    }
}
