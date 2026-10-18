//! Names shared with the app and the hook DLLs; they live in `wg_core::names`, which also
//! implements the `isolated` test variant.

pub use wg_core::names::*;

/// The overlay's own single-instance name.
pub const INSTANCE: &str = OVERLAY_INSTANCE;
