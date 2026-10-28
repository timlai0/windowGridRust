//! Appends to `WindowGridRust.log` next to the executable, in the format shared with the app
//! and the hook DLLs (see `wg_core::log`). Writers hold the named log mutex briefly.

use crate::wide;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, GetCurrentThreadId, ReleaseMutex, WaitForSingleObject,
};

static PATH: OnceLock<std::path::PathBuf> = OnceLock::new();

fn path() -> &'static std::path::Path {
    PATH.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("WindowGridRust.log")))
            .unwrap_or_else(|| "WindowGridRust.log".into())
    })
}

fn exe_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "WindowGridRust.exe".to_string())
}

pub fn log(msg: &str) {
    // SAFETY: plain queries and a named mutex used only within this call.
    let (line, mutex) = unsafe {
        let t = GetLocalTime();
        let line = wg_core::log::format_line(
            &wg_core::log::clock(u32::from(t.wHour), u32::from(t.wMinute), u32::from(t.wSecond), u32::from(t.wMilliseconds)),
            "APP",
            GetCurrentProcessId(),
            GetCurrentThreadId(),
            &exe_name(),
            msg,
        );
        let name = wide(wg_core::names::LOG_MUTEX);
        let mutex = CreateMutexW(None, false, PCWSTR(name.as_ptr())).ok();
        if let Some(m) = mutex {
            WaitForSingleObject(m, 250);
        }
        (line, mutex)
    };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path()) {
        let _ = writeln!(f, "{line}");
    }
    if let Some(m) = mutex {
        // SAFETY: the mutex handle is ours; releasing without ownership is harmless.
        unsafe {
            let _ = ReleaseMutex(m);
            let _ = CloseHandle(HANDLE(m.0));
        }
    }
}

/// Writes the separator and a start line, like the other WindowGridRust processes.
pub fn start(version: &str) {
    for line in wg_core::log::session_separator() {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path()) {
            let _ = writeln!(f, "{line}");
        }
    }
    log(&format!("Starting {} ({version})", exe_name()));
}
