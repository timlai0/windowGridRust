//! Command line parsing, compatible with the legacy app's `Arguments` class.
//!
//! A switch starts with `-`, `--` or `/`. Its value can follow as `=value`, `:value`, or as
//! the next argument if that is not itself a switch. Names are case-insensitive. A switch
//! without a value is still *present* (its value is the empty string), which is how
//! `/install` alone means "install to the default directory".

use std::path::PathBuf;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    entries: Vec<(String, String)>,
}

fn strip_quotes(v: &str) -> &str {
    let v = v.trim();
    let v = v.strip_prefix(['"', '\'']).unwrap_or(v);
    v.strip_suffix(['"', '\'']).unwrap_or(v)
}

/// Splits `arg` into `(name, value)` if it is a switch.
fn as_switch(arg: &str) -> Option<(&str, &str)> {
    let rest = arg.strip_prefix("--").or_else(|| arg.strip_prefix('-')).or_else(|| arg.strip_prefix('/'))?;
    if rest.is_empty() {
        return None;
    }
    match rest.find(['=', ':']) {
        Some(0) => None, // the legacy pattern needs at least one name character
        Some(i) => Some((&rest[..i], &rest[i + 1..])),
        None => Some((rest, "")),
    }
}

impl Args {
    pub fn parse<I, S>(args: I) -> Args
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out = Args::default();
        let mut pending: Option<String> = None;
        for arg in args {
            let arg = arg.as_ref();
            let switch = as_switch(arg);
            if let Some(name) = pending.take() {
                // The previous switch had no inline value: this argument is it, unless it is
                // a switch itself.
                out.add(&name, if switch.is_some() { "" } else { strip_quotes(arg) });
            }
            if let Some((name, value)) = switch {
                let value = strip_quotes(value);
                if value.is_empty() {
                    pending = Some(name.to_string());
                } else {
                    out.add(name, value);
                }
            }
        }
        if let Some(name) = pending {
            out.add(&name, "");
        }
        out
    }

    fn add(&mut self, name: &str, value: &str) {
        let key = name.to_lowercase();
        // First occurrence wins, like the legacy dictionary.
        if !self.entries.iter().any(|(k, _)| *k == key) {
            self.entries.push((key, value.to_string()));
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn has_any(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.has(n))
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        let key = name.to_lowercase();
        self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
    }
}

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Normal start: run in the tray.
    Run,
    Help,
    Install { dir: Option<PathBuf>, silent: bool, shortcut: bool, autostart: bool },
    /// Replace the running installation with the build at `path` and start it.
    Replace { path: PathBuf },
}

pub const USAGE: &str = "/install [path] [/silent|/s] [/shortcut] [/onstartup]\n/replace [path]";

impl Command {
    pub fn from_args(a: &Args) -> Command {
        if a.has_any(&["h", "help", "?"]) {
            return Command::Help;
        }
        if let Some(dir) = a.get("install") {
            return Command::Install {
                dir: (!dir.trim().is_empty()).then(|| PathBuf::from(dir)),
                silent: a.has_any(&["silent", "s"]),
                shortcut: a.has("shortcut"),
                autostart: a.has("onstartup"),
            };
        }
        if let Some(path) = a.get("replace") {
            return Command::Replace { path: PathBuf::from(path) };
        }
        Command::Run
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(args: &[&str]) -> Command {
        Command::from_args(&Args::parse(args.iter().copied()))
    }

    #[test]
    fn plain_start_runs() {
        assert_eq!(cmd(&[]), Command::Run);
        assert_eq!(cmd(&["something", "else"]), Command::Run);
    }

    #[test]
    fn help_in_all_spellings() {
        for h in ["/h", "/help", "/?", "-h", "--help", "-?", "/HELP"] {
            assert_eq!(cmd(&[h]), Command::Help, "{h}");
        }
    }

    #[test]
    fn install_forms() {
        assert_eq!(
            cmd(&["/install"]),
            Command::Install { dir: None, silent: false, shortcut: false, autostart: false }
        );
        assert_eq!(
            cmd(&["/install", r"C:\Program Files\WindowGridRust", "/s", "/shortcut", "/onstartup"]),
            Command::Install {
                dir: Some(PathBuf::from(r"C:\Program Files\WindowGridRust")),
                silent: true,
                shortcut: true,
                autostart: true
            }
        );
        assert_eq!(
            cmd(&[r"/install:C:\x y", "/silent"]),
            Command::Install { dir: Some(PathBuf::from(r"C:\x y")), silent: true, shortcut: false, autostart: false }
        );
        assert_eq!(
            cmd(&["--install=\"D:\\wg\""]),
            Command::Install { dir: Some(PathBuf::from(r"D:\wg")), silent: false, shortcut: false, autostart: false }
        );
    }

    #[test]
    fn install_followed_by_a_switch_has_no_directory() {
        assert_eq!(
            cmd(&["/install", "/silent"]),
            Command::Install { dir: None, silent: true, shortcut: false, autostart: false }
        );
    }

    #[test]
    fn replace_takes_a_path() {
        assert_eq!(cmd(&["/replace", r"C:\new\WindowGridRust.exe"]), Command::Replace { path: PathBuf::from(r"C:\new\WindowGridRust.exe") });
    }

    #[test]
    fn help_wins_over_install_and_install_over_replace() {
        assert_eq!(cmd(&["/install", "/help"]), Command::Help);
        assert!(matches!(cmd(&["/replace", "x", "/install"]), Command::Install { .. }));
    }

    #[test]
    fn names_are_case_insensitive_and_first_value_wins() {
        let a = Args::parse(["/Install", "A", "/INSTALL", "B"]);
        assert_eq!(a.get("install"), Some("A"));
        assert!(a.has("InStAlL"));
    }

    #[test]
    fn bare_dash_and_colon_are_not_switches() {
        let a = Args::parse(["-", "/", "/:x"]);
        assert!(!a.has("") && !a.has("x"));
    }
}
