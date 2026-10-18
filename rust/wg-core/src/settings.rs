//! The settings model and its (de)serialisation, compatible with the legacy
//! `WindowGrid.settings` files written by the C# app.
//!
//! Reading is lenient, like the legacy reader: unknown fields are ignored, missing fields
//! keep their defaults, enum fields that fail to parse are left at their default. Writing
//! reproduces the legacy layout byte for byte (see [`crate::json`]).

use crate::json::{self, Json};
use crate::keycode::{Command, KeyCode, Modifiers};
use serde_json::{Map, Value};
use std::fmt;

/// An ARGB colour. Text form is WPF's `Color.ToString()`: `#AARRGGBB`, upper-case.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> Color {
        Color { a, r, g, b }
    }

    /// Parses `#RGB`, `#ARGB`, `#RRGGBB` or `#AARRGGBB`.
    ///
    /// WPF's `ColorConverter` also accepts named colours such as `Red`; those are not
    /// supported here and fall back to the per-field default in [`GridTheme`] parsing.
    pub fn parse(s: &str) -> Option<Color> {
        let hex = s.trim().strip_prefix('#')?;
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let nibble = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|n| n * 17);
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        match hex.len() {
            3 => Some(Color::argb(255, nibble(0)?, nibble(1)?, nibble(2)?)),
            4 => Some(Color::argb(nibble(0)?, nibble(1)?, nibble(2)?, nibble(3)?)),
            6 => Some(Color::argb(255, byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Color::argb(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => None,
        }
    }

    /// Packed `0xAARRGGBB` as the signed int the shared-memory structs carry.
    pub fn to_argb_i32(self) -> i32 {
        u32::from_be_bytes([self.a, self.r, self.g, self.b]) as i32
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}{:02X}", self.a, self.r, self.g, self.b)
    }
}

/// Where a snapped window sits when it is larger than the selected cells.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WindowAnchor {
    #[default]
    TopLeft = 0,
    Center = 1,
    GrabPoint = 2,
}

impl WindowAnchor {
    /// Unknown values behave like `TopLeft`, as in the legacy overlay.
    pub fn from_i32(v: i32) -> WindowAnchor {
        match v {
            1 => WindowAnchor::Center,
            2 => WindowAnchor::GrabPoint,
            _ => WindowAnchor::TopLeft,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GridTheme {
    pub name: String,
    pub color_line: Color,
    pub color_start: Color,
    pub color_end: Color,
}

impl GridTheme {
    const FALLBACK_LINE: Color = Color::argb(255, 0, 189, 212);
    const FALLBACK_START: Color = Color::argb(255, 0, 151, 167);
    const FALLBACK_END: Color = Color::argb(255, 178, 235, 242);

    fn builtin(name: &str, line: Color, start: Color, end: Color) -> GridTheme {
        GridTheme { name: name.to_string(), color_line: line, color_start: start, color_end: end }
    }

    pub fn default_theme() -> GridTheme {
        Self::builtin(
            "default",
            Color::argb(255, 0x00, 0xBD, 0xD4),
            Color::argb(255, 0x00, 0x97, 0xA7),
            Color::argb(255, 0xB2, 0xEB, 0xF2),
        )
    }

    /// The six themes every fresh settings file carries.
    pub fn builtin_themes() -> Vec<GridTheme> {
        vec![
            Self::default_theme(),
            Self::builtin(
                "dark",
                Color::argb(255, 0x44, 0x44, 0x44),
                Color::argb(255, 0x22, 0x22, 0x22),
                Color::argb(255, 0x66, 0x66, 0x66),
            ),
            Self::builtin(
                "light",
                Color::argb(255, 0xAA, 0xAA, 0xAA),
                Color::argb(255, 0xEE, 0xEE, 0xEE),
                Color::argb(255, 0xFF, 0xFF, 0xFF),
            ),
            Self::builtin(
                "red",
                Color::argb(255, 0xFF, 0x00, 0x00),
                Color::argb(255, 0x8B, 0x00, 0x00),
                Color::argb(255, 0xFF, 0x66, 0x66),
            ),
            Self::builtin(
                "green",
                Color::argb(255, 0x00, 0xFF, 0x00),
                Color::argb(255, 0x00, 0x64, 0x00),
                Color::argb(255, 0x66, 0xFF, 0x66),
            ),
            Self::builtin(
                "blue",
                Color::argb(255, 0x00, 0x00, 0xFF),
                Color::argb(255, 0x00, 0x00, 0x8B),
                Color::argb(255, 0x66, 0x66, 0xFF),
            ),
        ]
    }

    fn from_value(v: &Value) -> Option<GridTheme> {
        let o = v.as_object()?;
        // A missing colour stays transparent black (an unset `Color` in the legacy app);
        // a present but unparsable one gets the legacy per-field fallback.
        let color = |key: &str, fallback: Color| match o.get(key) {
            Some(Value::String(s)) => Color::parse(s).unwrap_or(fallback),
            _ => Color::default(),
        };
        Some(GridTheme {
            name: o.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
            color_line: color("ColorLine", Self::FALLBACK_LINE),
            color_start: color("ColorStart", Self::FALLBACK_START),
            color_end: color("ColorEnd", Self::FALLBACK_END),
        })
    }

    fn to_json(&self) -> Json {
        Json::object([
            ("Name", Json::str(&self.name)),
            ("ColorLine", Json::str(self.color_line.to_string())),
            ("ColorStart", Json::str(self.color_start.to_string())),
            ("ColorEnd", Json::str(self.color_end.to_string())),
        ])
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppSettings {
    pub portable: bool,
    pub splash_screen: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings { portable: true, splash_screen: false }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct GridSettings {
    pub grid_x: i32,
    pub grid_y: i32,
    pub radial_gradient: bool,
    pub blur_radius: f32,
    pub fill_window: bool,
    pub opacity: f32,
    pub show_selection_hud: bool,
    pub show_dimensions: bool,
    pub show_cell_span: bool,
    pub show_screen_coverage: bool,
    pub show_aspect_ratio: bool,
    pub show_mode_indicator: bool,
    pub show_interaction_hints: bool,
    /// Stored as the raw integer; use [`GridSettings::anchor`] for the typed value.
    pub anchor: i32,
    pub compensate_window_margins: bool,
    pub preserve_size_on_move: bool,
    pub defer_resize_until_release: bool,
    pub theme_name: String,
    pub themes: Vec<GridTheme>,
}

impl Default for GridSettings {
    fn default() -> Self {
        GridSettings {
            grid_x: 12,
            grid_y: 6,
            radial_gradient: true,
            blur_radius: 3.0,
            fill_window: true,
            opacity: 0.75,
            show_selection_hud: true,
            show_dimensions: true,
            show_cell_span: true,
            show_screen_coverage: true,
            show_aspect_ratio: true,
            show_mode_indicator: true,
            show_interaction_hints: true,
            anchor: WindowAnchor::TopLeft as i32,
            compensate_window_margins: true,
            preserve_size_on_move: true,
            defer_resize_until_release: true,
            theme_name: "default".to_string(),
            themes: GridTheme::builtin_themes(),
        }
    }
}

impl GridSettings {
    pub fn anchor(&self) -> WindowAnchor {
        WindowAnchor::from_i32(self.anchor)
    }

    /// The selected theme (case-insensitive name match), or the default theme.
    pub fn active_theme(&self) -> GridTheme {
        self.themes
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(&self.theme_name))
            .cloned()
            .unwrap_or_else(GridTheme::default_theme)
    }

    /// Applies the limits the settings UI enforces: grid 1-64, blur 0-100, opacity 0.05-1.
    pub fn clamp_to_ui_limits(&mut self) {
        self.grid_x = self.grid_x.clamp(1, 64);
        self.grid_y = self.grid_y.clamp(1, 64);
        self.blur_radius = self.blur_radius.clamp(0.0, 100.0);
        self.opacity = self.opacity.clamp(0.05, 1.0);
    }

    fn merge_value(&mut self, o: &Map<String, Value>) {
        set_i32(o, "GridX", &mut self.grid_x);
        set_i32(o, "GridY", &mut self.grid_y);
        set_bool(o, "RadialGradient", &mut self.radial_gradient);
        set_f32(o, "BlurRadius", &mut self.blur_radius);
        set_bool(o, "FillWindow", &mut self.fill_window);
        set_f32(o, "Opacity", &mut self.opacity);
        set_bool(o, "ShowSelectionHUD", &mut self.show_selection_hud);
        set_bool(o, "ShowDimensions", &mut self.show_dimensions);
        set_bool(o, "ShowCellSpan", &mut self.show_cell_span);
        set_bool(o, "ShowScreenCoverage", &mut self.show_screen_coverage);
        set_bool(o, "ShowAspectRatio", &mut self.show_aspect_ratio);
        set_bool(o, "ShowModeIndicator", &mut self.show_mode_indicator);
        set_bool(o, "ShowInteractionHints", &mut self.show_interaction_hints);
        set_i32(o, "Anchor", &mut self.anchor);
        set_bool(o, "CompensateWindowMargins", &mut self.compensate_window_margins);
        set_bool(o, "PreserveSizeOnMove", &mut self.preserve_size_on_move);
        set_bool(o, "DeferResizeUntilRelease", &mut self.defer_resize_until_release);
        if let Some(Value::String(s)) = o.get("ThemeName") {
            self.theme_name = s.clone();
        }
        if let Some(Value::Array(items)) = o.get("Themes") {
            self.themes = items.iter().filter_map(GridTheme::from_value).collect();
        }
    }

    fn to_json(&self) -> Json {
        Json::object([
            ("GridX", Json::Int(self.grid_x.into())),
            ("GridY", Json::Int(self.grid_y.into())),
            ("BlurRadius", Json::Float(self.blur_radius)),
            ("FillWindow", Json::Bool(self.fill_window)),
            ("Opacity", Json::Float(self.opacity)),
            ("RadialGradient", Json::Bool(self.radial_gradient)),
            ("ShowAspectRatio", Json::Bool(self.show_aspect_ratio)),
            ("ShowCellSpan", Json::Bool(self.show_cell_span)),
            ("ShowDimensions", Json::Bool(self.show_dimensions)),
            ("ShowScreenCoverage", Json::Bool(self.show_screen_coverage)),
            ("ShowSelectionHUD", Json::Bool(self.show_selection_hud)),
            ("ShowInteractionHints", Json::Bool(self.show_interaction_hints)),
            ("ShowModeIndicator", Json::Bool(self.show_mode_indicator)),
            ("Anchor", Json::Int(self.anchor.into())),
            ("CompensateWindowMargins", Json::Bool(self.compensate_window_margins)),
            ("PreserveSizeOnMove", Json::Bool(self.preserve_size_on_move)),
            ("DeferResizeUntilRelease", Json::Bool(self.defer_resize_until_release)),
            ("ThemeName", Json::str(&self.theme_name)),
            ("Themes", Json::Array(self.themes.iter().map(GridTheme::to_json).collect())),
        ])
    }
}

/// One input binding. The legacy file omits `Key` when unassigned and `Modifier` when none.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InputSettings {
    pub key: KeyCode,
    pub modifier: Modifiers,
    pub command: Command,
}

impl InputSettings {
    pub const fn new(key: KeyCode, command: Command) -> InputSettings {
        InputSettings { key, modifier: Modifiers::NONE, command }
    }

    fn from_value(v: &Value) -> Option<InputSettings> {
        let o = v.as_object()?;
        let mut input = InputSettings::default();
        if let Some(k) = o.get("Key").and_then(Value::as_str).and_then(KeyCode::parse) {
            input.key = k;
        }
        if let Some(m) = o.get("Modifier").and_then(Value::as_str).and_then(Modifiers::parse) {
            input.modifier = m;
        }
        if let Some(c) = o.get("Command").and_then(Value::as_str).and_then(Command::parse) {
            input.command = c;
        }
        Some(input)
    }

    fn to_json(self) -> Json {
        let mut fields = Vec::new();
        if self.key != KeyCode::UNASSIGNED {
            fields.push(("Key".to_string(), Json::str(self.key.to_string())));
        }
        if self.modifier != Modifiers::NONE {
            fields.push(("Modifier".to_string(), Json::str(self.modifier.to_string())));
        }
        fields.push(("Command".to_string(), Json::str(self.command.to_string())));
        Json::Object(fields)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub app: AppSettings,
    pub grid: GridSettings,
    pub input: Vec<InputSettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            app: AppSettings::default(),
            grid: GridSettings::default(),
            input: Self::default_input(),
        }
    }
}

impl Settings {
    /// Space / right mouse button resize, left Ctrl / middle mouse button move.
    pub fn default_input() -> Vec<InputSettings> {
        vec![
            InputSettings::new(KeyCode::SPACE, Command::RESIZE),
            InputSettings::new(KeyCode::LCONTROL, Command::MOVE),
            InputSettings::new(KeyCode::MOUSE_RBUTTON, Command::RESIZE),
            InputSettings::new(KeyCode::MOUSE_MBUTTON, Command::MOVE),
        ]
    }

    /// Parses settings text. Anything that is not a JSON object is an error; everything
    /// inside it is read leniently.
    pub fn from_json_str(text: &str) -> Result<Settings, String> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let root = value.as_object().ok_or("settings must be a JSON object")?;

        let mut settings = Settings::default();
        if let Some(Value::Object(o)) = root.get("App") {
            set_bool(o, "Portable", &mut settings.app.portable);
            set_bool(o, "SplashScreen", &mut settings.app.splash_screen);
        }
        if let Some(Value::Object(o)) = root.get("Grid") {
            settings.grid.merge_value(o);
        }
        if let Some(Value::Array(items)) = root.get("Input") {
            settings.input = items.iter().filter_map(InputSettings::from_value).collect();
        }
        Ok(settings)
    }

    /// Serialises in the legacy layout (CRLF, four-space indent, trailing newline).
    pub fn to_json_string(&self) -> String {
        json::format(&Json::object([
            (
                "App",
                Json::object([
                    ("Portable", Json::Bool(self.app.portable)),
                    ("SplashScreen", Json::Bool(self.app.splash_screen)),
                ]),
            ),
            ("Grid", self.grid.to_json()),
            ("Input", Json::Array(self.input.iter().map(|i| i.to_json()).collect())),
        ]))
    }
}

fn set_bool(o: &Map<String, Value>, key: &str, dst: &mut bool) {
    if let Some(Value::Bool(b)) = o.get(key) {
        *dst = *b;
    }
}

fn set_i32(o: &Map<String, Value>, key: &str, dst: &mut i32) {
    if let Some(n) = o.get(key).and_then(Value::as_i64) {
        if let Ok(n) = i32::try_from(n) {
            *dst = n;
        }
    }
}

fn set_f32(o: &Map<String, Value>, key: &str, dst: &mut f32) {
    if let Some(n) = o.get(key).and_then(Value::as_f64) {
        *dst = n as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_parsing_and_display() {
        assert_eq!(Color::parse("#FF00BDD4"), Some(Color::argb(255, 0, 189, 212)));
        assert_eq!(Color::parse("#00BDD4"), Some(Color::argb(255, 0, 189, 212)));
        assert_eq!(Color::parse("#F0A"), Some(Color::argb(255, 255, 0, 170)));
        assert_eq!(Color::parse("#80F0A"), None);
        assert_eq!(Color::parse("#8F0A"), Some(Color::argb(136, 255, 0, 170)));
        assert_eq!(Color::parse("Red"), None);
        assert_eq!(Color::parse("#GG0000"), None);
        assert_eq!(Color::argb(255, 0, 189, 212).to_string(), "#FF00BDD4");
        assert_eq!(Color::argb(255, 0, 189, 212).to_argb_i32(), 0xFF00BDD4u32 as i32);
    }

    #[test]
    fn defaults_match_legacy() {
        let s = Settings::default();
        assert_eq!((s.grid.grid_x, s.grid.grid_y), (12, 6));
        assert_eq!(s.grid.opacity, 0.75);
        assert_eq!(s.grid.themes.len(), 6);
        assert_eq!(s.input.len(), 4);
        assert_eq!(s.input[0], InputSettings::new(KeyCode::SPACE, Command::RESIZE));
        assert_eq!(s.input[3], InputSettings::new(KeyCode::MOUSE_MBUTTON, Command::MOVE));
        assert!(s.app.portable && !s.app.splash_screen);
    }

    #[test]
    fn missing_fields_keep_defaults_and_unknown_are_ignored() {
        let s = Settings::from_json_str(r#"{"Grid": {"GridX": 8, "Mystery": 1}, "Extra": []}"#).unwrap();
        assert_eq!(s.grid.grid_x, 8);
        assert_eq!(s.grid.grid_y, 6);
        assert_eq!(s.input, Settings::default_input());
    }

    #[test]
    fn invalid_enum_strings_are_ignored_per_binding() {
        let s = Settings::from_json_str(
            r#"{"Input": [{"Key": "NOPE", "Modifier": "Meta", "Command": "Move"}]}"#,
        )
        .unwrap();
        assert_eq!(s.input, vec![InputSettings::new(KeyCode::UNASSIGNED, Command::MOVE)]);
    }

    #[test]
    fn theme_lookup_is_case_insensitive_with_default_fallback() {
        let mut g = GridSettings { theme_name: "DARK".into(), ..GridSettings::default() };
        assert_eq!(g.active_theme().name, "dark");
        g.theme_name = "does-not-exist".into();
        assert_eq!(g.active_theme(), GridTheme::default_theme());
    }

    #[test]
    fn unparsable_theme_colour_uses_field_fallback() {
        let s = Settings::from_json_str(
            r##"{"Grid": {"Themes": [{"Name": "x", "ColorLine": "Chartreuse", "ColorStart": "#112233"}]}}"##,
        )
        .unwrap();
        let t = &s.grid.themes[0];
        assert_eq!(t.color_line, GridTheme::FALLBACK_LINE);
        assert_eq!(t.color_start, Color::argb(255, 0x11, 0x22, 0x33));
        assert_eq!(t.color_end, Color::default());
    }

    #[test]
    fn bom_is_accepted_and_non_objects_rejected() {
        assert!(Settings::from_json_str("\u{feff}{}").is_ok());
        assert!(Settings::from_json_str("[]").is_err());
        assert!(Settings::from_json_str("not json").is_err());
    }

    #[test]
    fn clamps_to_ui_limits() {
        let mut g = GridSettings { grid_x: 500, grid_y: 0, blur_radius: -4.0, opacity: 0.0, ..GridSettings::default() };
        g.clamp_to_ui_limits();
        assert_eq!((g.grid_x, g.grid_y, g.blur_radius, g.opacity), (64, 1, 0.0, 0.05));
    }

    #[test]
    fn anchor_decoding() {
        assert_eq!(WindowAnchor::from_i32(1), WindowAnchor::Center);
        assert_eq!(WindowAnchor::from_i32(2), WindowAnchor::GrabPoint);
        assert_eq!(WindowAnchor::from_i32(7), WindowAnchor::TopLeft);
    }
}
