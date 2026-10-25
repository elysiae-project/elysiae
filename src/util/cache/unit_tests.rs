use super::*;

/// Rejects escaping asset keys and keeps game/locale catalogs distinct.
#[test]
fn cache_keys_stay_contained_and_distinct() {
    assert_eq!(
        cache_path(Path::new("/images/a.png")).unwrap(),
        PathBuf::from("cache/assets/images/a.png")
    );

    for path in [
        "relative.png",
        "/../outside",
        "/%2e%2e/outside",
        "//host/file",
    ] {
        assert!(cache_path(Path::new(path)).is_err());
    }

    assert_ne!(
        metadata_path(Game::Bh3, "en-us"),
        metadata_path(Game::Hk4e, "en-us")
    );
    assert_ne!(
        metadata_path(Game::Bh3, "en-us"),
        metadata_path(Game::Bh3, "fr-fr")
    );
}
