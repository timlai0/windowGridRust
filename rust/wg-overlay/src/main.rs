//! `WindowGridOverlay.exe`: draws the grid while a window is dragged and snaps it afterwards.
#![windows_subsystem = "windows"]

fn main() {
    if let Err(e) = wg_overlay::app::run() {
        wg_overlay::logger::log(&format!("Overlay failed: {e}"));
        std::process::exit(1);
    }
}
