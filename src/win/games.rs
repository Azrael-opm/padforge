//! Game selection and launching: list running apps, browse for an .exe,
//! start a game. Everything here runs only on explicit user action (nothing
//! is polled in the background).
//!
//! Listing uses a Toolhelp snapshot plus window enumeration, so no handle is
//! opened to other processes. A single query-only handle is opened when the
//! user picks a running app, to read its full path (the same thing Task
//! Manager does).

use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows_sys::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, SW_SHOWNORMAL, WS_EX_TOOLWINDOW,
};

#[derive(Clone, Debug)]
pub struct RunningApp {
    pub pid: u32,
    /// Lower-case file name, e.g. "eldenring.exe".
    pub exe: String,
    /// Title of its main window.
    pub title: String,
}

/// Desktop shell and system processes that are never games.
const IGNORE: &[&str] = &[
    "explorer.exe",
    "applicationframehost.exe",
    "textinputhost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "searchapp.exe",
    "systemsettings.exe",
    "lockapp.exe",
    "padforge.exe",
];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Apps that currently have a visible top-level window, sorted by name.
pub fn running_apps() -> Vec<RunningApp> {
    // pid -> exe name, without opening any process handle.
    let mut names = std::collections::HashMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap != INVALID_HANDLE_VALUE {
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut e) != 0;
            while ok {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                names.insert(e.th32ProcessID, String::from_utf16_lossy(&e.szExeFile[..len]).to_ascii_lowercase());
                ok = Process32NextW(snap, &mut e) != 0;
            }
            CloseHandle(snap);
        }
    }

    struct Acc {
        own: u32,
        windows: Vec<(u32, String)>,
    }
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> i32 {
        let acc = &mut *(lp as *mut Acc);
        if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() || (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW) != 0 {
            return 1;
        }
        let n = GetWindowTextLengthW(hwnd);
        if n <= 0 {
            return 1;
        }
        let mut buf = vec![0u16; n as usize + 1];
        let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != acc.own && got > 0 {
            acc.windows.push((pid, String::from_utf16_lossy(&buf[..got as usize])));
        }
        1
    }
    let mut acc = Acc { own: unsafe { GetCurrentProcessId() }, windows: Vec::new() };
    unsafe {
        EnumWindows(Some(each), &mut acc as *mut Acc as LPARAM);
    }

    let mut out: Vec<RunningApp> = Vec::new();
    for (pid, title) in acc.windows {
        let Some(exe) = names.get(&pid).cloned() else { continue };
        if IGNORE.contains(&exe.as_str()) || out.iter().any(|a| a.pid == pid) {
            continue;
        }
        out.push(RunningApp { pid, exe, title });
    }
    out.sort_by(|a, b| a.exe.cmp(&b.exe));
    out
}

/// Full path of a running process (one query-only handle, closed at once).
pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Standard Windows "Open" dialog filtered to executables. Blocks until closed.
pub fn browse_exe(owner: usize, start_in: &str) -> Option<String> {
    let filter: Vec<u16> = "Games and programs (*.exe)\0*.exe\0All files (*.*)\0*.*\0\0".encode_utf16().collect();
    let title = wide("Select the game executable");
    let dir = start_dir(start_in).map(|d| wide(&d));
    let mut file = vec![0u16; 4096];
    unsafe {
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner as HWND;
        ofn.lpstrFilter = filter.as_ptr();
        ofn.lpstrFile = file.as_mut_ptr();
        ofn.nMaxFile = file.len() as u32;
        ofn.lpstrTitle = title.as_ptr();
        ofn.lpstrInitialDir = dir.as_ref().map_or(null(), |d| d.as_ptr());
        ofn.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        if GetOpenFileNameW(&mut ofn) == 0 {
            return None;
        }
    }
    let len = file.iter().position(|&c| c == 0).unwrap_or(0);
    (len > 0).then(|| String::from_utf16_lossy(&file[..len]))
}

fn start_dir(path: &str) -> Option<String> {
    let p = std::path::Path::new(path.trim());
    p.parent().filter(|d| d.is_dir()).map(|d| d.display().to_string())
}

/// Start a game with its own folder as the working directory (many games
/// need that). ShellExecute also handles games that require elevation.
pub fn launch(path: &str, args: &str) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("No game executable set".into());
    }
    if !std::path::Path::new(path).exists() {
        return Err(format!("Game not found: {path}"));
    }
    let (op, file, params) = (wide("open"), wide(path), wide(args.trim()));
    let dir = start_dir(path).map(|d| wide(&d));
    let r = unsafe {
        ShellExecuteW(
            null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            if args.trim().is_empty() { null() } else { params.as_ptr() },
            dir.as_ref().map_or(null(), |d| d.as_ptr()),
            SW_SHOWNORMAL,
        )
    } as isize;
    if r > 32 {
        Ok(())
    } else {
        Err(match r {
            2 | 3 => "Game executable or folder not found".into(),
            5 => "Access denied (or elevation was cancelled)".into(),
            _ => format!("Could not start the game (error {r})"),
        })
    }
}
