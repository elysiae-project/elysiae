use super::*;

/// Checks that upstream completion cannot conceal scan errors or uncertain repairs.
#[test]
fn upstream_finished_does_not_override_failed_repairs() {
    let mut report = VerificationReport::default();
    assert!(report.result().is_err());

    report.observe(&SophonProgress::Verifying {
        scanned_files: 2,
        total_files: 2,
        error_count: 1,
    });
    report.observe(&SophonProgress::Finished);
    assert!(
        report
            .result()
            .unwrap_err()
            .to_string()
            .contains("cannot reliably confirm")
    );

    let mut clean = VerificationReport::default();
    clean.observe(&SophonProgress::Verifying {
        scanned_files: 2,
        total_files: 2,
        error_count: 0,
    });
    assert!(clean.result().is_ok());

    clean.observe(&SophonProgress::Error {
        message: "repair failed".into(),
    });
    assert!(clean.result().is_err());
}
