//! Windows platform layer: input capture hooks, ViGEmBus output, GUI.
pub mod app;
pub mod gui;
pub mod input;
pub mod tray;
pub mod vpad;

/// Opt-in diagnostics: set `PADFORGE_DEBUG=1` to append to `padforge-debug.log`
/// in the temp directory. Zero cost when disabled.
pub fn debug_log(msg: impl FnOnce() -> String) {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("PADFORGE_DEBUG").is_some()) {
        return;
    }
    use std::io::Write;
    let path = std::env::temp_dir().join("padforge-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{:?} {}", std::time::SystemTime::now(), msg());
    }
}
