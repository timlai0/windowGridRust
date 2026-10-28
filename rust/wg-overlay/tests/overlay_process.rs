//! End-to-end test of the real overlay process, run with `--features isolated`:
//! `cargo test -p wg-overlay --features isolated --test overlay_process`.
//!
//! It starts the overlay with test-only names, a small test rectangle in the top-left corner
//! of the screen, and a window of its own as the snap target. It never touches any other
//! window, and a running WindowGridRust is unaffected (separate names). The overlay is visible
//! for about two seconds. A screenshot of the overlay area is written to `target/tmp/`.

#![cfg(all(windows, feature = "isolated"))]

use std::ffi::c_void;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::*;
use wg_core::ipc::{pack_point, GridWindowMemory, SettingsMemory, Wire};
use wg_core::settings::GridTheme;
use wg_overlay::gfx::wide;
use wg_overlay::names;
use wg_overlay::png;
use wg_overlay::shm::Section;

/// The tests share named sections and a window class, so they run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

const TEST_RECT: (i32, i32, i32, i32) = (40, 40, 1000, 580); // 960 x 540, 12 x 6 cells of 80 x 90

/// Kills the overlay if the test fails before shutting it down.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

unsafe extern "system" fn target_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(h, m, w, l)
}

fn pump(duration: Duration) {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        // SAFETY: standard message pump on the creating thread.
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn create_target() -> HWND {
    let class = wide("WgTestTarget");
    // SAFETY: standard class registration and window creation.
    unsafe {
        let hinstance = GetModuleHandleW(None).unwrap();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(target_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(class.as_ptr()),
            PCWSTR(wide("WindowGridRust test target").as_ptr()),
            WS_OVERLAPPEDWINDOW,
            1100,
            700,
            300,
            200,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
        .unwrap();
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        hwnd
    }
}

fn message_id(name: &str) -> u32 {
    let w = wide(name);
    // SAFETY: NUL-terminated name.
    unsafe { RegisterWindowMessageW(PCWSTR(w.as_ptr())) }
}

fn post(hwnd: HWND, msg: u32, w: usize, l: isize) {
    // SAFETY: posting to the overlay window.
    unsafe { PostMessageW(Some(hwnd), msg, WPARAM(w), LPARAM(l)).unwrap() };
}

fn capture(rect: (i32, i32, i32, i32)) -> Vec<u8> {
    let (w, h) = (rect.2 - rect.0, rect.3 - rect.1);
    // SAFETY: screen capture with matching DC/bitmap sizes; resources released at the end.
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bmp.into());
        BitBlt(dc, 0, 0, w, h, Some(screen), rect.0, rect.1, SRCCOPY | CAPTUREBLT).unwrap();
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0;
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        GetDIBits(dc, bmp, 0, h as u32, Some(bgra.as_mut_ptr().cast::<c_void>()), &mut bi, DIB_RGB_COLORS);
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        let mut rgba = bgra;
        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
            px[3] = 255;
        }
        rgba
    }
}

fn frame_bounds(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: valid out pointer and size.
    unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut r as *mut RECT).cast::<c_void>(), std::mem::size_of::<RECT>() as u32)
            .unwrap();
    }
    r
}

#[test]
fn the_overlay_draws_is_click_through_and_snaps() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: process-wide DPI setup so window coordinates are physical pixels.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // The settings section the app would normally write.
    let theme = GridTheme::default_theme();
    let memory = GridWindowMemory {
        grid_x: 12,
        grid_y: 6,
        radial_gradient: 1,
        blur_radius: 3.0,
        fill_window: 1,
        opacity: 0.75,
        color_line: theme.color_line.to_argb_i32(),
        color_start: theme.color_start.to_argb_i32(),
        color_end: theme.color_end.to_argb_i32(),
        show_selection_hud: 1,
        show_dimensions: 1,
        show_cell_span: 1,
        show_screen_coverage: 1,
        show_aspect_ratio: 1,
        show_mode_indicator: 1,
        show_interaction_hints: 1,
        anchor: 0,
        compensate_window_margins: 1,
        preserve_size_on_move: 1,
        defer_resize_until_release: 1,
    };
    let grid = Section::open_or_create(names::MAP_GRID_WINDOW, GridWindowMemory::SIZE).unwrap();
    grid.write_bytes(0, &memory.to_bytes());
    let hook = Section::open_or_create(names::MAP_HOOK, SettingsMemory::SIZE).unwrap();
    let handle = Section::open_or_create(names::MAP_GRID_WINDOW_HANDLE, 4).unwrap();

    let target = create_target();
    pump(Duration::from_millis(150));

    let child = Command::new(env!("CARGO_BIN_EXE_wg-overlay"))
        .env("WG_TEST_RECT", format!("{},{},{},{}", TEST_RECT.0, TEST_RECT.1, TEST_RECT.2, TEST_RECT.3))
        .env("WG_TEST_TARGET", (target.0 as usize).to_string())
        .env("WG_TEST_NO_TIMER", "1")
        .spawn()
        .expect("start the overlay");
    let mut child = KillOnDrop(child);

    // The overlay announces its window in both sections.
    let deadline = Instant::now() + Duration::from_secs(5);
    let overlay_hwnd = loop {
        let v = handle.read::<i32>();
        if v != 0 {
            break v;
        }
        assert!(Instant::now() < deadline, "overlay never published its window handle");
        std::thread::sleep(Duration::from_millis(20));
    };
    let hook_view: SettingsMemory = hook.read();
    assert_eq!({ hook_view.grid_window }, overlay_hwnd, "also written to the hook section");
    let overlay = HWND(overlay_hwnd as isize as *mut c_void);

    // Resize session: pointer in the middle of the overlay, selection (2,1)-(5,3).
    let (show, area, stop) = (message_id(names::MSG_SHOW_GRID), message_id(names::MSG_AREA), message_id(names::MSG_STOP));
    let center = pack_point((TEST_RECT.0 + TEST_RECT.2) / 2, (TEST_RECT.1 + TEST_RECT.3) / 2);
    post(overlay, show, 1, center as isize);
    post(overlay, area, pack_point(2, 1), pack_point(5, 3) as isize);
    pump(Duration::from_millis(500));

    // Visible: it is on screen, and the selection area is clearly tinted cyan.
    let img = capture(TEST_RECT);
    let (w, h) = ((TEST_RECT.2 - TEST_RECT.0) as u32, (TEST_RECT.3 - TEST_RECT.1) as u32);
    let file = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("overlay-live-capture.png");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, png::encode_rgba(w, h, &img)).unwrap();
    let px = |x: u32, y: u32| {
        let o = ((y * w + x) * 4) as usize;
        (i32::from(img[o]), i32::from(img[o + 1]), i32::from(img[o + 2]))
    };
    // Selection spans x 160..480, y 90..360 of the overlay; sample its centre.
    let (r, _g, b) = px(320, 225);
    assert!(b - r > 25, "selection should be tinted cyan, got rgb {:?}", px(320, 225));

    // Click-through: hit-testing at a point inside the overlay never lands on it.
    // SAFETY: plain query.
    let under = unsafe { WindowFromPoint(POINT { x: 520, y: 310 }) };
    assert_ne!(under, overlay, "the overlay must not capture the pointer");

    // Drag end: the overlay snaps the target to the selected cells.
    post(overlay, show, 0, 0);
    pump(Duration::from_millis(600));
    // Visible frame (excluding the invisible resize border) must equal the cells' rectangle:
    // x 40 + 2*80, y 40 + 1*90, 4 cells wide, 3 cells high.
    let f = frame_bounds(target);
    let expected = RECT { left: 40 + 160, top: 40 + 90, right: 40 + 160 + 320, bottom: 40 + 90 + 270 };
    for (name, got, want) in [("left", f.left, expected.left), ("top", f.top, expected.top), ("right", f.right, expected.right), ("bottom", f.bottom, expected.bottom)] {
        assert!((got - want).abs() <= 1, "snapped frame {name}: got {got}, want {want} (frame {f:?})");
    }

    // And the overlay is gone again (hidden, not destroyed).
    // SAFETY: plain query.
    assert!(!unsafe { IsWindowVisible(overlay) }.as_bool(), "overlay should hide after the drag");

    post(overlay, stop, 0, 0);
    let end = Instant::now() + Duration::from_secs(3);
    let exited = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() > end {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // SAFETY: destroying our own test window.
    unsafe {
        let _ = DestroyWindow(target);
    }
    if exited.is_none() {
        panic!("overlay did not exit on the stop message");
    }
    assert!(exited.unwrap().success());
}

/// Starting a second overlay replaces the first (the legacy overlay did the same).
#[test]
fn a_second_overlay_replaces_the_first() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let handle = Section::open_or_create(names::MAP_GRID_WINDOW_HANDLE, 4).unwrap();
    handle.write_bytes(0, &0i32.to_le_bytes());
    let _grid = Section::open_or_create(names::MAP_GRID_WINDOW, GridWindowMemory::SIZE).unwrap();

    let spawn = || {
        KillOnDrop(
            Command::new(env!("CARGO_BIN_EXE_wg-overlay"))
                .env("WG_TEST_RECT", "40,40,1000,580")
                .env("WG_TEST_NO_TIMER", "1")
                .spawn()
                .expect("start the overlay"),
        )
    };
    let wait_for_handle = |previous: i32| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let v = handle.read::<i32>();
            if v != 0 && v != previous {
                return v;
            }
            assert!(Instant::now() < deadline, "no new overlay window handle appeared");
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    let mut first = spawn();
    let first_hwnd = wait_for_handle(0);
    let mut second = spawn();
    let second_hwnd = wait_for_handle(first_hwnd);
    assert_ne!(first_hwnd, second_hwnd);

    let deadline = Instant::now() + Duration::from_secs(3);
    while first.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the first overlay should have been terminated");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(second.0.try_wait().unwrap().is_none(), "the second overlay keeps running");

    let stop = message_id(names::MSG_STOP);
    post(HWND(second_hwnd as isize as *mut c_void), stop, 0, 0);
    let deadline = Instant::now() + Duration::from_secs(3);
    while second.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the second overlay should stop on request");
        std::thread::sleep(Duration::from_millis(20));
    }
}
