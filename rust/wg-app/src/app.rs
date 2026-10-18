//! Startup, shutdown and the command line flows: the Rust counterpart of the legacy
//! `WindowGridApp.Initialize` / `Run` / `Uninitialize`.

use crate::args::{Args, Command, USAGE};
use crate::components;
use crate::config::{Config, LoadState};
use crate::dialogs;
use crate::installer::{self, Outcome};
use crate::shared::Shared;
use crate::supervisor::{self, Supervisor};
use crate::tray::{Tray, TrayEvent};
use crate::ui::{self, AppState, InstallChoice, Paths};
use crate::winops::RealPorts;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use wg_core::names;
use wg_win::logger::log;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// True when the (isolated-build-only) test switch `name` is set to `1`.
fn test_flag(name: &str) -> bool {
    cfg!(feature = "isolated") && std::env::var(name).is_ok_and(|v| v == "1")
}

fn exe_and_dir() -> std::io::Result<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok((exe, dir))
}

fn ports(exe: &Path, config: &Config) -> RealPorts {
    RealPorts { exe: exe.to_path_buf(), settings: config.settings.clone(), shortcut_dir: None }
}

/// Runs the install and reports what happened. Returns true if this process should exit
/// (the installed copy was started, or an elevated copy took over).
fn do_install(exe: &Path, config: &Config, dir: Option<PathBuf>, shortcut: bool, autostart: bool) -> bool {
    match installer::install(&mut ports(exe, config), exe, dir, shortcut, autostart) {
        Ok(Outcome::Elevated(true)) => true,
        Ok(Outcome::Elevated(false)) => {
            dialogs::tell("WindowGrid", "Install failed: administrator permission is required for that folder.", true);
            true
        }
        Ok(Outcome::Installed { target, restarted }) => {
            log(&format!("Installed: {} (restarted={restarted})", target.display()));
            restarted
        }
        Err(e) => {
            log(&format!("Install failed: {e}"));
            dialogs::tell("WindowGrid", &format!("Install failed: {e}"), true);
            true
        }
    }
}

/// Shows the setup dialog and installs what the user chose. Blocks (runs the Slint event
/// loop) until the dialog is finished. Returns true if the process should exit.
fn setup_dialog(exe: &Path, work_dir: &Path, config: &mut Config, initial: Option<PathBuf>) -> bool {
    let result: Rc<RefCell<Option<InstallChoice>>> = Rc::new(RefCell::new(None));
    let res = result.clone();
    let dialog = ui::open_install_dialog(
        initial.unwrap_or_else(installer::default_dir),
        work_dir.to_path_buf(),
        config.settings.app.splash_screen,
        VERSION,
        true,
        move |choice| *res.borrow_mut() = choice,
    );
    let Ok(dialog) = dialog else {
        dialogs::tell("WindowGrid", "The setup window could not be opened.", true);
        return true;
    };
    let _ = slint::run_event_loop();
    drop(dialog);
    let Some(choice) = result.borrow_mut().take() else {
        // Cancelled: keep the defaults on disk so the question is not asked again.
        let _ = config.save();
        return false;
    };
    config.settings.app.splash_screen = choice.splash_screen;
    config.settings.app.portable = choice.portable;
    if choice.portable {
        // Portable: run from where we are with the chosen settings.
        let _ = config.save();
        return false;
    }
    do_install(exe, config, Some(choice.dir), choice.shortcut, choice.autostart)
}

/// The program. Returns the process exit code.
pub fn run() -> i32 {
    let (exe, work_dir) = match exe_and_dir() {
        Ok(v) => v,
        Err(e) => {
            dialogs::tell("WindowGrid", &format!("Cannot determine where WindowGrid is running from: {e}"), true);
            return 1;
        }
    };
    let _ = std::env::set_current_dir(&work_dir);
    wg_win::logger::start(VERSION);

    // Only one copy runs; a newer one replaces an older one (this is also how an update
    // or the installer's relaunch takes over).
    let Some(_instance) = wg_win::instance::acquire(names::APP_INSTANCE) else { return 0 };

    let args = Args::parse(std::env::args().skip(1));
    let command = Command::from_args(&args);
    let (mut config, state) = Config::load(&work_dir);

    match command {
        Command::Help => {
            dialogs::tell("WindowGrid Usage", USAGE, false);
            return 0;
        }
        Command::Install { dir, silent, shortcut, autostart } => {
            if silent {
                do_install(&exe, &config, dir, shortcut, autostart);
            } else {
                setup_dialog(&exe, &work_dir, &mut config, dir);
            }
            return 0;
        }
        Command::Replace { path } => {
            let mut p = ports(&exe, &config);
            if let Err(e) = installer::replace(&mut p, &exe, &path) {
                dialogs::tell("WindowGrid", &format!("Replace failed: {e}"), true);
            }
            return 0;
        }
        Command::Run => {}
    }

    match &state {
        LoadState::Loaded => {}
        LoadState::Missing => {
            let skip = test_flag("WG_TEST_NO_PROMPT");
            if !skip && dialogs::ask("WindowGrid", "Would you like to run the setup for WindowGrid?") {
                if setup_dialog(&exe, &work_dir, &mut config, None) {
                    return 0;
                }
            } else {
                let _ = config.save();
            }
        }
        LoadState::Invalid(why) => {
            log(&format!("Settings file unusable ({why}); using defaults"));
            if !test_flag("WG_TEST_NO_PROMPT") {
                dialogs::tell("WindowGrid", &format!("The settings file could not be read ({why}). Defaults are used until you save."), false);
            }
        }
    }

    run_tray_app(exe, work_dir, config)
}

fn run_tray_app(_exe: PathBuf, work_dir: PathBuf, config: Config) -> i32 {
    // Put the overlay and the hook DLLs next to the executable (they are compiled into it).
    components::remove_leftovers(&work_dir);
    for (file, result) in components::extract(&work_dir, &components::embedded()) {
        match result {
            Ok(r) => log(&format!("Component {file}: {r:?}")),
            Err(e) => log(&format!("Component {file} could not be written: {e}")),
        }
    }

    let shared = match Shared::create() {
        Ok(s) => s,
        Err(e) => {
            dialogs::tell("WindowGrid", &format!("Could not create the shared memory the hooks need: {e}"), true);
            return 1;
        }
    };
    shared.publish(&config.settings);
    log("Settings published");

    let machine = supervisor::native_machine();
    let (supervisor, problems) = match Supervisor::start(supervisor::specs(&work_dir, machine)) {
        Ok(v) => v,
        Err(e) => {
            dialogs::tell("WindowGrid", &format!("Could not start the helper processes: {e}"), true);
            return 1;
        }
    };
    if !problems.is_empty() {
        log(&format!("Startup problems: {problems:?}"));
        dialogs::tell("WindowGrid", &format!("Some parts of WindowGrid are missing:\n\n{}", problems.join("\n")), true);
    }

    let splash = config.settings.app.splash_screen;
    let state = Rc::new(RefCell::new(AppState { config, shared: Some(shared) }));
    let paths = Paths { work_dir: work_dir.clone(), log_file: work_dir.join("WindowGrid.log"), version: VERSION.to_string() };
    if splash {
        let _ = ui::show_splash(Duration::from_millis(1500));
    }

    let settings_window: Rc<RefCell<Option<ui::SettingsWindow>>> = Rc::new(RefCell::new(None));
    let quit: Rc<dyn Fn()> = Rc::new(|| {
        let _ = slint::quit_event_loop();
    });

    let open_settings: Rc<dyn Fn()> = {
        let (state, paths, window, quit) = (state.clone(), paths.clone(), settings_window.clone(), quit.clone());
        Rc::new(move || {
            let mut slot = window.borrow_mut();
            if slot.is_none() {
                match ui::create_settings_window(state.clone(), paths.clone(), quit.clone()) {
                    Ok(w) => *slot = Some(w),
                    Err(e) => {
                        log(&format!("Settings window failed: {e}"));
                        return;
                    }
                }
            }
            if let Some(w) = slot.as_ref() {
                let _ = w.show();
            }
        })
    };

    let open_install = {
        let (state, work_dir, exe) = (state.clone(), work_dir.clone(), _exe.clone());
        move || {
            let settings = state.borrow().config.settings.clone();
            let exe = exe.clone();
            let state2 = state.clone();
            let quit_after = |restarted| {
                if restarted {
                    let _ = slint::quit_event_loop();
                }
            };
            let dialog = ui::open_install_dialog(
                installer::default_dir(),
                work_dir.clone(),
                settings.app.splash_screen,
                VERSION,
                false,
                move |choice| {
                    let Some(choice) = choice else { return };
                    let mut s = state2.borrow_mut();
                    s.config.settings.app.splash_screen = choice.splash_screen;
                    s.config.settings.app.portable = choice.portable;
                    if choice.portable {
                        let _ = s.config.save();
                        return;
                    }
                    let cfg = Config { settings: s.config.settings.clone(), path: s.config.path.clone() };
                    drop(s);
                    quit_after(do_install(&exe, &cfg, Some(choice.dir), choice.shortcut, choice.autostart));
                },
            );
            // The dialog keeps itself alive through its window handle until it is closed.
            if let Ok(d) = dialog {
                std::mem::forget(d);
            }
        }
    };

    let tray = Tray::create("WindowGrid", {
        let (quit, open_settings) = (quit.clone(), open_settings.clone());
        move |event| match event {
            TrayEvent::Settings => open_settings(),
            TrayEvent::Install => open_install(),
            TrayEvent::Exit => quit(),
        }
    });
    if let Err(e) = &tray {
        log(&format!("Tray icon failed: {e}"));
    }

    if test_flag("WG_TEST_OPEN_SETTINGS") {
        open_settings();
    }
    if cfg!(feature = "isolated") {
        if let Some(ms) = std::env::var("WG_TEST_QUIT_AFTER_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
            slint::Timer::single_shot(Duration::from_millis(ms), || {
                let _ = slint::quit_event_loop();
            });
        }
    }

    log("Running");
    let _ = slint::run_event_loop_until_quit();

    log("Exiting");
    drop(tray);
    supervisor.stop();
    if let Some(w) = settings_window.borrow().as_ref() {
        let _ = w.hide();
    }
    drop(state);
    0
}
