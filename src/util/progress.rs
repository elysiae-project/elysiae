//! Bounded, best-effort UI notifications. Operation leases and their returned
//! results remain authoritative; no GTK objects cross the backend boundary.
use std::sync::{Mutex, OnceLock};

use async_channel::{Receiver, Sender};
use irmin::SophonProgress;

use super::web::DownloadProgress;
use crate::core::operations::OperationStatus;

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Started,
    File(DownloadProgress),
    Sophon(SophonProgress),
    Extracting(String),
    Message(String),
    Finished { error: Option<String> },
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub operation: OperationStatus,
    pub event: ProgressEvent,
}

fn subscribers() -> &'static Mutex<Vec<Sender<ProgressUpdate>>> {
    static SUBSCRIBERS: OnceLock<Mutex<Vec<Sender<ProgressUpdate>>>> = OnceLock::new();
    SUBSCRIBERS.get_or_init(Mutex::default)
}

pub fn subscribe() -> Receiver<ProgressUpdate> {
    let (sender, receiver) = async_channel::bounded(64);
    if let Ok(mut subscribers) = subscribers().lock() {
        subscribers.retain(|sender| !sender.is_closed());
        subscribers.push(sender);
    }
    receiver
}

pub fn publish(update: ProgressUpdate) {
    if let Ok(mut subscribers) = subscribers().lock() {
        subscribers.retain(|sender| sender.force_send(update.clone()).is_ok());
    }
}

/// Lightweight per-operation callback, safe to retain inside download workers.
#[derive(Clone)]
pub struct ProgressReporter(pub OperationStatus);

impl ProgressReporter {
    pub fn report(&self, event: ProgressEvent) {
        publish(ProgressUpdate {
            operation: self.0.clone(),
            event,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::operations::{OperationKind, Phase};

    #[test]
    fn bounded_progress_keeps_recent_events_and_operation_identity() {
        let receiver = subscribe();
        let operation = OperationStatus {
            id: uuid::Uuid::new_v4(),
            kind: OperationKind::Components,
            game: None,
            phase: Phase::Running,
            error: None,
        };
        let reporter = ProgressReporter(operation.clone());
        for _ in 0..100 {
            reporter.report(ProgressEvent::Message("Preparing".into()));
        }
        reporter.report(ProgressEvent::Finished { error: None });
        let mut finished = false;
        while let Ok(update) = receiver.try_recv() {
            if update.operation.id == operation.id
                && matches!(update.event, ProgressEvent::Finished { .. })
            {
                finished = true;
            }
        }
        assert!(finished);
    }
}
