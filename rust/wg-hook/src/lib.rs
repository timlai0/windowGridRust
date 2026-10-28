//! WindowGridRust hook DLL (`WindowGridRust64.dll` / `WindowGridRust32.dll`), see ADR 002.
//!
//! `grid` and `session` hold all behavior and are plain, testable Rust. The remaining
//! modules are the Windows glue; they are `no_std`, link without the C runtime, and are
//! only built for the MSVC targets.

#![cfg_attr(all(not(test), target_env = "msvc"), no_std)]

pub mod grid;
pub mod session;

#[cfg(all(not(test), target_env = "msvc"))]
mod entry;
#[cfg(all(not(test), target_env = "msvc"))]
mod log;
#[cfg(all(not(test), target_env = "msvc"))]
mod rt;
#[cfg(all(not(test), target_env = "msvc"))]
mod sys;
