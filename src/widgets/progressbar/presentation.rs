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

pub(super) fn format_sophon_progress(progress: &SophonProgress) -> String {
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
