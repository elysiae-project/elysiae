use super::*;
use crate::core::{
    game::Game,
    operations::{OperationStatus, Phase},
};

fn update(event: ProgressEvent) -> ProgressUpdate {
    ProgressUpdate {
        operation: OperationStatus {
            id: uuid::Uuid::nil(),
            kind: OperationKind::Install,
            game: Some(Game::Hk4e),
            phase: Phase::Running,
            error: None,
        },
        event,
    }
}

fn file(downloaded: u64, total: u64) -> ProgressEvent {
    ProgressEvent::File(DownloadProgress {
        download_id: uuid::Uuid::nil(),
        downloaded,
        total,
    })
}

fn downloading(downloaded_bytes: u64, total_bytes: u64) -> ProgressEvent {
    ProgressEvent::Sophon(SophonProgress::Downloading {
        downloaded_bytes,
        total_bytes,
        speed_bps: 1024.0,
        eta_seconds: 65.0,
    })
}

#[test]
fn file_to_sophon_verification_assembly_and_extraction() {
    let mut state = ProgressState::default();
    state.update(&update(file(50, 100)));
    assert_eq!(state.heading, "Downloading Genshin Impact");
    assert_eq!(state.bars, Bars::File);
    assert_eq!(state.download.fraction, Some(0.5));

    state.update(&update(ProgressEvent::Sophon(
        SophonProgress::FetchingManifest,
    )));
    assert_eq!(state.bars, Bars::Sophon);
    assert_eq!(state.download.fraction, None);
    assert_eq!(state.verification.value, "Not started");
    assert!(!state.verification.pulses());

    state.update(&update(downloading(80, 100)));
    state.update(&update(ProgressEvent::Sophon(SophonProgress::Verifying {
        scanned_files: 2,
        total_files: 4,
        error_count: 1,
    })));
    assert_eq!(state.download.fraction, Some(0.8));
    assert_eq!(state.verification.fraction, Some(0.5));
    assert!(state.detail.contains("errors: 1"));

    state.update(&update(ProgressEvent::Sophon(SophonProgress::Assembling {
        assembled_files: 3,
        total_files: 4,
    })));
    assert_eq!(state.download.fraction, Some(0.8));
    assert_eq!(state.verification.fraction, Some(0.5));

    state.update(&update(ProgressEvent::Extracting(
        "/tmp/Phlogiston.tar.xz".into(),
    )));
    assert_eq!(state.detail, "Extracting Phlogiston.tar.xz…");
    assert!(state.extracting);
    assert!(!state.shows_bars());
    assert!(!state.pulses());

    state.update(&update(ProgressEvent::Sophon(SophonProgress::Finished)));
    assert_eq!(state.download.fraction, Some(0.8));
    assert_eq!(state.verification.fraction, Some(0.5));
    assert!(!state.complete);
}

#[test]
fn unknown_totals_are_indeterminate_only_during_actual_work() {
    let mut state = ProgressState::default();
    state.update(&update(file(42, 0)));
    assert_eq!(state.download.fraction, None);
    assert!(state.pulses());
    assert!(state.download.value.contains("total unknown"));

    state.update(&update(downloading(42, 0)));
    assert!(state.pulses());
    assert!(!state.detail.contains("left"));
    assert!(!state.verification.pulses());

    state.update(&update(ProgressEvent::Sophon(SophonProgress::Paused {
        downloaded_bytes: 42,
        total_bytes: 0,
    })));
    assert!(!state.pulses());

    state.update(&update(ProgressEvent::Sophon(
        SophonProgress::CheckingFiles {
            checked_files: 3,
            total_files: 0,
        },
    )));
    assert!(state.verification.pulses());
    assert_eq!(state.verification.fraction, None);
}

#[test]
fn plugin_and_metadata_phases_do_not_invent_or_replace_main_progress() {
    let mut state = ProgressState::default();
    state.update(&update(downloading(75, 100)));

    for phase in [
        SophonProgress::CalculatingDownloads {
            checked_files: 2,
            total_files: 4,
        },
        SophonProgress::Assembling {
            assembled_files: 4,
            total_files: 4,
        },
        SophonProgress::DownloadingPlugin {
            name: "Plugin".into(),
            downloaded_bytes: 100,
            total_bytes: 100,
        },
        SophonProgress::InstallingPlugins {
            current_plugin: "Plugin".into(),
            total_plugins: 1,
        },
        SophonProgress::InstallingSdks {
            current_sdk: "SDK".into(),
            total_sdks: 1,
        },
        SophonProgress::ApplyingPreinstall {
            applied_files: 4,
            total_files: 4,
        },
        SophonProgress::Finished,
    ] {
        state.update(&update(ProgressEvent::Sophon(phase)));
        assert_eq!(state.download.fraction, Some(0.75));
        assert_eq!(state.verification.fraction, None);
        assert_eq!(state.verification.value, "Not started");
        assert!(!state.complete);
    }
}

#[test]
fn percentages_are_clamped_without_integer_overflow() {
    assert_eq!(fraction(u64::MAX, 1), Some(1.0));
    assert_eq!(fraction(0, 10), Some(0.0));
    assert_eq!(fraction(1, 0), None);

    let mut state = ProgressState::default();
    state.update(&update(file(u64::MAX, 1)));
    assert_eq!(state.download.fraction, Some(1.0));
    assert!(state.download.value.ends_with("100%"));

    state.update(&update(ProgressEvent::Sophon(SophonProgress::Verifying {
        scanned_files: 5,
        total_files: 4,
        error_count: 0,
    })));
    assert_eq!(state.verification.fraction, Some(1.0));
}

#[test]
fn only_operation_finished_is_authoritative_and_terminal() {
    let mut state = ProgressState::default();
    state.update(&update(downloading(75, 100)));
    state.update(&update(ProgressEvent::Sophon(SophonProgress::Finished)));
    assert!(!state.complete);
    assert!(!state.terminal);
    assert_eq!(state.detail, "Finalizing…");
    assert_eq!(state.download.fraction, Some(0.75));

    state.update(&update(ProgressEvent::Finished {
        error: Some("disk full".into()),
    }));
    assert!(state.error);
    assert!(state.terminal);
    assert!(!state.complete);
    assert!(state.detail.contains("disk full"));
    assert!(!state.update(&update(downloading(100, 100))));
    assert!(!state.update(&update(ProgressEvent::Finished { error: None })));
    assert!(state.error);

    let mut next = update(ProgressEvent::Started);
    next.operation.id = uuid::Uuid::new_v4();
    state.update(&next);
    assert!(!state.error);
    assert!(!state.terminal);
    assert_eq!(state.bars, Bars::None);

    next.event = ProgressEvent::Finished { error: None };
    state.update(&next);
    assert!(state.complete);
    assert!(!state.error);

    // Success does not fabricate verification or transfer counts.
    assert_eq!(state.verification.fraction, None);
    assert_eq!(state.download.fraction, None);
}

#[test]
fn upstream_error_is_visible_but_not_an_operation_result() {
    let mut state = ProgressState::default();
    state.update(&update(ProgressEvent::Sophon(SophonProgress::Error {
        message: "retry failed".into(),
    })));
    assert!(state.error);
    assert!(!state.terminal);

    state.update(&update(ProgressEvent::Finished {
        error: Some("installation failed".into()),
    }));
    assert_eq!(state.detail, "Operation failed: installation failed");
    assert!(state.terminal);
}

#[test]
fn components_heading_and_unrelated_operations() {
    let mut state = ProgressState::default();
    let mut event = update(ProgressEvent::Started);
    event.operation.kind = OperationKind::Components;
    event.operation.game = None;
    state.update(&event);
    assert_eq!(state.heading, "Downloading Phlogiston");

    for kind in [OperationKind::Launch, OperationKind::Recovery] {
        event.operation.kind = kind;
        event.operation.id = uuid::Uuid::new_v4();
        assert!(!state.update(&event));
        assert_eq!(state.heading, "Downloading Phlogiston");
    }

    state = ProgressState::default();
    assert!(!state.visible);
    assert!(!state.shows_bars());
}

#[test]
fn formats_bytes_speed_and_eta_without_nonfinite_values() {
    assert_eq!(bytes(1024.0), "1.0 KiB");
    assert_eq!(bytes(1024.0 * 1024.0), "1.0 MiB");
    assert_eq!(eta(65.0).as_deref(), Some("1m 5s left"));
    assert_eq!(eta(3601.0).as_deref(), Some("1h 0m left"));
    assert_eq!(eta(f64::INFINITY), None);
    assert_eq!(eta(-1.0), None);

    let mut state = ProgressState::default();
    state.update(&update(downloading(1, 100)));
    assert!(state.detail.contains("1.0 KiB/s"));
    assert!(state.detail.contains("1m 5s left"));

    for invalid in [f64::NAN, f64::INFINITY, -1.0] {
        state.update(&update(ProgressEvent::Sophon(
            SophonProgress::Downloading {
                downloaded_bytes: 1,
                total_bytes: 100,
                speed_bps: invalid,
                eta_seconds: invalid,
            },
        )));
        assert_eq!(state.detail, "Downloading…");
    }
}

/// Run explicitly with a display and --ignored --test-threads=1.
#[test]
#[ignore = "requires a GTK display"]
fn template_renders_progress_and_resets_safely() {
    gtk::init().expect("GTK display is available");
    gtk::gio::resources_register_include!("elysiae.gresource").unwrap();

    let panel = Progressbar::new();
    let imp = panel.imp();
    assert!(!panel.is_visible());
    assert!(panel.has_css_class("progress-panel"));

    panel.update(&update(file(50, 100)));
    assert!(panel.is_visible());
    assert!(imp.bars.is_visible());
    assert!(!imp.verification_row.is_visible());
    assert_eq!(imp.download_progress.fraction(), 0.5);
    assert_eq!(imp.heading.text(), "Downloading Genshin Impact");

    panel.update(&update(downloading(75, 100)));
    panel.update(&update(ProgressEvent::Sophon(SophonProgress::Verifying {
        scanned_files: 1,
        total_files: 4,
        error_count: 0,
    })));
    assert!(imp.verification_row.is_visible());
    assert_eq!(imp.download_progress.fraction(), 0.75);
    assert_eq!(imp.verification_progress.fraction(), 0.25);

    panel.update(&update(ProgressEvent::Extracting(
        "/tmp/runtime.tar.xz".into(),
    )));
    assert!(panel.has_css_class("is-extracting"));
    assert!(!imp.heading.is_visible());
    assert!(!imp.bars.is_visible());
    assert_eq!(imp.detail.text(), "Extracting runtime.tar.xz…");

    panel.update(&update(ProgressEvent::Finished {
        error: Some("disk full".into()),
    }));
    assert!(panel.has_css_class("is-error"));
    assert!(!panel.has_css_class("is-complete"));
    assert!(!panel.has_css_class("is-extracting"));

    panel.reset();
    panel.update(&update(file(5, 0)));
    assert!(imp.pulse_source.borrow().is_some());

    panel.reset();
    assert!(!panel.is_visible());
    assert!(!panel.has_css_class("is-error"));
    assert!(imp.pulse_source.borrow().is_none());
}

#[test]
fn legacy_text_api_remains_available() {
    let ProgressEvent::File(progress) = file(50, 100) else {
        unreachable!()
    };

    assert_eq!(
        Progressbar::handle_download_event(DownloadEvent::File(progress)),
        "Downloading: 50%"
    );
    assert_eq!(
        Progressbar::handle_download_event(DownloadEvent::Sophon(SophonProgress::Verifying {
            scanned_files: 2,
            total_files: 4,
            error_count: 0,
        })),
        "Verifying: 2/4 (errors: 0)"
    );
    assert_eq!(
        Progressbar::handle_download_event(DownloadEvent::Sophon(SophonProgress::Finished)),
        "Finalizing…"
    );
    assert_eq!(
        Progressbar::handle_download_event(DownloadEvent::Error("failed".into())),
        "Download failed: failed"
    );
}
