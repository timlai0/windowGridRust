//! The overlay and hook DLLs that ship inside the executable, and putting them on disk.
//!
//! The hook DLLs are loaded into other programs' processes, so a running installation's DLLs
//! cannot be overwritten (the file is mapped), but they can be renamed. Extraction therefore
//! tries to overwrite, and if that fails renames the old file aside and writes the new one;
//! the renamed leftovers are deleted at a later start.

use std::io;
use std::path::{Path, PathBuf};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/components.rs"));
}

#[derive(Clone, Copy)]
pub struct Component {
    pub file: &'static str,
    pub bytes: &'static [u8],
}

pub const OVERLAY: &str = "WindowGridRustOverlay.exe";
pub const HOOK_X64: &str = "WindowGridRust64.dll";
pub const HOOK_X86: &str = "WindowGridRust32.dll";
pub const HOOK_ARM64: &str = "WindowGridRustARM64.dll";

/// The components compiled into this executable (those that were available at build time).
pub fn embedded() -> Vec<Component> {
    [
        Component { file: OVERLAY, bytes: embedded::OVERLAY },
        Component { file: HOOK_X64, bytes: embedded::HOOK_X64 },
        Component { file: HOOK_X86, bytes: embedded::HOOK_X86 },
        Component { file: HOOK_ARM64, bytes: embedded::HOOK_ARM64 },
    ]
    .into_iter()
    .filter(|c| !c.bytes.is_empty())
    .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Extracted {
    /// The file on disk already had this content.
    Unchanged,
    Written,
    /// The old file was in use: it was renamed aside and the new one written.
    ReplacedInUse,
}

const OLD_MARKER: &str = ".old-";

fn same_content(path: &Path, bytes: &[u8]) -> bool {
    match std::fs::metadata(path) {
        Ok(m) if m.len() == bytes.len() as u64 => std::fs::read(path).is_ok_and(|d| d == bytes),
        _ => false,
    }
}

/// Writes `bytes` to `path` unless it already holds them.
pub fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<Extracted> {
    if same_content(path, bytes) {
        return Ok(Extracted::Unchanged);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    match std::fs::write(path, bytes) {
        Ok(()) => Ok(Extracted::Written),
        Err(first) if path.exists() => {
            // Probably mapped into a running process: move it out of the way.
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for n in 0..50 {
                let aside = path.with_file_name(format!("{name}{OLD_MARKER}{n}"));
                if !aside.exists() && std::fs::rename(path, &aside).is_ok() {
                    std::fs::write(path, bytes)?;
                    return Ok(Extracted::ReplacedInUse);
                }
            }
            Err(first)
        }
        Err(e) => Err(e),
    }
}

/// Extracts every component into `dir`. One result per component; a failure of one does not
/// stop the others.
pub fn extract(dir: &Path, components: &[Component]) -> Vec<(&'static str, io::Result<Extracted>)> {
    components.iter().map(|c| (c.file, write_if_changed(&dir.join(c.file), c.bytes))).collect()
}

/// Deletes files renamed aside by earlier extractions (best effort: those still in use stay).
pub fn remove_leftovers(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(OLD_MARKER))
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

/// Where a component is expected to be: next to the executable.
pub fn path_of(dir: &Path, file: &str) -> PathBuf {
    dir.join(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wg-app-comp-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writes_new_files_and_skips_identical_ones() {
        let d = temp_dir("skip");
        let f = d.join("a.dll");
        assert_eq!(write_if_changed(&f, b"one").unwrap(), Extracted::Written);
        assert_eq!(write_if_changed(&f, b"one").unwrap(), Extracted::Unchanged);
        assert_eq!(write_if_changed(&f, b"two!").unwrap(), Extracted::Written);
        assert_eq!(std::fs::read(&f).unwrap(), b"two!");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_file_in_use_is_renamed_aside_not_failed() {
        let d = temp_dir("inuse");
        let f = d.join("hook.dll");
        std::fs::write(&f, b"old build").unwrap();
        // Like a loaded DLL: readable and renamable by others, but not writable.
        let _held = std::fs::OpenOptions::new().read(true).share_mode(0x1 | 0x4).open(&f).unwrap();
        assert!(std::fs::write(&f, b"x").is_err(), "precondition: overwriting is blocked");
        assert_eq!(write_if_changed(&f, b"new build").unwrap(), Extracted::ReplacedInUse);
        assert_eq!(std::fs::read(&f).unwrap(), b"new build");
        let leftovers = std::fs::read_dir(&d).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains(OLD_MARKER)).count();
        assert_eq!(leftovers, 1);
        drop(_held);
        assert_eq!(remove_leftovers(&d), 1);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn extract_reports_each_component() {
        let d = temp_dir("extract");
        static A: &[u8] = b"AAA";
        static B: &[u8] = b"BB";
        let out = extract(&d, &[Component { file: "a.exe", bytes: A }, Component { file: "b.dll", bytes: B }]);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|(_, r)| matches!(r, Ok(Extracted::Written))));
        let again = extract(&d, &[Component { file: "a.exe", bytes: A }]);
        assert!(matches!(again[0].1, Ok(Extracted::Unchanged)));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn embedded_list_only_contains_available_components() {
        for c in embedded() {
            assert!(!c.bytes.is_empty());
        }
    }
}
