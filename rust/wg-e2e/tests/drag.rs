//! Real window drags with the Rust hook DLL and the Rust overlay, both isolated builds.
//!
//! Run with WindowGridRust CLOSED (a running one would react to the same drags), after building
//! the isolated artifacts:
//!
//! ```text
//! .\build-hooks.ps1 -Isolated ; .\build-overlay.ps1 -Isolated
//! cargo test -p wg-e2e -- --nocapture --test-threads=1
//! ```
//!
//! It takes control of the mouse and keyboard for a few seconds per scenario: a test window
//! is dragged by its caption on the primary monitor with real input events, and the result
//! is checked against the grid. Nothing is hard-coded to a monitor size.

#![cfg(windows)]

use std::ffi::c_void;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use wg_core::ipc::{GridWindowMemory, InputMemory, SettingsMemory, Wire, DEFER_FLAG};
use wg_core::settings::{GridTheme, Settings};
use wg_overlay::gfx::wide;
use wg_overlay::names;
use wg_overlay::png;
use wg_overlay::shm::Section;

const COLS: i32 = 12;
const ROWS: i32 = 6;
const SCAN_ESC: u16 = 0x01;
const SCAN_CTRL: u16 = 0x1D;
const SCAN_SPACE: u16 = 0x39;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

// ---- processes -------------------------------------------------------------------------

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// ---- input -----------------------------------------------------------------------------

fn screen() -> (i32, i32) {
    // SAFETY: plain queries (the process is per-monitor DPI aware, so these are pixels).
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

fn send(input: INPUT) {
    // SAFETY: a single, fully initialised INPUT.
    unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
}

fn mouse(flags: MOUSE_EVENT_FLAGS, x: i32, y: i32) {
    send(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: x, dy: y, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    });
}

fn move_to(x: i32, y: i32) {
    let (w, h) = screen();
    let nx = (i64::from(x) * 65535 / i64::from(w - 1)) as i32;
    let ny = (i64::from(y) * 65535 / i64::from(h - 1)) as i32;
    mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, nx, ny);
}

fn button(down: bool) {
    mouse(if down { MOUSEEVENTF_LEFTDOWN } else { MOUSEEVENTF_LEFTUP }, 0, 0);
}

fn key(scan: u16, down: bool) {
    let mut flags = KEYEVENTF_SCANCODE;
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }
    send(INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    });
}

fn tap(scan: u16) {
    key(scan, true);
    thread::sleep(Duration::from_millis(120));
    key(scan, false);
    thread::sleep(Duration::from_millis(120));
}

fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    // SAFETY: valid out pointer.
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// Moves in small steps so the modal move loop sees a real drag.
fn glide(to: (i32, i32)) {
    let from = cursor();
    let dist = (((to.0 - from.0).pow(2) + (to.1 - from.1).pow(2)) as f64).sqrt();
    let steps = (dist / 18.0).ceil().max(1.0) as i32;
    for i in 1..=steps {
        move_to(from.0 + (to.0 - from.0) * i / steps, from.1 + (to.1 - from.1) * i / steps);
        thread::sleep(Duration::from_millis(7));
    }
}

// ---- test window -------------------------------------------------------------------------

unsafe extern "system" fn target_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(h, m, w, l)
}

struct TestWindow {
    hwnd: HWND,
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl TestWindow {
    fn spawn(x: i32, y: i32, w: i32, h: i32) -> TestWindow {
        let (tx, rx) = mpsc::channel();
        let join = thread::spawn(move || {
            let class = wide("WgDragTarget");
            // SAFETY: standard class registration, window creation and message loop on this thread.
            unsafe {
                let hinstance = GetModuleHandleW(None).unwrap();
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(target_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: PCWSTR(class.as_ptr()),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
                    ..Default::default()
                };
                RegisterClassExW(&wc);
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR(class.as_ptr()),
                    PCWSTR(wide("WindowGridRust drag test").as_ptr()),
                    WS_OVERLAPPEDWINDOW,
                    x,
                    y,
                    w,
                    h,
                    None,
                    None,
                    Some(hinstance.into()),
                    None,
                )
                .unwrap();
                let _ = ShowWindow(hwnd, SW_SHOW);
                tx.send((hwnd.0 as isize, windows::Win32::System::Threading::GetCurrentThreadId())).unwrap();
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let _ = DestroyWindow(hwnd);
            }
        });
        let (h, thread_id) = rx.recv().unwrap();
        thread::sleep(Duration::from_millis(250));
        TestWindow { hwnd: HWND(h as *mut c_void), thread_id, join: Some(join) }
    }

    fn rect(&self) -> RECT {
        let mut r = RECT::default();
        // SAFETY: valid out pointer.
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut r);
        }
        r
    }

    /// The visible frame (without the invisible resize border).
    fn frame(&self) -> RECT {
        let mut r = RECT::default();
        // SAFETY: valid out pointer and size.
        unsafe {
            DwmGetWindowAttribute(self.hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut r as *mut RECT).cast::<c_void>(), std::mem::size_of::<RECT>() as u32)
                .unwrap();
        }
        r
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        // SAFETY: asking our window thread to leave its message loop.
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

// ---- screenshots ---------------------------------------------------------------------------

/// Captures the primary monitor at one third of its size and writes a PNG.
fn screenshot(name: &str) {
    let (w, h) = screen();
    let (tw, th) = (w / 3, h / 3);
    // SAFETY: GDI capture with matching DC/bitmap sizes; every resource is released.
    let rgba = unsafe {
        let screen_dc = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen_dc));
        let bmp = CreateCompatibleBitmap(screen_dc, tw, th);
        let old = SelectObject(dc, bmp.into());
        SetStretchBltMode(dc, HALFTONE);
        let _ = StretchBlt(dc, 0, 0, tw, th, Some(screen_dc), 0, 0, w, h, SRCCOPY | CAPTUREBLT);
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = tw;
        bi.bmiHeader.biHeight = -th;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0;
        let mut px = vec![0u8; (tw * th * 4) as usize];
        GetDIBits(dc, bmp, 0, th as u32, Some(px.as_mut_ptr().cast::<c_void>()), &mut bi, DIB_RGB_COLORS);
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen_dc);
        for p in px.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            p[3] = 255;
        }
        px
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("drag-{name}.png")), png::encode_rgba(tw as u32, th as u32, &rgba)).unwrap();
}

// ---- the rig -----------------------------------------------------------------------------------

struct Rig {
    /// The processes under test: the overlay and hook host (harness) or just the app.
    _procs: Vec<Proc>,
    hook_map: Section,
    _grid: Section,
    _input: Section,
    handle: Section,
    work: RECT,
    log: PathBuf,
    /// True when `wg-app` owns the settings, the sections and the child processes.
    app_owned: bool,
}

fn work_area() -> RECT {
    // SAFETY: valid out pointers.
    unsafe {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        mi.rcWork
    }
}

impl Rig {
    fn start() -> Rig {
        let dist = root().join("dist-iso");
        let dll = dist.join("WindowGridRust64.dll");
        let exe = dist.join("WindowGridRustOverlay.exe");
        for f in [&dll, &exe] {
            assert!(f.exists(), "{} is missing: run build-hooks.ps1 -Isolated and build-overlay.ps1 -Isolated first", f.display());
        }
        let log = dist.join("WindowGridRust.log");
        let _ = std::fs::remove_file(&log);

        // What the app normally publishes.
        let theme = GridTheme::default_theme();
        let memory = GridWindowMemory {
            grid_x: COLS,
            grid_y: ROWS,
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
        let input = Section::open_or_create(names::MAP_INPUT, 64).unwrap();
        input.write_bytes(0, &InputMemory::section_bytes(&Settings::default_input()));
        let hook_map = Section::open_or_create(names::MAP_HOOK, SettingsMemory::SIZE).unwrap();
        let handle = Section::open_or_create(names::MAP_GRID_WINDOW_HANDLE, 4).unwrap();
        handle.write_bytes(0, &0i32.to_le_bytes());

        // The overlay first (it publishes its window handle), then the hook host.
        let overlay = Proc(Command::new(&exe).spawn().expect("start the overlay"));
        let hook = Proc(
            Command::new(r"C:\Windows\System32\rundll32.exe")
                .arg(format!("{},RunDll", dll.display()))
                .spawn()
                .expect("start the hook host"),
        );
        let mut rig = Rig { _procs: vec![overlay, hook], hook_map, _grid: grid, _input: input, handle, work: work_area(), log, app_owned: false };
        rig.set_defer(true);

        let deadline = Instant::now() + Duration::from_secs(5);
        while rig.handle.read::<i32>() == 0 {
            assert!(Instant::now() < deadline, "the overlay never published its window");
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(Duration::from_millis(700)); // let the hook host install its event hook
        rig
    }

    /// Starts the isolated `wg-app` in a scratch directory with a settings file saying whether
    /// resizes are deferred. The app extracts, publishes and supervises everything itself.
    fn start_app(defer: bool) -> Rig {
        let src = root().join("dist-iso").join("WindowGridRust.exe");
        assert!(src.exists(), "{} is missing: run build-app.ps1 -Isolated first", src.display());
        let dir = std::env::temp_dir().join(format!("wg-e2e-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("WindowGridRust.exe");
        std::fs::copy(&src, &exe).unwrap();
        let mut settings = Settings::default();
        settings.grid.grid_x = COLS;
        settings.grid.grid_y = ROWS;
        settings.grid.defer_resize_until_release = defer;
        std::fs::write(dir.join("WindowGridRust.settings"), settings.to_json_string()).unwrap();

        // Only observe the sections: the app creates and fills them.
        let hook_map = Section::open_or_create(names::MAP_HOOK, SettingsMemory::SIZE).unwrap();
        hook_map.write_bytes(0, &0i32.to_le_bytes());
        let grid = Section::open_or_create(names::MAP_GRID_WINDOW, GridWindowMemory::SIZE).unwrap();
        let input = Section::open_or_create(names::MAP_INPUT, 64).unwrap();
        let handle = Section::open_or_create(names::MAP_GRID_WINDOW_HANDLE, 4).unwrap();
        handle.write_bytes(0, &0i32.to_le_bytes());

        let app = Proc(Command::new(&exe).current_dir(&dir).env("WG_TEST_NO_PROMPT", "1").spawn().expect("start wg-app"));
        let rig = Rig { _procs: vec![app], hook_map, _grid: grid, _input: input, handle, work: work_area(), log: dir.join("WindowGridRust.log"), app_owned: true };
        let deadline = Instant::now() + Duration::from_secs(20);
        while rig.hook_map.read::<SettingsMemory>().grid_window == 0 {
            assert!(Instant::now() < deadline, "the app never started the overlay
{}", rig.log_tail());
            thread::sleep(Duration::from_millis(30));
        }
        thread::sleep(Duration::from_millis(1200)); // hook hosts install their event hook
        rig
    }

    /// Writes the app-owned fields of the hook section (the overlay owns `grid_window`).
    /// Under `wg-app` the app owns them and the settings file decides, so this does nothing.
    fn set_defer(&mut self, defer: bool) {
        if self.app_owned {
            return;
        }
        let grid_x = COLS | if defer { DEFER_FLAG } else { 0 };
        let mut b = Vec::new();
        b.extend_from_slice(&grid_x.to_le_bytes());
        b.extend_from_slice(&ROWS.to_le_bytes());
        b.extend_from_slice(&4i32.to_le_bytes());
        self.hook_map.write_bytes(4, &b);
    }

    fn overlay_visible(&self) -> bool {
        let h = self.handle.read::<i32>();
        // SAFETY: plain query on a window handle.
        unsafe { IsWindowVisible(HWND(h as isize as *mut c_void)).as_bool() }
    }

    /// Expected visible-frame rectangle of the cells `(x0, y0)` to `(x1, y1)` inclusive.
    fn cells(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> RECT {
        let (w, h) = (f64::from(self.work.right - self.work.left), f64::from(self.work.bottom - self.work.top));
        let edge = |origin: i32, extent: f64, i: i32, n: i32| (f64::from(origin) + (extent * f64::from(i) / f64::from(n)).round_ties_even()) as i32;
        RECT {
            left: edge(self.work.left, w, x0, COLS),
            top: edge(self.work.top, h, y0, ROWS),
            right: edge(self.work.left, w, x1 + 1, COLS),
            bottom: edge(self.work.top, h, y1 + 1, ROWS),
        }
    }

    fn center(&self, cx: i32, cy: i32) -> (i32, i32) {
        let r = self.cells(cx, cy, cx, cy);
        ((r.left + r.right) / 2, (r.top + r.bottom) / 2)
    }

    /// Starts a drag on the caption of a fresh test window. Returns the window and its frame
    /// from before the drag started (what Esc restores). Retries if the window did not start
    /// following the mouse (a stray focus change can swallow the first click).
    fn begin_drag(&self, w: i32, h: i32) -> (TestWindow, RECT) {
        for attempt in 1..=3 {
            let win = TestWindow::spawn(self.work.left + 400, self.work.top + 500, w, h);
            let origin = win.frame();
            let r = win.rect();
            let grab = (r.left + w / 2, r.top + 18);
            move_to(grab.0, grab.1);
            thread::sleep(Duration::from_millis(200));
            button(true);
            thread::sleep(Duration::from_millis(150));
            glide((grab.0 + 40, grab.1 + 30)); // enter the modal move loop
            thread::sleep(Duration::from_millis(100));
            if win.rect().left > r.left + 20 {
                return (win, origin);
            }
            println!("drag attempt {attempt}: the window did not follow the mouse, retrying");
            button(false);
            thread::sleep(Duration::from_millis(300));
        }
        panic!("could not start a drag on the test window");
    }

    fn log_tail(&self) -> String {
        std::fs::read_to_string(&self.log).map(|s| s.lines().rev().take(14).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")).unwrap_or_default()
    }
}

fn near(a: RECT, b: RECT, tol: i32) -> bool {
    (a.left - b.left).abs() <= tol && (a.top - b.top).abs() <= tol && (a.right - b.right).abs() <= tol && (a.bottom - b.bottom).abs() <= tol
}

fn show(r: RECT) -> String {
    format!("({}, {}, {}, {}) = {} x {}", r.left, r.top, r.right, r.bottom, r.right - r.left, r.bottom - r.top)
}

fn size(r: RECT) -> (i32, i32) {
    (r.right - r.left, r.bottom - r.top)
}

fn dpi_aware() {
    // SAFETY: process-wide DPI setup before any window exists.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// Scenarios 1-3, with resizes deferred until release.
fn deferred_scenarios(rig: &mut Rig, failures: &mut Vec<String>) {
    // ---- 1. Resize: hold Space over the first cell, release, drag to the second, release mouse.
    {
        rig.set_defer(true);
        let (win, _) = rig.begin_drag(600, 400);
        let before = win.frame();
        glide(rig.center(2, 1));
        thread::sleep(Duration::from_millis(150));
        tap(SCAN_SPACE);
        glide(rig.center(5, 3));
        thread::sleep(Duration::from_millis(450));
        screenshot("1-resize");
        let live = win.frame();
        let visible = rig.overlay_visible();
        button(false);
        thread::sleep(Duration::from_millis(900));
        let after = win.frame();
        let want = rig.cells(2, 1, 5, 3);
        println!("1 resize/defer: before {} | during {} | after {} | want {} | overlay visible during: {visible}", show(before), show(live), show(after), show(want));
        if !visible {
            failures.push("1: the overlay was not visible during the drag".into());
        }
        if size(live) != size(before) {
            failures.push(format!("1: with deferral the window must not resize during the drag, but went {} -> {}", show(before), show(live)));
        }
        if !near(after, want, 2) {
            failures.push(format!("1: after release the frame should be the cells {}, got {}", show(want), show(after)));
        }
        if rig.overlay_visible() {
            failures.push("1: the overlay should hide after the drag".into());
        }
    }

    // ---- 2. Move: Ctrl on a cell, release, drag on; the window keeps its size and lands on the cell.
    {
        let (win, _) = rig.begin_drag(600, 400);
        let before = win.frame();
        glide(rig.center(6, 2));
        thread::sleep(Duration::from_millis(150));
        tap(SCAN_CTRL);
        glide(rig.center(8, 3));
        thread::sleep(Duration::from_millis(450));
        screenshot("2-move");
        button(false);
        thread::sleep(Duration::from_millis(900));
        let after = win.frame();
        let cell = rig.cells(8, 3, 8, 3);
        println!("2 move: before {} | after {} | cell origin ({}, {})", show(before), show(after), cell.left, cell.top);
        if size(after) != size(before) {
            failures.push(format!("2: moving must keep the visible size {:?}, got {:?}", size(before), size(after)));
        }
        if (after.left - cell.left).abs() > 2 || (after.top - cell.top).abs() > 2 {
            failures.push(format!("2: the window should start at the cell origin ({}, {}), got ({}, {})", cell.left, cell.top, after.left, after.top));
        }
    }

    // ---- 3. Esc cancels: the window returns to where it started and nothing is snapped.
    {
        let (win, before) = rig.begin_drag(600, 400);
        glide(rig.center(2, 1));
        tap(SCAN_SPACE);
        glide(rig.center(5, 3));
        thread::sleep(Duration::from_millis(300));
        tap(SCAN_ESC);
        thread::sleep(Duration::from_millis(200));
        button(false);
        thread::sleep(Duration::from_millis(900));
        let after = win.frame();
        println!("3 escape: window before the drag {} | after Esc {}", show(before), show(after));
        if !near(after, before, 2) {
            failures.push(format!("3: Esc should restore the window to where the drag began, {}, got {}", show(before), show(after)));
        }
        if rig.overlay_visible() {
            failures.push("3: the overlay should hide after Esc".into());
        }
    }

}

/// Scenario 4: without deferral the window follows the selection live.
fn live_scenario(rig: &mut Rig, failures: &mut Vec<String>) {
    {
        rig.set_defer(false);
        let (win, _) = rig.begin_drag(600, 400);
        glide(rig.center(2, 1));
        thread::sleep(Duration::from_millis(150));
        tap(SCAN_SPACE);
        glide(rig.center(5, 3));
        thread::sleep(Duration::from_millis(450));
        screenshot("4-live");
        let live = win.frame();
        button(false);
        thread::sleep(Duration::from_millis(900));
        let after = win.frame();
        let want = rig.cells(2, 1, 5, 3);
        println!("4 live resize: during {} | after {} | want {}", show(live), show(after), show(want));
        if !near(live, want, 3) {
            failures.push(format!("4: without deferral the window should already be {} during the drag, got {}", show(want), show(live)));
        }
        if !near(after, want, 2) {
            failures.push(format!("4: after release the frame should be {}, got {}", show(want), show(after)));
        }
    }

}

#[test]
#[ignore = "takes over the mouse and keyboard; run with WindowGridRust closed: cargo test -p wg-e2e -- --ignored --test-threads=1"]
fn real_drags() {
    dpi_aware();
    let mut rig = Rig::start();
    let mut failures: Vec<String> = Vec::new();
    println!("work area {}", show(rig.work));
    deferred_scenarios(&mut rig, &mut failures);
    live_scenario(&mut rig, &mut failures);
    println!("--- log ---\n{}", rig.log_tail());
    assert!(failures.is_empty(), "drag test failures:\n  {}", failures.join("\n  "));
}

/// The same scenarios, but `wg-app` extracts, configures and supervises the overlay and the
/// hook hosts from its settings file (deferral on, then off in a second run of the app).
#[test]
#[ignore = "takes over the mouse and keyboard; run with WindowGridRust closed: cargo test -p wg-e2e -- --ignored --test-threads=1"]
fn real_drags_supervised_by_the_app() {
    dpi_aware();
    let mut failures: Vec<String> = Vec::new();
    {
        let mut rig = Rig::start_app(true);
        println!("work area {}", show(rig.work));
        deferred_scenarios(&mut rig, &mut failures);
        println!("--- app log (deferred) ---\n{}", rig.log_tail());
    }
    thread::sleep(Duration::from_millis(1500));
    {
        let mut rig = Rig::start_app(false);
        live_scenario(&mut rig, &mut failures);
        println!("--- app log (live) ---\n{}", rig.log_tail());
    }
    assert!(failures.is_empty(), "drag test failures:\n  {}", failures.join("\n  "));
}

/// Baseline: the harness itself can drag a window with real input (no hook, no overlay).
#[test]
#[ignore = "takes over the mouse and keyboard; run with WindowGridRust closed: cargo test -p wg-e2e -- --ignored --test-threads=1"]
fn plain_drag_moves_the_window() {
    // SAFETY: process-wide DPI setup before any window exists.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let win = TestWindow::spawn(400, 500, 600, 400);
    let before = win.rect();
    let grab = (before.left + 300, before.top + 18);
    move_to(grab.0, grab.1);
    thread::sleep(Duration::from_millis(250));
    // SAFETY: plain queries.
    let (under, fg) = unsafe { (WindowFromPoint(POINT { x: grab.0, y: grab.1 }), GetForegroundWindow()) };
    println!("cursor now {:?}, want {:?}; window under cursor {:?} (ours {:?}), foreground {:?}", cursor(), grab, under.0, win.hwnd.0, fg.0);
    button(true);
    thread::sleep(Duration::from_millis(150));
    glide((grab.0 + 300, grab.1 + 200));
    thread::sleep(Duration::from_millis(200));
    let during = win.rect();
    button(false);
    thread::sleep(Duration::from_millis(300));
    let after = win.rect();
    println!("plain drag: before {} | during {} | after {}", show(before), show(during), show(after));
    assert!((after.left - before.left - 300).abs() <= 3 && (after.top - before.top - 200).abs() <= 3, "the window should have followed the mouse");
}

/// With the hook engaged, a plain drag (no activation key) must still move the window.
#[test]
#[ignore = "takes over the mouse and keyboard; run with WindowGridRust closed: cargo test -p wg-e2e -- --ignored --test-threads=1"]
fn hooked_plain_drag_moves_the_window() {
    // SAFETY: process-wide DPI setup before any window exists.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut rig = Rig::start();
    for defer in [true, false] {
        rig.set_defer(defer);
        let (win, _) = rig.begin_drag(600, 400);
        let before = win.rect();
        glide((cursor().0 + 300, cursor().1 + 200));
        thread::sleep(Duration::from_millis(200));
        let during = win.rect();
        button(false);
        thread::sleep(Duration::from_millis(400));
        println!("defer={defer}: before {} | during {} | after {}", show(before), show(during), show(win.rect()));
    }
    println!("--- log ---\n{}", rig.log_tail());
}
