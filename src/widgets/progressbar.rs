use gtk::{glib, prelude::*, subclass::prelude::*};
use irmin::SophonProgress;

use crate::{
    core::operations::OperationKind,
    util::{
        progress::{ProgressEvent, ProgressUpdate},
        web::DownloadProgress,
    },
};

/// Compatibility with callers that only need a textual progress summary.
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    File(DownloadProgress),
    Sophon(SophonProgress),
    Error(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Bars {
    #[default]
    None,
    File,
    Sophon,
}

#[derive(Debug, Clone, PartialEq)]
struct BarState {
    // None means unknown, not zero. An inactive unknown bar never pulses.
    fraction: Option<f64>,
    value: String,
    active: bool,
}

impl Default for BarState {
    fn default() -> Self {
        Self {
            fraction: None,
            value: "Not started".into(),
            active: false,
        }
    }
}

impl BarState {
    fn bytes(downloaded: u64, total: u64) -> Self {
        Self {
            fraction: fraction(downloaded, total),
            value: byte_counts(downloaded, total),
            active: true,
        }
    }

    fn files(checked: u64, total: u64) -> Self {
        Self {
            fraction: fraction(checked, total),
            value: if total == 0 {
                format!("{checked} files · total unknown")
            } else {
                format!(
                    "{checked} / {total} files · {:.0}%",
                    fraction(checked, total).unwrap_or(0.0) * 100.0
                )
            },
            active: true,
        }
    }

    fn pulses(&self) -> bool {
        self.active && self.fraction.is_none()
    }
}

/// Pure presentation state: only the operation's Finished event is terminal.
#[derive(Debug, Default, PartialEq)]
struct ProgressState {
    operation_id: Option<uuid::Uuid>,
    heading: String,
    detail: String,
    bars: Bars,
    download: BarState,
    verification: BarState,
    visible: bool,
    extracting: bool,
    error: bool,
    complete: bool,
    terminal: bool,
}

impl ProgressState {
    fn update(&mut self, update: &ProgressUpdate) -> bool {
        if matches!(
            update.operation.kind,
            OperationKind::Launch | OperationKind::Recovery
        ) {
            return false;
        }
        if self.operation_id != Some(update.operation.id) {
            *self = Self {
                operation_id: Some(update.operation.id),
                heading: if update.operation.kind == OperationKind::Components {
                    "Downloading Phlogiston".into()
                } else if let Some(game) = update.operation.game {
                    format!("Downloading {}", game.display_name())
                } else {
                    "Preparing operation".into()
                },
                visible: true,
                ..Self::default()
            };
        } else if self.terminal {
            // A queued worker notification must not erase the final result.
            return false;
        }
        self.error = false;
        self.extracting = false;
        self.download.active = false;
        self.verification.active = false;
        match &update.event {
            ProgressEvent::Started => self.detail = "Preparing…".into(),
            ProgressEvent::File(progress) => {
                self.bars = Bars::File;
                self.detail = "Downloading…".into();
                self.download = BarState::bytes(progress.downloaded, progress.total);
                self.verification = BarState::default();
            }
            ProgressEvent::Sophon(progress) => {
                if self.bars != Bars::Sophon {
                    // A preceding file transfer belongs to another stage, not Sophon.
                    self.download = BarState::default();
                    self.verification = BarState::default();
                }
                self.bars = Bars::Sophon;
                self.detail = format_sophon_progress(progress);
                match progress {
                    SophonProgress::Downloading {
                        downloaded_bytes,
                        total_bytes,
                        ..
                    } => {
                        self.download = BarState::bytes(*downloaded_bytes, *total_bytes);
                    }
                    SophonProgress::Paused {
                        downloaded_bytes,
                        total_bytes,
                    } => {
                        self.download = BarState::bytes(*downloaded_bytes, *total_bytes);
                        self.download.active = false;
                    }
                    SophonProgress::CheckingFiles {
                        checked_files,
                        total_files,
                    } => {
                        self.verification = BarState::files(*checked_files, *total_files);
                    }
                    SophonProgress::Verifying {
                        scanned_files,
                        total_files,
                        ..
                    } => {
                        self.verification = BarState::files(*scanned_files, *total_files);
                    }
                    SophonProgress::Error { .. } => self.error = true,
                    // Assembly, plugins, metadata and upstream Finished never invent
                    // transferred or verified work, or mark the operation complete.
                    _ => {}
                }
            }
            ProgressEvent::Extracting(path) => {
                let name = path
                    .rsplit(['/', '\\'])
                    .find(|part| !part.is_empty())
                    .unwrap_or("archive");
                self.detail = format!("Extracting {name}…");
                self.extracting = true;
            }
            ProgressEvent::Message(message) => self.detail = message.clone(),
            ProgressEvent::Finished { error } => {
                self.terminal = true;
                self.error = error.is_some();
                self.complete = error.is_none();
                self.detail = error.as_ref().map_or_else(
                    || "Completed".into(),
                    |error| format!("Operation failed: {error}"),
                );
            }
        }
        true
    }

    fn shows_bars(&self) -> bool {
        self.visible && !self.extracting && self.bars != Bars::None
    }

    fn pulses(&self) -> bool {
        self.shows_bars()
            && (self.download.pulses() || (self.bars == Bars::Sophon && self.verification.pulses()))
    }
}

fn fraction(done: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| (done as f64 / total as f64).clamp(0.0, 1.0))
}

fn bytes(bytes: f64) -> String {
    let mut value = bytes.max(0.0);
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

fn byte_counts(downloaded: u64, total: u64) -> String {
    match fraction(downloaded, total) {
        Some(fraction) => format!(
            "{} / {} · {:.0}%",
            bytes(downloaded as f64),
            bytes(total as f64),
            fraction * 100.0
        ),
        None => format!("{} · total unknown", bytes(downloaded as f64)),
    }
}

fn eta(seconds: f64) -> Option<String> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    let seconds = seconds.ceil() as u64;
    Some(if seconds >= 86_400 {
        format!("{}d {}h left", seconds / 86_400, (seconds % 86_400) / 3600)
    } else if seconds >= 3600 {
        format!("{}h {}m left", seconds / 3600, (seconds % 3600) / 60)
    } else if seconds >= 60 {
        format!("{}m {}s left", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s left")
    })
}

fn format_sophon_progress(progress: &SophonProgress) -> String {
    match progress {
        SophonProgress::FetchingManifest => "Fetching manifest…".into(),
        SophonProgress::CalculatingDownloads {
            checked_files,
            total_files,
        } => {
            format!("Calculating downloads: {checked_files}/{total_files}")
        }
        SophonProgress::Downloading {
            downloaded_bytes,
            total_bytes,
            speed_bps,
            eta_seconds,
        } => {
            let mut detail = "Downloading…".to_string();
            if speed_bps.is_finite() && *speed_bps >= 0.0 {
                detail.push_str(&format!(" · {}/s", bytes(*speed_bps)));
                if *speed_bps > 0.0 && *total_bytes > *downloaded_bytes {
                    if let Some(eta) = eta(*eta_seconds) {
                        detail.push_str(&format!(" · {eta}"));
                    }
                }
            }
            detail
        }
        SophonProgress::Paused { .. } => "Download paused".into(),
        SophonProgress::Assembling {
            assembled_files,
            total_files,
        } => {
            format!("Assembling files: {assembled_files}/{total_files}")
        }
        SophonProgress::CheckingFiles {
            checked_files,
            total_files,
        } => {
            format!("Checking files: {checked_files}/{total_files}")
        }
        SophonProgress::Verifying {
            scanned_files,
            total_files,
            error_count,
        } => {
            format!("Verifying: {scanned_files}/{total_files} (errors: {error_count})")
        }
        SophonProgress::Warning { message } => format!("Warning: {message}"),
        SophonProgress::Error { message } => format!("Error: {message}"),
        SophonProgress::InstallingPlugins {
            current_plugin,
            total_plugins,
        } => {
            format!("Installing plugin {current_plugin} ({total_plugins} total)")
        }
        SophonProgress::InstallingSdks {
            current_sdk,
            total_sdks,
        } => {
            format!("Installing SDK {current_sdk} ({total_sdks} total)")
        }
        SophonProgress::DownloadingPlugin {
            name,
            downloaded_bytes,
            total_bytes,
        } => {
            format!(
                "Downloading {name}: {}",
                byte_counts(*downloaded_bytes, *total_bytes)
            )
        }
        SophonProgress::ApplyingPreinstall {
            applied_files,
            total_files,
        } => {
            format!("Applying preinstall: {applied_files}/{total_files}")
        }
        SophonProgress::Finished => "Finalizing…".into(),
    }
}

mod imp {
    use std::cell::RefCell;

    use gtk::{CompositeTemplate, glib, subclass::prelude::*};

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/progressbar.ui")]
    pub struct Progressbar {
        #[template_child]
        pub heading: TemplateChild<gtk::Label>,
        #[template_child]
        pub detail: TemplateChild<gtk::Label>,
        #[template_child]
        pub bars: TemplateChild<gtk::Box>,
        #[template_child]
        pub verification_row: TemplateChild<gtk::Box>,
        #[template_child]
        pub download_value: TemplateChild<gtk::Label>,
        #[template_child]
        pub verification_value: TemplateChild<gtk::Label>,
        #[template_child]
        pub download_progress: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub verification_progress: TemplateChild<gtk::ProgressBar>,
        pub(super) state: RefCell<super::ProgressState>,
        pub(super) pulse_source: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Progressbar {
        const NAME: &'static str = "ElysiaeProgressbar";
        type Type = super::Progressbar;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Progressbar {
        fn constructed(&self) {
            self.parent_constructed();
            // Children are bound now; never access them from Default/init_template.
            self.obj().reset();
        }

        fn dispose(&self) {
            if let Some(source) = self.pulse_source.borrow_mut().take() {
                source.remove();
            }
            self.dispose_template();
        }
    }
    impl WidgetImpl for Progressbar {}
    impl BoxImpl for Progressbar {}
}

glib::wrapper! {
    pub struct Progressbar(ObjectSubclass<imp::Progressbar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Progressbar {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// The window selects relevant operations; launch/recovery are always ignored.
    pub fn update(&self, update: &ProgressUpdate) {
        let changed = self.imp().state.borrow_mut().update(update);
        if changed {
            self.render();
        }
    }

    /// Clear the previous operation and hide the entire panel.
    pub fn reset(&self) {
        *self.imp().state.borrow_mut() = ProgressState::default();
        self.render();
    }

    fn render(&self) {
        let imp = self.imp();
        let state = imp.state.borrow();
        self.set_visible(state.visible);
        imp.heading.set_text(&state.heading);
        imp.heading.set_visible(!state.extracting);
        imp.detail.set_text(&state.detail);
        imp.bars.set_visible(state.shows_bars());
        imp.verification_row.set_visible(state.bars == Bars::Sophon);
        for (bar, label, value) in [
            (
                &*imp.download_progress,
                &*imp.download_value,
                &state.download,
            ),
            (
                &*imp.verification_progress,
                &*imp.verification_value,
                &state.verification,
            ),
        ] {
            label.set_text(&value.value);
            bar.set_tooltip_text(Some(&value.value));
            if value.pulses() {
                bar.pulse();
            } else {
                // Exit activity mode when a total becomes known or work pauses.
                bar.set_fraction(value.fraction.unwrap_or(0.0));
            }
        }
        for (class, enabled) in [
            ("is-error", state.error),
            ("is-complete", state.complete),
            ("is-extracting", state.extracting),
        ] {
            if enabled {
                self.add_css_class(class);
            } else {
                self.remove_css_class(class);
            }
        }
        let pulses = state.pulses();
        drop(state);
        if !pulses {
            if let Some(source) = imp.pulse_source.borrow_mut().take() {
                source.remove();
            }
        } else if imp.pulse_source.borrow().is_none() {
            let weak = self.downgrade();
            let source =
                glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
                    let Some(panel) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let imp = panel.imp();
                    let state = imp.state.borrow();
                    if !panel.is_visible() || !state.pulses() {
                        imp.pulse_source.borrow_mut().take();
                        return glib::ControlFlow::Break;
                    }
                    if panel.is_mapped() {
                        if state.download.pulses() {
                            imp.download_progress.pulse();
                        }
                        if state.bars == Bars::Sophon && state.verification.pulses() {
                            imp.verification_progress.pulse();
                        }
                    }
                    glib::ControlFlow::Continue
                });
            *imp.pulse_source.borrow_mut() = Some(source);
        }
    }

    pub fn handle_download_event(event: DownloadEvent) -> String {
        match event {
            DownloadEvent::File(progress) => match fraction(progress.downloaded, progress.total) {
                Some(fraction) => format!("Downloading: {:.0}%", fraction * 100.0),
                None => format!(
                    "Downloading: {} · total unknown",
                    bytes(progress.downloaded as f64)
                ),
            },
            DownloadEvent::Sophon(progress) => format_sophon_progress(&progress),
            DownloadEvent::Error(error) => format!("Download failed: {error}"),
        }
    }
}

impl Default for Progressbar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
}
