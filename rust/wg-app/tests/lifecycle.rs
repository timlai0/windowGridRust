//! Runs the real app (isolated build) and checks its lifecycle: what it extracts and starts,
//! what it publishes, that helpers are restarted and cleaned up, and the install flow.
//!
//! ```text
//! .\build-hooks.ps1 -Isolated ; .\build-overlay.ps1 -Isolated
//! cargo test -p wg-app --features isolated --test lifecycle -- --test-threads=1
//! ```
//! It shows a tray icon for a few seconds per scenario, and uses test-only names, so a
//! running WindowGridRust is unaffected.

#![cfg(all(windows, feature = "isolated"))]

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use wg_core::ipc::{GridWindowMemory, SettingsMemory, Wire, DEFER_FLAG};
use wg_core::names;
use wg_win::shm::Section;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

const APP: &str = env!("CARGO_BIN_EXE_wg-app");

struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wg-lifecycle-{}-{name}", std::process::id()));
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

fn window_pid(hwnd: i32) -> u32 {
    let mut pid = 0u32;
    // SAFETY: valid out pointer.
    unsafe {
        GetWindowThreadProcessId(HWND(hwnd as isize as *mut std::ffi::c_void), Some(&mut pid));
    }
    pid
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

fn start_app(exe: &Path, extra: &[&str], quit_after_ms: Option<u64>) -> Kill {
    let mut cmd = Command::new(exe);
    cmd.args(extra).current_dir(exe.parent().unwrap()).env("WG_TEST_NO_PROMPT", "1");
    if let Some(ms) = quit_after_ms {
        cmd.env("WG_TEST_QUIT_AFTER_MS", ms.to_string());
    }
    Kill(cmd.spawn().expect("start the app"))
}

/// The overlay's window handle as written into the hook section by the overlay itself.
fn overlay_hwnd(hook: &Section) -> i32 {
    hook.read::<SettingsMemory>().grid_window
}

fn embedded_ok() -> bool {
    let ok = wg_app::components::embedded().len() == 4;
    if !ok {
        eprintln!("SKIPPED: isolated components are not embedded; run build-hooks.ps1 -Isolated and build-overlay.ps1 -Isolated, then rebuild");
    }
    ok
}

#[test]
fn starts_publishes_restarts_and_cleans_up() {
    if !embedded_ok() {
        return;
    }
    let work = temp_dir("run");
    let exe = work.join("WindowGridRust.exe");
    std::fs::copy(APP, &exe).unwrap();

    // The test reads the same sections the hooks and overlay use.
    let hook = Section::open_or_create(names::MAP_HOOK, SettingsMemory::SIZE).unwrap();
    let grid = Section::open_or_create(names::MAP_GRID_WINDOW, GridWindowMemory::SIZE).unwrap();
    hook.write_bytes(0, &0i32.to_le_bytes());

    let mut app = start_app(&exe, &[], None);
    let app_pid = app.0.id();

    // 1. The overlay was extracted, started, and told us (via the section) where its window is.
    let first = wait_for("the overlay to publish its window", 20, || (overlay_hwnd(&hook) != 0).then(|| overlay_hwnd(&hook)));
    for f in ["WindowGridRustOverlay.exe", "WindowGridRust64.dll", "WindowGridRust32.dll", "WindowGridRustARM64.dll", "WindowGridRust.settings"] {
        assert!(work.join(f).exists(), "{f} should exist in the work directory");
    }

    // 2. Settings were published for the hooks and the overlay.
    let h: SettingsMemory = hook.read();
    assert_eq!(({ h.grid_x }, { h.grid_y }, { h.input_size }), (12 | DEFER_FLAG, 6, 4));
    let g: GridWindowMemory = grid.read();
    assert_eq!(({ g.grid_x }, { g.grid_y }, { g.opacity }), (12, 6, 0.75));

    // 3. Both hook hosts started (they log when they start) and the overlay is a child.
    wait_for("the hook hosts to log their start", 10, || {
        let log = log_of(&work);
        (log.contains("[X64]") && log.contains("[X86]") && log.contains("Starting WindowGridRust hook")).then_some(())
    });
    let kids = children_of(app_pid);
    assert!(kids.len() >= 3, "overlay and two hook hosts should be children, got {kids:?}");
    assert!(kids.contains(&window_pid(first)), "the overlay process is a child of the app");

    // 4. A killed overlay is started again.
    let overlay_pid = window_pid(first);
    Command::new("taskkill").args(["/PID", &overlay_pid.to_string(), "/F"]).output().unwrap();
    hook.write_bytes(0, &0i32.to_le_bytes());
    let second = wait_for("the overlay to be restarted", 15, || (overlay_hwnd(&hook) != 0).then(|| overlay_hwnd(&hook)));
    assert_ne!(window_pid(second), overlay_pid, "a new overlay process");
    assert!(log_of(&work).contains("exited"), "the restart is logged");

    // 5. Killing the app takes every helper with it (job object).
    let kids = children_of(app_pid);
    assert!(!kids.is_empty());
    app.0.kill().unwrap();
    app.0.wait().unwrap();
    wait_for("the helpers to die with the app", 10, || kids.iter().all(|p| !alive(*p)).then_some(()));
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn quits_cleanly_and_stops_its_helpers() {
    if !embedded_ok() {
        return;
    }
    let work = temp_dir("quit");
    let exe = work.join("WindowGridRust.exe");
    std::fs::copy(APP, &exe).unwrap();
    let hook = Section::open_or_create(names::MAP_HOOK, SettingsMemory::SIZE).unwrap();
    hook.write_bytes(0, &0i32.to_le_bytes());

    let mut app = start_app(&exe, &[], Some(4000));
    let app_pid = app.0.id();
    wait_for("the overlay to publish its window", 20, || (overlay_hwnd(&hook) != 0).then_some(()));
    let kids = children_of(app_pid);
    assert!(!kids.is_empty());

    let status = wait_for("the app to quit by itself", 20, || app.0.try_wait().unwrap());
    assert!(status.success(), "clean exit code, got {status:?}");
    wait_for("the helpers to be gone", 10, || kids.iter().all(|p| !alive(*p)).then_some(()));
    let log = log_of(&work);
    assert!(log.contains("Running") && log.contains("Exiting"), "{log}");
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn silent_install_copies_configures_and_starts_the_installed_copy() {
    if !embedded_ok() {
        return;
    }
    let work = temp_dir("install-src");
    let target = temp_dir("install-dst").join("WindowGridRust");
    let exe = work.join("WindowGridRust.exe");
    std::fs::copy(APP, &exe).unwrap();
    let hook = Section::open_or_create(names::MAP_HOOK, SettingsMemory::SIZE).unwrap();
    hook.write_bytes(0, &0i32.to_le_bytes());

    // No shortcut and no autostart: those touch the all-users Start Menu and the logon task.
    let mut installer = start_app(&exe, &["/install", &target.display().to_string(), "/s"], None);
    let status = wait_for("the installer to finish", 30, || installer.0.try_wait().unwrap());
    assert!(status.success(), "{status:?}\n{}", log_of(&work));

    let installed = target.join("WindowGridRust.exe");
    assert!(installed.exists(), "the executable was copied");
    assert!(target.join("WindowGridRust.settings").exists(), "settings were written to the install directory");

    // The installed copy was started and runs from the install directory.
    wait_for("the installed copy to start the overlay", 25, || (overlay_hwnd(&hook) != 0).then_some(()));
    assert!(target.join("WindowGridRustOverlay.exe").exists(), "components were extracted into the install directory");
    let overlay_pid = window_pid(overlay_hwnd(&hook));
    // Stop it: kill the installed app (found as the overlay's parent) and clean up.
    let parent = Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId={overlay_pid}').ParentProcessId")])
        .output()
        .unwrap();
    let parent: u32 = String::from_utf8_lossy(&parent.stdout).trim().parse().expect("parent pid");
    Command::new("taskkill").args(["/PID", &parent.to_string(), "/F"]).output().unwrap();
    wait_for("the installed copy's helpers to die", 10, || (!alive(overlay_pid)).then_some(()));
    std::thread::sleep(Duration::from_millis(300));
    let _ = std::fs::remove_dir_all(target.parent().unwrap());
    let _ = std::fs::remove_dir_all(&work);
}

// ---- the real settings window on the real desktop ----------------------------------------------

struct Found {
    hwnd: HWND,
    width: i32,
    height: i32,
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: windows::Win32::Foundation::LPARAM) -> windows::core::BOOL {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, GetWindowTextW, IsWindowVisible};
    let out = &mut *(lparam.0 as *mut (u32, Vec<Found>));
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid != out.0 || !IsWindowVisible(hwnd).as_bool() {
        return true.into();
    }
    let mut title = [0u16; 64];
    let n = GetWindowTextW(hwnd, &mut title) as usize;
    let mut r = RECT::default();
    let _ = GetWindowRect(hwnd, &mut r);
    if String::from_utf16_lossy(&title[..n]) == "WindowGridRust" && r.right - r.left > 300 {
        out.1.push(Found { hwnd, width: r.right - r.left, height: r.bottom - r.top });
    }
    true.into()
}

fn find_window(pid: u32) -> Option<Found> {
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    let mut state = (pid, Vec::<Found>::new());
    // SAFETY: the callback only touches `state`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(collect), windows::Win32::Foundation::LPARAM(&mut state as *mut _ as isize));
    }
    state.1.pop()
}

/// Renders the window's own content (independent of what covers it) into RGBA.
fn print_window(w: &Found) -> Vec<u8> {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
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

#[test]
fn the_settings_window_opens_on_the_real_desktop() {
    if !embedded_ok() {
        return;
    }
    let work = temp_dir("window");
    let exe = work.join("WindowGridRust.exe");
    std::fs::copy(APP, &exe).unwrap();
    let mut cmd = Command::new(&exe);
    cmd.current_dir(&work).env("WG_TEST_NO_PROMPT", "1").env("WG_TEST_OPEN_SETTINGS", "1").env("WG_TEST_QUIT_AFTER_MS", "9000");
    let app = Kill(cmd.spawn().unwrap());

    let win = wait_for("the settings window to appear", 25, || find_window(app.0.id()));
    std::thread::sleep(Duration::from_millis(1200)); // let it finish its first paint
    let pixels = print_window(&win);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("app-live-settings.png"), wg_win::png::encode_rgba(win.width as u32, win.height as u32, &pixels)).unwrap();
    println!("settings window: {} x {} physical pixels", win.width, win.height);

    assert!(win.width >= 800 && win.height >= 600, "a sensible window size");
    let first = &[pixels[0], pixels[1], pixels[2], pixels[3]];
    assert!(pixels.as_chunks::<4>().0.iter().any(|p| p != first), "the window must not be blank");
    // Something blue-ish (the accent colour of the selected navigation entry) is drawn.
    assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[2] > 150 && p[0] < 60), "accent-coloured controls are drawn");
    let _ = std::fs::remove_dir_all(&work);
}
