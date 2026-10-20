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
mod tests {
    use super::*;

    /// Checks destination-side free space and rejects invalid storage requests.
    #[test]
    fn storage_follows_destination_filesystem_not_root() {
        assert!(get_available_storage("relative/path").is_err());
        assert!(required_storage_available("/", f64::NAN).is_err());
        assert!(required_storage_available("/", -1.0).is_err());
        assert!(required_storage_available("/", f64::INFINITY).is_err());

        // /dev/shm is a separate tmpfs on Linux; skip only if it is unavailable.
        if !Path::new("/dev/shm").is_dir() {
            return;
        }

        let root = std::env::temp_dir().join(format!("elysiae-storage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink("/dev/shm", root.join("mounted")).unwrap();
        let destination = root
            .join("mounted")
            .join(uuid::Uuid::new_v4().to_string())
            .join("game");
        let actual = get_available_storage(destination.to_str().unwrap()).unwrap();
        let mut stats = MaybeUninit::<libc::statvfs>::uninit();

        // Compare to the mount itself, independently of the helper under test.
        assert_eq!(
            unsafe { libc::statvfs(c"/dev/shm".as_ptr(), stats.as_mut_ptr()) },
            0
        );
        let stats = unsafe { stats.assume_init() };
        let expected = stats.f_bavail as f64 * stats.f_frsize as f64;

        assert!((actual - expected).abs() < 1024.0 * 1024.0);
        assert!(required_storage_available(destination.to_str().unwrap(), 0.0).unwrap());
        assert!(!required_storage_available(destination.to_str().unwrap(), f64::MAX).unwrap());

        fs::remove_dir_all(root).unwrap();
    }
}
