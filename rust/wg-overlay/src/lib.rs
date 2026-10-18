//! WindowGrid overlay: the pure logic lives here and is unit-tested; the Windows glue
//! (window, GPU rendering, shared memory) sits alongside it.

pub mod geom;
pub mod hud;
pub mod names;
pub mod snap;
pub mod state;
pub mod theme;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod gfx;

// Shared Windows helpers, re-exported so existing paths keep working.
#[cfg(windows)]
pub use wg_win::{logger, shm};
pub use wg_win::png;
