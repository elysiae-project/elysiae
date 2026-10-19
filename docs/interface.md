# Interface customization

The GTK interface uses `data/resources/styles.css`, embedded by the existing
Cargo/GResource build. Rebuild after changing it. No libadwaita dependency is
required.

## Theme and widget classes

All authored colors are defined in the stylesheet's `:root` block. The original
`--bg-color-dark`, `--btn-primary` and `--btn-primary-hover` variables remain.
The additional surface, text, border, focus, progress, fallback and backdrop
variables control the new widgets. Color literals belong in that block, not in
component rules or Rust rendering code.

| Area | CSS hooks |
| --- | --- |
| Window and controls | `.launcher-window`, `.launcher-content`, `.launcher-footer`, `.launcher-actions`, `.operation-button`, `.preinstall-button`, `.settings-button` |
| Titlebar | `.launcher-titlebar`, `.launcher-title`, `.titlebar-close` |
| Game selector | `.game-selector`, `.game-selector-scroll`, `.game-selector-items`, `.game-button`, `.game-icon`, `.game-icon-image`, `.game-icon-fallback` |
| Game states | `.is-selected`, `.is-installed`, `.is-uninstalled`, `.is-downloading` |
| Background | `.background`, `.background-layers`, `.background-fallback`, `.background-wordmark`, `.background-image`, `.background-video`, `.background-art-overlay` |
| Progress | `.progress-panel`, `.progress-heading`, `.progress-detail`, `.progress-bars`, `.progress-row`, `.progress-label`, `.progress-value`, `.download-progress`, `.verification-progress` |
| Progress states | `.is-extracting`, `.is-error`, `.is-complete` |
| Settings | `.settings-layer`, `.settings-backdrop`, `.settings-panel`, `.settings-header`, `.settings-heading`, `.settings-close`, `.settings-scroll`, `.settings-content`, `.settings-row`, `.settings-label`, `.settings-description`, `.settings-switch`, `.settings-dropdown`, `.settings-error`, `.settings-footer` |

GTK CSS subnodes are scoped beneath their component classes, for example
`.download-progress trough` and `.settings-switch slider`. Dimensions, spacing
and rounding are grouped by component in the stylesheet. Icon desaturation uses
a snapshot color matrix rather than separate grayscale files; its coefficients
are luminance weights, not theme colors.

## Behavior

- The selector is populated from `Game::all()` in `src/core/game.rs`.
- Browsing never changes the target of an active download. Progress events carry
  an operation ID and game identity.
- One install/update/preinstall can coexist with one supervised game session.
  An update cannot modify the running game's live files. Staged preinstalls may
  coexist with that game's current version. Runtime replacement and recovery
  remain exclusive, and the profile lock survives until all owners finish.
- When a preinstall is available, **Launch** remains the primary action and
  **Preinstall** is a separate button. A missing or interrupted shared runtime
  must be prepared before launching; it cannot be replaced during another task.
- Normal file downloads show one bar; Sophon shows download and verification
  separately. Extraction shows only its filename/status. Only the backend's
  operation result marks completion.
- Settings save immediately. Close, Escape and outside click dismiss the modal
  without reverting edits. The six user-facing schema keys have controls;
  first-launch/version bookkeeping remains managed by startup.
- The display-language preference currently selects localized artwork. Launcher
  text remains English; this change does not add a translation catalog.

## Artwork startup

The fallback surface paints immediately. Existing cached artwork is resolved
without waiting for the network. The selected game's static image is published
before icons, video and other games complete, and GTK receives incremental
notifications. Catalog requests are bounded to three; asset transfers stay
serialized for accurate cache accounting. Visible icons are refreshed only when
that game's file changes. Failed refreshes retain usable cached media, and video
keeps a static poster until playback starts.

## Verification

Ordinary tests use a generated temporary profile and in-memory preferences:

```sh
cargo test --workspace --offline --locked
cargo build --offline --locked
cargo clippy --workspace --all-targets --offline --locked
cargo fmt --all -- --check
glib-compile-schemas --strict --dry-run data
```

GTK interaction tests are opt-in and require a display. Run them separately:
GTK must not be initialized on different test threads in the same process.

```sh
G_DEBUG=fatal-warnings cargo test --offline --locked \
  window::tests::gtk_launcher_interactions_and_layout -- --ignored --test-threads=1
G_DEBUG=fatal-warnings cargo test --offline --locked \
  widgets::settings::tests::gtk_settings_template_persists_syncs_and_restores -- --ignored --test-threads=1
G_DEBUG=fatal-warnings cargo test --offline --locked \
  widgets::progressbar::tests::template_renders_progress_and_resets_safely -- --ignored --test-threads=1
```

Set `ELYSIAE_TEST_SCREENSHOTS=/tmp/elysiae-captures` for the window test to save
native GTK renders of desktop/compact layouts and progress/modal states. These
tests do not download or launch real games, and never override Cargo's `HOME`.
