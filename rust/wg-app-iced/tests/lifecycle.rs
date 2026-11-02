//! Runs the real iced app (isolated build): what it extracts and starts, that it quits
//! cleanly and takes its helpers with it, and what its windows look like on the real desktop.
//!
//! ```text
//! .\build-hooks.ps1 -Isolated ; .\build-overlay.ps1 -Isolated
//! cargo test -p wg-app-iced --features isolated --test lifecycle -- --test-threads=1
//! ```
//! It shows a tray icon and windows for a few seconds per scenario (`hovering_the_navigation_shows_up_quickly` moves the mouse pointer for a second and puts it back, so it is `#[ignore]`d: run it with `--ignored`), and uses test-only names,
//! so a running WindowGridRust is unaffected. Captures of the windows are written to
//! `target\tmp\iced-*.png`.

#![cfg(all(windows, feature = "isolated"))]

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use wg_core::ipc::{SettingsMemory, Wire};
use wg_core::names;
use wg_win::shm::Section;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetCursorPos, GetWindowRect, SetCursorPos, SetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SetWindowPos,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, WM_APP, WM_LBUTTONDBLCLK,
};

const APP: &str = env!("CARGO_BIN_EXE_wg-app-iced");

struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wg-iced-lifecycle-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn wait_for<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn alive(pid: u32) -> bool {
    // SAFETY: opening a process for synchronisation only; the handle is closed.
    unsafe {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(h) => {
                let running = WaitForSingleObject(h, 0).0 == 0x102; // WAIT_TIMEOUT
                let _ = CloseHandle(h);
                running
            }
            Err(_) => false,
        }
    }
}

/// PIDs of the direct children of `parent`.
fn children_of(parent: u32) -> Vec<u32> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!("(Get-CimInstance Win32_Process -Filter 'ParentProcessId={parent}').ProcessId")])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect()
}

fn log_of(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("WindowGridRust.log")).unwrap_or_default()
}

fn embedded_ok() -> bool {
    let ok = wg_app::components::embedded().len() == 4;
    if !ok {
        eprintln!("SKIPPED: isolated components are not embedded; run build-hooks.ps1 -Isolated and build-overlay.ps1 -Isolated, then rebuild");
    }
    ok
}

/// A copy of the app in a fresh directory, started with `env` and `args`.
fn start(name: &str, args: &[&str], env: &[(&str, &str)]) -> (Kill, PathBuf) {
    let work = temp_dir(name);
    let exe = work.join("WindowGridRust.exe");
    std::fs::copy(APP, &exe).unwrap();
    let mut cmd = Command::new(&exe);
    cmd.args(args).current_dir(&work).env("WG_TEST_NO_PROMPT", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    (Kill(cmd.spawn().expect("start the app")), work)
}

// ---- lifecycle ----------------------------------------------------------------------------------

#[test]
fn starts_publishes_and_quits_cleanly_with_its_helpers() {
    if !embedded_ok() {
        return;
    }
    let hook = Section::open_or_create(names::MAP_HOOK, SettingsMemory::SIZE).unwrap();
    hook.write_bytes(0, &0i32.to_le_bytes());

    let (mut app, work) = start("quit", &[], &[("WG_TEST_QUIT_AFTER_MS", "4000")]);
    let app_pid = app.0.id();
    // The overlay was extracted, started, and told us (via the section) where its window is.
    wait_for("the overlay to publish its window", 20, || (hook.read::<SettingsMemory>().grid_window != 0).then_some(()));
    for f in ["WindowGridRustOverlay.exe", "WindowGridRust64.dll", "WindowGridRust32.dll", "WindowGridRustARM64.dll", "WindowGridRust.settings"] {
        assert!(work.join(f).exists(), "{f} should exist in the work directory");
    }
    let kids = children_of(app_pid);
    assert!(kids.len() >= 3, "overlay and two hook hosts should be children, got {kids:?}");

    let status = wait_for("the app to quit by itself", 20, || app.0.try_wait().unwrap());
    assert!(status.success(), "clean exit code, got {status:?}");
    wait_for("the helpers to be gone", 10, || kids.iter().all(|p| !alive(*p)).then_some(()));
    let log = log_of(&work);
    assert!(log.contains("Running") && log.contains("Exiting"), "{log}");
    let _ = std::fs::remove_dir_all(&work);
}

// ---- the tray icon's events reach the iced app ----------------------------------------------------

struct TraySearch {
    pid: u32,
    found: Option<HWND>,
}

unsafe extern "system" fn collect_tray(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let search = &mut *(lparam.0 as *mut TraySearch);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == search.pid {
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class) as usize;
        if String::from_utf16_lossy(&class[..n]) == "WindowGridRustTrayWindow" {
            search.found = Some(hwnd);
        }
    }
    true.into()
}

/// The hidden window that owns the tray icon, of this process only: a running WindowGridRust
/// has one with the same class name, and it must not be touched.
fn find_tray_window(pid: u32) -> Option<HWND> {
    let mut search = TraySearch { pid, found: None };
    // SAFETY: the callback only touches `search`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(collect_tray), LPARAM(&mut search as *mut _ as isize));
    }
    search.found
}

#[test]
fn a_tray_double_click_opens_the_settings_window() {
    if !embedded_ok() {
        return;
    }
    let (app, work) = start("tray", &[], &[("WG_TEST_NO_HELPERS", "1"), ("WG_TEST_QUIT_AFTER_MS", "60000")]);
    let pid = app.0.id();
    let tray = wait_for("the tray window", 20, || find_tray_window(pid));
    assert!(find_window(pid, "WindowGridRust").is_none(), "no settings window before the click");

    // What the shell sends the tray window for a double-click on the icon: the callback
    // message (WM_APP + 1) with the mouse message in the low word of lparam.
    // SAFETY: posting a message to a window of the process just started.
    unsafe {
        PostMessageW(Some(tray), WM_APP + 1, WPARAM(0), LPARAM(WM_LBUTTONDBLCLK as isize)).unwrap();
    }
    wait_for("the settings window to open from the tray event", 15, || find_window(pid, "WindowGridRust"));
    drop(app);
    let _ = std::fs::remove_dir_all(&work);
}

// ---- the real windows on the real desktop ---------------------------------------------------------

struct Found {
    hwnd: HWND,
    width: i32,
    height: i32,
}

struct Search {
    pid: u32,
    title: &'static str,
    found: Vec<Found>,
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let search = &mut *(lparam.0 as *mut Search);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid != search.pid || !IsWindowVisible(hwnd).as_bool() {
        return true.into();
    }
    let mut title = [0u16; 64];
    let n = GetWindowTextW(hwnd, &mut title) as usize;
    let mut r = RECT::default();
    let _ = GetWindowRect(hwnd, &mut r);
    if String::from_utf16_lossy(&title[..n]) == search.title && r.right - r.left > 300 {
        search.found.push(Found { hwnd, width: r.right - r.left, height: r.bottom - r.top });
    }
    true.into()
}

fn find_window(pid: u32, title: &'static str) -> Option<Found> {
    let mut search = Search { pid, title, found: Vec::new() };
    // SAFETY: the callback only touches `search`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut search as *mut _ as isize));
    }
    search.found.pop()
}

/// Renders the window's own content (independent of what covers it) into RGBA.
fn print_window(w: &Found) -> Vec<u8> {
    // SAFETY: GDI objects sized to the window and released afterwards.
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w.width, w.height);
        let old = SelectObject(dc, bmp.into());
        let _ = PrintWindow(w.hwnd, dc, PRINT_WINDOW_FLAGS(2)); // PW_RENDERFULLCONTENT
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w.width;
        bi.bmiHeader.biHeight = -w.height;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0;
        let mut px = vec![0u8; (w.width * w.height * 4) as usize];
        GetDIBits(dc, bmp, 0, w.height as u32, Some(px.as_mut_ptr().cast()), &mut bi, DIB_RGB_COLORS);
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        for p in px.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            p[3] = 255;
        }
        px
    }
}

/// Resizes the window to an outer size in physical pixels. It is applied as asked: like
/// WindowGridRust snapping a window into a grid cell, this bypasses the window's minimum size.
fn resize(win: &Found, width: i32, height: i32) {
    // SAFETY: resizing a window of the process just started.
    unsafe {
        SetWindowPos(win.hwnd, None, 0, 0, width, height, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE).unwrap();
    }
}

/// One capture of a series: the file to write, and the outer size to give the window first.
type Shot = (String, Option<(i32, i32)>);

/// Starts the app, waits for the window titled `title`, lets it paint, and saves a capture of
/// it for each shot as `target\tmp\<file>`.
fn capture_series(name: &str, args: &[&str], env: &[(&str, &str)], title: &'static str, shots: &[Shot]) -> Vec<(Found, Vec<u8>)> {
    let (app, work) = start(name, args, env);
    let mut win = wait_for("the window to appear", 25, || find_window(app.0.id(), title));
    std::thread::sleep(Duration::from_millis(1200)); // let it finish its first paint
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut taken = Vec::new();
    for (file, size) in shots {
        if let Some((width, height)) = size {
            resize(&win, *width, *height);
            std::thread::sleep(Duration::from_millis(900));
            win = wait_for("the resized window", 5, || find_window(app.0.id(), title));
        }
        let pixels = print_window(&win);
        std::fs::write(dir.join(file), wg_win::png::encode_rgba(win.width as u32, win.height as u32, &pixels)).unwrap();
        println!("{file}: {} x {} physical pixels", win.width, win.height);
        taken.push((Found { hwnd: win.hwnd, width: win.width, height: win.height }, pixels));
    }
    drop(app);
    let _ = std::fs::remove_dir_all(&work);
    taken
}

/// Starts the app, waits for the window titled `title`, lets it paint, and saves a capture of
/// it as `target\tmp\<file>`.
fn capture(name: &str, args: &[&str], env: &[(&str, &str)], title: &'static str, file: &str) -> (Found, Vec<u8>) {
    capture_series(name, args, env, title, &[(file.to_string(), None)]).pop().unwrap()
}

fn distinct_colors(pixels: &[u8]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for p in pixels.as_chunks::<4>().0 {
        seen.insert([p[0], p[1], p[2]]);
    }
    seen.len()
}

/// How many pixels in the page area have the colour of a card (dark or light mode). The area is
/// the right 60% of the window without its title bar and button bar, so the navigation and the
/// buttons (which are card-coloured too) do not count. A page whose cards fail to lay out
/// still has the navigation, the grid and the buttons, so "not blank" is not enough.
fn card_pixels(win: &Found, pixels: &[u8]) -> usize {
    let (w, h) = (win.width as usize, win.height as usize);
    (150..h.saturating_sub(90))
        .flat_map(|y| ((w * 2 / 5)..w.saturating_sub(12)).map(move |x| (y * w + x) * 4))
        .filter(|&i| [pixels[i], pixels[i + 1], pixels[i + 2]] == [0x26; 3] || [pixels[i], pixels[i + 1], pixels[i + 2]] == [0xff; 3])
        .count()
}

#[test]
fn every_settings_page_renders_on_the_real_desktop() {
    if !embedded_ok() {
        return;
    }
    let names = ["grid", "appearance", "behavior", "hud", "shortcuts", "info", "hood"];
    for (i, page) in names.iter().enumerate() {
        let (win, pixels) = capture(
            page,
            &[],
            &[("WG_TEST_OPEN_SETTINGS", "1"), ("WG_TEST_NO_HELPERS", "1"), ("WG_TEST_PAGE", &i.to_string()), ("WG_TEST_QUIT_AFTER_MS", "30000")],
            "WindowGridRust",
            &format!("iced-settings-{i}-{page}.png"),
        );
        assert!(win.width >= 800 && win.height >= 520, "{page}: a sensible window size, got {} x {}", win.width, win.height);
        let first = &[pixels[0], pixels[1], pixels[2], pixels[3]];
        assert!(pixels.as_chunks::<4>().0.iter().any(|p| p != first), "{page}: the window must not be blank");
        assert!(distinct_colors(&pixels) > 20, "{page}: text and controls are drawn, not just a flat fill");
        assert!(card_pixels(&win, &pixels) > 8000, "{page}: the page's cards are laid out ({} card pixels)", card_pixels(&win, &pixels));
    }
}

/// The window may be snapped into any grid cell, so every page has to work at any size: the
/// full navigation with stacked cards, the slim rail, and a narrow cell.
#[test]
fn every_settings_page_holds_up_at_every_window_size() {
    if !embedded_ok() {
        return;
    }
    let sizes = [("medium", (816, 560)), ("rail", (620, 520)), ("narrow", (380, 700))];
    let names = ["grid", "appearance", "behavior", "hud", "shortcuts", "info", "hood"];
    for (i, page) in names.iter().enumerate() {
        let shots: Vec<Shot> = sizes.iter().map(|(label, size)| (format!("iced-settings-{i}-{page}-{label}.png"), Some(*size))).collect();
        let taken = capture_series(
            &format!("sizes-{page}"),
            &[],
            &[("WG_TEST_OPEN_SETTINGS", "1"), ("WG_TEST_NO_HELPERS", "1"), ("WG_TEST_PAGE", &i.to_string()), ("WG_TEST_QUIT_AFTER_MS", "60000")],
            "WindowGridRust",
            &shots,
        );
        for ((label, (width, height)), (win, pixels)) in sizes.iter().zip(&taken) {
            assert_eq!((win.width, win.height), (*width, *height), "{page} {label}: the window took the size asked for");
            assert!(distinct_colors(pixels) > 20, "{page} {label}: text and controls are drawn");
            assert!(card_pixels(win, pixels) > 1500, "{page} {label}: the page's cards are laid out ({} card pixels)", card_pixels(win, pixels));
        }
    }
}

/// Where the client area starts inside the window's captured pixels (the frame and title bar).
fn client_offset(win: &Found) -> (i32, i32) {
    let mut origin = POINT::default();
    let mut rect = RECT::default();
    // SAFETY: plain queries on a window of the process just started.
    unsafe {
        let _ = ClientToScreen(win.hwnd, &mut origin);
        let _ = GetWindowRect(win.hwnd, &mut rect);
    }
    (origin.x - rect.left, origin.y - rect.top)
}

/// The navigation's pixels in a capture: the left 220 client pixels below the title bar.
fn nav_pixels(win: &Found, pixels: &[u8], offset: (i32, i32)) -> Vec<u8> {
    let (w, h) = (win.width as usize, win.height as usize);
    let (x0, y0) = (offset.0 as usize, offset.1 as usize + 60);
    let mut out = Vec::new();
    for y in y0..h.saturating_sub(20) {
        out.extend_from_slice(&pixels[(y * w + x0) * 4..(y * w + x0 + 220) * 4]);
    }
    out
}

/// What the user felt as lag: how long after the pointer moves onto another navigation entry the
/// picture changes. This moves the real pointer (and puts it back afterwards): the system takes a
/// pointer that is not really over the window as having left it and cancels the hover at once, so
/// posting mouse-move messages to the window does not work.
///
/// Each time includes taking a capture, about 30 ms. With the original defects (see
/// `frame_cost.rs`) the hovers measured 67-97 ms here and with the fixes 31-50 ms: a real
/// difference, but too close to the capture's own cost for a hard limit, so the limit only
/// leaves room for slow machines. The checks that fail on the old behaviour are in
/// `frame_cost.rs`, which counts what a hover repaints and does not depend on a clock.
#[test]
#[ignore = "moves the real mouse pointer and needs the window in front of everything; run with --ignored"]
fn hovering_the_navigation_shows_up_quickly() {
    if !embedded_ok() {
        return;
    }
    let (app, work) = start("hover", &[], &[("WG_TEST_OPEN_SETTINGS", "1"), ("WG_TEST_NO_HELPERS", "1"), ("WG_TEST_QUIT_AFTER_MS", "60000")]);
    let win = wait_for("the window to appear", 25, || find_window(app.0.id(), "WindowGridRust"));
    // SAFETY: bringing a window of the process just started to the front.
    unsafe {
        let _ = SetForegroundWindow(win.hwnd);
    }
    std::thread::sleep(Duration::from_millis(1500));
    let offset = client_offset(&win);
    let mut rect = RECT::default();
    let mut saved = POINT::default();
    // SAFETY: plain queries.
    unsafe {
        let _ = GetWindowRect(win.hwnd, &mut rect);
        let _ = GetCursorPos(&mut saved);
    }
    let first = print_window(&win);
    let _ = std::fs::write(
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("iced-hover-before.png"),
        wg_win::png::encode_rgba(win.width as u32, win.height as u32, &first),
    );
    let mut before = nav_pixels(&win, &first, offset);
    let mut latencies = Vec::new();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Entries are 45 px apart; the pointer visits seven positions down the navigation, so
        // it enters a different entry each time.
        for client_y in [150, 195, 240, 285, 330, 375, 420] {
            // SAFETY: moving the pointer onto the window under test.
            unsafe {
                let _ = SetCursorPos(rect.left + offset.0 + 100, rect.top + offset.1 + client_y);
            }
            let started = Instant::now();
            let seen = loop {
                let now = nav_pixels(&win, &print_window(&win), offset);
                if now != before {
                    before = now;
                    break started.elapsed();
                }
                if started.elapsed() > Duration::from_secs(2) {
                    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
                    let _ = std::fs::write(dir.join("iced-hover-after.png"), wg_win::png::encode_rgba(win.width as u32, win.height as u32, &print_window(&win)));
                    panic!("the hover at client y={client_y} never showed (the capture is in target/tmp/iced-hover-after.png; is another window covering this one?)");
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            latencies.push(seen);
        }
    }));
    // SAFETY: putting the pointer back where it was.
    unsafe {
        let _ = SetCursorPos(saved.x, saved.y);
    }
    drop(app);
    let _ = std::fs::remove_dir_all(&work);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    println!("hover latencies (ms): {:?}", latencies.iter().map(|d| d.as_millis()).collect::<Vec<_>>());
    let worst = latencies.iter().max().unwrap();
    assert!(*worst < Duration::from_millis(400), "a hover took {worst:?} to show");
}

/// How long the app takes from being started to showing its tray icon's window, and from a
/// double-click on the icon to a settings window with something painted in it. Prints a report
/// (three runs, each a fresh process, so the font search at the first text is included); the
/// limit only catches a start that has gone badly wrong.
#[test]
#[ignore = "prints a timing report; run with --ignored --nocapture"]
fn opening_the_settings_window_report() {
    if !embedded_ok() {
        return;
    }
    let mut rows = Vec::new();
    for run in 0..3 {
        let started = Instant::now();
        let (app, work) = start("open", &[], &[("WG_TEST_NO_HELPERS", "1"), ("WG_TEST_QUIT_AFTER_MS", "60000")]);
        let pid = app.0.id();
        let tray = wait_for("the tray window", 20, || find_tray_window(pid));
        let to_tray = started.elapsed();
        std::thread::sleep(Duration::from_millis(300));

        let clicked = Instant::now();
        // SAFETY: posting a message to a window of the process just started.
        unsafe {
            PostMessageW(Some(tray), WM_APP + 1, WPARAM(0), LPARAM(WM_LBUTTONDBLCLK as isize)).unwrap();
        }
        let win = loop {
            if let Some(w) = find_window(pid, "WindowGridRust") {
                break w;
            }
            assert!(clicked.elapsed() < Duration::from_secs(15), "the settings window never opened");
            std::thread::sleep(Duration::from_millis(2));
        };
        let to_window = clicked.elapsed();
        // Painted: the capture has the many colours of the navigation, cards and text.
        let painted = loop {
            if distinct_colors(&print_window(&win)) > 30 {
                break clicked.elapsed();
            }
            assert!(clicked.elapsed() < Duration::from_secs(15), "the settings window never painted");
            std::thread::sleep(Duration::from_millis(2));
        };
        rows.push((run, to_tray, to_window, painted));
        drop(app);
        let _ = std::fs::remove_dir_all(&work);
    }
    println!("\n{:<5} {:>14} {:>16} {:>14}", "run", "start -> tray", "click -> window", "click -> paint");
    for (run, a, b, c) in &rows {
        println!("{:<5} {:>11} ms {:>13} ms {:>11} ms", run, a.as_millis(), b.as_millis(), c.as_millis());
    }
    let worst = rows.iter().map(|r| r.3).max().unwrap();
    assert!(worst < Duration::from_secs(5), "opening the settings window took {worst:?}");
}

/// How long the real window takes to show a new size: from the resize until a few points of its
/// content match what the same size looks like once settled. Each time includes taking captures,
/// 30-60 ms each at these sizes, so it only shows stalls (the headless `resize_report` in
/// `frame_cost.rs` has the finer figures; this adds what it cannot see: reallocating the window's
/// surface and presenting it).
#[test]
#[ignore = "prints a timing report; run with --ignored --nocapture"]
fn resizing_the_window_report() {
    if !embedded_ok() {
        return;
    }
    let (app, work) = start("resize-time", &[], &[("WG_TEST_OPEN_SETTINGS", "1"), ("WG_TEST_NO_HELPERS", "1"), ("WG_TEST_QUIT_AFTER_MS", "90000")]);
    let pid = app.0.id();
    let mut win = wait_for("the window to appear", 25, || find_window(pid, "WindowGridRust"));
    std::thread::sleep(Duration::from_millis(1500));

    let settle = |win: &Found, w: i32, h: i32| {
        resize(win, w, h);
        std::thread::sleep(Duration::from_millis(1200));
        wait_for("the resized window", 5, || find_window(pid, "WindowGridRust"))
    };
    // Points in the page area, the footer and the corner: all deterministic once settled.
    let points = |w: i32, h: i32| [(w * 7 / 10, h / 2), (w * 9 / 10, h * 9 / 10), (w / 2, h * 8 / 10), (w * 19 / 20, h / 5), (w * 3 / 5, h * 6 / 10), (w - 30, h - 30)];
    let sample = |win: &Found, px: &[u8]| -> Vec<[u8; 3]> {
        points(win.width, win.height)
            .iter()
            .map(|&(x, y)| {
                let i = (y as usize * win.width as usize + x as usize) * 4;
                [px[i], px[i + 1], px[i + 2]]
            })
            .collect()
    };

    println!("\n{:<14} {:>12}", "window size", "to show it");
    let mut worst = Duration::ZERO;
    for (w, h) in [(1280, 800), (1920, 1080), (2560, 1400), (960, 680)] {
        win = settle(&win, w, h);
        let want = sample(&win, &print_window(&win));
        win = settle(&win, 700, 560);

        let started = Instant::now();
        resize(&win, w, h);
        let shown = loop {
            if let Some(now) = find_window(pid, "WindowGridRust").filter(|f| f.width == w && f.height == h) {
                if sample(&now, &print_window(&now)) == want {
                    break started.elapsed();
                }
            }
            assert!(started.elapsed() < Duration::from_secs(5), "the window never showed {w} x {h}");
        };
        worst = worst.max(shown);
        println!("{:<14} {:>9} ms", format!("{w} x {h}"), shown.as_millis());
        win = wait_for("the window", 5, || find_window(pid, "WindowGridRust"));
    }
    drop(app);
    let _ = std::fs::remove_dir_all(&work);
    assert!(worst < Duration::from_secs(2), "showing a new size took {worst:?}");
}

#[test]
fn the_confirmation_is_drawn_over_the_settings_window() {
    if !embedded_ok() {
        return;
    }
    let env = [("WG_TEST_OPEN_SETTINGS", "1"), ("WG_TEST_NO_HELPERS", "1"), ("WG_TEST_QUIT_AFTER_MS", "30000")];
    let (_, plain) = capture("plain", &[], &env, "WindowGridRust", "iced-settings-plain.png");
    let asked = [env[0], env[1], env[2], ("WG_TEST_ASK", "reset")];
    let (_, dimmed) = capture("ask", &[], &asked, "WindowGridRust", "iced-settings-reset-question.png");
    // The dimmed layer makes the whole window darker than without the question.
    let brightness = |p: &[u8]| p.as_chunks::<4>().0.iter().map(|px| u64::from(px[0]) + u64::from(px[1]) + u64::from(px[2])).sum::<u64>();
    assert!(brightness(&dimmed) < brightness(&plain), "the question dims the window");
}

#[test]
fn the_setup_dialog_opens() {
    if !embedded_ok() {
        return;
    }
    let target = temp_dir("setup-target");
    let (win, pixels) = capture("setup", &["/install", &target.display().to_string()], &[], "WindowGridRust Setup", "iced-setup.png");
    assert!(win.width >= 500 && win.width < 800, "a small fixed-size dialog, got {} x {}", win.width, win.height);
    assert!(distinct_colors(&pixels) > 20, "text and controls are drawn");
    let _ = std::fs::remove_dir_all(&target);
}
