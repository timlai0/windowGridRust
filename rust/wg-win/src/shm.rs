//! Named shared-memory sections (pagefile-backed), created if missing like the legacy app.

use crate::wide;
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS,
    PAGE_READWRITE,
};
use wg_core::ipc::Wire;

pub struct Section {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    size: usize,
}

impl Section {
    pub fn open_or_create(name: &str, size: usize) -> Result<Section> {
        let name = wide(name);
        // SAFETY: `name` is NUL-terminated and outlives the calls.
        unsafe {
            let handle =
                CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, size as u32, PCWSTR(name.as_ptr()))?;
            let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size);
            if view.Value.is_null() {
                let err = windows::core::Error::from_thread();
                let _ = CloseHandle(handle);
                return Err(err);
            }
            Ok(Section { handle, view, size })
        }
    }

    pub fn read<T: Wire>(&self) -> T {
        assert!(T::SIZE <= self.size);
        // SAFETY: the view is `size` bytes and stays mapped for `self`'s lifetime.
        let bytes = unsafe { std::slice::from_raw_parts(self.view.Value as *const u8, T::SIZE) };
        T::from_bytes(bytes).expect("length checked")
    }

    /// Overwrites `bytes.len()` bytes at `offset` (so other fields are not clobbered).
    pub fn write_bytes(&self, offset: usize, bytes: &[u8]) {
        assert!(offset + bytes.len() <= self.size);
        // SAFETY: bounds checked above; the view is writable.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), (self.view.Value as *mut u8).add(offset), bytes.len());
        }
    }
}

impl Drop for Section {
    fn drop(&mut self) {
        // SAFETY: the view and handle were created in `open_or_create`.
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}
