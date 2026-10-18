//! Win32 declarations the hook needs. Imports use `raw-dylib`, so linking needs neither
//! an import library nor the Windows SDK (and the DLL has no C runtime dependency).

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use crate::grid::{Point, Rect};

macro_rules! imports {
    ($lib:literal { $( fn $name:ident($($arg:ident : $ty:ty),* $(,)?) $(-> $ret:ty)?; )* }) => {
        #[cfg_attr(target_arch = "x86", link(name = $lib, kind = "raw-dylib", import_name_type = "undecorated"))]
        #[cfg_attr(not(target_arch = "x86"), link(name = $lib, kind = "raw-dylib"))]
        extern "system" {
            $( pub fn $name($($arg: $ty),*) $(-> $ret)?; )*
        }
    };
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MONITORINFO {
    pub cb_size: u32,
    pub rc_monitor: RECT,
    pub rc_work: RECT,
    pub flags: u32,
}

#[repr(C)]
pub struct WINDOWPOS {
    pub hwnd: isize,
    pub hwnd_insert_after: isize,
    pub x: i32,
    pub y: i32,
    pub cx: i32,
    pub cy: i32,
    pub flags: u32,
}

#[repr(C)]
pub struct MINMAXINFO {
    pub reserved: POINT,
    pub max_size: POINT,
    pub max_position: POINT,
    pub min_track_size: POINT,
    pub max_track_size: POINT,
}

#[repr(C)]
pub struct MOUSEHOOKSTRUCTEX {
    pub pt: POINT,
    pub hwnd: isize,
    pub hit_test_code: u32,
    pub extra_info: usize,
    pub mouse_data: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct MSG {
    pub hwnd: isize,
    pub message: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub time: u32,
    pub pt: POINT,
}

#[repr(C)]
#[derive(Default)]
pub struct SYSTEMTIME {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Rect {
        Rect::new(r.left, r.top, r.right, r.bottom)
    }
}

impl From<Rect> for RECT {
    fn from(r: Rect) -> RECT {
        RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

impl From<POINT> for Point {
    fn from(p: POINT) -> Point {
        Point::new(p.x, p.y)
    }
}

pub type WinEventProc = unsafe extern "system" fn(isize, u32, isize, i32, i32, u32, u32);
pub type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

imports!("kernel32" {
    fn CreateMutexW(attrs: *const u8, initial_owner: i32, name: *const u16) -> isize;
    fn ReleaseMutex(h: isize) -> i32;
    fn WaitForSingleObject(h: isize, millis: u32) -> u32;
    fn GetLastError() -> u32;
    fn CloseHandle(h: isize) -> i32;
    fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> isize;
    fn MapViewOfFile(h: isize, access: u32, off_hi: u32, off_lo: u32, bytes: usize) -> *mut u8;
    fn UnmapViewOfFile(p: *const u8) -> i32;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcessId() -> u32;
    fn GetModuleFileNameW(module: isize, buf: *mut u16, size: u32) -> u32;
    fn GetLocalTime(st: *mut SYSTEMTIME);
    fn CreateFileW(name: *const u16, access: u32, share: u32, sec: *const u8, disp: u32, flags: u32, template: isize) -> isize;
    fn WriteFile(h: isize, buf: *const u8, n: u32, written: *mut u32, overlapped: *mut u8) -> i32;
});

imports!("user32" {
    fn SetWinEventHook(min: u32, max: u32, module: isize, proc_: WinEventProc, pid: u32, tid: u32, flags: u32) -> isize;
    fn UnhookWinEvent(h: isize) -> i32;
    fn SetWindowsHookExW(id: i32, proc_: HookProc, module: isize, tid: u32) -> isize;
    fn UnhookWindowsHookEx(h: isize) -> i32;
    fn CallNextHookEx(h: isize, code: i32, wparam: usize, lparam: isize) -> isize;
    fn CallWindowProcW(prev: isize, hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn ChangeWindowMessageFilterEx(hwnd: isize, msg: u32, action: u32, change: *mut u8) -> i32;
    fn PostMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn GetMessageW(msg: *mut MSG, hwnd: isize, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> isize;
    fn GetCursorPos(pt: *mut POINT) -> i32;
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn MonitorFromPoint(pt: POINT, flags: u32) -> isize;
    fn GetMonitorInfoW(monitor: isize, info: *mut MONITORINFO) -> i32;
    fn GetWindowRect(hwnd: isize, rect: *mut RECT) -> i32;
    fn IsZoomed(hwnd: isize) -> i32;
    fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
    fn BringWindowToTop(hwnd: isize) -> i32;
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut u8, win_ini: u32) -> i32;
    fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
    fn PhysicalToLogicalPointForPerMonitorDPI(hwnd: isize, pt: *mut POINT) -> i32;
});

imports!("dwmapi" {
    fn DwmGetWindowAttribute(hwnd: isize, attr: u32, pv: *mut u8, size: u32) -> i32;
    fn DwmIsCompositionEnabled(enabled: *mut i32) -> i32;
});

#[cfg(not(target_arch = "x86"))]
imports!("user32" {
    fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
});

#[cfg(target_arch = "x86")]
imports!("user32" {
    fn SetWindowLongW(hwnd: isize, index: i32, value: isize) -> isize;
});

/// `SetWindowLongPtrW` on both bitnesses (32-bit user32 only exports `SetWindowLongW`).
#[cfg(not(target_arch = "x86"))]
pub unsafe fn set_window_proc(hwnd: isize, proc_: isize) -> isize {
    SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc_)
}

#[cfg(target_arch = "x86")]
pub unsafe fn set_window_proc(hwnd: isize, proc_: isize) -> isize {
    SetWindowLongW(hwnd, GWLP_WNDPROC, proc_)
}

pub const GWLP_WNDPROC: i32 = -4;
pub const EVENT_SYSTEM_MOVESIZESTART: u32 = 0x000A;
pub const EVENT_SYSTEM_MOVESIZEEND: u32 = 0x000B;
pub const WINEVENT_INCONTEXT: u32 = 0x0004;
pub const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;
pub const WH_KEYBOARD: i32 = 2;
pub const WH_MOUSE: i32 = 7;
pub const HC_ACTION: i32 = 0;
pub const HC_NOREMOVE: i32 = 3;
pub const FILE_MAP_READ: u32 = 0x0004;
pub const SPI_GETDRAGFULLWINDOWS: u32 = 0x0026;
pub const MONITOR_DEFAULTTONEAREST: u32 = 2;
pub const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
pub const SW_SHOWNORMAL: i32 = 1;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const MSGFLT_ALLOW: u32 = 1;
pub const VK_ESCAPE: usize = 0x1B;
pub const VK_PAUSE: usize = 0x13;
pub const WAIT_OBJECT_0: u32 = 0;
pub const FILE_APPEND_DATA: u32 = 0x0004;
pub const FILE_SHARE_READ_WRITE: u32 = 0x0003;
pub const OPEN_ALWAYS: u32 = 4;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

/// NUL-terminated UTF-16 literal.
macro_rules! wide {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const N: usize = S.len();
        const fn encode() -> [u16; N] {
            let b = S.as_bytes();
            let mut out = [0u16; N];
            let mut i = 0;
            while i < N {
                out[i] = b[i] as u16;
                i += 1;
            }
            out
        }
        static W: [u16; N] = encode();
        W.as_ptr()
    }};
}
pub(crate) use wide;
