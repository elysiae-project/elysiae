use gtk::glib;
use irmin::SophonProgress;

use crate::util::web::DownloadProgress;

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    File(DownloadProgress),
    Sophon(SophonProgress),
    Error(String),
}

mod imp {
    use gtk::CompositeTemplate;
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/progressbar.ui")]
    pub struct Progressbar;

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

    impl ObjectImpl for Progressbar {}
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

    pub fn handle_download_event(event: DownloadEvent) -> String {
        match event {
            DownloadEvent::File(progress) => {
                let percentage = if progress.total == 0 {
                    0
                } else {
                    progress.downloaded.saturating_mul(100) / progress.total
                };
                format!("Downloading: {percentage}%")
            }
            DownloadEvent::Sophon(progress) => Self::format_sophon_progress(&progress),
            DownloadEvent::Error(error) => format!("Download failed: {error}"),
        }
    }

    fn format_sophon_progress(progress: &SophonProgress) -> String {
        match progress {
            SophonProgress::FetchingManifest => "Fetching manifest".into(),
            SophonProgress::CalculatingDownloads { checked_files, total_files } => {
                format!("Calculating downloads: {checked_files}/{total_files}")
            }
            SophonProgress::Downloading { downloaded_bytes, total_bytes, speed_bps, eta_seconds } => {
                format!("Downloading: {downloaded_bytes}/{total_bytes} bytes ({speed_bps:.0} B/s, ETA {eta_seconds:.0}s)")
            }
            SophonProgress::Paused { downloaded_bytes, total_bytes } => {
                format!("Paused: {downloaded_bytes}/{total_bytes} bytes")
            }
            SophonProgress::Assembling { assembled_files, total_files } => {
                format!("Assembling: {assembled_files}/{total_files}")
            }
            SophonProgress::CheckingFiles { checked_files, total_files } => {
                format!("Checking files: {checked_files}/{total_files}")
            }
            SophonProgress::Verifying { scanned_files, total_files, error_count } => {
                format!("Verifying: {scanned_files}/{total_files} (errors: {error_count})")
            }
            SophonProgress::Warning { message } => format!("Warning: {message}"),
            SophonProgress::Error { message } => format!("Error: {message}"),
            SophonProgress::InstallingPlugins { current_plugin, total_plugins } => {
                format!("Installing plugin {current_plugin}/{total_plugins}")
            }
            SophonProgress::InstallingSdks { current_sdk, total_sdks } => {
                format!("Installing SDK {current_sdk}/{total_sdks}")
            }
            SophonProgress::DownloadingPlugin { name, downloaded_bytes, total_bytes } => {
                format!("Downloading {name}: {downloaded_bytes}/{total_bytes} bytes")
            }
            SophonProgress::ApplyingPreinstall { applied_files, total_files } => {
                format!("Applying preinstall: {applied_files}/{total_files}")
            }
            SophonProgress::Finished => "Finished".into(),
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

    #[test]
    fn formats_regular_download_progress() {
        let progress = DownloadProgress {
            download_id: uuid::Uuid::nil(),
            downloaded: 50,
            total: 100,
        };
        assert_eq!(Progressbar::handle_download_event(DownloadEvent::File(progress)), "Downloading: 50%");
    }

    #[test]
    fn formats_sophon_verification_progress() {
        let progress = SophonProgress::Verifying {
            scanned_files: 2,
            total_files: 4,
            error_count: 0,
        };
        assert_eq!(Progressbar::handle_download_event(DownloadEvent::Sophon(progress)), "Verifying: 2/4 (errors: 0)");
    }
}
