//! Native Windows dialogs and shell helpers: message boxes, the folder picker, and opening
//! files and folders.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use wg_win::wide;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, ShellExecuteW, FOS_FORCEFILESYSTEM,
    FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_TOPMOST, MB_YESNO, SW_SHOWNORMAL,
};

/// Yes/No question; true for Yes.
pub fn ask(title: &str, text: &str) -> bool {
    let (t, c) = (wide(text), wide(title));
    // SAFETY: NUL-terminated strings.
    unsafe { MessageBoxW(None, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_YESNO | MB_ICONQUESTION | MB_TOPMOST) == IDYES }
}

pub fn tell(title: &str, text: &str, error: bool) {
    let (t, c) = (wide(text), wide(title));
    let icon = if error { MB_ICONERROR } else { MB_ICONINFORMATION };
    // SAFETY: NUL-terminated strings.
    unsafe {
        MessageBoxW(None, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OK | icon | MB_TOPMOST);
    }
}

/// Opens a file or folder with its default program.
pub fn open_path(path: &Path) -> bool {
    let (verb, file) = (wide("open"), wide(&path.display().to_string()));
    // SAFETY: NUL-terminated strings; results above 32 mean success.
    unsafe {
        ShellExecuteW(None, PCWSTR(verb.as_ptr()), PCWSTR(file.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL).0
            as usize
            > 32
    }
}

/// Shows the system folder picker, starting near `initial`; `None` if cancelled.
pub fn pick_folder(initial: &Path) -> Option<PathBuf> {
    // SAFETY: standard COM usage; the result string is copied and then freed.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dialog.SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        let title = wide("Select Install Directory");
        let _ = dialog.SetTitle(PCWSTR(title.as_ptr()));
        if let Some(dir) = initial.ancestors().find(|p| p.is_dir()) {
            let w = wide(&dir.display().to_string());
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(w.as_ptr()), None) {
                let _ = dialog.SetFolder(&item);
            }
        }
        dialog.Show(None).ok()?;
        let item = dialog.GetResult().ok()?;
        let name: PWSTR = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const c_void));
        s.map(PathBuf::from)
    }
}
