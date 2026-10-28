//! Names shared by the app, the overlay and the hook DLLs. The `isolated` feature swaps in
//! test-only names so test builds never touch a running WindowGridRust (see `rust/README.md`).
//!
//! (`wg-hook` declares the same names as wide literals of its own, because it cannot use
//! this module's `&str`s in a `no_std`, CRT-free DLL; keep both in sync.)

macro_rules! pick {
    ($normal:literal, $isolated:literal) => {{
        #[cfg(not(feature = "isolated"))]
        {
            $normal
        }
        #[cfg(feature = "isolated")]
        {
            $isolated
        }
    }};
}

pub const MSG_SHOW_GRID: &str = pick!("WindowGridRustShowGrid", "WindowGridRustIsoShowGrid");
pub const MSG_AREA: &str = pick!("WindowGridRustArea", "WindowGridRustIsoArea");
pub const MSG_TARGET: &str = pick!("WindowGridRustTarget", "WindowGridRustIsoTarget");
pub const MSG_STOP: &str = pick!("WindowGridRustStop", "WindowGridRustIsoStop");

pub const MAP_HOOK: &str = pick!("WindowGridRustMapping_Hook", "WindowGridRustIsoMapping_Hook");
pub const MAP_GRID_WINDOW: &str = pick!("WindowGridRustMapping_GridWindow", "WindowGridRustIsoMapping_GridWindow");
pub const MAP_GRID_WINDOW_HANDLE: &str =
    pick!("WindowGridRustMapping_GridWindowHandle", "WindowGridRustIsoMapping_GridWindowHandle");

/// Single-instance mutex and PID section of the overlay.
pub const OVERLAY_INSTANCE: &str = pick!(
    "C28BB0DC-B9EA-4472-9BBC-F0BC63EA5D16",
    "C28BB0DC-B9EA-4472-9BBC-F0BC63EA5D16-iso"
);

/// Single-instance mutex and PID section of the main app.
pub const APP_INSTANCE: &str = pick!(
    "643cf41b-7290-4b37-8eb4-82ca50576665",
    "643cf41b-7290-4b37-8eb4-82ca50576665-iso"
);

pub const LOG_MUTEX: &str = "Global\\WindowGridRustLog";

/// Input table section (written by the app, read by the hook DLLs).
pub const MAP_INPUT: &str = pick!("WindowGridRustMapping_Input", "WindowGridRustIsoMapping_Input");
