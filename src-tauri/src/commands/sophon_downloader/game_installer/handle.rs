use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use super::error::{SophonError, SophonResult};
use crate::commands::sophon_downloader::SophonProgress;

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

    pub fn pause(&self) {
        *self.state.lock().unwrap() = ControlState::Paused;
    }

    pub fn resume(&self) {
        *self.state.lock().unwrap() = ControlState::Running;
        self.pause_notify.notify_one();
    }

    pub fn cancel(&self) {
        *self.state.lock().unwrap() = ControlState::Cancelled;
        self.pause_notify.notify_one();
    }

    pub fn is_cancelled(&self) -> bool {
        *self.state.lock().unwrap() == ControlState::Cancelled
    }

    pub async fn wait_if_paused(
        &self,
        updater: &(impl Fn(SophonProgress) + Send + Sync),
        downloaded_bytes: u64,
        total_bytes: u64,
    ) -> SophonResult<()> {
        loop {
            let state = self.state.lock().unwrap().clone();
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
