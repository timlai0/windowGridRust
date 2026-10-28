//! Appends lines to `WindowGridRust.log` (next to the DLL), in the format shared with the app:
//! `HH:mm:ss.fff [X64] (PPPPP:TTTTT) rundll32.exe        : message`. Writers hold the
//! named mutex `Global\WindowGridRustLog` for up to 250 ms. Used for lifecycle events and
//! failures only, never on the per-message hot path.
//!
//! No `core::fmt`: integer formatting there needs 64-bit division helpers that the
//! C-runtime-free 32-bit build does not have.

use crate::sys::{self, wide};
use core::sync::atomic::{AtomicIsize, Ordering};

#[cfg(target_arch = "x86_64")]
const TAG: &str = "X64";
#[cfg(target_arch = "x86")]
const TAG: &str = "X86";
#[cfg(target_arch = "aarch64")]
const TAG: &str = "A64";

static MODULE: AtomicIsize = AtomicIsize::new(0);

pub fn set_module(h: isize) {
    MODULE.store(h, Ordering::Relaxed);
}

pub fn module() -> isize {
    MODULE.load(Ordering::Relaxed)
}

/// Fixed-size byte buffer that silently truncates.
struct Buf<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { bytes: [0; N], len: 0 }
    }

    fn push(&mut self, b: u8) {
        if self.len < N {
            self.bytes[self.len] = b;
            self.len += 1;
        }
    }

    fn push_str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            self.push(b);
        }
    }

    /// Decimal, zero-padded to `width` digits.
    fn push_dec(&mut self, mut n: u32, width: usize) {
        let mut digits = [b'0'; 10];
        let mut count = 0;
        while n > 0 || count == 0 {
            digits[count] = b'0' + (n % 10) as u8;
            n /= 10;
            count += 1;
        }
        for _ in count..width {
            self.push(b'0');
        }
        for i in (0..count).rev() {
            self.push(digits[i]);
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// The executable's file name (ASCII-folded) for the log prefix.
fn exe_name(out: &mut Buf<32>) {
    let mut path = [0u16; 260];
    // SAFETY: buffer and length describe `path`.
    let n = unsafe { sys::GetModuleFileNameW(0, path.as_mut_ptr(), path.len() as u32) } as usize;
    let n = n.min(path.len());
    let start = path[..n].iter().rposition(|&c| c == b'\\' as u16).map_or(0, |i| i + 1);
    for &c in &path[start..n] {
        out.push(if c < 128 { c as u8 } else { b'?' });
    }
}

/// Path of `WindowGridRust.log` beside this DLL, as NUL-terminated UTF-16.
fn log_path(out: &mut [u16; 300]) -> bool {
    // SAFETY: buffer and length describe `out`.
    let n = unsafe { sys::GetModuleFileNameW(module(), out.as_mut_ptr(), 280) } as usize;
    if n == 0 || n >= 280 {
        return false;
    }
    let dir_end = out[..n].iter().rposition(|&c| c == b'\\' as u16).map_or(0, |i| i + 1);
    for (i, &b) in b"WindowGridRust.log\0".iter().enumerate() {
        out[dir_end + i] = u16::from(b);
    }
    true
}

fn write_record(msg: &str, code: Option<u32>) {
    let mut line = Buf::<400>::new();
    let mut st = sys::SYSTEMTIME::default();
    let mut exe = Buf::<32>::new();
    exe_name(&mut exe);
    // SAFETY: `st` is a valid SYSTEMTIME; the other calls are plain queries.
    let (pid, tid) = unsafe {
        sys::GetLocalTime(&mut st);
        (sys::GetCurrentProcessId(), sys::GetCurrentThreadId())
    };

    line.push_dec(u32::from(st.hour), 2);
    line.push(b':');
    line.push_dec(u32::from(st.minute), 2);
    line.push(b':');
    line.push_dec(u32::from(st.second), 2);
    line.push(b'.');
    line.push_dec(u32::from(st.millis), 3);
    line.push_str(" [");
    line.push_str(TAG);
    line.push_str("] (");
    line.push_dec(pid, 5);
    line.push(b':');
    line.push_dec(tid, 5);
    line.push_str(") ");
    for b in exe.as_slice() {
        line.push(*b);
    }
    for _ in exe.len..20 {
        line.push(b' ');
    }
    line.push_str(": ");
    if code.is_some() {
        line.push_str("[ERROR] ");
    }
    for &b in msg.as_bytes() {
        // Keep every record on one line.
        match b {
            b'\r' => line.push_str("\\r"),
            b'\n' => line.push_str("\\n"),
            _ => line.push(b),
        }
    }
    if let Some(code) = code {
        line.push_str(" (error ");
        line.push_dec(code, 0);
        line.push(b')');
    }
    line.push_str("\r\n");

    let mut path = [0u16; 300];
    if !log_path(&mut path) {
        return;
    }
    // SAFETY: all pointers are to live, NUL-terminated or sized buffers.
    unsafe {
        let mutex = sys::CreateMutexW(core::ptr::null(), 0, wide!("Global\\WindowGridRustLog"));
        let locked = mutex != 0 && sys::WaitForSingleObject(mutex, 250) == sys::WAIT_OBJECT_0;
        let file = sys::CreateFileW(
            path.as_ptr(),
            sys::FILE_APPEND_DATA,
            sys::FILE_SHARE_READ_WRITE,
            core::ptr::null(),
            sys::OPEN_ALWAYS,
            sys::FILE_ATTRIBUTE_NORMAL,
            0,
        );
        if file != 0 && file != -1 {
            let mut written = 0u32;
            sys::WriteFile(file, line.bytes.as_ptr(), line.len as u32, &mut written, core::ptr::null_mut());
            sys::CloseHandle(file);
        }
        if locked {
            sys::ReleaseMutex(mutex);
        }
        if mutex != 0 {
            sys::CloseHandle(mutex);
        }
    }
}

pub fn info(msg: &str) {
    write_record(msg, None);
}

/// Logs a failure with the Win32 error code.
pub fn error(msg: &str, code: u32) {
    write_record(msg, Some(code));
}
