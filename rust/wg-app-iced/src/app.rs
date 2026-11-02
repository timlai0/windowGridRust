//! Startup, shutdown, the command line flows and the windows.
//!
//! The whole process is one `iced::daemon`: it has no window of its own, keeps running when
//! its windows are closed, and ends only through [`iced::exit`]. That is the shape of a tray
//! app. The settings, setup and splash windows are daemon windows opened on demand, and the
//! tray icon (a hidden Win32 window owned by `wg_app::tray`) feeds the daemon through a
//! channel.

use crate::install::{self, InstallChoice, InstallWindow, Origin};
use crate::settings::{self, SettingsWindow};
use crate::{assets, style, AppState, Paths};
use iced::futures::channel::{mpsc, oneshot};
use iced::futures::{stream, Stream, StreamExt};
use iced::keyboard::{self, key::Named};
use iced::widget::{image, space};
use iced::{event, system, theme, window, Element, Font, Size, Subscription, Task, Theme};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;
use wg_app::args::{Args, Command, USAGE};
use wg_app::config::{Config, LoadState};
use wg_app::installer;
use wg_app::osopts;
use wg_app::shared::Shared;
use wg_app::supervisor::{self, Supervisor};
use wg_app::tray::{Tray, TrayEvent};
use wg_app::winops::{self, RealPorts};
use wg_app::{components, dialogs};
use wg_core::names;
use wg_win::logger::log;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long the splash screen stays.
const SPLASH: Duration = Duration::from_millis(1500);

/// Tray events wait here from the moment the tray icon exists until its subscription starts.
static TRAY_EVENTS: Mutex<Option<mpsc::UnboundedReceiver<TrayEvent>>> = Mutex::new(None);

/// The process exit code, set by whatever ends the daemon.
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

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
        Ok(installer::Outcome::Elevated(true)) => true,
        Ok(installer::Outcome::Elevated(false)) => {
            dialogs::tell("WindowGridRust", "Install failed: administrator permission is required for that folder.", true);
            true
        }
        Ok(installer::Outcome::Installed { target, restarted }) => {
            log(&format!("Installed: {} (restarted={restarted})", target.display()));
            restarted
        }
        Err(e) => {
            log(&format!("Install failed: {e}"));
            dialogs::tell("WindowGridRust", &format!("Install failed: {e}"), true);
            true
        }
    }
}

/// Whether apps should be drawn dark, from the Windows setting (light only when it says so
/// or the setting does not exist).
fn system_prefers_dark() -> bool {
    winops::reg_get_dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme") == Some(0)
}

/// A task that runs `work`, which may block, on a thread of its own. Neither the interface thread
/// nor an executor thread waits for it.
fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Task<T> {
    let (done, wait) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = done.send(work());
    });
    Task::future(async move { wait.await.ok() }).then(|done| done.map_or_else(Task::none, Task::done))
}

/// A task that finishes after `delay`.
fn after(delay: Duration) -> Task<()> {
    blocking(move || std::thread::sleep(delay))
}

// ---- the program ------------------------------------------------------------------------------

/// What the process starts as, decided from the command line and the settings file.
enum Start {
    /// The tray app: helpers, tray icon, settings on demand.
    Tray,
    /// The setup dialog first. With `then_run` the tray app follows unless the installed copy
    /// took over.
    Setup { initial: Option<PathBuf>, then_run: bool },
}

struct Init {
    exe: PathBuf,
    work_dir: PathBuf,
    config: Config,
    start: Start,
}

/// The program. Returns the process exit code.
pub fn run() -> i32 {
    let (exe, work_dir) = match exe_and_dir() {
        Ok(v) => v,
        Err(e) => {
            dialogs::tell("WindowGridRust", &format!("Cannot determine where WindowGridRust is running from: {e}"), true);
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
    let (config, state) = Config::load(&work_dir);

    let start = match command {
        Command::Help => {
            dialogs::tell("WindowGridRust Usage", USAGE, false);
            return 0;
        }
        Command::Install { dir, silent, shortcut, autostart } => {
            if silent {
                do_install(&exe, &config, dir, shortcut, autostart);
                return 0;
            }
            Start::Setup { initial: dir, then_run: false }
        }
        Command::Replace { path } => {
            let mut p = ports(&exe, &config);
            if let Err(e) = installer::replace(&mut p, &exe, &path) {
                dialogs::tell("WindowGridRust", &format!("Replace failed: {e}"), true);
            }
            return 0;
        }
        Command::Run => match &state {
            LoadState::Loaded => Start::Tray,
            LoadState::Missing => {
                let skip = test_flag("WG_TEST_NO_PROMPT");
                if !skip && dialogs::ask("WindowGridRust", "Would you like to run the setup for WindowGridRust?") {
                    Start::Setup { initial: None, then_run: true }
                } else {
                    let _ = config.save();
                    Start::Tray
                }
            }
            LoadState::Invalid(why) => {
                log(&format!("Settings file unusable ({why}); using defaults"));
                if !test_flag("WG_TEST_NO_PROMPT") {
                    dialogs::tell("WindowGridRust", &format!("The settings file could not be read ({why}). Defaults are used until you save."), false);
                }
                Start::Tray
            }
        },
    };

    run_daemon(Init { exe, work_dir, config, start })
}

fn run_daemon(init: Init) -> i32 {
    let init = RefCell::new(Some(init));
    let result = iced::daemon(move || App::boot(init.borrow_mut().take().expect("the daemon boots once")), App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .style(App::style)
        .subscription(App::subscription)
        .default_font(Font::with_name("Segoe UI"))
        .run();
    match result {
        Ok(()) => EXIT_CODE.load(Ordering::Relaxed),
        Err(e) => {
            log(&format!("The UI failed: {e}"));
            dialogs::tell("WindowGridRust", &format!("The user interface could not start: {e}"), true);
            1
        }
    }
}

/// What runs while the tray app is up.
struct Services {
    tray: Option<Tray>,
    /// `None` only in tests that do not want the helper processes (see `WG_TEST_NO_HELPERS`).
    supervisor: Option<Supervisor>,
}

impl Services {
    fn stop(self) {
        drop(self.tray);
        if let Some(supervisor) = self.supervisor {
            supervisor.stop();
        }
    }
}

struct App {
    exe: PathBuf,
    work_dir: PathBuf,
    state: AppState,
    paths: Paths,
    dark: bool,
    settings: Option<SettingsWindow>,
    install: Option<InstallWindow>,
    splash: Option<window::Id>,
    services: Option<Services>,
}

#[derive(Debug, Clone)]
enum Message {
    Tray(TrayEvent),
    SystemTheme(theme::Mode),
    Settings(settings::Msg),
    Install(install::Msg),
    Opened,
    WindowClosed(window::Id),
    CloseRequested(window::Id),
    Escape(window::Id),
    Enter(window::Id),
    SplashDone,
    Quit,
}

fn tray_events() -> impl Stream<Item = Message> {
    let receiver = TRAY_EVENTS.lock().unwrap_or_else(PoisonError::into_inner).take();
    stream::iter(receiver).flatten().map(Message::Tray)
}

/// Escape and Enter answer a question drawn over a window.
fn keys(event: event::Event, status: event::Status, window: window::Id) -> Option<Message> {
    if status == event::Status::Captured {
        return None;
    }
    match event {
        event::Event::Keyboard(keyboard::Event::KeyPressed { key: keyboard::Key::Named(Named::Escape), .. }) => Some(Message::Escape(window)),
        event::Event::Keyboard(keyboard::Event::KeyPressed { key: keyboard::Key::Named(Named::Enter), .. }) => Some(Message::Enter(window)),
        _ => None,
    }
}

impl App {
    fn boot(init: Init) -> (App, Task<Message>) {
        let Init { exe, work_dir, config, start } = init;
        let paths = Paths { work_dir: work_dir.clone(), log_file: work_dir.join("WindowGridRust.log"), version: VERSION.to_string() };
        let mut app = App {
            exe,
            work_dir,
            state: AppState { config, shared: None },
            paths,
            dark: system_prefers_dark(),
            settings: None,
            install: None,
            splash: None,
            services: None,
        };
        let task = match start {
            Start::Tray => app.enter_tray_mode(),
            Start::Setup { initial, then_run } => {
                app.open_install(initial.unwrap_or_else(installer::default_dir), Origin::Startup { then_run })
            }
        };
        (app, task)
    }

    fn title(&self, id: window::Id) -> String {
        if self.install.as_ref().is_some_and(|w| w.id == id) {
            "WindowGridRust Setup".to_string()
        } else {
            "WindowGridRust".to_string()
        }
    }

    fn theme(&self, id: window::Id) -> Theme {
        if self.splash == Some(id) {
            style::clear_theme(self.dark)
        } else {
            style::theme(self.dark)
        }
    }

    /// The window background is the theme's background: the page colour, or clear for the splash.
    fn style(&self, theme: &Theme) -> theme::Style {
        theme::Style { background_color: theme.palette().background, text_color: theme.palette().text }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut all = vec![
            window::close_events().map(Message::WindowClosed),
            window::close_requests().map(Message::CloseRequested),
            system::theme_changes().map(Message::SystemTheme),
            event::listen_with(keys),
        ];
        // The tray exists once the tray app is up; its channel is taken when this starts.
        if self.services.is_some() {
            all.push(Subscription::run(tray_events));
        }
        Subscription::batch(all)
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        if let Some(w) = self.settings.as_ref().filter(|w| w.id == id) {
            return w.view().map(Message::Settings);
        }
        if let Some(w) = self.install.as_ref().filter(|w| w.id == id) {
            return w.view().map(Message::Install);
        }
        match (self.splash == Some(id)).then(assets::image_handle).flatten() {
            Some(handle) => image(handle).width(256).height(256).into(),
            None => space().into(),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tray(TrayEvent::Settings) => self.open_settings(),
            Message::Tray(TrayEvent::Install) => self.open_install(installer::default_dir(), Origin::Tray),
            Message::Tray(TrayEvent::Exit) | Message::Quit => self.quit(0),
            Message::SystemTheme(mode) => {
                match mode {
                    theme::Mode::Dark => self.dark = true,
                    theme::Mode::Light => self.dark = false,
                    theme::Mode::None => {}
                }
                Task::none()
            }
            Message::Settings(msg) => self.settings_msg(msg),
            Message::Install(msg) => self.install_msg(msg),
            Message::Opened => Task::none(),
            Message::WindowClosed(id) => {
                if self.settings.as_ref().is_some_and(|w| w.id == id) {
                    self.settings = None;
                }
                if self.splash == Some(id) {
                    self.splash = None;
                }
                Task::none()
            }
            Message::CloseRequested(id) => {
                if let Some(w) = self.install.as_mut().filter(|w| w.id == id) {
                    w.request_cancel();
                }
                Task::none()
            }
            Message::Escape(id) => {
                if self.settings.as_ref().is_some_and(|w| w.id == id && w.has_question()) {
                    self.settings_msg(settings::Msg::Dismiss)
                } else if self.install.as_ref().is_some_and(|w| w.id == id && w.has_question()) {
                    self.install_msg(install::Msg::KeepSettingUp)
                } else {
                    Task::none()
                }
            }
            Message::Enter(id) => {
                if self.settings.as_ref().is_some_and(|w| w.id == id && w.has_question()) {
                    self.settings_msg(settings::Msg::Confirmed)
                } else if self.install.as_ref().is_some_and(|w| w.id == id && w.has_question()) {
                    self.install_msg(install::Msg::ConfirmCancel)
                } else {
                    Task::none()
                }
            }
            Message::SplashDone => match self.splash.take() {
                Some(id) => window::close(id),
                None => Task::none(),
            },
        }
    }

    fn settings_msg(&mut self, msg: settings::Msg) -> Task<Message> {
        let Some(w) = self.settings.as_mut() else { return Task::none() };
        let id = w.id;
        match w.update(msg, &mut self.state) {
            settings::Outcome::None => Task::none(),
            settings::Outcome::Close => window::close(id),
            settings::Outcome::Quit => self.quit(0),
            settings::Outcome::SetDragFullWindows(on) => {
                blocking(move || osopts::set_drag_full_windows(on)).map(|ok| Message::Settings(settings::Msg::DragFullWindowsDone(ok)))
            }
        }
    }

    fn install_msg(&mut self, msg: install::Msg) -> Task<Message> {
        let Some(w) = self.install.as_mut() else { return Task::none() };
        match w.update(msg) {
            install::Outcome::None => Task::none(),
            install::Outcome::Done(choice) => self.finish_install(choice),
        }
    }

    /// Ends the process: stops the tray and the helpers, then the daemon.
    fn quit(&mut self, code: i32) -> Task<Message> {
        EXIT_CODE.store(code, Ordering::Relaxed);
        log("Exiting");
        if let Some(services) = self.services.take() {
            services.stop();
        }
        iced::exit()
    }

    // ---- windows ---------------------------------------------------------------------------

    fn open_settings(&mut self) -> Task<Message> {
        if let Some(w) = &self.settings {
            return Task::batch([window::minimize(w.id, false), window::gain_focus(w.id)]);
        }
        let (id, open) = window::open(settings::window_settings());
        let mut w = SettingsWindow::new(id, &self.state, &self.paths);
        if cfg!(feature = "isolated") {
            w.apply_test_switches();
        }
        self.settings = Some(w);
        open.map(|_| Message::Opened)
    }

    fn open_install(&mut self, initial: PathBuf, origin: Origin) -> Task<Message> {
        if let Some(w) = &self.install {
            return Task::batch([window::minimize(w.id, false), window::gain_focus(w.id)]);
        }
        let (id, open) = window::open(install::window_settings());
        let splash = self.state.config.settings.app.splash_screen;
        self.install = Some(InstallWindow::new(id, origin, VERSION, initial, self.work_dir.clone(), splash));
        open.map(|_| Message::Opened)
    }

    /// Shows the logo for a moment, then closes it.
    fn show_splash(&mut self) -> Task<Message> {
        let (id, open) = window::open(window::Settings {
            size: Size::new(256.0, 256.0),
            position: window::Position::Centered,
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            ..Default::default()
        });
        self.splash = Some(id);
        Task::batch([open.map(|_| Message::Opened), after(SPLASH).map(|()| Message::SplashDone)])
    }

    /// The setup dialog is done. What happens next depends on where it was opened from.
    fn finish_install(&mut self, choice: Option<InstallChoice>) -> Task<Message> {
        let Some(dialog) = self.install.take() else { return Task::none() };
        let close = window::close(dialog.id);
        // Whether this process should end: the installed copy took over, or setup is all
        // that was asked for.
        let exit = match choice {
            // Cancelled: keep the defaults on disk so the question is not asked again.
            None => {
                if matches!(dialog.origin, Origin::Startup { .. }) {
                    let _ = self.state.config.save();
                }
                false
            }
            Some(choice) => {
                let settings = &mut self.state.config.settings;
                settings.app.splash_screen = choice.splash_screen;
                settings.app.portable = choice.portable;
                if choice.portable {
                    // Portable: run from where we are with the chosen settings.
                    let _ = self.state.config.save();
                    false
                } else {
                    do_install(&self.exe, &self.state.config, Some(choice.dir), choice.shortcut, choice.autostart)
                }
            }
        };
        match dialog.origin {
            Origin::Startup { then_run: true } if !exit => Task::batch([close, self.enter_tray_mode()]),
            Origin::Startup { .. } => Task::batch([close, self.quit(0)]),
            Origin::Tray if exit => Task::batch([close, self.quit(0)]),
            Origin::Tray => close,
        }
    }

    /// Starts everything the tray app consists of: the helper processes, the shared memory
    /// they read, and the tray icon.
    fn enter_tray_mode(&mut self) -> Task<Message> {
        // Tests of the windows do not need the helpers, and without them nothing can inject a hook
        // DLL into another process while they run.
        let supervisor = if test_flag("WG_TEST_NO_HELPERS") {
            log("Helper processes skipped (WG_TEST_NO_HELPERS)");
            None
        } else {
            match self.start_helpers() {
                Ok(supervisor) => Some(supervisor),
                Err(task) => return task,
            }
        };

        let (events, receiver) = mpsc::unbounded();
        *TRAY_EVENTS.lock().unwrap_or_else(PoisonError::into_inner) = Some(receiver);
        let tray = Tray::create("WindowGridRust", move |event| {
            let _ = events.unbounded_send(event);
        });
        if let Err(e) = &tray {
            log(&format!("Tray icon failed: {e}"));
        }
        self.services = Some(Services { tray: tray.ok(), supervisor });

        let mut tasks = Vec::new();
        if self.state.config.settings.app.splash_screen {
            tasks.push(self.show_splash());
        }
        if test_flag("WG_TEST_OPEN_SETTINGS") {
            tasks.push(self.open_settings());
        }
        if cfg!(feature = "isolated") {
            if let Some(ms) = std::env::var("WG_TEST_QUIT_AFTER_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
                tasks.push(after(Duration::from_millis(ms)).map(|()| Message::Quit));
            }
        }
        log("Running");
        Task::batch(tasks)
    }

    /// Puts the overlay and the hook DLLs next to the executable, publishes the settings they
    /// read, and starts them. On failure the user has been told and the app is quitting: the
    /// task to return.
    fn start_helpers(&mut self) -> Result<Supervisor, Task<Message>> {
        // Put the overlay and the hook DLLs next to the executable (they are compiled into it).
        components::remove_leftovers(&self.work_dir);
        for (file, result) in components::extract(&self.work_dir, &components::embedded()) {
            match result {
                Ok(r) => log(&format!("Component {file}: {r:?}")),
                Err(e) => log(&format!("Component {file} could not be written: {e}")),
            }
        }

        let shared = match Shared::create() {
            Ok(s) => s,
            Err(e) => {
                dialogs::tell("WindowGridRust", &format!("Could not create the shared memory the hooks need: {e}"), true);
                return Err(self.quit(1));
            }
        };
        shared.publish(&self.state.config.settings);
        log("Settings published");

        let machine = supervisor::native_machine();
        let (supervisor, problems) = match Supervisor::start(supervisor::specs(&self.work_dir, machine)) {
            Ok(v) => v,
            Err(e) => {
                dialogs::tell("WindowGridRust", &format!("Could not start the helper processes: {e}"), true);
                return Err(self.quit(1));
            }
        };
        if !problems.is_empty() {
            log(&format!("Startup problems: {problems:?}"));
            dialogs::tell("WindowGridRust", &format!("Some parts of WindowGridRust are missing:

{}", problems.join("
")), true);
        }
        self.state.shared = Some(shared);
        Ok(supervisor)
    }
}
