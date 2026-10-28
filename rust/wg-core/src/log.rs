//! Formatting for `WindowGridRust.log`, which the app, overlay and hook DLLs all append to.
//!
//! A line looks like
//! `16:36:38.176 [APP] (90016:48856) WindowGrid.exe      : Starting WindowGrid.exe (1.4.0.0)`.
//! The tag is `APP` for the managed side and `X86` / `X64` for the hook DLLs. The file is
//! shared, so writers must hold the [`crate::ipc::LOG_MUTEX`] named mutex (250 ms timeout)
//! while appending; that part lives with the Windows-specific code, not here.

/// Newlines are escaped so every log record stays on one line.
pub fn escape_newlines(msg: &str) -> String {
    msg.replace('\r', "\\r").replace('\n', "\\n")
}

/// ` [TAG] (PPPPP:TTTTT) file(20 wide): ` including the leading space.
pub fn prefix(tag: &str, pid: u32, tid: u32, file: &str) -> String {
    format!(" [{tag}] ({pid:05}:{tid:05}) {file:<20}: ")
}

/// `HH:mm:ss.fff`.
pub fn clock(hour: u32, minute: u32, second: u32, millis: u32) -> String {
    format!("{hour:02}:{minute:02}:{second:02}.{millis:03}")
}

/// A complete prefixed log line (without the trailing newline).
pub fn format_line(clock: &str, tag: &str, pid: u32, tid: u32, file: &str, msg: &str) -> String {
    format!("{clock}{}{}", prefix(tag, pid, tid, file), escape_newlines(msg))
}

/// The three unprefixed lines written when a process opens the log: blank, 80 `=`, blank.
pub fn session_separator() -> [String; 3] {
    [String::new(), "=".repeat(80), String::new()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_real_legacy_line() {
        let line = format_line(
            &clock(16, 36, 38, 176),
            "APP",
            90016,
            48856,
            "WindowGrid.exe",
            "Starting WindowGrid.exe (1.4.0.0)",
        );
        assert_eq!(
            line,
            "16:36:38.176 [APP] (90016:48856) WindowGrid.exe      : Starting WindowGrid.exe (1.4.0.0)"
        );
    }

    #[test]
    fn pads_short_ids_and_escapes_newlines() {
        let line = format_line(&clock(1, 2, 3, 4), "X64", 7, 9, "rundll32.exe", "a\r\nb");
        assert_eq!(line, "01:02:03.004 [X64] (00007:00009) rundll32.exe        : a\\r\\nb");
    }

    #[test]
    fn separator_shape() {
        let s = session_separator();
        assert_eq!(s[1].len(), 80);
        assert!(s[0].is_empty() && s[2].is_empty());
    }
}
