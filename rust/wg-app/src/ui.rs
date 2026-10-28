//! Glue between the Slint windows (`ui/app.slint`) and the app's logic.

use crate::config::Config;
use crate::dialogs;
use crate::installer;
use crate::osopts;
use crate::shared::Shared;
use crate::viewmodel::{screen_summary, SettingsModel, PRESETS};
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use wg_core::keycode::{all_key_names, Command, KeyCode, Modifiers};
use wg_core::settings::WindowAnchor;
use wg_win::logger::log;

slint::include_modules!();

/// What the settings window needs to know about the running app.
pub struct AppState {
    pub config: Config,
    pub shared: Option<Shared>,
}

#[derive(Clone)]
pub struct Paths {
    pub work_dir: PathBuf,
    pub log_file: PathBuf,
    pub version: String,
}

fn shared_strings<I: IntoIterator<Item = S>, S: Into<SharedString>>(items: I) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.into_iter().map(Into::into).collect::<Vec<SharedString>>()))
}

fn modifier_names() -> [&'static str; 5] {
    ["None", "Alt", "Control", "Shift", "Windows"]
}

fn binding_rows(model: &SettingsModel) -> Vec<BindingRow> {
    model
        .bindings
        .iter()
        .map(|b| BindingRow {
            command: b.command.to_string().into(),
            modifier: b.modifier.to_string().into(),
            key: b.key.to_string().into(),
        })
        .collect()
}

fn theme_index(state: &AppState, model: &SettingsModel) -> i32 {
    state.config.settings.grid.themes.iter().position(|t| t.name.eq_ignore_ascii_case(&model.theme)).unwrap_or(0) as i32
}

fn show_status(ui: &SettingsWindow, message: &str, kind: i32) {
    ui.set_status_message(message.into());
    ui.set_status_kind(kind);
    ui.set_status_open(true);
}

fn primary_screen() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    // SAFETY: plain queries.
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    (w.max(1), h.max(1))
}

/// Pushes the model into the window.
fn write_ui(ui: &SettingsWindow, state: &AppState, model: &SettingsModel, rows: &VecModel<BindingRow>) {
    ui.set_grid_x(model.grid_x as f32);
    ui.set_grid_y(model.grid_y as f32);
    ui.set_theme_index(theme_index(state, model));
    ui.set_blur(model.blur_radius);
    ui.set_overlay_opacity(model.opacity);
    ui.set_fill_window(model.fill_window);
    ui.set_radial_gradient(model.radial_gradient);
    ui.set_compensate_margins(model.compensate_window_margins);
    ui.set_preserve_size(model.preserve_size_on_move);
    ui.set_defer_resize(model.defer_resize_until_release);
    ui.set_anchor_index(model.anchor as i32);
    ui.set_show_hud(model.show_selection_hud);
    ui.set_show_dimensions(model.show_dimensions);
    ui.set_show_cell_span(model.show_cell_span);
    ui.set_show_coverage(model.show_screen_coverage);
    ui.set_show_aspect(model.show_aspect_ratio);
    ui.set_show_mode(model.show_mode_indicator);
    ui.set_show_hints(model.show_interaction_hints);
    ui.set_splash_screen(model.splash_screen);
    ui.set_aero_shake_disabled(osopts::aero_shake_disabled());
    ui.set_drag_full_windows(osopts::drag_full_windows());
    rows.set_vec(binding_rows(model));
}

/// Reads the window's values into the model (the shortcut list is kept in the model).
fn read_ui(ui: &SettingsWindow, state: &AppState, model: &mut SettingsModel) {
    model.set_grid(ui.get_grid_x().round() as i32, ui.get_grid_y().round() as i32);
    model.set_blur(ui.get_blur());
    model.set_opacity(ui.get_overlay_opacity());
    model.fill_window = ui.get_fill_window();
    model.radial_gradient = ui.get_radial_gradient();
    model.compensate_window_margins = ui.get_compensate_margins();
    model.preserve_size_on_move = ui.get_preserve_size();
    model.defer_resize_until_release = ui.get_defer_resize();
    model.anchor = WindowAnchor::from_i32(ui.get_anchor_index());
    model.show_selection_hud = ui.get_show_hud();
    model.show_dimensions = ui.get_show_dimensions();
    model.show_cell_span = ui.get_show_cell_span();
    model.show_screen_coverage = ui.get_show_coverage();
    model.show_aspect_ratio = ui.get_show_aspect();
    model.show_mode_indicator = ui.get_show_mode();
    model.show_interaction_hints = ui.get_show_hints();
    model.splash_screen = ui.get_splash_screen();
    if let Some(t) = state.config.settings.grid.themes.get(ui.get_theme_index().max(0) as usize) {
        model.theme = t.name.clone();
    }
}

/// Builds the settings window. `on_exit` quits the whole app.
pub fn create_settings_window(
    state: Rc<RefCell<AppState>>,
    paths: Paths,
    on_exit: Rc<dyn Fn()>,
) -> Result<SettingsWindow, slint::PlatformError> {
    let ui = SettingsWindow::new()?;
    let model = Rc::new(RefCell::new(SettingsModel::from_settings(&state.borrow().config.settings)));
    let rows = Rc::new(VecModel::from(binding_rows(&model.borrow())));

    // Static content.
    ui.set_version(paths.version.clone().into());
    ui.set_work_dir(paths.work_dir.display().to_string().into());
    ui.set_log_file(paths.log_file.display().to_string().into());
    ui.set_preset_labels(shared_strings(PRESETS.iter().map(|(x, y)| format!("{x} \u{d7} {y}"))));
    ui.set_command_names(shared_strings(["Resize", "Move"]));
    ui.set_modifier_names(shared_strings(modifier_names()));
    ui.set_key_names(shared_strings(all_key_names()));
    ui.set_bindings(ModelRc::from(rows.clone()));
    let (sw, sh) = primary_screen();
    ui.set_screen_summary(screen_summary(sw, sh).into());
    ui.set_screen_ratio(sw as f32 / sh as f32);
    {
        let s = state.borrow();
        let themes: Vec<ThemeItem> = s
            .config
            .settings
            .grid
            .themes
            .iter()
            .map(|t| {
                let c = |c: wg_core::settings::Color| slint::Color::from_argb_u8(c.a, c.r, c.g, c.b);
                ThemeItem { name: t.name.clone().into(), line: c(t.color_line), start: c(t.color_start), end: c(t.color_end) }
            })
            .collect();
        ui.set_theme_names(shared_strings(s.config.settings.grid.themes.iter().map(|t| t.name.clone())));
        ui.set_themes(ModelRc::new(VecModel::from(themes)));
        write_ui(&ui, &s, &model.borrow(), &rows);
    }

    // Derived text computed in Rust.
    ui.on_grid_summary(|x, y| {
        let mut m = SettingsModel::from_settings(&wg_core::settings::Settings::default());
        m.set_grid(x, y);
        m.grid_summary().into()
    });
    ui.on_cell_shape(|x, y, ratio| {
        let mut m = SettingsModel::from_settings(&wg_core::settings::Settings::default());
        m.set_grid(x, y);
        m.cell_shape_summary(f64::from(ratio)).into()
    });

    let weak = ui.as_weak();

    ui.on_apply_preset({
        let weak = weak.clone();
        move |i| {
            if let (Some(ui), Some(&(x, y))) = (weak.upgrade(), PRESETS.get(i.max(0) as usize)) {
                ui.set_grid_x(x as f32);
                ui.set_grid_y(y as f32);
            }
        }
    });

    ui.on_save({
        let (weak, state, model, rows) = (weak.clone(), state.clone(), model.clone(), rows.clone());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            {
                let mut m = model.borrow_mut();
                read_ui(&ui, &s, &mut m);
                m.apply_to(&mut s.config.settings);
                write_ui(&ui, &s, &m, &rows);
            }
            match s.config.save() {
                Ok(()) => {
                    if let Some(shared) = &s.shared {
                        shared.publish(&s.config.settings);
                    }
                    log("Settings saved");
                    show_status(&ui, "Settings saved successfully.", 1);
                }
                Err(e) => {
                    log(&format!("Saving settings failed: {e}"));
                    show_status(&ui, &format!("Could not save settings: {e}"), 2);
                }
            }
        }
    });

    ui.on_reload({
        let (weak, state, model, rows) = (weak.clone(), state.clone(), model.clone(), rows.clone());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            match s.config.reload() {
                Ok(()) => {
                    *model.borrow_mut() = SettingsModel::from_settings(&s.config.settings);
                    write_ui(&ui, &s, &model.borrow(), &rows);
                    if let Some(shared) = &s.shared {
                        shared.publish(&s.config.settings);
                    }
                    show_status(&ui, "Settings reloaded.", 0);
                }
                Err(e) => show_status(&ui, &format!("Could not reload settings: {e}"), 2),
            }
        }
    });

    ui.on_reset({
        let (weak, state, model, rows) = (weak.clone(), state.clone(), model.clone(), rows.clone());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            s.config.settings = wg_core::settings::Settings::default();
            let saved = s.config.save();
            *model.borrow_mut() = SettingsModel::from_settings(&s.config.settings);
            write_ui(&ui, &s, &model.borrow(), &rows);
            if let Some(shared) = &s.shared {
                shared.publish(&s.config.settings);
            }
            match saved {
                Ok(()) => show_status(&ui, "Settings reset to default values.", 2),
                Err(e) => show_status(&ui, &format!("Could not save the reset settings: {e}"), 2),
            }
        }
    });

    ui.on_cancel({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let _ = ui.hide();
            }
        }
    });

    // The settings window asks for confirmation itself before invoking these two.
    ui.on_exit_app(move || on_exit());

    // Shortcut editing.
    ui.on_add_binding({
        let (model, rows) = (model.clone(), rows.clone());
        move || {
            model.borrow_mut().add_binding();
            rows.set_vec(binding_rows(&model.borrow()));
        }
    });
    ui.on_remove_binding({
        let (model, rows) = (model.clone(), rows.clone());
        move |i| {
            model.borrow_mut().remove_binding(i.max(0) as usize);
            rows.set_vec(binding_rows(&model.borrow()));
        }
    });
    ui.on_edit_binding({
        let (model, rows) = (model.clone(), rows.clone());
        move |i, field, value| {
            let mut m = model.borrow_mut();
            if let Some(b) = m.bindings.get_mut(i.max(0) as usize) {
                match field.as_str() {
                    "command" => b.command = Command::parse(&value).unwrap_or(b.command),
                    "modifier" => b.modifier = Modifiers::parse(&value).unwrap_or(b.modifier),
                    "key" => b.key = KeyCode::parse(&value).unwrap_or(b.key),
                    _ => {}
                }
            }
            rows.set_vec(binding_rows(&m));
        }
    });

    // Options that apply to Windows itself, immediately.
    ui.on_aero_shake_toggled({
        let weak = weak.clone();
        move |on| {
            let ok = osopts::set_aero_shake_disabled(on);
            if let Some(ui) = weak.upgrade() {
                ui.set_aero_shake_disabled(osopts::aero_shake_disabled());
                if !ok {
                    show_status(&ui, "Could not change the Aero Shake setting.", 2);
                }
            }
        }
    });
    ui.on_drag_full_windows_toggled({
        let weak = weak.clone();
        move |on| {
            let ok = osopts::set_drag_full_windows(on);
            if let Some(ui) = weak.upgrade() {
                ui.set_drag_full_windows(osopts::drag_full_windows());
                if !ok {
                    show_status(&ui, "Could not change the window dragging setting.", 2);
                }
            }
        }
    });

    ui.on_open_dir({
        let p = paths.work_dir.clone();
        move || {
            if !dialogs::open_path(&p) {
                dialogs::tell("WindowGridRust", "Could not open the working directory.", true);
            }
        }
    });
    ui.on_open_log({
        let p = paths.log_file.clone();
        move || {
            if p.exists() {
                if !dialogs::open_path(&p) {
                    dialogs::tell("WindowGridRust", "Could not open the log file.", true);
                }
            } else {
                dialogs::tell("WindowGridRust", "The log file does not exist yet.", false);
            }
        }
    });

    ui.window().on_close_requested(|| CloseRequestResponse::HideWindow);
    Ok(ui)
}

// ---- install dialog ---------------------------------------------------------------------------

/// What the user chose in the setup dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallChoice {
    pub dir: PathBuf,
    pub portable: bool,
    pub shortcut: bool,
    pub autostart: bool,
    pub splash_screen: bool,
}

type DoneCallback = Box<dyn FnOnce(Option<InstallChoice>)>;

/// Opens the setup dialog. `on_done` gets `Some(choice)` on OK, `None` if cancelled.
/// With `quit_when_done` the Slint event loop is stopped afterwards (for use before the
/// tray exists, where the dialog is the only thing running).
pub fn open_install_dialog(
    initial_dir: PathBuf,
    current_dir: PathBuf,
    splash_screen: bool,
    version: &str,
    quit_when_done: bool,
    on_done: impl FnOnce(Option<InstallChoice>) + 'static,
) -> Result<InstallWindow, slint::PlatformError> {
    let ui = InstallWindow::new()?;
    ui.set_version(version.into());
    ui.set_path(initial_dir.display().to_string().into());
    ui.set_splash_screen(splash_screen);
    ui.set_needs_admin(!installer::can_write(&initial_dir));

    let done: Rc<RefCell<Option<DoneCallback>>> = Rc::new(RefCell::new(Some(Box::new(on_done))));
    let finish = {
        let (weak, done) = (ui.as_weak(), done.clone());
        Rc::new(move |choice: Option<InstallChoice>| {
            if let Some(ui) = weak.upgrade() {
                let _ = ui.hide();
            }
            if let Some(f) = done.borrow_mut().take() {
                f(choice);
            }
            if quit_when_done {
                let _ = slint::quit_event_loop();
            }
        })
    };

    let weak = ui.as_weak();
    ui.on_path_edited({
        let weak = weak.clone();
        move |text| {
            if let Some(ui) = weak.upgrade() {
                ui.set_needs_admin(!installer::can_write(&PathBuf::from(text.as_str())));
            }
        }
    });
    ui.on_browse({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(dir) = dialogs::pick_folder(&PathBuf::from(ui.get_path().as_str())) {
                ui.set_needs_admin(!installer::can_write(&dir));
                ui.set_path(dir.display().to_string().into());
            }
        }
    });
    ui.on_use_default({
        let (weak, current) = (weak.clone(), current_dir.clone());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let dir = if ui.get_portable() { current.clone() } else { installer::default_dir() };
            ui.set_needs_admin(!installer::can_write(&dir));
            ui.set_path(dir.display().to_string().into());
        }
    });
    ui.on_accept({
        let (weak, finish) = (weak.clone(), finish.clone());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let portable = ui.get_portable();
            finish(Some(InstallChoice {
                dir: PathBuf::from(ui.get_path().as_str()),
                portable,
                shortcut: ui.get_shortcut() && !portable,
                autostart: ui.get_autostart() && !portable,
                splash_screen: ui.get_splash_screen(),
            }));
        }
    });
    let cancel = {
        let finish = finish.clone();
        Rc::new(move || {
            if dialogs::ask("WindowGridRust Setup", "Cancel Installation?") {
                finish(None);
                true
            } else {
                false
            }
        })
    };
    ui.on_cancel({
        let cancel = cancel.clone();
        move || {
            cancel();
        }
    });
    ui.window().on_close_requested(move || {
        if cancel() {
            CloseRequestResponse::HideWindow
        } else {
            CloseRequestResponse::KeepWindowShown
        }
    });
    ui.show()?;
    Ok(ui)
}

/// Shows the logo for `duration`, then closes it.
pub fn show_splash(duration: Duration) -> Result<(), slint::PlatformError> {
    let splash = SplashWindow::new()?;
    splash.show()?;
    slint::Timer::single_shot(duration, move || {
        let _ = splash.hide();
    });
    Ok(())
}
