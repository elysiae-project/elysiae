use irmin::{SophonProgress, game_installer::SophonError};

/// Event sent to the gtk frontend for regular downloads
pub enum DownloadEvent {
    Progress { downloaded: u64, total: u64 },
    Finished,
    Failed(String),
}
