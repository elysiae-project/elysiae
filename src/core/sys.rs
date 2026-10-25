use anyhow::{Context, Result, ensure};
use std::{ffi::CString, fs, mem::MaybeUninit, os::unix::ffi::OsStrExt, path::Path};

/// Available bytes on the filesystem containing an absolute destination path.
/// For a not-yet-created destination, query its nearest existing ancestor.
pub fn get_available_storage(destination: &str) -> Result<f64> {
    let path = Path::new(destination);
    ensure!(
        path.is_absolute(),
        "storage destination must be an absolute path"
    );
    ensure!(
        !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir)),
        "storage destination must not contain '..'"
    );
    CString::new(destination).context("storage destination contains a NUL byte")?;

    let mut ancestor = path;

    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ancestor = ancestor
                    .parent()
                    .context("storage destination has no existing ancestor")?;
            }
            Err(error) => return Err(error).context("Could not inspect storage destination"),
        }
    }

    // Canonicalization rejects dangling symlinks rather than quietly querying a
    // different filesystem above them.
    let resolved = ancestor
        .canonicalize()
        .context("Could not resolve storage destination")?;
    let path = CString::new(resolved.as_os_str().as_bytes())?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();

    // SAFETY: path is NUL-terminated and stats points to writable storage. The
    // output is only read after statvfs reports success.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error())
            .context("Could not query destination filesystem");
    }

    let stats = unsafe { stats.assume_init() };

    Ok(stats.f_bavail as f64 * stats.f_frsize as f64)
}

/// Checks whether the destination filesystem has room for an estimate in bytes.
pub fn required_storage_available(destination: &str, estimated_bytes: f64) -> Result<bool> {
    ensure!(
        estimated_bytes.is_finite() && estimated_bytes >= 0.0,
        "invalid storage estimate"
    );

    Ok(get_available_storage(destination)? >= estimated_bytes)
}

#[cfg(test)]
#[path = "sys/tests.rs"]
mod tests;
