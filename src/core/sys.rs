use crate::core::fs::{Sizes, size_as};
use anyhow::{Context, Result, ensure};
use std::{ffi::CString, fs, mem::MaybeUninit, os::unix::ffi::OsStrExt, path::Path};
use sysinfo::System;

/// Returns the running kernel's version without its distribution suffix.
pub fn kernel_version() -> Result<String> {
    System::kernel_version()
        .context("Kernel version unavailable")
        .map(|v| v.split('-').next().unwrap_or(&v).to_string())
}

/// Available space on the filesystem containing an absolute destination path.
/// For a not-yet-created destination, query its nearest existing ancestor.
pub fn get_available_storage(destination: &str, unit: Sizes) -> Result<f64> {
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
    Ok(size_as(
        stats.f_bavail as f64 * stats.f_frsize as f64,
        Sizes::Bytes,
        unit,
    ))
}

/// Checks whether the destination filesystem has room for the estimated size.
/// The estimate is expressed in the supplied unit.
pub fn required_storage_available(
    destination: &str,
    estimated_size: f64,
    unit: Sizes,
) -> Result<bool> {
    ensure!(
        estimated_size.is_finite() && estimated_size >= 0.0,
        "invalid storage estimate"
    );
    Ok(get_available_storage(destination, Sizes::Bytes)?
        >= size_as(estimated_size, unit, Sizes::Bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks that missing destinations use their nearest existing ancestor.
    #[test]
    fn storage_uses_existing_destination_ancestor() {
        let root = std::env::temp_dir().join(format!("elysiae-storage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let future = root.join("not-yet-created/runtime");
        let actual = get_available_storage(future.to_str().unwrap(), Sizes::Bytes).unwrap();
        let expected = get_available_storage(root.to_str().unwrap(), Sizes::Bytes).unwrap();
        // Free space can change between the two syscalls on a shared filesystem.
        assert!((actual - expected).abs() < 64.0 * 1024.0 * 1024.0);
        assert!(required_storage_available(future.to_str().unwrap(), 0.0, Sizes::Bytes).unwrap());
        assert!(
            !required_storage_available(future.to_str().unwrap(), f64::MAX, Sizes::Bytes).unwrap()
        );
        fs::remove_dir(root).unwrap();
    }

    /// Checks that invalid paths and storage estimates return errors.
    #[test]
    fn storage_rejects_invalid_paths_and_estimates() {
        for path in ["", "relative/path", "/tmp/../tmp", "/tmp/invalid\0path"] {
            assert!(
                get_available_storage(path, Sizes::Bytes).is_err(),
                "{path:?}"
            );
        }
        for size in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(required_storage_available("/", size, Sizes::Bytes).is_err());
        }
        let root = std::env::temp_dir().join(format!("elysiae-storage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("file"), b"not a directory").unwrap();
        std::os::unix::fs::symlink(root.join("missing"), root.join("dangling")).unwrap();
        for path in [root.join("file/child"), root.join("dangling/child")] {
            assert!(get_available_storage(path.to_str().unwrap(), Sizes::Bytes).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// Checks space against the destination mount rather than the root filesystem.
    #[test]
    fn storage_follows_destination_filesystem_not_root() {
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
        let actual = get_available_storage(destination.to_str().unwrap(), Sizes::Bytes).unwrap();
        let mut stats = MaybeUninit::<libc::statvfs>::uninit();
        // Compare to the mount itself, independently of the helper under test.
        assert_eq!(
            unsafe { libc::statvfs(c"/dev/shm".as_ptr(), stats.as_mut_ptr()) },
            0
        );
        let stats = unsafe { stats.assume_init() };
        let expected = stats.f_bavail as f64 * stats.f_frsize as f64;
        assert!((actual - expected).abs() < 1024.0 * 1024.0);
        let kibibytes =
            get_available_storage(destination.to_str().unwrap(), Sizes::Kilobytes).unwrap();
        assert!((kibibytes * 1024.0 - expected).abs() < 1024.0 * 1024.0);
        fs::remove_dir_all(root).unwrap();
    }
}
