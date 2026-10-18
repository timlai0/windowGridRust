//! Reads real settings files written by the C# app and checks that writing them back
//! reproduces them byte for byte (apart from fields newer than the file).

use wg_core::keycode::{Command, KeyCode};
use wg_core::settings::Settings;

const RELEASE: &str = include_str!("fixtures/legacy-release.settings");
const DEBUG: &str = include_str!("fixtures/legacy-debug.settings");

/// The fixtures were written before `DeferResizeUntilRelease` existed; the writer now
/// emits it (default `true`) right after `PreserveSizeOnMove`.
fn with_defer_line(text: &str) -> String {
    assert!(!text.contains("DeferResizeUntilRelease"));
    text.replace(
        "        \"PreserveSizeOnMove\": true,\r\n",
        "        \"PreserveSizeOnMove\": true,\r\n        \"DeferResizeUntilRelease\": true,\r\n",
    )
}

#[test]
fn fixtures_use_crlf_without_bom() {
    for text in [RELEASE, DEBUG] {
        assert!(text.starts_with("{\r\n"));
        assert!(text.ends_with("}\r\n"));
        assert!(!text.contains("\n\n") && !text.replace("\r\n", "").contains('\n'));
    }
}

#[test]
fn release_file_round_trips_byte_for_byte() {
    let settings = Settings::from_json_str(RELEASE).unwrap();
    assert_eq!(settings.grid.grid_x, 6);
    assert_eq!(settings.grid.grid_y, 12);
    assert_eq!(settings.grid.theme_name, "default");
    assert!(settings.grid.defer_resize_until_release, "missing field keeps its default");
    assert_eq!(settings.to_json_string(), with_defer_line(RELEASE));
}

#[test]
fn debug_file_with_non_default_values_round_trips() {
    let settings = Settings::from_json_str(DEBUG).unwrap();
    assert_eq!(settings.grid.grid_x, 12);
    assert!(!settings.grid.radial_gradient);
    assert_eq!(settings.grid.active_theme().name, "dark");
    assert_eq!(settings.to_json_string(), with_defer_line(DEBUG));
}

#[test]
fn input_bindings_are_read_from_the_file() {
    let settings = Settings::from_json_str(RELEASE).unwrap();
    assert_eq!(settings.input.len(), 4);
    assert_eq!(settings.input[0].key, KeyCode::SPACE);
    assert_eq!(settings.input[0].command, Command::RESIZE);
    assert_eq!(settings.input[3].key, KeyCode::MOUSE_MBUTTON);
    assert_eq!(settings.input[3].command, Command::MOVE);
}

#[test]
fn writing_is_stable_after_one_round() {
    let once = Settings::from_json_str(RELEASE).unwrap().to_json_string();
    let twice = Settings::from_json_str(&once).unwrap().to_json_string();
    assert_eq!(once, twice);
}

#[test]
fn default_settings_serialise_and_reload_equal() {
    let defaults = Settings::default();
    let reloaded = Settings::from_json_str(&defaults.to_json_string()).unwrap();
    assert_eq!(defaults, reloaded);
}
