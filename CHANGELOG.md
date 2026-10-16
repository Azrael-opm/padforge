# Changelog

All notable changes to PadForge. Versions follow [Semantic Versioning](https://semver.org/).

## [0.3.0] - 2026-10-16

### Added
- **New interface**: gold / black / white design with a **dark (default) and light theme** (top-right toggle or Settings > Appearance).
  Sidebar with an emulation status card and the profile list (game exe, LIVE badge); tabs for Mapping, Mouse tuning, Settings and Help.
- **Device previews** for mapping, inspired by reWASD: ANSI keyboard (key caps show their action), 5-button mouse and
  Xbox 360 / DualShock 4 controllers with callouts showing which keys drive each button, stick, trigger and the D-pad. All vector-drawn.
- **Click-to-map**: click an element in the preview to edit it. One-click assignments for keys (button, stick direction, trigger,
  walk modifier, macro, layer, block) and for controller elements (assign a key, stick directions, stick click, "mouse moves this stick").
  *Detect a key* selects whatever physical key you press.
- **List view** keeps the classic row editor with the preview visible; hovering a row highlights it on the preview and vice versa.
- **Game selection**: pick a running app (process list) or browse for the game's `.exe`. Profiles store the game path and launch options.
- **Launch from PadForge** (Launch button, or `--launch "<profile>"` from a shortcut): starts the game, enables emulation and hides to the tray
  (both configurable under Settings > Game launch).
- Layer chips with rename (updates Layer actions) and delete; tooltips show which key activates a layer.
- Profile duplicate / delete buttons and a right-click menu in the sidebar.

### Changed
- The 20 Hz live view only runs while emulating **and** the live controller (gamepad view or mouse curve) is on screen.
- `--enable` no longer gets saved into the config when settings are changed during that run.
- Empty `game_path` / `launch_args` are not written to `padforge.toml`.

## [0.2.0] - 2026-10-16

### Added
- **System tray icon.** The colour shows the state (green = emulating, amber = paused, grey = off) and the tooltip shows profile and state.
  Left-click opens the window (in `--headless` mode it toggles emulation). Right-click menu: active profile, *Emulation enabled*, *Show PadForge*, *Open config folder*, *Quit*.
  The icon comes back automatically if Explorer restarts.
- **Close to tray** (default on) with a one-time notification, and **Start hidden in tray** (default off).
- **Sound on toggle** (default on): a short system sound when emulation is switched on/off, so you get feedback in fullscreen games.
- Window and tray icon, drawn in code (no image assets).
- `--console` flag for headless mode; headless is now tray-only by default.
- `PADFORGE_DEBUG=1` writes a diagnostic log to `%TEMP%\padforge-debug.log`.
- GitHub Actions CI: clippy + tests on Linux and Windows, release build artifact, and automatic GitHub releases (exe + SHA-256) for `v*` tags.
- CHANGELOG.

### Changed
- The GUI is now fully event-driven: engine status changes wake it directly instead of a 500 ms poll. Nothing is rendered while hidden in the tray.
- Release binaries are published via GitHub Releases instead of being committed to the repository.

## [0.1.0]

### Added
- First release: virtual Xbox 360 / DualShock 4 via ViGEmBus, key → button/trigger/stick, mouse → stick with velocity smoothing and curves, input blocking, profiles with per-exe auto-switch, layers, turbo, toggles, macros, F8 toggle, egui GUI, headless mode.
