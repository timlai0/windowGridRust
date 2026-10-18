//! The notification-area icon: a hidden window that owns the icon and its menu.
//!
//! It is an ordinary window on the UI thread, so the Slint event loop (which pumps that
//! thread's messages) delivers its events; no separate loop is needed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wg_win::wide;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    Settings,
    Install,
    Exit,
}

const WM_TRAY: u32 = WM_APP + 1;
const ID_SETTINGS: usize = 1;
const ID_INSTALL: usize = 2;
const ID_EXIT: usize = 3;

type Handler = Rc<dyn Fn(TrayEvent)>;

thread_local! {
    static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static TOOLTIP: RefCell<String> = const { RefCell::new(String::new()) };
}

pub struct Tray {
    hwnd: HWND,
}

fn icon() -> HICON {
    // SAFETY: loads the icon resource embedded in the executable (id 1) at the small-icon
    // size; without a resource (no resource compiler at build time) the generic icon is used.
    unsafe {
        let (cx, cy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        if let Ok(h) = GetModuleHandleW(None) {
            if let Ok(i) = LoadImageW(Some(h.into()), PCWSTR(std::ptr::dangling::<u8>().cast()), IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR) {
                return HICON(i.0);
            }
        }
        LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
    }
}

fn notify_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon(),
        ..Default::default()
    };
    TOOLTIP.with(|t| {
        for (slot, c) in nid.szTip.iter_mut().zip(t.borrow().encode_utf16().take(127)) {
            *slot = c;
        }
    });
    nid
}

fn dispatch(e: TrayEvent) {
    let handler = HANDLER.with(|h| h.borrow().clone());
    if let Some(h) = handler {
        h(e);
    }
}

fn show_menu(hwnd: HWND) {
    // SAFETY: a popup menu created, shown and destroyed within this call.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let settings = wide("Settings");
        let install = wide("Install / Setup\u{2026}");
        let exit = wide("Exit");
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, PCWSTR(settings.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_INSTALL, PCWSTR(install.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, PCWSTR(exit.as_ptr()));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // The window must be foreground for the menu to close when the user clicks away.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY, pt.x, pt.y, Some(0), hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match cmd.0 as usize {
            ID_SETTINGS => dispatch(TrayEvent::Settings),
            ID_INSTALL => dispatch(TrayEvent::Install),
            ID_EXIT => dispatch(TrayEvent::Exit),
            _ => {}
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_TRAY {
        match (lparam.0 as u32) & 0xFFFF {
            WM_LBUTTONDBLCLK => dispatch(TrayEvent::Settings),
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
            _ => {}
        }
        return LRESULT(0);
    }
    if msg != 0 && msg == TASKBAR_CREATED.with(Cell::get) {
        // Explorer restarted: its notification area is empty again.
        let _ = Shell_NotifyIconW(NIM_ADD, &notify_data(hwnd));
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

impl Tray {
    /// Adds the icon. `on_event` runs on the UI thread.
    pub fn create(tooltip: &str, on_event: impl Fn(TrayEvent) + 'static) -> windows::core::Result<Tray> {
        HANDLER.with(|h| *h.borrow_mut() = Some(Rc::new(on_event)));
        TOOLTIP.with(|t| *t.borrow_mut() = tooltip.to_string());
        let class = wide("WindowGridTrayWindow");
        let title = wide("WindowGrid");
        let created = wide("TaskbarCreated");
        // SAFETY: standard class registration and creation of a hidden window.
        unsafe {
            TASKBAR_CREATED.with(|t| t.set(RegisterWindowMessageW(PCWSTR(created.as_ptr()))));
            let hinstance = GetModuleHandleW(None)?;
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            RegisterClassExW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinstance.into()),
                None,
            )?;
            Shell_NotifyIconW(NIM_ADD, &notify_data(hwnd)).ok()?;
            Ok(Tray { hwnd })
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // SAFETY: removes the icon added in `create` and destroys the hidden window.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &notify_data(self.hwnd));
            let _ = DestroyWindow(self.hwnd);
        }
        HANDLER.with(|h| *h.borrow_mut() = None);
    }
}
