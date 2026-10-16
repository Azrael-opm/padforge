#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#[cfg(windows)]
fn main() {
    padforge::win::app::main();
}
#[cfg(not(windows))]
fn main() {
    eprintln!("PadForge runs on Windows. Build with --target x86_64-pc-windows-gnu (or msvc).");
}
