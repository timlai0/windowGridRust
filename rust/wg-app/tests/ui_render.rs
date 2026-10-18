//! Renders the app's windows headlessly with Slint's software renderer and writes PNGs to
//! `target/tmp/` for visual review. The assertions only check that something sensible was
//! drawn and that the callbacks wired to the logic behave.

#![cfg(windows)]

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, Model, PhysicalSize, Rgb8Pixel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use wg_app::config::Config;
use wg_app::ui::{self, AppState, Paths};

thread_local! {
    static LAST: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
}

struct Headless;

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let w = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        LAST.with(|l| *l.borrow_mut() = Some(w.clone()));
        Ok(w)
    }
}

fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Renders the most recently created window at `w` x `h` and writes `name`.png.
fn snapshot(name: &str, w: u32, h: u32) -> Vec<Rgb8Pixel> {
    let win = LAST.with(|l| l.borrow().clone()).expect("a window was created");
    win.set_size(PhysicalSize::new(w, h));
    let mut buf = vec![Rgb8Pixel::default(); (w * h) as usize];
    // The first pass lays things out; a second makes sure animations and late bindings settled.
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        win.draw_if_needed(|r| {
            r.render(&mut buf, w as usize);
        });
    }
    let mut rgba = Vec::with_capacity(buf.len() * 4);
    for p in &buf {
        rgba.extend_from_slice(&[p.r, p.g, p.b, 255]);
    }
    std::fs::write(out_dir().join(format!("app-{name}.png")), wg_win::png::encode_rgba(w, h, &rgba)).unwrap();
    buf
}

fn temp_config() -> Config {
    let d = std::env::temp_dir().join(format!("wg-app-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Config::load(&d).0
}

fn paths() -> Paths {
    Paths {
        work_dir: PathBuf::from(r"C:\Program Files\WindowGrid"),
        log_file: PathBuf::from(r"C:\Program Files\WindowGrid\WindowGrid.log"),
        version: "2.0.0".into(),
    }
}

fn not_blank(buf: &[Rgb8Pixel]) -> bool {
    let first = buf[0];
    buf.iter().any(|p| (p.r, p.g, p.b) != (first.r, first.g, first.b))
}

#[test]
fn windows_render() {
    slint::platform::set_platform(Box::new(Headless)).expect("platform");

    let state = Rc::new(RefCell::new(AppState { config: temp_config(), shared: None }));
    let ui = ui::create_settings_window(state.clone(), paths(), Rc::new(|| {})).expect("settings window");
    ui.show().unwrap();

    // Every settings page at the default window size: none of them should need scrolling.
    let names = ["grid", "appearance", "behavior", "hud", "shortcuts", "info", "technical"];
    for (i, name) in names.iter().enumerate() {
        ui.set_page(i as i32);
        let shot = snapshot(&format!("settings-{name}"), 960, 680);
        assert!(not_blank(&shot), "{name}");
    }
    ui.set_page(0);

    // The in-window confirmations (exit, reset).
    for (n, name) in [(1, "confirm-exit"), (2, "confirm-reset")] {
        ui.set_confirm(n);
        assert!(not_blank(&snapshot(name, 960, 680)), "{name}");
    }
    ui.set_confirm(0);

    // Wiring: presets change the grid sliders; summaries come from the Rust logic.
    ui.invoke_apply_preset(1);
    assert_eq!((ui.get_grid_x(), ui.get_grid_y()), (16.0, 9.0));
    assert_eq!(ui.invoke_grid_summary(16, 9).as_str(), "16 \u{d7} 9 (144 cells)");
    assert_eq!(ui.invoke_cell_shape(4, 4, 16.0 / 9.0).as_str(), "1.8:1 Wide");

    // Shortcut editing goes through the model.
    let before = ui.get_bindings().row_count();
    assert_eq!(before, 4);
    ui.invoke_add_binding();
    assert_eq!(ui.get_bindings().row_count(), 5);
    ui.invoke_edit_binding(4, "command".into(), "Move".into());
    ui.invoke_edit_binding(4, "key".into(), "KEY_LCONTROL".into());
    let row = ui.get_bindings().row_data(4).unwrap();
    assert_eq!((row.command.as_str(), row.key.as_str()), ("Move", "KEY_LCONTROL"));
    ui.invoke_remove_binding(4);
    assert_eq!(ui.get_bindings().row_count(), 4);

    // Save writes the file in the legacy format and reports success.
    ui.invoke_save();
    assert!(ui.get_status_open() && ui.get_status_kind() == 1, "{}", ui.get_status_message());
    let saved = std::fs::read_to_string(&state.borrow().config.path).unwrap();
    assert!(saved.contains("\"GridX\": 16"), "{saved}");
    let snap_status = snapshot("settings-saved", 960, 680);
    assert!(not_blank(&snap_status));
    ui.hide().unwrap();

    // Install dialog.
    let d = ui::open_install_dialog(
        PathBuf::from(r"C:\Program Files\WindowGrid"),
        PathBuf::from(r"D:\portable"),
        false,
        "2.0.0",
        false,
        |_| {},
    )
    .expect("install dialog");
    let install = snapshot("install", 520, 400);
    assert!(not_blank(&install));
    drop(d);
}
