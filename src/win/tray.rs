//! System tray icon (raw Shell_NotifyIcon, no extra crates).
//!
//! * Icon colour follows state: green = emulating, amber = enabled but paused,
//!   grey = off.
//! * Left click: show the window (GUI) / toggle emulation (headless).
//!   Right click: menu.
//! * Updates are event driven through `Shared::notify` (no polling).
//! * Survives Explorer restarts (re-adds itself on `TaskbarCreated`).

use crate::engine::Msg;
use crate::icon;
use crate::shared::{Shared, GUI_REQ_QUIT, GUI_REQ_SHOW};
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, Ordering::*};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, OnceLock};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 10;
const WM_REFRESH: u32 = WM_APP + 11;
const WM_BALLOON: u32 = WM_APP + 12;

const ID_TOGGLE: usize = 1;
const ID_SHOW: usize = 2;
const ID_FOLDER: usize = 3;
const ID_QUIT: usize = 4;

struct Ctx {
    shared: Arc<Shared>,
    tx: SyncSender<Msg>,
    config_dir: PathBuf,
    gui: bool,
    icons: [HICON; 3], // on, paused, off
    taskbar_created: u32,
}

// Only touched from the tray thread.
static mut CTX: Option<Ctx> = None;
static HWND_TRAY: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());
static LAST_STATE: AtomicU8 = AtomicU8::new(255);
static LAST_ENABLED: AtomicBool = AtomicBool::new(false);
static BALLOON: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();

fn wide_into(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16()) {
        *d = c;
        n += 1;
    }
    let end = n.min(dst.len() - 1);
    dst[end] = 0;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Build an HICON from our procedural RGBA pixels.
unsafe fn make_icon(rgb: [u8; 3]) -> HICON {
    let size = GetSystemMetrics(SM_CXSMICON).max(16) as u32;
    let rgba = icon::render_rgba(size, rgb);
    let mut bi: BITMAPINFO = zeroed();
    bi.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: size as i32,
        biHeight: -(size as i32), // top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..zeroed()
    };
    let dc = GetDC(null_mut());
    let mut bits: *mut core::ffi::c_void = null_mut();
    let color = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    ReleaseDC(null_mut(), dc);
    if color.is_null() || bits.is_null() {
        return LoadIconW(null_mut(), IDI_APPLICATION);
    }
    // RGBA -> premultiplied BGRA.
    let dst = std::slice::from_raw_parts_mut(bits as *mut u8, rgba.len());
    for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_chunks::<4>().0.iter()) {
        let a = s[3] as u32;
        d[0] = (s[2] as u32 * a / 255) as u8;
        d[1] = (s[1] as u32 * a / 255) as u8;
        d[2] = (s[0] as u32 * a / 255) as u8;
        d[3] = s[3];
    }
    let mask = CreateBitmap(size as i32, size as i32, 1, 1, null());
    let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let h = CreateIconIndirect(&info);
    DeleteObject(color);
    DeleteObject(mask);
    if h.is_null() {
        LoadIconW(null_mut(), IDI_APPLICATION)
    } else {
        h
    }
}

unsafe fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut d: NOTIFYICONDATAW = zeroed();
    d.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = 1;
    d
}

#[allow(static_mut_refs)]
unsafe fn ctx() -> &'static Ctx {
    CTX.as_ref().expect("tray ctx")
}

/// Refresh icon + tooltip from the shared status; beep on user toggles.
unsafe fn refresh(hwnd: HWND, add: bool) {
    let c = ctx();
    let st = c.shared.status_snapshot();
    let state: u8 = if st.active {
        0
    } else if st.user_enabled {
        1
    } else {
        2
    };
    let tip = match state {
        0 => format!("PadForge - emulating ({})", st.profile),
        1 => format!("PadForge - paused: {}", st.suspended_reason.as_deref().unwrap_or("")),
        _ => "PadForge - off".to_string(),
    };
    let mut d = base_data(hwnd);
    d.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
    d.uCallbackMessage = WM_TRAY;
    d.hIcon = c.icons[state as usize];
    wide_into(&mut d.szTip, &tip);
    Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &d);

    let prev_enabled = LAST_ENABLED.swap(st.user_enabled, Relaxed);
    let first = LAST_STATE.swap(state, Relaxed) == 255;
    if !first && prev_enabled != st.user_enabled && c.shared.toggle_sound.load(Relaxed) {
        MessageBeep(if st.user_enabled { MB_ICONASTERISK } else { MB_OK });
    }
}

unsafe fn show_balloon(hwnd: HWND) {
    let Some(m) = BALLOON.get() else { return };
    let Some((title, text)) = m.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let mut d = base_data(hwnd);
    d.uFlags = NIF_INFO;
    d.dwInfoFlags = NIIF_INFO;
    wide_into(&mut d.szInfoTitle, &title);
    wide_into(&mut d.szInfo, &text);
    Shell_NotifyIconW(NIM_MODIFY, &d);
}

/// Ask the GUI thread to show its window. The window is shown through
/// eframe (`App::logic` keeps running while hidden), so winit's own
/// visibility state stays consistent.
pub fn show_gui(shared: &Shared) {
    super::debug_log(|| "tray: show request".into());
    shared.gui_request.store(GUI_REQ_SHOW, Relaxed);
    shared.notify();
}

unsafe fn open_folder(dir: &std::path::Path) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let (op, p) = (wide("open"), wide(&dir.display().to_string()));
    ShellExecuteW(null_mut(), op.as_ptr(), p.as_ptr(), null(), null(), SW_SHOWNORMAL);
}

unsafe fn quit() {
    let c = ctx();
    c.shared.quit.store(true, Relaxed);
    if c.gui {
        c.shared.gui_request.store(GUI_REQ_QUIT, Relaxed);
        c.shared.notify();
    } else {
        let _ = c.tx.try_send(Msg::Shutdown);
    }
}

unsafe fn menu(hwnd: HWND) {
    let c = ctx();
    let st = c.shared.status_snapshot();
    let m = CreatePopupMenu();
    let item = |id: usize, text: &str, flags: u32| {
        let w = wide(text);
        AppendMenuW(m, MF_STRING | flags, id, w.as_ptr());
    };
    let label = format!("Profile: {}", if st.profile.is_empty() { "-" } else { &st.profile });
    item(0, &label, MF_GRAYED);
    AppendMenuW(m, MF_SEPARATOR, 0, null());
    item(ID_TOGGLE, "Emulation enabled", if st.user_enabled { MF_CHECKED } else { 0 });
    if c.gui {
        item(ID_SHOW, "Show PadForge", 0);
    }
    item(ID_FOLDER, "Open config folder", 0);
    AppendMenuW(m, MF_SEPARATOR, 0, null());
    item(ID_QUIT, "Quit", 0);

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd); // required so the menu closes when clicking away
    let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, null()) as usize;
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(m);
    match cmd {
        ID_TOGGLE => {
            let _ = c.tx.try_send(Msg::SetEnabled(!st.user_enabled));
        }
        ID_SHOW => show_gui(&c.shared),
        ID_FOLDER => open_folder(&c.config_dir),
        ID_QUIT => quit(),
        _ => {}
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            super::debug_log(|| format!("tray event lparam={:#x}", lparam as u32));
            match lparam as u32 {
                WM_LBUTTONUP => {
                    let c = ctx();
                    if c.gui {
                        show_gui(&c.shared); // restores or brings to front
                    } else {
                        let _ = c.tx.try_send(Msg::ToggleEmulation);
                    }
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => menu(hwnd),
                _ => {}
            }
            0
        }
        WM_REFRESH => {
            refresh(hwnd, false);
            0
        }
        WM_BALLOON => {
            show_balloon(hwnd);
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            let d = base_data(hwnd);
            Shell_NotifyIconW(NIM_DELETE, &d);
            PostQuitMessage(0);
            0
        }
        m if m == ctx().taskbar_created && m != 0 => {
            refresh(hwnd, true); // Explorer restarted
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Queue a one-off balloon notification.
pub fn balloon(title: &str, text: &str) {
    let m = BALLOON.get_or_init(|| Mutex::new(None));
    *m.lock().unwrap_or_else(|e| e.into_inner()) = Some((title.into(), text.into()));
    let h = HWND_TRAY.load(Relaxed);
    if !h.is_null() {
        unsafe {
            PostMessageW(h, WM_BALLOON, 0, 0);
        }
    }
}

/// Start the tray thread.
pub fn spawn(shared: Arc<Shared>, tx: SyncSender<Msg>, config_dir: PathBuf, gui: bool) -> Option<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("padforge-tray".into())
        .spawn(move || unsafe {
            let hinst = GetModuleHandleW(null());
            let class = wide("PadForgeTray");
            let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: hinst, lpszClassName: class.as_ptr(), ..zeroed() };
            RegisterClassW(&wc);
            let tc = wide("TaskbarCreated");
            CTX = Some(Ctx {
                shared: shared.clone(),
                tx,
                config_dir,
                gui,
                icons: [make_icon(icon::COLOR_ON), make_icon(icon::COLOR_PAUSED), make_icon(icon::COLOR_OFF)],
                taskbar_created: RegisterWindowMessageW(tc.as_ptr()),
            });
            // A hidden top-level window (not message-only) so it also
            // receives the TaskbarCreated broadcast.
            let title = wide("PadForgeTray");
            let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0, null_mut(), null_mut(), hinst, null());
            if hwnd.is_null() {
                return;
            }
            HWND_TRAY.store(hwnd, Relaxed);
            refresh(hwnd, true);
            let h = hwnd as usize;
            shared.add_listener(move || {
                PostMessageW(h as HWND, WM_REFRESH, 0, 0);
            });

            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            HWND_TRAY.store(null_mut(), Relaxed);
            #[allow(static_mut_refs)]
            if let Some(c) = CTX.take() {
                for i in c.icons {
                    DestroyIcon(i);
                }
            }
        })
        .ok()
}

/// Remove the tray icon and end its thread.
pub fn stop() {
    let h = HWND_TRAY.load(Relaxed);
    if !h.is_null() {
        unsafe {
            PostMessageW(h, WM_CLOSE, 0, 0);
        }
    }
}
