//! The settings file and what is published from it to the hook DLLs and the overlay.

use std::io;
use std::path::{Path, PathBuf};
use wg_core::ipc::{GridWindowMemory, InputMemory, SettingsMemory, INPUT_MAP_SIZE};
use wg_core::settings::Settings;

pub const SETTINGS_FILE: &str = "WindowGrid.settings";

/// How the settings were obtained at startup.
#[derive(Debug, PartialEq, Eq)]
pub enum LoadState {
    /// The file existed and parsed.
    Loaded,
    /// There is no settings file yet: defaults, and the first-run setup should be offered.
    Missing,
    /// The file exists but is unusable (the message says why); defaults are used.
    Invalid(String),
}

#[derive(Debug)]
pub struct Config {
    pub settings: Settings,
    pub path: PathBuf,
}

impl Config {
    pub fn path_in(dir: &Path) -> PathBuf {
        dir.join(SETTINGS_FILE)
    }

    pub fn load(dir: &Path) -> (Config, LoadState) {
        let path = Config::path_in(dir);
        let (settings, state) = match std::fs::read(&path) {
            Ok(bytes) => match Settings::from_json_str(&String::from_utf8_lossy(&bytes)) {
                Ok(s) => (s, LoadState::Loaded),
                Err(e) => (Settings::default(), LoadState::Invalid(e)),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (Settings::default(), LoadState::Missing),
            Err(e) => (Settings::default(), LoadState::Invalid(e.to_string())),
        };
        (Config { settings, path }, state)
    }

    /// Writes the file in the legacy layout. The write goes through a temporary file so an
    /// interrupted save cannot leave a truncated settings file behind.
    pub fn save(&self) -> io::Result<()> {
        let tmp = self.path.with_extension("settings.tmp");
        std::fs::write(&tmp, self.settings.to_json_string())?;
        std::fs::rename(&tmp, &self.path)
    }

    /// Re-reads the file; keeps the current settings (and says why) if it cannot be read.
    pub fn reload(&mut self) -> Result<(), String> {
        let bytes = std::fs::read(&self.path).map_err(|e| e.to_string())?;
        self.settings = Settings::from_json_str(&String::from_utf8_lossy(&bytes))?;
        Ok(())
    }
}

/// The three shared-memory images derived from the settings.
pub struct Published {
    /// Everything but `grid_window`, which the overlay owns (see [`crate::shared`]).
    pub hook: SettingsMemory,
    pub input: [u8; INPUT_MAP_SIZE],
    pub grid_window: GridWindowMemory,
}

pub fn publish(settings: &Settings) -> Published {
    Published {
        hook: SettingsMemory::from_settings(settings, 0),
        input: InputMemory::section_bytes(&settings.input),
        grid_window: GridWindowMemory::from_settings(&settings.grid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wg_core::ipc::DEFER_FLAG;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wg-app-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn missing_file_gives_defaults_and_asks_for_setup() {
        let d = temp_dir("missing");
        let (c, state) = Config::load(&d);
        assert_eq!(state, LoadState::Missing);
        assert_eq!(c.settings, Settings::default());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn save_then_load_round_trips_in_legacy_format() {
        let d = temp_dir("roundtrip");
        let (mut c, _) = Config::load(&d);
        c.settings.grid.grid_x = 20;
        c.settings.grid.theme_name = "blue".into();
        c.save().unwrap();
        let text = std::fs::read_to_string(Config::path_in(&d)).unwrap();
        assert!(text.starts_with("{\r\n    \"App\":\r\n"), "legacy layout");
        assert!(!d.join("WindowGrid.settings.tmp").exists(), "temporary file is gone");
        let (again, state) = Config::load(&d);
        assert_eq!(state, LoadState::Loaded);
        assert_eq!(again.settings.grid.grid_x, 20);
        assert_eq!(again.settings.grid.theme_name, "blue");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn broken_file_is_reported_and_defaults_are_used() {
        let d = temp_dir("broken");
        std::fs::write(Config::path_in(&d), "this is not json").unwrap();
        let (c, state) = Config::load(&d);
        assert!(matches!(state, LoadState::Invalid(_)));
        assert_eq!(c.settings, Settings::default());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn reload_picks_up_external_edits_and_keeps_state_on_errors() {
        let d = temp_dir("reload");
        let (mut c, _) = Config::load(&d);
        c.save().unwrap();
        let mut edited = c.settings.clone();
        edited.grid.grid_y = 9;
        std::fs::write(Config::path_in(&d), edited.to_json_string()).unwrap();
        c.reload().unwrap();
        assert_eq!(c.settings.grid.grid_y, 9);
        std::fs::write(Config::path_in(&d), "oops").unwrap();
        assert!(c.reload().is_err());
        assert_eq!(c.settings.grid.grid_y, 9, "a failed reload changes nothing");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn publish_matches_what_the_hooks_and_overlay_expect() {
        let mut s = Settings::default();
        s.grid.grid_x = 14;
        let p = publish(&s);
        assert_eq!({ p.hook.grid_x }, 14 | DEFER_FLAG);
        assert_eq!({ p.hook.grid_y }, 6);
        assert_eq!({ p.hook.input_size }, 4);
        assert_eq!(&p.input[..8], &[57, 0, 0, 0, 0, 0, 1, 0]);
        assert_eq!({ p.grid_window.grid_x }, 14);
        s.grid.defer_resize_until_release = false;
        assert_eq!({ publish(&s).hook.grid_x }, 14);
    }
}
