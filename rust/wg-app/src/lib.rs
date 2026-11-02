//! WindowGridRust app logic: the tray icon, the installer and the supervisor of the overlay and
//! the hook DLLs, and the settings model the windows edit. Pure logic (arguments, view-model,
//! installer sequencing, configuration) is unit-tested; the rest is Windows glue. The windows
//! and the executable are in `wg-app-iced`.

pub mod args;
pub mod config;
pub mod installer;
pub mod viewmodel;

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
pub mod winops;
