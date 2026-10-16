//! Input capture thread.
//!
//! * `WH_KEYBOARD_LL` / `WH_MOUSE_LL` hooks see every physical key, button,
//!   wheel notch and mouse move *before* any application (including games that
//!   read Raw Input). Returning 1 swallows the event, which is how mapped keys
//!   and mouse movement are hidden from the game so it doesn't flip back to
//!   keyboard/mouse prompts.
//! * Hook callbacks only do atomic loads and a non-blocking channel send
//!   (no locks, no allocation) - Windows silently unhooks slow LL hooks.
//! * Down/up pairing is tracked per input: if a key-down was swallowed its
//!   key-up is swallowed too (and vice versa), so toggling emulation mid-press
//!   can never leave a key stuck in the game.
//! * Raw Input (RIDEV_INPUTSINK) is registered as an unaccelerated delta source;
//!   the engine uses it in `auto` mode whenever Windows delivers it.
//! * A WinEvent hook reports foreground-window changes for per-game profiles,
//!   fully event driven (no polling).

use crate::engine::Msg;
use crate::keys::{self, InputId, NUM_INPUTS};
use crate::shared::Shared;
use std::ffi::c_void;
use std::mem::{size_of, MaybeUninit};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering::*};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThread, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, SetThreadPriority,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEMOUSE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
static TX: OnceLock<SyncSender<Msg>> = OnceLock::new();
static MSG_HWND: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static THREAD_ID: AtomicU32 = AtomicU32::new(0);
static LAST_CAPTURE: AtomicBool = AtomicBool::new(false);
/// Bit set = the down event of this input was swallowed, so swallow its up.
static SWALLOWED: [AtomicU64; NUM_INPUTS / 64] = [const { AtomicU64::new(0) }; NUM_INPUTS / 64];

const WM_APP_REREGISTER: u32 = WM_APP + 1;
const TIMER_REREGISTER: usize = 1;

#[inline]
fn shared() -> &'static Shared {
    SHARED.get().expect("input thread not initialised")
}

#[inline]
fn send(msg: Msg) {
    if let Some(tx) = TX.get() {
        if let Err(TrySendError::Full(_)) = tx.try_send(msg) {
            shared().stats.dropped.fetch_add(1, Relaxed);
        }
    }
}

#[inline]
fn mark(id: InputId, on: bool) {
    let i = id as usize;
    if i >= NUM_INPUTS {
        return;
    }
    let bit = 1u64 << (i % 64);
    if on {
        SWALLOWED[i / 64].fetch_or(bit, Relaxed);
    } else {
        SWALLOWED[i / 64].fetch_and(!bit, Relaxed);
    }
}

#[inline]
fn is_marked(id: InputId) -> bool {
    let i = id as usize;
    i < NUM_INPUTS && SWALLOWED[i / 64].load(Relaxed) & (1u64 << (i % 64)) != 0
}

/// Decide whether to swallow a key / mouse-button transition. Hot path.
#[inline]
fn on_button(id: InputId, down: bool) -> bool {
    let sh = shared();
    let t = Instant::now();

    if down {
        // GUI "press a key to bind" capture.
        if sh.capture_next.load(Relaxed) {
            sh.captured.store(id as u32 + 1, Relaxed);
            sh.capture_next.store(false, Relaxed);
            mark(id, true);
            return true;
        }
        if id == sh.toggle_input.load(Relaxed) {
            if !is_marked(id) {
                send(Msg::ToggleEmulation); // ignore autorepeat
            }
            mark(id, true);
            return true;
        }
        if !sh.active.load(Acquire) {
            return false;
        }
        send(Msg::Input { id, down: true, t });
        let block = sh.is_blocked(id);
        mark(id, block);
        block
    } else {
        let swallowed = is_marked(id);
        if swallowed {
            mark(id, false);
        }
        if swallowed || sh.active.load(Acquire) {
            send(Msg::Input { id, down: false, t });
        }
        swallowed
    }
}

#[inline]
fn on_wheel(id: InputId) -> bool {
    let sh = shared();
    if sh.capture_next.load(Relaxed) {
        sh.captured.store(id as u32 + 1, Relaxed);
        sh.capture_next.store(false, Relaxed);
        return true;
    }
    if !sh.active.load(Acquire) {
        return false;
    }
    send(Msg::Wheel { id, t: Instant::now() });
    sh.is_blocked(id)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = &*(lparam as *const KBDLLHOOKSTRUCT);
        if k.flags & LLKHF_INJECTED == 0 {
            let msg = wparam as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            if on_button((k.vkCode & 0xFF) as InputId, down) {
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe fn mouse_move(m: &MSLLHOOKSTRUCT) -> bool {
    let sh = shared();
    let capture = sh.active.load(Acquire) && sh.capture_mouse.load(Relaxed);
    if capture != LAST_CAPTURE.swap(capture, Relaxed) && capture {
        // Make sure Raw Input still targets our window (the GUI toolkit may
        // have re-registered the mouse usage for itself).
        let hwnd = MSG_HWND.load(Relaxed);
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_APP_REREGISTER, 0, 0);
        }
    }
    if !capture {
        return false;
    }
    // The cursor stays frozen because we swallow the move, so the would-be
    // position minus the current position is the movement delta.
    let mut cur = POINT { x: 0, y: 0 };
    if GetCursorPos(&mut cur) == 0 {
        return true;
    }
    let dx = m.pt.x - cur.x;
    let dy = m.pt.y - cur.y;
    if dx != 0 || dy != 0 {
        send(Msg::MouseDelta { dx, dy, raw: false, t: Instant::now() });
    }
    if sh.recenter_cursor.load(Relaxed) {
        // Deltas get clamped at screen/clip edges; keep the cursor away from them.
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if GetClipCursor(&mut r) != 0 {
            const M: i32 = 48;
            if cur.x - r.left < M || r.right - cur.x < M || cur.y - r.top < M || r.bottom - cur.y < M {
                SetCursorPos((r.left + r.right) / 2, (r.top + r.bottom) / 2);
            }
        }
    }
    true
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let m = &*(lparam as *const MSLLHOOKSTRUCT);
        if m.flags & LLMHF_INJECTED == 0 {
            let wheel = ((m.mouseData >> 16) & 0xFFFF) as u16 as i16;
            let swallow = match wparam as u32 {
                WM_MOUSEMOVE => mouse_move(m),
                WM_LBUTTONDOWN => on_button(keys::MOUSE_LEFT, true),
                WM_LBUTTONUP => on_button(keys::MOUSE_LEFT, false),
                WM_RBUTTONDOWN => on_button(keys::MOUSE_RIGHT, true),
                WM_RBUTTONUP => on_button(keys::MOUSE_RIGHT, false),
                WM_MBUTTONDOWN => on_button(keys::MOUSE_MIDDLE, true),
                WM_MBUTTONUP => on_button(keys::MOUSE_MIDDLE, false),
                WM_XBUTTONDOWN | WM_XBUTTONUP => {
                    let id = if wheel as u16 == XBUTTON1 { keys::MOUSE_X1 } else { keys::MOUSE_X2 };
                    on_button(id, wparam as u32 == WM_XBUTTONDOWN)
                }
                WM_MOUSEWHEEL if wheel != 0 => on_wheel(if wheel > 0 { keys::WHEEL_UP } else { keys::WHEEL_DOWN }),
                WM_MOUSEHWHEEL if wheel != 0 => on_wheel(if wheel > 0 { keys::WHEEL_RIGHT } else { keys::WHEEL_LEFT }),
                _ => false,
            };
            if swallow {
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

fn register_raw_mouse(hwnd: HWND) {
    let dev = RAWINPUTDEVICE { usUsagePage: 0x01, usUsage: 0x02, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
    unsafe {
        RegisterRawInputDevices(&dev, 1, size_of::<RAWINPUTDEVICE>() as u32);
    }
}

unsafe fn on_raw_input(lparam: LPARAM) {
    let sh = shared();
    if !(sh.active.load(Acquire) && sh.capture_mouse.load(Relaxed)) {
        return;
    }
    let mut buf = MaybeUninit::<RAWINPUT>::uninit();
    let mut size = size_of::<RAWINPUT>() as u32;
    let n = GetRawInputData(lparam as HRAWINPUT, RID_INPUT, buf.as_mut_ptr() as *mut c_void, &mut size, size_of::<RAWINPUTHEADER>() as u32);
    if n == u32::MAX || n == 0 {
        return;
    }
    let ri = buf.assume_init_ref();
    if ri.header.dwType != RIM_TYPEMOUSE {
        return;
    }
    let mouse = &ri.data.mouse;
    const MOUSE_MOVE_ABSOLUTE: u16 = 0x01;
    if mouse.usFlags & MOUSE_MOVE_ABSOLUTE != 0 {
        return; // tablets / remote desktop: not relative motion
    }
    let (dx, dy) = (mouse.lLastX, mouse.lLastY);
    if dx != 0 || dy != 0 {
        send(Msg::MouseDelta { dx, dy, raw: true, t: Instant::now() });
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_INPUT => {
            on_raw_input(lparam);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_APP_REREGISTER => {
            register_raw_mouse(hwnd);
            0
        }
        WM_TIMER if wparam == TIMER_REREGISTER => {
            if shared().capture_mouse.load(Relaxed) {
                register_raw_mouse(hwnd);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Lower-cased executable file name of a process.
pub fn process_exe_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(|s| s.to_ascii_lowercase())
    }
}

fn report_foreground(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    if pid == 0 {
        return;
    }
    let own = pid == unsafe { GetCurrentProcessId() };
    let exe = process_exe_name(pid).unwrap_or_default();
    send(Msg::Focus { exe, own });
}

unsafe extern "system" fn win_event_proc(_h: HWINEVENTHOOK, event: u32, hwnd: HWND, _obj: i32, _child: i32, _thread: u32, _time: u32) {
    if event == EVENT_SYSTEM_FOREGROUND {
        report_foreground(hwnd);
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Spawn the capture thread. Returns once hooks are installed.
pub fn spawn(shared: Arc<Shared>, tx: SyncSender<Msg>) -> Result<std::thread::JoinHandle<()>, String> {
    let _ = SHARED.set(shared);
    let _ = TX.set(tx);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let handle = std::thread::Builder::new()
        .name("padforge-input".into())
        .spawn(move || unsafe {
            // Hook latency affects every input on the system: run at top priority.
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
            THREAD_ID.store(GetCurrentThreadId(), Relaxed);
            let hinst = GetModuleHandleW(null());

            let class = wide("PadForgeInputSink");
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinst,
                hIcon: null_mut(),
                hCursor: null_mut(),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: class.as_ptr(),
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(0, class.as_ptr(), null(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), hinst, null());
            if hwnd.is_null() {
                let _ = ready_tx.send(Err("failed to create input window".into()));
                return;
            }
            MSG_HWND.store(hwnd, Relaxed);
            register_raw_mouse(hwnd);
            SetTimer(hwnd, TIMER_REREGISTER, 3000, None);

            let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hinst, 0);
            let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hinst, 0);
            if kb.is_null() || ms.is_null() {
                let _ = ready_tx.send(Err("failed to install input hooks".into()));
                return;
            }
            let wev = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, null_mut(), Some(win_event_proc), 0, 0, WINEVENT_OUTOFCONTEXT);
            report_foreground(GetForegroundWindow());
            let _ = ready_tx.send(Ok(()));

            let mut msg = MaybeUninit::<MSG>::zeroed();
            while GetMessageW(msg.as_mut_ptr(), null_mut(), 0, 0) > 0 {
                TranslateMessage(msg.as_ptr());
                DispatchMessageW(msg.as_ptr());
            }

            UnhookWindowsHookEx(kb);
            UnhookWindowsHookEx(ms);
            if !wev.is_null() {
                UnhookWinEvent(wev);
            }
            KillTimer(hwnd, TIMER_REREGISTER);
            DestroyWindow(hwnd);
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("input thread exited during startup".into()),
    }
}

/// Ask the capture thread to unhook and exit.
pub fn stop() {
    let id = THREAD_ID.load(Relaxed);
    if id != 0 {
        unsafe {
            PostThreadMessageW(id, WM_QUIT, 0, 0);
        }
    }
}
