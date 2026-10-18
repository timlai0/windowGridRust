//! Starts the overlay and the hook hosts, restarts them if they die, and stops them.
//!
//! All children live in one job object that kills them when it closes, so they cannot
//! outlive the app, even if it is killed. Unlike the legacy supervisor this never kills
//! other programs' `rundll32` processes: stale hosts are asked to quit with the stop
//! message, and a stale overlay is replaced by the overlay's own single-instance logic.

use crate::components::{HOOK_ARM64, HOOK_X64, HOOK_X86, OVERLAY};
use crate::winops::system32;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, WPARAM};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
};
use windows::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, RegisterWindowMessageW, HWND_BROADCAST};
use wg_core::names;
use wg_win::logger::log;
use wg_win::wide;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// One child to keep running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub label: String,
    pub exe: PathBuf,
    /// Working directory (the installation directory, as in the legacy supervisor).
    pub cwd: PathBuf,
    /// Raw command line after the program name (so `rundll32`'s quoting is exact).
    pub raw_args: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Machine {
    X86,
    X64,
    Arm64,
}

/// The architecture of the operating system (not of this process, which may be emulated).
pub fn native_machine() -> Machine {
    let (mut process, mut native) = (IMAGE_FILE_MACHINE(0), IMAGE_FILE_MACHINE(0));
    // SAFETY: valid out pointers for the current process.
    let ok = unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, Some(&mut native)).is_ok() };
    if ok && native == IMAGE_FILE_MACHINE_ARM64 {
        Machine::Arm64
    } else if ok && native == IMAGE_FILE_MACHINE_AMD64 {
        Machine::X64
    } else if ok && native == IMAGE_FILE_MACHINE_I386 {
        Machine::X86
    } else if cfg!(target_pointer_width = "64") {
        Machine::X64
    } else {
        Machine::X86
    }
}

fn windir_dir(sub: &str) -> PathBuf {
    system32("").parent().map_or_else(|| PathBuf::from(r"C:\Windows"), Path::to_path_buf).join(sub)
}

fn rundll(rundll32: PathBuf, dir: &Path, dll: &str) -> Spec {
    Spec { label: dll.to_string(), exe: rundll32, cwd: dir.to_path_buf(), raw_args: format!("\"{}\", RunDll", dir.join(dll).display()) }
}

/// The processes to run in `dir` on a machine of type `machine`.
///
/// Every architecture needs a hook host of its own, because an in-context hook can only be
/// loaded into processes of the same architecture. On ARM64 Windows the x64 host (for
/// emulated x64 programs) is not started: how to launch an emulated `rundll32` there is
/// unverified, so x64 programs are not covered on ARM64 yet.
pub fn specs(dir: &Path, machine: Machine) -> Vec<Spec> {
    let overlay = Spec { label: OVERLAY.into(), exe: dir.join(OVERLAY), cwd: dir.to_path_buf(), raw_args: String::new() };
    let native = system32("rundll32.exe");
    let wow = windir_dir("SysWOW64").join("rundll32.exe");
    let mut v = vec![overlay];
    match machine {
        Machine::X86 => v.push(rundll(native, dir, HOOK_X86)),
        Machine::X64 => {
            v.push(rundll(wow, dir, HOOK_X86));
            v.push(rundll(native, dir, HOOK_X64));
        }
        Machine::Arm64 => {
            v.push(rundll(wow, dir, HOOK_X86));
            v.push(rundll(native, dir, HOOK_ARM64));
        }
    }
    v
}

/// Broadcasts the stop message: overlays and hook hosts quit when they receive it.
pub fn broadcast_stop() {
    let name = wide(names::MSG_STOP);
    // SAFETY: NUL-terminated name; a broadcast post needs no window of ours.
    unsafe {
        let msg = RegisterWindowMessageW(PCWSTR(name.as_ptr()));
        if msg != 0 {
            let _ = PostMessageW(Some(HWND_BROADCAST), msg, WPARAM(0), LPARAM(0));
        }
    }
}

struct Job(HANDLE);

// SAFETY: a job handle can be used from any thread.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    fn new() -> windows::core::Result<Job> {
        // SAFETY: standard job creation; the limit structure is fully initialised.
        unsafe {
            let h = CreateJobObjectW(None, PCWSTR::null())?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            Ok(Job(h))
        }
    }

    fn assign(&self, child: &Child) {
        // SAFETY: both handles are valid; a failure just means the child is not in the job.
        unsafe {
            let _ = AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle()));
        }
    }

    fn terminate(&self) {
        // SAFETY: valid job handle.
        unsafe {
            let _ = TerminateJobObject(self.0, 1);
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is ours; closing it kills any children still in the job.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Restart policy: a child that keeps dying right after it started is given up on.
pub struct Backoff {
    window: Duration,
    max_in_window: usize,
    starts: Vec<Instant>,
}

impl Backoff {
    pub fn new(window: Duration, max_in_window: usize) -> Backoff {
        Backoff { window, max_in_window, starts: Vec::new() }
    }

    /// Records a start at `now`; false means "too many recent starts: stop restarting".
    pub fn allow(&mut self, now: Instant) -> bool {
        self.starts.retain(|t| now.duration_since(*t) < self.window);
        if self.starts.len() >= self.max_in_window {
            return false;
        }
        self.starts.push(now);
        true
    }
}

struct Shared {
    stopping: AtomicBool,
    job: Job,
}

pub struct Supervisor {
    shared: Arc<Shared>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Supervisor {
    /// Asks stale hosts and overlays to quit, then starts every spec in its own watcher
    /// thread. A spec whose file is missing is reported in the returned list and skipped.
    pub fn start(specs: Vec<Spec>) -> windows::core::Result<(Supervisor, Vec<String>)> {
        broadcast_stop();
        thread::sleep(Duration::from_millis(300));
        let shared = Arc::new(Shared { stopping: AtomicBool::new(false), job: Job::new()? });
        let mut problems = Vec::new();
        let mut threads = Vec::new();
        for spec in specs {
            if !spec.exe.exists() {
                problems.push(format!("{} is missing", spec.exe.display()));
                continue;
            }
            let shared = Arc::clone(&shared);
            threads.push(thread::spawn(move || watch(&shared, &spec)));
        }
        Ok((Supervisor { shared, threads: Mutex::new(threads) }, problems))
    }

    /// Stops everything: marks the children as expected to exit, asks them to quit, and
    /// terminates whatever is left.
    pub fn stop(&self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        broadcast_stop();
        thread::sleep(Duration::from_millis(250));
        self.shared.job.terminate();
        let threads = std::mem::take(&mut *self.threads.lock().unwrap());
        for t in threads {
            let _ = t.join();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
    }
}

fn spawn(spec: &Spec, shared: &Shared) -> std::io::Result<Child> {
    let mut cmd = Command::new(&spec.exe);
    cmd.raw_arg(&spec.raw_args).creation_flags(CREATE_NO_WINDOW);
    cmd.current_dir(&spec.cwd);
    let child = cmd.spawn()?;
    shared.job.assign(&child);
    Ok(child)
}

/// Keeps one child running until the supervisor stops.
fn watch(shared: &Shared, spec: &Spec) {
    let mut backoff = Backoff::new(Duration::from_secs(30), 8);
    while !shared.stopping.load(Ordering::SeqCst) {
        if !backoff.allow(Instant::now()) {
            log(&format!("{} keeps exiting; giving up on restarting it", spec.label));
            return;
        }
        match spawn(spec, shared) {
            Ok(mut child) => {
                log(&format!("Process {} started=true", spec.label));
                let status = child.wait();
                if shared.stopping.load(Ordering::SeqCst) {
                    return;
                }
                log(&format!("Process {} exited ({status:?}); restarting", spec.label));
            }
            Err(e) => {
                log(&format!("Process {} could not start: {e}", spec.label));
                return;
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x64_machine_runs_a_32_and_a_64_bit_host_plus_the_overlay() {
        let s = specs(Path::new(r"D:\wg"), Machine::X64);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].exe, PathBuf::from(r"D:\wg\WindowGridOverlay.exe"));
        assert!(s[1].exe.to_string_lossy().to_lowercase().contains("syswow64"));
        assert_eq!(s[1].raw_args, r#""D:\wg\WindowGrid32.dll", RunDll"#);
        assert!(s[2].exe.to_string_lossy().to_lowercase().contains("system32"));
        assert_eq!(s[2].raw_args, r#""D:\wg\WindowGrid64.dll", RunDll"#);
    }

    #[test]
    fn arm64_machine_gets_the_native_host() {
        let s = specs(Path::new(r"D:\wg"), Machine::Arm64);
        assert_eq!(s.len(), 3);
        assert!(s.iter().any(|x| x.raw_args.contains("WindowGridARM64.dll")));
        assert!(!s.iter().any(|x| x.raw_args.contains("WindowGrid64.dll")), "x64 host is not started on ARM64");
    }

    #[test]
    fn a_32_bit_os_only_needs_the_32_bit_host() {
        let s = specs(Path::new(r"D:\wg"), Machine::X86);
        assert_eq!(s.len(), 2);
        assert!(s[1].raw_args.contains("WindowGrid32.dll"));
    }

    #[test]
    fn backoff_gives_up_on_a_crash_loop_but_not_on_slow_restarts() {
        let t0 = Instant::now();
        let mut b = Backoff::new(Duration::from_secs(30), 3);
        assert!(b.allow(t0) && b.allow(t0 + Duration::from_secs(1)) && b.allow(t0 + Duration::from_secs(2)));
        assert!(!b.allow(t0 + Duration::from_secs(3)));
        // Once the old starts age out of the window it may restart again.
        assert!(b.allow(t0 + Duration::from_secs(40)));
    }

    #[test]
    fn this_machine_has_a_known_architecture() {
        let m = native_machine();
        assert!(matches!(m, Machine::X64 | Machine::Arm64 | Machine::X86));
    }
}
