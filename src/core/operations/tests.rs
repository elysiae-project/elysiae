use super::*;

#[test]
fn transfers_and_launches_coexist_in_either_order() {
    for kind in [
        OperationKind::Install,
        OperationKind::Update,
        OperationKind::Preinstall,
    ] {
        for launch_first in [false, true] {
            let owner = Operations::default();
            let (first_kind, first_game, second_kind, second_game) = if launch_first {
                (OperationKind::Launch, Game::Hk4e, kind, Game::Hkrpg)
            } else {
                (kind, Game::Hkrpg, OperationKind::Launch, Game::Hk4e)
            };
            let first = owner.begin(first_kind, Some(first_game)).unwrap();
            let second = owner.begin(second_kind, Some(second_game)).unwrap();

            assert_eq!(owner.active().unwrap().len(), 2);
            assert!(
                owner
                    .begin(OperationKind::Install, Some(Game::Nap))
                    .is_err()
            );
            assert!(owner.begin(OperationKind::Launch, Some(Game::Nap)).is_err());
            assert!(owner.begin(OperationKind::Components, None).is_err());
            assert!(owner.begin(OperationKind::Recovery, None).is_err());

            first.finish(Ok(())).unwrap();
            assert!(owner.is_busy().unwrap());

            second.finish(Ok(())).unwrap();
            assert!(!owner.is_busy().unwrap());
        }
    }
}

#[test]
fn same_game_requires_staging_only_preinstall() {
    let owner = Operations::default();
    let update = owner.begin(OperationKind::Update, Some(Game::Bh3)).unwrap();

    assert!(
        !owner
            .can_begin(OperationKind::Launch, Some(Game::Bh3))
            .unwrap()
    );

    update.mark_preinstall().unwrap();
    let launch = owner.begin(OperationKind::Launch, Some(Game::Bh3)).unwrap();
    assert!(launch.check_game(Game::Bh3).is_err());
    assert!(launch.mark_preinstall().is_err());
    assert!(owner.begin(OperationKind::Repair, Some(Game::Nap)).is_err());

    owner.cancel().unwrap();
    assert!(update.check_cancelled().is_err());
    assert!(launch.check_cancelled().is_ok());

    drop(update);
    assert!(owner.is_busy().unwrap());
    launch.finish(Ok(())).unwrap();
}

#[test]
fn controls_and_completion_remain_operation_local() {
    let owner = Operations::default();
    let operation = owner
        .begin(OperationKind::Install, Some(Game::Bh3))
        .unwrap();
    owner.toggle_pause().unwrap();
    assert!(operation.handle().is_paused());

    owner.toggle_pause().unwrap();
    operation.set_phase(Phase::Verifying).unwrap();
    assert!(owner.cancel().is_err());
    assert!(owner.toggle_pause().is_err());
    assert!(operation.check_game(Game::Nap).is_err());

    assert!(
        operation
            .finish::<()>(Err(anyhow::anyhow!("verification failed")))
            .is_err()
    );
    assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Failed);

    let next = owner
        .begin(OperationKind::Install, Some(Game::Nap))
        .unwrap();
    assert!(!next.handle().is_cancelled());
    owner.cancel().unwrap();
    assert!(next.set_phase(Phase::Committing).is_err());
    drop(next);
    assert_eq!(owner.status().unwrap().unwrap().phase, Phase::Cancelled);
}

#[test]
fn profile_lock_lives_until_last_compatible_lease() {
    let root = std::env::temp_dir().join(format!("elysiae-lock-{}", Uuid::new_v4()));
    let path = root.join("operations.lock");
    let owner = Operations::default();
    owner.0.lock().unwrap().profile_lock = Some(lock_profile(&path).unwrap());
    let transfer = owner
        .begin(OperationKind::Install, Some(Game::Bh3))
        .unwrap();
    let launch = owner.begin(OperationKind::Launch, Some(Game::Nap)).unwrap();

    drop(transfer);
    assert!(lock_profile(&path).is_err());

    drop(launch);
    drop(lock_profile(&path).unwrap());

    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink("outside", &path).unwrap();
    assert!(lock_profile(&path).is_err());
    assert!(!root.join("outside").exists());

    std::fs::remove_dir_all(root).unwrap();
}
