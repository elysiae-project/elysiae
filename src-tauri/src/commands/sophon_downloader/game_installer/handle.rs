use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use tauri_plugin_log::log;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::error::{SophonError, SophonResult};
use crate::commands::sophon_downloader::SophonProgress;

const STATE_RUNNING: u8 = 0;
const STATE_PAUSED: u8 = 1;
/// Terminal cancelled state - cannot be undone by resume().
/// Uses value 3 to avoid collision with future intermediate states.
const STATE_CANCELLED: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    Running,
    Paused,
    Cancelled,
}

#[derive(Clone)]
pub struct DownloadHandle {
    state: Arc<AtomicU8>,
    cancel_token: CancellationToken,
    pause_notify: Arc<Notify>,
}

impl DownloadHandle {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(STATE_RUNNING)),
            cancel_token: CancellationToken::new(),
            pause_notify: Arc::new(Notify::new()),
        }
    }

    pub fn pause(&self) {
        // Use compare_exchange to avoid race with concurrent cancel.
        // Cancellation is terminal - never overwrite it with PAUSED.
        while let Err(current) = self.state.compare_exchange(
            STATE_RUNNING,
            STATE_PAUSED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            // If we lost the race to something other than RUNNING, give up.
            // STATE_CANCELLED is terminal; STATE_PAUSED means already paused.
            if current != STATE_RUNNING {
                return;
            }
        }
    }

    pub fn resume(&self) {
        // Never resume a cancelled download, cancellation is terminal
        if self.state.load(Ordering::Acquire) == STATE_CANCELLED {
            return;
        }
        // Only transition from PAUSED to RUNNING and only wake waiters when the
        // transition actually happened; otherwise we'd spuriously notify when no
        // task is parked on `pause_notify` (e.g. resume called without a prior
        // pause, or resume called twice in a row).
        if self
            .state
            .compare_exchange(
                STATE_PAUSED,
                STATE_RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.pause_notify.notify_waiters();
        }
    }

    pub fn cancel(&self) {
        self.state.store(STATE_CANCELLED, Ordering::Release);
        self.cancel_token.cancel();
        self.pause_notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == STATE_CANCELLED
    }

    pub fn cancelled_future(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
        self.cancel_token.cancelled()
    }

    fn get_state(&self) -> ControlState {
        match self.state.load(Ordering::Acquire) {
            STATE_RUNNING => ControlState::Running,
            STATE_PAUSED => ControlState::Paused,
            STATE_CANCELLED => ControlState::Cancelled,
            raw => {
                log::error!("DownloadHandle in invalid state: {raw}");
                ControlState::Cancelled
            }
        }
    }

    pub async fn wait_if_paused(
        &self,
        updater: &(impl Fn(SophonProgress) + Send + Sync + ?Sized),
        downloaded_bytes: u64,
        total_bytes: u64,
    ) -> SophonResult<()> {
        loop {
            match self.get_state() {
                ControlState::Running => return Ok(()),
                ControlState::Cancelled => return Err(SophonError::Cancelled),
                ControlState::Paused => {
                    updater(SophonProgress::Paused {
                        downloaded_bytes,
                        total_bytes,
                    });
                    self.pause_notify.notified().await;
                }
            }
        }
    }
}

impl Default for DownloadHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_new_is_running() {
        let handle = DownloadHandle::new();
        assert!(!handle.is_cancelled());
    }

    #[test]
    fn handle_cancel() {
        let handle = DownloadHandle::new();
        handle.cancel();
        assert!(handle.is_cancelled());
    }

    #[test]
    fn handle_pause_resume() {
        let handle = DownloadHandle::new();
        assert_eq!(handle.get_state(), ControlState::Running);

        handle.pause();
        assert_eq!(handle.get_state(), ControlState::Paused);

        handle.resume();
        assert_eq!(handle.get_state(), ControlState::Running);
    }

    #[test]
    fn handle_is_cancelled_after_cancel() {
        let handle = DownloadHandle::new();
        assert!(!handle.is_cancelled());
        handle.cancel();
        assert!(handle.is_cancelled());
    }

    #[tokio::test]
    async fn handle_resume_notifies_waiters() {
        let handle = DownloadHandle::new();
        handle.pause();
        let updater = |_progress: crate::commands::sophon_downloader::SophonProgress| {};
        let h = handle.clone();
        let result = tokio::spawn(async move { h.wait_if_paused(&updater, 0, 100).await });
        tokio::task::yield_now().await;
        handle.resume();
        let state = result.await.unwrap();
        assert!(state.is_ok());
    }

    #[tokio::test]
    async fn handle_wait_if_paused_returns_cancelled() {
        let handle = DownloadHandle::new();
        handle.pause();
        let updater = |_progress: crate::commands::sophon_downloader::SophonProgress| {};
        let h = handle.clone();
        let result = tokio::spawn(async move { h.wait_if_paused(&updater, 0, 100).await });
        tokio::task::yield_now().await;
        handle.cancel();
        let state = result.await.unwrap();
        assert!(state.is_err());
        assert!(matches!(state.unwrap_err(), super::SophonError::Cancelled));
    }

    #[test]
    fn handle_multiple_pause_calls() {
        let handle = DownloadHandle::new();
        handle.pause();
        assert_eq!(handle.get_state(), ControlState::Paused);
        handle.pause();
        assert_eq!(handle.get_state(), ControlState::Paused);
        handle.pause();
        assert_eq!(handle.get_state(), ControlState::Paused);
        handle.resume();
        assert_eq!(handle.get_state(), ControlState::Running);
    }

    #[test]
    fn handle_resume_after_cancel_is_noop() {
        let handle = DownloadHandle::new();
        handle.cancel();
        handle.resume();
        assert_eq!(handle.get_state(), ControlState::Cancelled);
        assert!(handle.is_cancelled());
    }

    #[test]
    fn handle_pause_after_cancel_is_noop() {
        let handle = DownloadHandle::new();
        handle.cancel();
        handle.pause();
        assert_eq!(handle.get_state(), ControlState::Cancelled);
        assert!(handle.is_cancelled());
    }

    #[test]
    fn handle_resume_without_prior_pause_is_noop() {
        let handle = DownloadHandle::new();
        // Resume while state is RUNNING (not PAUSED) must not transition or notify
        handle.resume();
        assert_eq!(handle.get_state(), ControlState::Running);
    }

    #[tokio::test]
    async fn handle_concurrent_pause_cancel_idempotent() {
        for _ in 0..50 {
            let handle = DownloadHandle::new();
            let h = handle.clone();
            tokio::spawn(async move {
                h.pause();
            });
            handle.cancel();
            tokio::task::yield_now().await;
            // final state must be Cancelled (cancel takes precedence)
            assert!(handle.is_cancelled());
        }
    }
}
