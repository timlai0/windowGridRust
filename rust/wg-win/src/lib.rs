//! Windows helpers shared by the WindowGrid overlay and app.

pub mod instance;
pub mod logger;
pub mod png;
pub mod shm;

/// Wide, NUL-terminated copy of `s` for Win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
