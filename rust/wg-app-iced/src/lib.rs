//! WindowGridRust with an iced UI: the all-Rust fork of `wg-app`. The settings model, the
//! installer, the supervisor, the tray icon and the settings file are `wg-app`'s; only the
//! windows (settings, setup, splash) and the event loop around them are new.

pub mod app;
pub mod assets;
pub mod backdrop;
pub mod install;
pub mod memo;
pub mod preview;
pub mod settings;
pub mod style;
pub mod widgets;

use std::path::PathBuf;
use wg_app::config::Config;
use wg_app::shared::Shared;

/// What the windows need to know about the running app.
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
