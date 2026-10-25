use super::*;

/// Checks supported game codes round-trip and executable names stay game-local.
#[test]
fn game_metadata_round_trips_and_uses_local_executables() {
    for game in Game::all() {
        assert_eq!(Game::try_from(game.code()).unwrap(), game);

        let executable = PathBuf::from(game.executable().unwrap());
        assert_eq!(executable.components().count(), 1);
        assert_eq!(executable.extension().unwrap(), "exe");
    }

    assert!(Game::try_from("unknown").is_err());
}
