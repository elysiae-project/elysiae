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
