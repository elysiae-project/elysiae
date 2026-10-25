use super::*;

/// Checks that game URIs only select known games and reject foreign inputs.
#[test]
fn game_uri_is_navigation_only_and_rejects_foreign_inputs() {
    assert_eq!(parse_game_uri("elysiae://open-game/hk4e"), Some(Game::Hk4e));

    for uri in [
        "https://open-game/hk4e",
        "elysiae://execute/hk4e",
        "elysiae://open-game/unknown",
        "elysiae://user@open-game/hk4e",
        "elysiae://open-game/hk4e?run=true",
        "elysiae://open-game/hk4e#run",
        "elysiae://open-game/hk4e/other",
    ] {
        assert_eq!(parse_game_uri(uri), None, "{uri}");
    }
}
