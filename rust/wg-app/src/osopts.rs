//! Two Windows options the settings page toggles directly (they apply immediately, not on
//! Save): "Aero Shake" (shaking a window minimises the others) and "show window contents
//! while dragging", which the hook's live behavior depends on.

use crate::winops::{reg_delete_value, reg_get_dword, reg_set_dword};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETDRAGFULLWINDOWS, SPI_SETDRAGFULLWINDOWS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

/// A per-user registry DWORD that is "on" when it has `on_value` and "off" when absent.
pub struct RegistryFlag {
    pub subkey: &'static str,
    pub name: &'static str,
    pub on_value: u32,
}

impl RegistryFlag {
    pub fn get(&self) -> bool {
        reg_get_dword(self.subkey, self.name) == Some(self.on_value)
    }

    pub fn set(&self, on: bool) -> bool {
        if on {
            reg_set_dword(self.subkey, self.name, self.on_value)
        } else {
            reg_delete_value(self.subkey, self.name)
        }
    }
}

pub const AERO_SHAKE: RegistryFlag = RegistryFlag {
    subkey: r"Software\Policies\Microsoft\Windows\Explorer",
    name: "NoWindowMinimizingShortcuts",
    on_value: 1,
};

pub fn aero_shake_disabled() -> bool {
    AERO_SHAKE.get()
}

pub fn set_aero_shake_disabled(disabled: bool) -> bool {
    AERO_SHAKE.set(disabled)
}

pub fn drag_full_windows() -> bool {
    let mut on: i32 = 0;
    // SAFETY: `on` is a valid BOOL out value for this action.
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETDRAGFULLWINDOWS,
            0,
            Some((&mut on as *mut i32).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    on != 0
}

/// Sets and persists the option and notifies running programs.
pub fn set_drag_full_windows(on: bool) -> bool {
    const SPIF_UPDATEINIFILE_SENDCHANGE: u32 = 0x1 | 0x2;
    // SAFETY: this action takes its value in `uiParam`; no buffer is passed.
    unsafe {
        SystemParametersInfoW(
            SPI_SETDRAGFULLWINDOWS,
            u32::from(on),
            None,
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(SPIF_UPDATEINIFILE_SENDCHANGE),
        )
        .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_flag_on_off_in_a_scratch_key() {
        let key = format!(r"Software\WindowGridRustTest-{}-flag", std::process::id());
        let flag = RegistryFlag { subkey: Box::leak(key.into_boxed_str()), name: "Toggle", on_value: 1 };
        assert!(!flag.get());
        assert!(flag.set(true));
        assert!(flag.get());
        assert!(flag.set(false));
        assert!(!flag.get());
        assert!(flag.set(false), "switching off twice is fine");
        // Another value does not count as "on".
        assert!(reg_set_dword(flag.subkey, flag.name, 5));
        assert!(!flag.get());
        assert!(flag.set(false));
        assert!(crate::winops::reg_delete_tree(flag.subkey));
    }

    #[test]
    fn real_options_can_be_read() {
        // Only reads: tests must not change the machine's settings.
        let _ = aero_shake_disabled();
        let _ = drag_full_windows();
    }
}
