//! Shared building blocks for WindowGridRust: the settings model and its legacy-compatible
//! JSON format, the IPC contract between the app, overlay and hook DLLs, and log helpers.
//!
//! With the default `std` feature everything is available. Without it the crate is
//! `no_std` and only [`ipc`] and [`keycode`] exist, which is all the hook DLLs need.

#![cfg_attr(not(feature = "std"), no_std)]

pub mod ipc;
pub mod keycode;
mod keycode_table;
pub mod names;

#[cfg(feature = "std")]
pub mod json;
#[cfg(feature = "std")]
pub mod log;
#[cfg(feature = "std")]
pub mod settings;
