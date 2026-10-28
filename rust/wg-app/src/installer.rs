//! Installing and replacing: the sequencing of the legacy installer, written against
//! [`InstallPorts`] so it can be tested without touching the system. The real Windows
//! operations live in [`crate::winops`].

use std::io;
use std::path::{Path, PathBuf};

pub const PRODUCT: &str = "WindowGridRust";

/// `%ProgramFiles%\WindowGridRust`.
pub fn default_dir() -> PathBuf {
    let base = std::env::var_os("ProgramFiles").map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
    base.join(PRODUCT)
}

/// True if both paths name the same existing file.
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Whether the current user can create files in `dir` (or in the nearest directory that
/// exists above it, if `dir` does not exist yet).
pub fn can_write(dir: &Path) -> bool {
    let mut probe = Some(dir);
    while let Some(p) = probe {
        if p.is_dir() {
            let file = p.join(format!(".wg-write-test-{}", std::process::id()));
            let ok = std::fs::File::create(&file).is_ok();
            let _ = std::fs::remove_file(&file);
            return ok;
        }
        probe = p.parent();
    }
    false
}

#[derive(Debug, PartialEq, Eq)]
pub enum CopyResult {
    /// Source and destination are the same file: nothing was copied.
    SameFile,
    Copied,
}

/// Copies `src` over `dst`, clearing a read-only flag on `dst` first.
pub fn copy_file(src: &Path, dst: &Path) -> io::Result<CopyResult> {
    if same_file(src, dst) {
        return Ok(CopyResult::SameFile);
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Ok(meta) = std::fs::metadata(dst) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(dst, perms);
        }
    }
    std::fs::copy(src, dst)?;
    Ok(CopyResult::Copied)
}

/// The command line that makes an elevated copy repeat this installation silently.
pub fn elevated_args(dir: &Path, shortcut: bool, autostart: bool) -> String {
    let mut s = format!("/install \"{}\" /s", dir.display().to_string().trim_end_matches('\\'));
    if shortcut {
        s.push_str(" /shortcut");
    }
    if autostart {
        s.push_str(" /onstartup");
    }
    s
}

/// Everything installing does to the outside world.
pub trait InstallPorts {
    fn can_write(&mut self, dir: &Path) -> bool;
    /// Relaunches this program elevated with `args`; true if the request was accepted.
    fn elevate(&mut self, args: &str) -> bool;
    fn create_dir(&mut self, dir: &Path) -> io::Result<()>;
    /// Lets all users read and run what is in `dir`.
    fn secure(&mut self, dir: &Path) -> io::Result<()>;
    fn copy(&mut self, src: &Path, dst: &Path) -> io::Result<CopyResult>;
    /// Writes the settings into `dir`. After a fresh copy the settings already in `dir`
    /// (from an earlier installation) are loaded first so they are not overwritten.
    fn save_settings(&mut self, dir: &Path, adopt_existing: bool) -> io::Result<()>;
    fn shortcut(&mut self, target: &Path) -> bool;
    fn autostart(&mut self, target: &Path) -> bool;
    fn launch(&mut self, target: &Path) -> bool;
    fn log(&mut self, msg: &str);
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The directory needs administrator rights; an elevated copy was started (true) or
    /// could not be (false). Nothing was installed by this process.
    Elevated(bool),
    /// Installed. `restarted` means the installed copy was launched and this process
    /// should exit; false when this already is the installed copy.
    Installed { target: PathBuf, restarted: bool },
}

pub fn install(
    ports: &mut dyn InstallPorts,
    exe: &Path,
    dir: Option<PathBuf>,
    shortcut: bool,
    autostart: bool,
) -> io::Result<Outcome> {
    let dir = dir.filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(default_dir);
    if !ports.can_write(&dir) {
        ports.log(&format!("Current user cannot write to \"{}\"", dir.display()));
        let ok = ports.elevate(&elevated_args(&dir, shortcut, autostart));
        ports.log(if ok { "Running as administrator" } else { "Install failed: requires elevation" });
        return Ok(Outcome::Elevated(ok));
    }
    ports.log(&format!("Installing \"{}\" -> \"{}\"", exe.display(), dir.display()));
    ports.create_dir(&dir)?;
    if let Err(e) = ports.secure(&dir) {
        ports.log(&format!("Could not set permissions on \"{}\": {e}", dir.display()));
    }
    let target = dir.join(exe.file_name().unwrap_or_else(|| "WindowGridRust.exe".as_ref()));
    let copied = ports.copy(exe, &target)? == CopyResult::Copied;
    ports.save_settings(&dir, copied)?;
    if shortcut && !ports.shortcut(&target) {
        ports.log("Could not create the Start Menu shortcut");
    }
    if autostart && !ports.autostart(&target) {
        ports.log("Could not set up starting at logon");
    }
    let restarted = copied && ports.launch(&target);
    Ok(Outcome::Installed { target, restarted })
}

/// `/replace path`: installs this executable at `path` and starts it there.
/// Returns true if this process should exit because the new copy was started.
pub fn replace(ports: &mut dyn InstallPorts, exe: &Path, path: &Path) -> io::Result<bool> {
    if ports.copy(exe, path)? == CopyResult::Copied {
        return Ok(ports.launch(path));
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wg-app-inst-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[derive(Default)]
    struct Fake {
        writable: bool,
        elevate_ok: bool,
        copy_result: Option<CopyResult>,
        calls: Vec<String>,
    }

    impl InstallPorts for Fake {
        fn can_write(&mut self, _: &Path) -> bool {
            self.writable
        }
        fn elevate(&mut self, args: &str) -> bool {
            self.calls.push(format!("elevate {args}"));
            self.elevate_ok
        }
        fn create_dir(&mut self, d: &Path) -> io::Result<()> {
            self.calls.push(format!("mkdir {}", d.display()));
            Ok(())
        }
        fn secure(&mut self, _: &Path) -> io::Result<()> {
            self.calls.push("secure".into());
            Ok(())
        }
        fn copy(&mut self, s: &Path, d: &Path) -> io::Result<CopyResult> {
            self.calls.push(format!("copy {} -> {}", s.display(), d.display()));
            Ok(match self.copy_result.take() {
                Some(CopyResult::SameFile) => CopyResult::SameFile,
                _ => CopyResult::Copied,
            })
        }
        fn save_settings(&mut self, d: &Path, adopt: bool) -> io::Result<()> {
            self.calls.push(format!("save {} adopt={adopt}", d.display()));
            Ok(())
        }
        fn shortcut(&mut self, _: &Path) -> bool {
            self.calls.push("shortcut".into());
            true
        }
        fn autostart(&mut self, _: &Path) -> bool {
            self.calls.push("autostart".into());
            true
        }
        fn launch(&mut self, t: &Path) -> bool {
            self.calls.push(format!("launch {}", t.display()));
            true
        }
        fn log(&mut self, _: &str) {}
    }

    #[test]
    fn elevation_command_line() {
        assert_eq!(elevated_args(Path::new(r"C:\Program Files\WindowGridRust\"), true, true), r#"/install "C:\Program Files\WindowGridRust" /s /shortcut /onstartup"#);
        assert_eq!(elevated_args(Path::new(r"D:\wg"), false, false), r#"/install "D:\wg" /s"#);
    }

    #[test]
    fn unwritable_directory_requests_elevation_and_installs_nothing() {
        let mut f = Fake { writable: false, elevate_ok: true, ..Fake::default() };
        let out = install(&mut f, Path::new(r"C:\x\WindowGridRust.exe"), Some(PathBuf::from(r"C:\locked")), true, false).unwrap();
        assert_eq!(out, Outcome::Elevated(true));
        assert_eq!(f.calls, [r#"elevate /install "C:\locked" /s /shortcut"#]);
        let mut f = Fake { writable: false, elevate_ok: false, ..Fake::default() };
        assert_eq!(install(&mut f, Path::new("a.exe"), None, false, false).unwrap(), Outcome::Elevated(false));
    }

    #[test]
    fn fresh_install_copies_saves_integrates_and_restarts() {
        let mut f = Fake { writable: true, ..Fake::default() };
        let out = install(&mut f, Path::new(r"C:\dl\WindowGridRust.exe"), Some(PathBuf::from(r"D:\wg")), true, true).unwrap();
        assert_eq!(out, Outcome::Installed { target: PathBuf::from(r"D:\wg\WindowGridRust.exe"), restarted: true });
        assert_eq!(
            f.calls,
            [
                r"mkdir D:\wg",
                "secure",
                r"copy C:\dl\WindowGridRust.exe -> D:\wg\WindowGridRust.exe",
                r"save D:\wg adopt=true",
                "shortcut",
                "autostart",
                r"launch D:\wg\WindowGridRust.exe",
            ]
        );
    }

    #[test]
    fn running_from_the_install_directory_does_not_restart() {
        let mut f = Fake { writable: true, copy_result: Some(CopyResult::SameFile), ..Fake::default() };
        let out = install(&mut f, Path::new(r"D:\wg\WindowGridRust.exe"), Some(PathBuf::from(r"D:\wg")), false, true).unwrap();
        assert_eq!(out, Outcome::Installed { target: PathBuf::from(r"D:\wg\WindowGridRust.exe"), restarted: false });
        assert!(f.calls.contains(&r"save D:\wg adopt=false".to_string()));
        assert!(f.calls.contains(&"autostart".to_string()));
        assert!(!f.calls.iter().any(|c| c.starts_with("launch") || c == "shortcut"));
    }

    #[test]
    fn default_directory_is_used_when_none_is_given() {
        let mut f = Fake { writable: true, ..Fake::default() };
        install(&mut f, Path::new("WindowGridRust.exe"), Some(PathBuf::new()), false, false).unwrap();
        assert!(f.calls[0].starts_with("mkdir ") && f.calls[0].ends_with("WindowGridRust"), "{:?}", f.calls[0]);
    }

    #[test]
    fn replace_launches_only_after_a_real_copy() {
        let mut f = Fake::default();
        assert!(replace(&mut f, Path::new("a.exe"), Path::new("b.exe")).unwrap());
        assert_eq!(f.calls, ["copy a.exe -> b.exe", "launch b.exe"]);
        let mut f = Fake { copy_result: Some(CopyResult::SameFile), ..Fake::default() };
        assert!(!replace(&mut f, Path::new("a.exe"), Path::new("a.exe")).unwrap());
    }

    #[test]
    fn same_file_detection_and_copy_on_the_real_file_system() {
        let d = temp_dir("copy");
        let src = d.join("src.bin");
        std::fs::write(&src, b"one").unwrap();
        assert!(same_file(&src, &d.join(".").join("src.bin")));
        assert!(!same_file(&src, &d.join("missing.bin")));
        assert_eq!(copy_file(&src, &src).unwrap(), CopyResult::SameFile);

        let dst = d.join("sub").join("dst.bin");
        assert_eq!(copy_file(&src, &dst).unwrap(), CopyResult::Copied);
        assert_eq!(std::fs::read(&dst).unwrap(), b"one");

        // A read-only destination is overwritten.
        let mut perms = std::fs::metadata(&dst).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&dst, perms).unwrap();
        std::fs::write(&src, b"two").unwrap();
        assert_eq!(copy_file(&src, &dst).unwrap(), CopyResult::Copied);
        assert_eq!(std::fs::read(&dst).unwrap(), b"two");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn write_access_probe() {
        let d = temp_dir("probe");
        assert!(can_write(&d));
        assert!(can_write(&d.join("does").join("not").join("exist")), "judged by the nearest existing parent");
        assert!(!d.join(format!(".wg-write-test-{}", std::process::id())).exists(), "probe file removed");
        let _ = std::fs::remove_dir_all(d);
    }
}
