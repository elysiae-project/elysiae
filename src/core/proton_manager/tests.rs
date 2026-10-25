use super::*;

/// Matches the server's single-release metadata contract.
#[test]
fn latest_release_uses_flat_metadata() {
    assert!(
        component_url()
            .unwrap()
            .query_pairs()
            .any(|(key, value)| key == "latest" && value == "true")
    );

    let data = serde_json::json!({"tag": "11-0", "prerelease": false,
        "download": {"url": "https://example.invalid/runtime.tar.gz", "checksum": "a".repeat(64)}});
    let mut release: ComponentRelease = serde_json::from_value(data.clone()).unwrap();

    release.validate().unwrap();
    assert!(serde_json::from_value::<ComponentRelease>(serde_json::json!([data])).is_err());
}

/// Rejects archive traversal and excessive path sizes before extraction.
#[test]
fn archive_paths_are_relative_and_bounded() {
    assert_eq!(
        bounded_archive_path(Path::new("./runtime/proton")).unwrap(),
        PathBuf::from("runtime/proton")
    );

    for path in [
        "../outside".to_owned(),
        "/outside".to_owned(),
        "a".repeat(MAX_TAR_PATH_BYTES + 1),
        "a/".repeat(MAX_TAR_DEPTH + 1),
    ] {
        assert!(bounded_archive_path(Path::new(&path)).is_err());
    }
}
