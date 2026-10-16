# Changelog

All notable changes to PadForge. Versions follow [Semantic Versioning](https://semver.org/).

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
