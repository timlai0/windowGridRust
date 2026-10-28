//! `WindowGridRust.exe`.
#![windows_subsystem = "windows"]

fn main() {
    std::process::exit(wg_app::app::run());
}
