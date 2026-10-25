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
