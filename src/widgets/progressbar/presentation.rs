use irmin::SophonProgress;

pub(super) fn fraction(done: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| (done as f64 / total as f64).clamp(0.0, 1.0))
}

pub(super) fn bytes(bytes: f64) -> String {
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

pub(super) fn byte_counts(downloaded: u64, total: u64) -> String {
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

pub(super) fn eta(seconds: f64) -> Option<String> {
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

/// Speed and remaining time for a transfer, empty when speed is unknown.
pub(super) fn speed_and_eta(
    speed_bps: f64,
    eta_seconds: f64,
    downloaded: u64,
    total: u64,
) -> String {
    let mut detail = String::new();
    if speed_bps.is_finite() && speed_bps >= 0.0 {
        detail.push_str(&format!("{}/s", bytes(speed_bps)));
        if speed_bps > 0.0
            && total > downloaded
            && let Some(eta) = eta(eta_seconds)
        {
            detail.push_str(&format!(" · {eta}"));
        }
    }

    detail
}

/// The panel's detail line names only the current state: speed and ETA live
/// beside the heading, byte and file counts live on the bars. Pause, warnings
/// and errors keep their own text because they are not download/assembly work.
pub(super) fn format_sophon_progress(progress: &SophonProgress) -> String {
    match progress {
        SophonProgress::FetchingManifest
        | SophonProgress::CalculatingDownloads { .. }
        | SophonProgress::Downloading { .. }
        | SophonProgress::DownloadingPlugin { .. } => "Downloading".into(),
        SophonProgress::Paused { .. } => "Download paused".into(),
        SophonProgress::Assembling { .. }
        | SophonProgress::CheckingFiles { .. }
        | SophonProgress::Verifying { .. }
        | SophonProgress::InstallingPlugins { .. }
        | SophonProgress::InstallingSdks { .. }
        | SophonProgress::ApplyingPreinstall { .. }
        | SophonProgress::Finished => "Assembling".into(),
        SophonProgress::Warning { message } => format!("Warning: {message}"),
        SophonProgress::Error { message } => format!("Error: {message}"),
    }
}
