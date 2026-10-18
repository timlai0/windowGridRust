//! "Only one of me": a named mutex plus a small section holding the owner's PID.
//!
//! Like the legacy apps, a newer instance *replaces* an older one: it terminates the PID
//! recorded in the section and takes over the mutex. This is what makes starting WindowGrid
//! again (or the installer relaunching it) leave exactly one running copy.

use crate::logger::log;
use crate::shm::Section;
use crate::wide;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE,
};

/// Held for the life of the process; dropping it releases the instance.
pub struct Instance {
    mutex: HANDLE,
    _pid: Section,
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the handle is ours.
        unsafe {
            let _ = CloseHandle(self.mutex);
        }
    }
}

/// Becomes the one running instance for `id` (a GUID-like name, see `wg_core::names`).
/// Returns `None` if an older instance could not be replaced.
pub fn acquire(id: &str) -> Option<Instance> {
    let mutex_name = wide(&format!("Global\\{{{id}}}"));
    // SAFETY: NUL-terminated name; handles are closed on every exit path below.
    unsafe {
        let mutex = CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr())).ok()?;
        let existed = GetLastError() == ERROR_ALREADY_EXISTS;
        let pid = match Section::open_or_create(id, 4) {
            Ok(s) => s,
            Err(_) => {
                let _ = CloseHandle(mutex);
                return None;
            }
        };
        if existed {
            let old = pid.read::<i32>() as u32;
            log(&format!("Another instance (PID {old}) is running; replacing it"));
            if let Ok(p) = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, old) {
                let _ = TerminateProcess(p, 0);
                WaitForSingleObject(p, 1000);
                let _ = CloseHandle(p);
            }
            let w = WaitForSingleObject(mutex, 1000);
            if w != WAIT_OBJECT_0 && w != WAIT_ABANDONED {
                log("Could not take over from the running instance; exiting");
                let _ = CloseHandle(mutex);
                return None;
            }
        }
        pid.write_bytes(0, &GetCurrentProcessId().to_le_bytes());
        Some(Instance { mutex, _pid: pid })
    }
}
