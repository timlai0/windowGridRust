//! Real Windows operations used by the installer and the OS options.
//! System binaries are always started by absolute path under `%WINDIR%\System32`.

use crate::installer::{self, CopyResult, InstallPorts};
use std::ffi::c_void;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RRF_RT_REG_DWORD,
    RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, IShellLinkW, SHGetKnownFolderPath, ShellExecuteExW, ShellLink, KF_FLAG_DEFAULT,
    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use wg_win::logger::log;
use wg_win::wide;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn system32(file: &str) -> PathBuf {
    let win = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    win.join("System32").join(file)
}

// ---- registry (HKEY_CURRENT_USER) -----------------------------------------------------

pub fn reg_set_string(subkey: &str, name: &str, value: &str) -> bool {
    let (k, n, v) = (wide(subkey), wide(name), wide(value));
    // SAFETY: NUL-terminated wide strings; the byte length covers the value including its NUL.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(k.as_ptr()),
            PCWSTR(n.as_ptr()),
            REG_SZ.0,
            Some(v.as_ptr().cast::<c_void>()),
            (v.len() * 2) as u32,
        )
        .is_ok()
    }
}

pub fn reg_set_dword(subkey: &str, name: &str, value: u32) -> bool {
    let (k, n) = (wide(subkey), wide(name));
    // SAFETY: valid pointers; a DWORD is 4 bytes.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(k.as_ptr()),
            PCWSTR(n.as_ptr()),
            REG_DWORD.0,
            Some((&value as *const u32).cast::<c_void>()),
            4,
        )
        .is_ok()
    }
}

/// Deleting a value that does not exist counts as success.
pub fn reg_delete_value(subkey: &str, name: &str) -> bool {
    let (k, n) = (wide(subkey), wide(name));
    // SAFETY: NUL-terminated wide strings.
    let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr())) };
    r.is_ok() || r.0 == 2 || r.0 == 3 // ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND
}

/// Deletes a whole key below `HKEY_CURRENT_USER` (used by tests to clean up their scratch keys).
pub fn reg_delete_tree(subkey: &str) -> bool {
    let k = wide(subkey);
    // SAFETY: NUL-terminated wide string.
    let r = unsafe { windows::Win32::System::Registry::RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr())) };
    r.is_ok() || r.0 == 2
}

pub fn reg_get_string(subkey: &str, name: &str) -> Option<String> {
    let (k, n) = (wide(subkey), wide(name));
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its byte size describe the same allocation.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(k.as_ptr()),
            PCWSTR(n.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast::<c_void>()),
            Some(&mut size),
        )
    };
    r.is_ok().then(|| {
        let chars = (size as usize / 2).saturating_sub(1);
        String::from_utf16_lossy(&buf[..chars.min(buf.len())])
    })
}

pub fn reg_get_dword(subkey: &str, name: &str) -> Option<u32> {
    let (k, n) = (wide(subkey), wide(name));
    let mut value = 0u32;
    let mut size = 4u32;
    // SAFETY: valid out pointers for a DWORD.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(k.as_ptr()),
            PCWSTR(n.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast::<c_void>()),
            Some(&mut size),
        )
    };
    r.is_ok().then_some(value)
}

// ---- shortcuts ---------------------------------------------------------------------------

/// Creates (or overwrites) a `.lnk` file. `link` gets the extension if it lacks one.
pub fn create_shortcut(target: &Path, link: &Path, description: &str) -> windows::core::Result<()> {
    let link = if link.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
        link.to_path_buf()
    } else {
        link.with_extension("lnk")
    };
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir).map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x8007_0003u32 as i32), e.to_string()))?;
    }
    let (t, d, w, l) = (
        wide(&target.display().to_string()),
        wide(description),
        wide(&target.parent().map(|p| p.display().to_string()).unwrap_or_default()),
        wide(&link.display().to_string()),
    );
    // SAFETY: COM is initialised for this thread (a repeat or a different mode is tolerated);
    // all strings are NUL-terminated and outlive the calls.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        shell.SetPath(PCWSTR(t.as_ptr()))?;
        shell.SetDescription(PCWSTR(d.as_ptr()))?;
        shell.SetWorkingDirectory(PCWSTR(w.as_ptr()))?;
        let file: IPersistFile = shell.cast()?;
        file.Save(PCWSTR(l.as_ptr()), true)?;
    }
    Ok(())
}

/// The target a shortcut points to (used by the tests).
pub fn shortcut_target(link: &Path) -> windows::core::Result<PathBuf> {
    let l = wide(&link.display().to_string());
    // SAFETY: as in `create_shortcut`; the buffer is large enough for MAX_PATH.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let file: IPersistFile = shell.cast()?;
        file.Load(PCWSTR(l.as_ptr()), windows::Win32::System::Com::STGM(0))?;
        let mut buf = [0u16; 520];
        shell.GetPath(&mut buf, std::ptr::null_mut(), 0)?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(PathBuf::from(String::from_utf16_lossy(&buf[..n])))
    }
}

/// `%ProgramData%\Microsoft\Windows\Start Menu\Programs` (all users).
pub fn common_programs_dir() -> Option<PathBuf> {
    // SAFETY: the returned buffer is copied and then freed with CoTaskMemFree.
    unsafe {
        let p: PWSTR = SHGetKnownFolderPath(&FOLDERID_CommonPrograms, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const c_void));
        s.map(PathBuf::from)
    }
}

// ---- processes -------------------------------------------------------------------------------

/// Starts `exe` elevated (UAC prompt) with `args`. True if the request was accepted.
pub fn run_elevated(exe: &Path, args: &str) -> bool {
    let (file, params, verb) = (wide(&exe.display().to_string()), wide(args), wide("runas"));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: HWND::default(),
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is fully initialised and the strings outlive the call.
    unsafe { ShellExecuteExW(&mut info).is_ok() }
}

/// Starts a program detached, in its own directory.
pub fn launch(exe: &Path) -> bool {
    let mut cmd = Command::new(exe);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

/// Runs a console tool by absolute path without a window; kills it after `timeout`.
/// Returns its exit code, or `None` if it could not run or timed out.
pub fn run_tool(exe: &Path, args: &[&str], timeout: Duration) -> Option<i32> {
    let mut child =
        Command::new(exe).args(args).creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
    let end = Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return status.code();
        }
        if Instant::now() > end {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Lets all users read and run what is in `dir` (`BUILTIN\Users`: read and execute,
/// inherited), like the legacy installer did through the .NET ACL API.
pub fn set_directory_security(dir: &Path) -> io::Result<()> {
    let arg = dir.display().to_string();
    match run_tool(&system32("icacls.exe"), &[&arg, "/grant", "*S-1-5-32-545:(OI)(CI)RX"], Duration::from_secs(20)) {
        Some(0) => Ok(()),
        other => Err(io::Error::other(format!("icacls failed ({other:?})"))),
    }
}

// ---- production ports ---------------------------------------------------------------------------

/// The real implementation of [`InstallPorts`].
pub struct RealPorts {
    /// This running program (what gets relaunched elevated).
    pub exe: PathBuf,
    /// The settings in memory, written to the installation directory.
    pub settings: wg_core::settings::Settings,
    /// Where the shortcut goes; `None` = the all-users Start Menu.
    pub shortcut_dir: Option<PathBuf>,
}

impl InstallPorts for RealPorts {
    fn can_write(&mut self, dir: &Path) -> bool {
        installer::can_write(dir)
    }

    fn elevate(&mut self, args: &str) -> bool {
        run_elevated(&self.exe, args)
    }

    fn create_dir(&mut self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    fn secure(&mut self, dir: &Path) -> io::Result<()> {
        set_directory_security(dir)
    }

    fn copy(&mut self, src: &Path, dst: &Path) -> io::Result<CopyResult> {
        installer::copy_file(src, dst)
    }

    fn save_settings(&mut self, dir: &Path, adopt_existing: bool) -> io::Result<()> {
        let path = crate::config::Config::path_in(dir);
        if adopt_existing && path.exists() {
            return Ok(()); // keep the settings of the earlier installation
        }
        crate::config::Config { settings: self.settings.clone(), path }.save()
    }

    fn shortcut(&mut self, target: &Path) -> bool {
        let dir = self.shortcut_dir.clone().or_else(common_programs_dir);
        let Some(dir) = dir else { return false };
        let link = dir.join(format!("{}.lnk", installer::PRODUCT));
        log(&format!("Adding start menu shortcut: \"{}\"", target.display()));
        match create_shortcut(target, &link, "WindowGridRust Window Management Tool") {
            Ok(()) => true,
            Err(e) => {
                log(&format!("Unable to create shortcut \"{}\": {e}", link.display()));
                false
            }
        }
    }

    fn autostart(&mut self, target: &Path) -> bool {
        crate::autostart::set(installer::PRODUCT, target, true)
    }

    fn launch(&mut self, target: &Path) -> bool {
        launch(target)
    }

    fn log(&mut self, msg: &str) {
        log(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_is_created_and_points_at_the_target() {
        let dir = std::env::temp_dir().join(format!("wg-shortcut-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let target = std::env::current_exe().unwrap();
        // Missing extension is added; missing directory is created.
        let link = dir.join("sub").join("WindowGridRust Test");
        create_shortcut(&target, &link, "A test shortcut").unwrap();
        let file = dir.join("sub").join("WindowGridRust Test.lnk");
        assert!(file.exists());
        let resolved = shortcut_target(&file).unwrap();
        assert!(same_path(&resolved, &target), "{resolved:?} vs {target:?}");
        // Overwriting works.
        create_shortcut(&target, &file, "Again").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn same_path(a: &Path, b: &Path) -> bool {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    #[test]
    fn system32_paths_are_absolute() {
        let p = system32("schtasks.exe");
        assert!(p.is_absolute() && p.exists(), "{p:?}");
    }

    #[test]
    fn registry_round_trip_in_a_unique_scratch_key() {
        let key = format!(r"Software\WindowGridRustTest-{}-winops", std::process::id());
        assert!(reg_set_string(&key, "s", "v"));
        assert!(reg_set_dword(&key, "d", 7));
        assert_eq!(reg_get_string(&key, "s").as_deref(), Some("v"));
        assert_eq!(reg_get_dword(&key, "d"), Some(7));
        assert_eq!(reg_get_dword(&key, "missing"), None);
        assert!(reg_delete_tree(&key));
        assert_eq!(reg_get_string(&key, "s"), None);
    }
}
