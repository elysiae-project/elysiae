use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use super::error::{SophonError, SophonResult};
use crate::commands::sophon_downloader::SophonProgress;
use tauri_plugin_log::log;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    Running,
    Paused,
    Cancelled,
}

#[derive(Clone)]
pub struct DownloadHandle {
    state: Arc<Mutex<ControlState>>,
    pause_notify: Arc<Notify>,
}

impl DownloadHandle {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ControlState::Running)),
            pause_notify: Arc::new(Notify::new()),
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, ControlState> {
        self.state.lock().unwrap_or_else(|e| {
            log::error!("Mutex poisoned in DownloadHandle, recovering state");
            e.into_inner()
        })
    }

    pub fn pause(&self) {
        *self.lock_state() = ControlState::Paused;
    }

    pub fn resume(&self) {
        *self.lock_state() = ControlState::Running;
        self.pause_notify.notify_one();
    }

    pub fn cancel(&self) {
        *self.lock_state() = ControlState::Cancelled;
        self.pause_notify.notify_one();
    }

    pub fn is_cancelled(&self) -> bool {
        *self.lock_state() == ControlState::Cancelled
    }

    pub async fn wait_if_paused(
        &self,
        updater: &(impl Fn(SophonProgress) + Send + Sync + ?Sized),
        downloaded_bytes: u64,
        total_bytes: u64,
    ) -> SophonResult<()> {
        loop {
            let state = *self.lock_state();
            match state {
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
