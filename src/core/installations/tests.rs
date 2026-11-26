use super::*;
use crate::core::operations::{OperationKind, Operations};

/// Resumes owned installations without allowing writes after ownership is lost.
#[test]
fn managed_writes_require_ownership() {
    let store = Store(format!("registry-test-{}", Uuid::new_v4()).into());
    let owner = Operations::default();
    let operation = owner
        .begin(OperationKind::Install, Some(Game::Bh3))
        .unwrap();
    let root = store.prepare_install(Game::Bh3, &operation).unwrap();

    std::fs::write(root.join("partial-download"), b"keep").unwrap();
    assert_eq!(store.prepare_install(Game::Bh3, &operation).unwrap(), root);
    assert_eq!(store.writable_path(Game::Bh3, &operation).unwrap(), root);

    std::fs::remove_file(root.join(OWNER_MARKER)).unwrap();
    assert!(store.writable_path(Game::Bh3, &operation).is_err());
    assert_eq!(
        std::fs::read(root.join("partial-download")).unwrap(),
        b"keep"
    );

    std::fs::remove_dir_all(store.root().unwrap()).unwrap();
}

/// An unknown game code from a retired build must not poison the registry load.
#[test]
fn unknown_codes_do_not_block_the_registry() {
    let uuid = Uuid::new_v4();
    let base = std::env::temp_dir().join(format!("registry-test-{uuid}"));
    let store = Store(format!("registry-test-{uuid}").into());
    let hk4e_root = base.join("hk4e");
    let orphan_root = base.join("hyg");
    for root in [&hk4e_root, &orphan_root] {
        std::fs::create_dir_all(root).unwrap();
    }

    // The orphan intentionally violates the known-entry invariants (managed
    // without write consent), proving that unknown codes are never validated.
    let registry = Registry {
        schema_version: SCHEMA_VERSION,
        installations: [
            (
                "bogus".to_string(),
                Installation {
                    path: orphan_root.clone(),
                    managed: true,
                    allow_modify: false,
                    ownership_token: "not-a-token".to_string(),
                },
            ),
            (
                Game::Hk4e.code().to_string(),
                Installation {
                    path: hk4e_root.clone(),
                    managed: false,
                    allow_modify: true,
                    ownership_token: Uuid::new_v4().to_string(),
                },
            ),
        ]
        .into_iter()
        .collect(),
    };
    let file = store.resolve(Path::new(REGISTRY)).unwrap();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, serde_json::to_vec_pretty(&registry).unwrap()).unwrap();

    assert_eq!(store.path(Game::Hk4e).unwrap(), hk4e_root);

    let registry = store.load().unwrap();
    store.save(&registry).unwrap();
    let persisted: Registry =
        serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert!(persisted.installations.contains_key("bogus"));
    assert_eq!(
        persisted.installations["bogus"].path,
        orphan_root,
        "Unknown records must round-trip verbatim"
    );

    std::fs::remove_dir_all(base).unwrap();
    std::fs::remove_dir_all(store.root().unwrap()).unwrap();
}
