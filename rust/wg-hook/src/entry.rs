//! Exports and callbacks: `RunDll`, the in-context WinEvent callback, the per-drag
//! input hooks and the window-procedure subclass (behavior: ADR 002).

use crate::grid::{Point, Rect};
use crate::log;
use crate::session::{
    Config, Effect, Host, Msg, MonitorGeometry, Session, WindowMetrics, WM_CAPTURECHANGED, WM_EXITSIZEMOVE,
    WM_GETMINMAXINFO, WM_MOVING, WM_NCDESTROY, WM_SIZING, WM_WINDOWPOSCHANGING,
};
use crate::sys::{self, wide, MINMAXINFO, MONITORINFO, MOUSEHOOKSTRUCTEX, MSG, POINT, RECT, WINDOWPOS};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use wg_core::ipc::{InputMemory, SettingsMemory, Wire, INPUT_MAP_SIZE, MAX_INPUTS};

/// Picks a name for the normal build or, with the `isolated` feature, a test-only name.
/// Isolated builds share no mutex, window message or shared-memory section with a running
/// WindowGridRust, so tests can never disturb (or be disturbed by) a real installation.
macro_rules! name {
    ($normal:literal, $isolated:literal) => {{
        #[cfg(not(feature = "isolated"))]
        {
            wide!($normal)
        }
        #[cfg(feature = "isolated")]
        {
            wide!($isolated)
        }
    }};
}

#[cfg(target_arch = "x86_64")]
const MUTEX_NAME: *const u16 =
    name!("Global\\{D82B85E1-BAA7-419F-AD0E-C1540EF8BFAB}", "Global\\{D82B85E1-BAA7-419F-AD0E-C1540EF8BFAB}-iso");
#[cfg(target_arch = "x86")]
const MUTEX_NAME: *const u16 =
    name!("Global\\{FD439CB8-8CFC-4209-BB19-DF8AA54B6E56}", "Global\\{FD439CB8-8CFC-4209-BB19-DF8AA54B6E56}-iso");
#[cfg(target_arch = "aarch64")]
const MUTEX_NAME: *const u16 =
    name!("Global\\{7F3A1C52-5B2E-4C8D-9A41-3E6D0B8C2F17}", "Global\\{7F3A1C52-5B2E-4C8D-9A41-3E6D0B8C2F17}-iso");

// ---- process-wide state ---------------------------------------------------------------

/// A cell that is `Sync` because each use is confined to a short, non-re-entrant section
/// on the single thread that is running the drag (see `with_session`).
struct Global<T>(UnsafeCell<T>);
// SAFETY: access is serialised by the one-drag-at-a-time modal loop; see `with_session`.
unsafe impl<T> Sync for Global<T> {}

static SESSION: Global<Option<Session>> = Global(UnsafeCell::new(None));

static HOOK_KEYBOARD: AtomicIsize = AtomicIsize::new(0);
static HOOK_MOUSE: AtomicIsize = AtomicIsize::new(0);
static ORIG_PROC: AtomicIsize = AtomicIsize::new(0);
static EVENT_HOOK: AtomicIsize = AtomicIsize::new(0);
static MSG_SHOW_GRID: AtomicU32 = AtomicU32::new(0);
static MSG_AREA: AtomicU32 = AtomicU32::new(0);
static MSG_TARGET: AtomicU32 = AtomicU32::new(0);
static MSG_STOP: AtomicU32 = AtomicU32::new(0);

/// Runs `f` on the session, if one exists.
///
/// # Safety
/// `f` must not call anything that can synchronously re-enter our callbacks (window
/// messages sent to the dragged window). The [`Host`] methods used by `Session` only
/// query state or `PostMessage`, which is asynchronous. Side effects that *can* re-enter
/// (raising the window) are returned as [`Effect`]s and run after this returns.
unsafe fn with_session<R>(f: impl FnOnce(&mut Session, &mut RealHost) -> R) -> Option<R> {
    let slot = &mut *SESSION.0.get();
    slot.as_mut().map(|s| f(s, &mut RealHost))
}

// ---- Host implementation -----------------------------------------------------------------

struct RealHost;

impl Host for RealHost {
    fn cursor_pos(&mut self) -> Point {
        let mut p = POINT::default();
        // SAFETY: valid out pointer.
        unsafe { sys::GetCursorPos(&mut p) };
        p.into()
    }

    fn monitor_from_point(&mut self, pt: Point) -> isize {
        // SAFETY: plain call.
        unsafe { sys::MonitorFromPoint(POINT { x: pt.x, y: pt.y }, sys::MONITOR_DEFAULTTONEAREST) }
    }

    fn monitor_geometry(&mut self, monitor: isize) -> Option<MonitorGeometry> {
        let mut mi = MONITORINFO { cb_size: core::mem::size_of::<MONITORINFO>() as u32, ..MONITORINFO::default() };
        // SAFETY: `mi.cb_size` is set and the pointer is valid.
        if unsafe { sys::GetMonitorInfoW(monitor, &mut mi) } == 0 {
            return None;
        }
        Some(MonitorGeometry { work: mi.rc_work.into(), monitor: mi.rc_monitor.into() })
    }

    fn window_metrics(&mut self, hwnd: isize) -> WindowMetrics {
        let mut window = RECT::default();
        let mut frame = RECT::default();
        // SAFETY: valid out pointers; sizes match.
        unsafe {
            sys::GetWindowRect(hwnd, &mut window);
            let hr = sys::DwmGetWindowAttribute(
                hwnd,
                sys::DWMWA_EXTENDED_FRAME_BOUNDS,
                (&mut frame as *mut RECT).cast(),
                core::mem::size_of::<RECT>() as u32,
            );
            let frame = if hr >= 0 {
                // Frame bounds are physical pixels; convert for DPI-virtualised windows.
                let mut tl = POINT { x: frame.left, y: frame.top };
                let mut br = POINT { x: frame.right, y: frame.bottom };
                if sys::PhysicalToLogicalPointForPerMonitorDPI(hwnd, &mut tl) != 0
                    && sys::PhysicalToLogicalPointForPerMonitorDPI(hwnd, &mut br) != 0
                {
                    frame = RECT { left: tl.x, top: tl.y, right: br.x, bottom: br.y };
                }
                Some(Rect::from(frame))
            } else {
                None
            };
            WindowMetrics { window: window.into(), frame, maximized: sys::IsZoomed(hwnd) != 0 }
        }
    }

    fn post_overlay(&mut self, overlay: isize, msg: Msg, wparam: usize, lparam: isize) {
        let id = match msg {
            Msg::ShowGrid => MSG_SHOW_GRID.load(Ordering::Relaxed),
            Msg::Area => MSG_AREA.load(Ordering::Relaxed),
            Msg::Target => MSG_TARGET.load(Ordering::Relaxed),
        };
        if id != 0 {
            // SAFETY: plain call; failure (overlay gone) is harmless.
            unsafe { sys::PostMessageW(overlay, id, wparam, lparam) };
        }
    }

    fn set_cursor_pos(&mut self, x: i32, y: i32) {
        // SAFETY: plain call.
        unsafe { sys::SetCursorPos(x, y) };
    }

    fn show_normal_and_raise(&mut self, hwnd: isize) {
        // SAFETY: plain calls. These send WM_WINDOWPOSCHANGING synchronously.
        unsafe {
            sys::ShowWindow(hwnd, sys::SW_SHOWNORMAL);
            sys::BringWindowToTop(hwnd);
        }
    }
}

fn run_effect(effect: Effect, hwnd: isize) {
    effect.run(&mut RealHost, hwnd);
}

// ---- exports ------------------------------------------------------------------------------

#[no_mangle]
pub extern "system" fn DllMain(module: isize, _reason: u32, _reserved: *mut u8) -> i32 {
    log::set_module(module);
    1
}

fn register_messages() {
    // SAFETY: the names are NUL-terminated wide literals.
    unsafe {
        MSG_SHOW_GRID.store(sys::RegisterWindowMessageW(name!("WindowGridRustShowGrid", "WindowGridRustIsoShowGrid")), Ordering::Relaxed);
        MSG_AREA.store(sys::RegisterWindowMessageW(name!("WindowGridRustArea", "WindowGridRustIsoArea")), Ordering::Relaxed);
        MSG_TARGET.store(sys::RegisterWindowMessageW(name!("WindowGridRustTarget", "WindowGridRustIsoTarget")), Ordering::Relaxed);
        MSG_STOP.store(sys::RegisterWindowMessageW(name!("WindowGridRustStop", "WindowGridRustIsoStop")), Ordering::Relaxed);
    }
}

/// `rundll32.exe <dll>,RunDll`: installs the WinEvent hook and pumps messages until the
/// app broadcasts `WindowGridRustStop`.
#[no_mangle]
pub extern "system" fn RunDll(hwnd: isize, _instance: isize, _cmdline: *const u8, _show: i32) {
    // SAFETY: Win32 calls with valid arguments.
    unsafe {
        let mutex = sys::CreateMutexW(core::ptr::null(), 0, MUTEX_NAME);
        let already_running = sys::GetLastError() == sys::ERROR_ALREADY_EXISTS;
        if !already_running {
            run_message_loop(hwnd);
        }
        if mutex != 0 {
            sys::CloseHandle(mutex);
        }
    }
}

unsafe fn run_message_loop(hwnd: isize) {
    log::info(concat!("Starting WindowGridRust hook (", env!("CARGO_PKG_VERSION"), ")"));
    register_messages();
    let stop = MSG_STOP.load(Ordering::Relaxed);
    sys::ChangeWindowMessageFilterEx(hwnd, stop, sys::MSGFLT_ALLOW, core::ptr::null_mut());

    let hook = sys::SetWinEventHook(
        sys::EVENT_SYSTEM_MOVESIZESTART,
        sys::EVENT_SYSTEM_MOVESIZEEND,
        log::module(),
        win_event_proc,
        0,
        0,
        sys::WINEVENT_INCONTEXT | sys::WINEVENT_SKIPOWNPROCESS,
    );
    if hook == 0 {
        log::error("SetWinEventHook failed", sys::GetLastError());
        return;
    }
    EVENT_HOOK.store(hook, Ordering::Relaxed);

    let mut msg = MSG::default();
    loop {
        let r = sys::GetMessageW(&mut msg, 0, 0, 0);
        if r == 0 {
            break;
        }
        if r < 0 {
            log::error("GetMessage failed", sys::GetLastError());
            break;
        }
        if msg.message == stop {
            break;
        }
        sys::TranslateMessage(&msg);
        sys::DispatchMessageW(&msg);
    }

    if sys::UnhookWinEvent(hook) == 0 {
        log::error("UnhookWinEvent failed", sys::GetLastError());
    }
    EVENT_HOOK.store(0, Ordering::Relaxed);
    log::info("Exiting WindowGridRust hook");
}

// ---- WinEvent callback (runs inside the dragged window's process) -----------------------

unsafe extern "system" fn win_event_proc(
    _hook: isize,
    event: u32,
    hwnd: isize,
    _id_object: i32,
    _id_child: i32,
    event_thread: u32,
    _time: u32,
) {
    if hwnd == 0 || event_thread != sys::GetCurrentThreadId() {
        return;
    }
    match event {
        sys::EVENT_SYSTEM_MOVESIZESTART => begin_session(hwnd),
        sys::EVENT_SYSTEM_MOVESIZEEND => end_session(hwnd),
        _ => {}
    }
}

/// Reads `bytes` from a named, read-only section into `out`; false if it can't be mapped.
unsafe fn read_section(name: *const u16, bytes: usize, out: &mut [u8]) -> bool {
    let map = sys::OpenFileMappingW(sys::FILE_MAP_READ, 0, name);
    if map == 0 {
        return false;
    }
    let view = sys::MapViewOfFile(map, sys::FILE_MAP_READ, 0, 0, bytes);
    let ok = !view.is_null();
    if ok {
        core::ptr::copy_nonoverlapping(view, out.as_mut_ptr(), bytes.min(out.len()));
        sys::UnmapViewOfFile(view);
    }
    sys::CloseHandle(map);
    ok
}

unsafe fn begin_session(hwnd: isize) {
    if (*SESSION.0.get()).is_some() {
        // A previous session was never closed (lost event): clean it up first.
        let old = (*SESSION.0.get()).as_ref().map_or(0, Session::hwnd);
        end_session(old);
    }
    register_messages();

    let mut hook_bytes = [0u8; SettingsMemory::SIZE];
    let mut input_bytes = [0u8; INPUT_MAP_SIZE];
    if !read_section(name!("WindowGridRustMapping_Hook", "WindowGridRustIsoMapping_Hook"), SettingsMemory::SIZE, &mut hook_bytes)
        || !read_section(name!("WindowGridRustMapping_Input", "WindowGridRustIsoMapping_Input"), INPUT_MAP_SIZE, &mut input_bytes)
    {
        log::error("Unable to map memory", sys::GetLastError());
        return;
    }
    let Some(hook) = SettingsMemory::from_bytes(&hook_bytes) else { return };
    let mut inputs = [InputMemory::default(); MAX_INPUTS];
    let input_count = InputMemory::read_section(&input_bytes, hook.input_size.max(0) as usize, &mut inputs);

    let mut drag_full: i32 = 0;
    sys::SystemParametersInfoW(sys::SPI_GETDRAGFULLWINDOWS, 0, (&mut drag_full as *mut i32).cast(), 0);
    let mut dwm: i32 = 0;
    let dwm_ok = sys::DwmIsCompositionEnabled(&mut dwm) >= 0 && dwm != 0;

    let cfg = Config {
        hwnd,
        overlay: hook.grid_window as isize,
        grid_x: hook.columns(),
        grid_y: hook.grid_y,
        defer_resize: hook.defer_resize(),
        drag_full_windows: drag_full != 0,
        dwm_enabled: dwm_ok,
        inputs,
        input_count,
    };
    *SESSION.0.get() = Some(Session::new(cfg));

    let prev = sys::set_window_proc(hwnd, subclass_proc as *const () as isize);
    if prev == 0 {
        log::error("SetWindowProc failed", sys::GetLastError());
        end_session(hwnd);
        return;
    }
    ORIG_PROC.store(prev, Ordering::Relaxed);

    // The modal move/size loop pumps messages on this thread, so thread-scoped hooks see
    // everything the grid needs (the original installed global hooks).
    let tid = sys::GetCurrentThreadId();
    let module = log::module();
    let mouse = sys::SetWindowsHookExW(sys::WH_MOUSE, mouse_proc, module, tid);
    let keyboard = sys::SetWindowsHookExW(sys::WH_KEYBOARD, keyboard_proc, module, tid);
    HOOK_MOUSE.store(mouse, Ordering::Relaxed);
    HOOK_KEYBOARD.store(keyboard, Ordering::Relaxed);
    if mouse == 0 || keyboard == 0 {
        log::error("SetWindowsHookEx failed", sys::GetLastError());
    }
}

unsafe fn end_session(hwnd: isize) {
    let slot = &mut *SESSION.0.get();
    let matches = slot.as_ref().is_some_and(|s| s.hwnd() == hwnd);
    if !matches {
        return;
    }
    for h in [&HOOK_MOUSE, &HOOK_KEYBOARD] {
        let handle = h.swap(0, Ordering::Relaxed);
        if handle != 0 && sys::UnhookWindowsHookEx(handle) == 0 {
            log::error("UnhookWindowsHookEx failed", sys::GetLastError());
        }
    }
    if let Some(mut s) = slot.take() {
        s.end(&mut RealHost);
    }
    let orig = ORIG_PROC.swap(0, Ordering::Relaxed);
    if orig != 0 {
        sys::set_window_proc(hwnd, orig);
    }
}

// ---- subclass procedure -----------------------------------------------------------------

unsafe extern "system" fn subclass_proc(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize {
    let orig = ORIG_PROC.load(Ordering::Relaxed);
    let ours = with_session(|s, _| s.hwnd() == hwnd).unwrap_or(false);
    if ours && msg == WM_NCDESTROY {
        // Always clean up when the window dies, even after Esc cancelled the grid.
        end_session(hwnd);
        return sys::CallWindowProcW(orig, hwnd, msg, wparam, lparam);
    }
    let intercept = ours && with_session(|s, _| s.intercepting()).unwrap_or(false);
    if !intercept {
        return sys::CallWindowProcW(orig, hwnd, msg, wparam, lparam);
    }

    let mut msg_out = msg;
    match msg {
        WM_MOVING => {
            if let Some(action) = with_session(|s, h| s.on_moving(h)) {
                if action.to_sizing {
                    msg_out = WM_SIZING;
                }
                if let (Some(r), true) = (action.rect, lparam != 0) {
                    *(lparam as *mut RECT) = r.into();
                }
            }
        }
        WM_WINDOWPOSCHANGING => {
            if let Some(Some(update)) = with_session(|s, h| s.on_window_pos_changing(h)) {
                if lparam != 0 {
                    let pos = &mut *(lparam as *mut WINDOWPOS);
                    pos.x = update.x;
                    pos.y = update.y;
                    pos.cx = update.cx;
                    pos.cy = update.cy;
                    pos.flags &= !(crate::session::SWP_NOSIZE | crate::session::SWP_NOMOVE);
                }
            }
        }
        WM_CAPTURECHANGED => {
            with_session(|s, _| s.on_capture_changed());
        }
        _ => {}
    }

    let result = sys::CallWindowProcW(orig, hwnd, msg_out, wparam, lparam);

    match msg {
        WM_GETMINMAXINFO if lparam != 0 => {
            let info = &*(lparam as *const MINMAXINFO);
            let (min, max) = (info.min_track_size, info.max_track_size);
            with_session(|s, _| {
                s.on_min_max_info(Point::new(min.x, min.y), Point::new(max.x, max.y));
            });
        }
        WM_EXITSIZEMOVE => {
            with_session(|s, _| s.cancel());
        }
        _ => {}
    }
    result
}

// ---- input hooks --------------------------------------------------------------------------

fn dispatch_input(key: i32, pressed: bool, released: bool) {
    // SAFETY: see `with_session`; the effect runs after the borrow ends.
    let (effect, hwnd) = match unsafe { with_session(|s, _| (s.on_input(key, pressed, released), s.hwnd())) } {
        Some(v) => v,
        None => return,
    };
    run_effect(effect, hwnd);
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code >= 0 && code != sys::HC_NOREMOVE {
        let l = lparam as u32;
        let scan = (l >> 16) & 0xFF;
        let extended = (l >> 24) & 1 != 0;
        let mut key = if wparam == sys::VK_PAUSE {
            0xC5
        } else if scan != 0 {
            scan
        } else {
            sys::MapVirtualKeyW(wparam as u32, 0) & 0xFFFF
        };
        if extended {
            key |= 0x80;
        }
        let transition = l >> 30;
        let (pressed, released) = (transition == 0, transition == 3);
        if wparam == sys::VK_ESCAPE {
            with_session(|s, _| s.cancel());
        } else {
            dispatch_input(key as i32, pressed, released);
        }
    }
    sys::CallNextHookEx(0, code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == sys::HC_ACTION {
        let (key, pressed, released) = match wparam {
            0x201 => (0x101, true, false),
            0x202 => (0x101, false, true),
            0x204 => (0x102, true, false),
            0x205 => (0x102, false, true),
            0x207 => (0x103, true, false),
            0x208 => (0x103, false, true),
            0x20B | 0x20C if lparam != 0 => {
                let data = (*(lparam as *const MOUSEHOOKSTRUCTEX)).mouse_data;
                let key = if (data >> 16) as u16 != 1 { 0x105 } else { 0x104 };
                (key, wparam == 0x20B, wparam == 0x20C)
            }
            _ => (0, false, false),
        };
        dispatch_input(key, pressed, released);
    }
    sys::CallNextHookEx(0, code, wparam, lparam)
}
