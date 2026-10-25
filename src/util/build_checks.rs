pub(super) fn version_at_least(release: &str, minimum: (u64, u64, u64)) -> bool {
    let mut parts = release
        .trim()
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok());

    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    ) >= minimum
}

/// Checks version ordering, distribution suffixes and invalid release strings.
#[test]
fn compares_kernel_versions() {
    assert!(version_at_least("6.14.0", (6, 14, 0)));
    assert!(version_at_least("6.15.2-arch1", (6, 14, 0)));
    assert!(!version_at_least("6.13.9", (6, 14, 0)));
    assert!(version_at_least("6.14-foo", (6, 14, 0)));
    assert!(!version_at_least("invalid", (6, 14, 0)));
}
