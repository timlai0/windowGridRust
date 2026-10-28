//! Starting at logon: a value in the `Run` key (starts the app) plus a scheduled task that
//! runs it with the highest available privileges (so the hooks can reach elevated windows),
//! as the legacy installer set up. Both are keyed by the product name.

use crate::winops::{reg_delete_value, reg_get_string, reg_set_string, run_tool, system32};
use std::path::Path;
use std::time::Duration;
use wg_win::logger::log;

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Escapes text for an XML element or attribute value.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The scheduled task definition: at logon, highest available run level, one instance,
/// no time limit, runs on battery.
pub fn task_xml(name: &str, exe: &Path) -> String {
    let exe = xml_escape(&exe.display().to_string());
    let name = xml_escape(name);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\r\n\
<Task version=\"1.3\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\r\n\
  <RegistrationInfo>\r\n    <Author>{name}</Author>\r\n    <URI>\\{name}</URI>\r\n  </RegistrationInfo>\r\n\
  <Triggers>\r\n    <LogonTrigger>\r\n      <Enabled>true</Enabled>\r\n    </LogonTrigger>\r\n  </Triggers>\r\n\
  <Principals>\r\n    <Principal>\r\n      <RunLevel>HighestAvailable</RunLevel>\r\n    </Principal>\r\n  </Principals>\r\n\
  <Settings>\r\n\
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\r\n\
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\r\n\
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\r\n\
    <AllowHardTerminate>true</AllowHardTerminate>\r\n\
    <StartWhenAvailable>false</StartWhenAvailable>\r\n\
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>\r\n\
    <IdleSettings>\r\n      <StopOnIdleEnd>true</StopOnIdleEnd>\r\n      <RestartOnIdle>false</RestartOnIdle>\r\n    </IdleSettings>\r\n\
    <AllowStartOnDemand>true</AllowStartOnDemand>\r\n\
    <Enabled>true</Enabled>\r\n\
    <Hidden>false</Hidden>\r\n\
    <RunOnlyIfIdle>false</RunOnlyIfIdle>\r\n\
    <WakeToRun>false</WakeToRun>\r\n\
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\r\n\
    <Priority>7</Priority>\r\n\
  </Settings>\r\n\
  <Actions>\r\n    <Exec>\r\n      <Command>{exe}</Command>\r\n    </Exec>\r\n  </Actions>\r\n\
</Task>\r\n"
    )
}

/// UTF-16 little endian with a byte-order mark, as `schtasks /XML` expects.
pub fn utf16le_with_bom(s: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

fn scheduler(args: &[&str]) -> bool {
    log(&format!("schtasks {}", args.join(" ")));
    let code = run_tool(&system32("schtasks.exe"), args, Duration::from_secs(10));
    log(&format!("schtasks finished: {code:?}"));
    matches!(code, Some(c) if c >= 0)
}

/// Enables or disables starting at logon. The task is only touched if the `Run` value
/// succeeded, like the original installer.
pub fn set(name: &str, exe: &Path, enable: bool) -> bool {
    let run_ok = if enable {
        reg_set_string(RUN_KEY, name, &exe.display().to_string())
    } else {
        reg_delete_value(RUN_KEY, name)
    };
    log(&format!("Autostart Run value {name} -> {enable} ({run_ok})"));
    if !run_ok || name.contains('"') || exe.to_string_lossy().contains('"') {
        return false;
    }
    if !enable {
        return scheduler(&["/DELETE", "/TN", name, "/F"]);
    }
    let dir = std::env::temp_dir();
    let xml = dir.join(format!("{name}-autostart-{}.xml", std::process::id()));
    if std::fs::write(&xml, utf16le_with_bom(&task_xml(name, exe))).is_err() {
        return false;
    }
    let ok = scheduler(&["/CREATE", "/TN", name, "/XML", &xml.display().to_string(), "/F"]);
    let _ = std::fs::remove_file(&xml);
    ok
}

/// True if the `Run` value points at `exe`.
pub fn is_enabled(name: &str, exe: &Path) -> bool {
    reg_get_string(RUN_KEY, name).is_some_and(|v| v == exe.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_xml_runs_the_exe_at_logon_with_highest_privileges() {
        let xml = task_xml("WindowGridRust", Path::new(r"C:\Program Files\WindowGridRust\WindowGridRust.exe"));
        assert!(xml.contains("<Command>C:\\Program Files\\WindowGridRust\\WindowGridRust.exe</Command>"));
        assert!(xml.contains("<LogonTrigger>") && xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<URI>\\WindowGridRust</URI>"));
        assert!(!xml.to_lowercase().contains("windowgrid.net"), "no third-party attribution");
    }

    #[test]
    fn xml_special_characters_are_escaped() {
        let xml = task_xml("A&B", Path::new(r"C:\x<y>\a&b.exe"));
        assert!(xml.contains("<Author>A&amp;B</Author>"));
        assert!(xml.contains("<Command>C:\\x&lt;y&gt;\\a&amp;b.exe</Command>"));
    }

    #[test]
    fn schtasks_wants_utf16_with_a_bom() {
        let b = utf16le_with_bom("<a>é");
        assert_eq!(&b[..2], &[0xFF, 0xFE]);
        assert_eq!(&b[2..4], &[b'<', 0]);
        assert_eq!(&b[b.len() - 2..], &[0xE9, 0x00]);
    }

    #[test]
    fn run_key_value_round_trip_in_a_scratch_key() {
        // Uses a throwaway subkey; the real Run key is never touched by tests.
        let key = format!(r"Software\WindowGridRustTest-{}-autostart", std::process::id());
        let (key, name) = (key.as_str(), "t");
        assert!(reg_set_string(key, name, r"C:\a b\c.exe"));
        assert_eq!(reg_get_string(key, name).as_deref(), Some(r"C:\a b\c.exe"));
        assert!(reg_delete_value(key, name));
        assert_eq!(reg_get_string(key, name), None);
        assert!(reg_delete_value(key, name), "deleting a missing value is fine");
        assert!(crate::winops::reg_delete_tree(key));
    }
}
