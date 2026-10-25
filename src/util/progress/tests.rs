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
