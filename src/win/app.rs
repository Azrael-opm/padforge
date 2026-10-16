//! Process entry: load config, start engine + capture threads, run GUI or headless.

use super::{gui, input, vpad::VigemBackend};
use crate::config::Config;
use crate::engine::{Engine, Msg};
use crate::shared::Shared;
use std::path::PathBuf;
use std::sync::atomic::Ordering::*;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, OnceLock};
use windows_sys::Win32::Foundation::TRUE;
use windows_sys::Win32::System::Console::{AllocConsole, AttachConsole, SetConsoleCtrlHandler, ATTACH_PARENT_PROCESS};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, SetPriorityClass, SetThreadPriority, ABOVE_NORMAL_PRIORITY_CLASS, THREAD_PRIORITY_HIGHEST,
};
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

/// Bounded so the hook can never block or allocate unboundedly.
const QUEUE: usize = 8192;

pub struct Paths {
    pub config: PathBuf,
}

/// Portable by default: `padforge.toml` next to the exe, falling back to
/// %APPDATA%\PadForge when the exe directory isn't writable.
pub fn default_config_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let probe = dir.join(".padforge-write-test");
            if std::fs::write(&probe, b"").is_ok() {
                let _ = std::fs::remove_file(&probe);
                return dir.join("padforge.toml");
            }
        }
    }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("PadForge");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("padforge.toml")
}

/// Load config; a broken file is reported but never overwritten.
pub fn load_config(path: &PathBuf) -> (Config, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(s) => match Config::from_toml(&s) {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(format!("{} is invalid, using defaults (file left untouched):\n{e}", path.display()))),
        },
        Err(_) => {
            let c = Config::default();
            let _ = save_config(path, &c);
            (c, None)
        }
    }
}

pub fn save_config(path: &PathBuf, c: &Config) -> Result<(), String> {
    let tmp = path.with_extension("toml.tmp");
    let header = "# PadForge configuration. Edit here or in the GUI.\n# Docs: see README.md (actions, inputs, mouse tuning).\n\n";
    std::fs::write(&tmp, format!("{header}{}", c.to_toml())).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

static CTRL_TX: OnceLock<SyncSender<Msg>> = OnceLock::new();

unsafe extern "system" fn console_ctrl(_ty: u32) -> i32 {
    if let Some(tx) = CTRL_TX.get() {
        let _ = tx.try_send(Msg::Shutdown);
    }
    TRUE
}

fn fatal(msg: &str) -> ! {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let text: Vec<u16> = msg.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "PadForge".encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
    }
    std::process::exit(1);
}

pub fn main() {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");
    let force_enable = args.iter().any(|a| a == "--enable");
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let (mut config, load_error) = load_config(&config_path);
    if force_enable {
        config.settings.start_enabled = true;
    }
    if config.settings.high_priority {
        unsafe {
            SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS);
        }
    }

    let shared = Arc::new(Shared::default());
    let (tx, rx) = sync_channel::<Msg>(QUEUE);

    let engine_shared = shared.clone();
    let engine_config = config.clone();
    let engine = std::thread::Builder::new()
        .name("padforge-engine".into())
        .spawn(move || {
            unsafe {
                SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
            }
            let e = Engine::new(VigemBackend::default(), engine_shared, engine_config);
            drop(e.run(rx)); // backend Drop unplugs the controller
        })
        .expect("spawn engine thread");

    let input_thread = match input::spawn(shared.clone(), tx.clone()) {
        Ok(h) => h,
        Err(e) => fatal(&format!("Could not start input capture: {e}")),
    };

    if headless {
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                AllocConsole();
            }
        }
        let _ = CTRL_TX.set(tx.clone());
        unsafe {
            SetConsoleCtrlHandler(Some(console_ctrl), TRUE);
        }
        if let Some(e) = &load_error {
            eprintln!("{e}");
        }
        println!("PadForge running headless. Config: {}", config_path.display());
        println!("Toggle emulation with {}. Ctrl+C to quit.", config.settings.toggle_hotkey);
        let _ = engine.join();
    } else {
        let paths = Paths { config: config_path };
        gui::run(shared.clone(), tx.clone(), config, paths, load_error);
        let _ = tx.send(Msg::Shutdown);
        let _ = engine.join();
    }

    shared.quit.store(true, Relaxed);
    input::stop();
    let _ = input_thread.join();
}
