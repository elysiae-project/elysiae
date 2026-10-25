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
                if *speed_bps > 0.0
                    && *total_bytes > *downloaded_bytes
                    && let Some(eta) = eta(*eta_seconds)
                {
                    detail.push_str(&format!(" · {eta}"));
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
#[path = "progressbar/tests.rs"]
mod tests;
