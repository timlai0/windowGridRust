//! Names shared by the app, the overlay and the hook DLLs. The `isolated` feature swaps in
//! test-only names so test builds never touch a running WindowGrid (see `rust/README.md`).
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

pub const MSG_SHOW_GRID: &str = pick!("WindowGridShowGrid", "WindowGridIsoShowGrid");
pub const MSG_AREA: &str = pick!("WindowGridArea", "WindowGridIsoArea");
pub const MSG_STOP: &str = pick!("WindowGridStop", "WindowGridIsoStop");

pub const MAP_HOOK: &str = pick!("WindowGridMapping_Hook", "WindowGridIsoMapping_Hook");
pub const MAP_GRID_WINDOW: &str = pick!("WindowGridMapping_GridWindow", "WindowGridIsoMapping_GridWindow");
pub const MAP_GRID_WINDOW_HANDLE: &str =
    pick!("WindowGridMapping_GridWindowHandle", "WindowGridIsoMapping_GridWindowHandle");

/// Single-instance mutex and PID section of the overlay (its assembly GUID in the C# app).
pub const OVERLAY_INSTANCE: &str = pick!(
    "4AB23252-ACFE-487D-BA61-0C32CE3DB81E",
    "4AB23252-ACFE-487D-BA61-0C32CE3DB81E-iso"
);

/// Single-instance mutex and PID section of the main app (its assembly GUID in the C# app).
pub const APP_INSTANCE: &str = pick!(
    "fde9b139-746f-4ca9-898f-b6dcff4c8f5e",
    "fde9b139-746f-4ca9-898f-b6dcff4c8f5e-iso"
);

pub const LOG_MUTEX: &str = "Global\\WindowGridLog";

/// Input table section (written by the app, read by the hook DLLs).
pub const MAP_INPUT: &str = pick!("WindowGridMapping_Input", "WindowGridIsoMapping_Input");
