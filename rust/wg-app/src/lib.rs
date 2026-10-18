//! WindowGrid app: settings UI, tray icon, installer and the supervisor of the overlay and
//! the hook DLLs. Pure logic (arguments, view-model, installer sequencing, configuration)
//! is unit-tested; the rest is Windows glue.

pub mod args;
pub mod config;
pub mod installer;
pub mod viewmodel;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod components;
#[cfg(windows)]
pub mod dialogs;
#[cfg(windows)]
pub mod osopts;
#[cfg(windows)]
pub mod shared;
#[cfg(windows)]
pub mod supervisor;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod ui;
#[cfg(windows)]
pub mod winops;
