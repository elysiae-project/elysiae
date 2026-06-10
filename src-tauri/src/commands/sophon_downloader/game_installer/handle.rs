use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use tauri_plugin_log::log;
use tokio::sync::Notify;

use super::error::{SophonError, SophonResult};
use crate::commands::sophon_downloader::SophonProgress;

const STATE_RUNNING: u8 = 0;
const STATE_PAUSED: u8 = 1;
/// Terminal cancelled state — cannot be undone by resume().
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
    pause_notify: Arc<Notify>,
}

impl DownloadHandle {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(STATE_RUNNING)),
            pause_notify: Arc::new(Notify::new()),
        }
    }

    pub fn pause(&self) {
        self.state.store(STATE_PAUSED, Ordering::Release);
    }

    pub fn resume(&self) {
        // Never resume a cancelled download — cancellation is terminal
        if self.state.load(Ordering::Acquire) == STATE_CANCELLED {
            return;
        }
        // Only transition from PAUSED to RUNNING
        let _ = self.state.compare_exchange(
            STATE_PAUSED,
            STATE_RUNNING,
            Ordering::Release,
            Ordering::Relaxed,
        );
        self.pause_notify.notify_waiters();
    }

    pub fn cancel(&self) {
        self.state.store(STATE_CANCELLED, Ordering::Release);
        self.pause_notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == STATE_CANCELLED
    }

    fn get_state(&self) -> ControlState {
        match self.state.load(Ordering::Acquire) {
            STATE_RUNNING => ControlState::Running,
            STATE_PAUSED => ControlState::Paused,
            STATE_CANCELLED => ControlState::Cancelled,
            raw => {
                log::error!("DownloadHandle in invalid state: {}", raw);
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
}
