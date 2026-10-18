//! The settings window's model: everything the C# `SettingsContext` did that is not
//! presentation. It edits a copy of the settings; nothing changes until `apply_to`.

use wg_core::keycode::{Command, KeyCode, Modifiers};
use wg_core::settings::{InputSettings, Settings, WindowAnchor};

/// Grid presets offered next to the sliders.
pub const PRESETS: [(i32, i32); 7] = [(12, 6), (16, 9), (8, 4), (16, 8), (6, 4), (4, 2), (24, 12)];

/// Largest grid the UI allows.
pub const MAX_GRID: i32 = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsModel {
    pub splash_screen: bool,
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
    pub anchor: WindowAnchor,
    pub compensate_window_margins: bool,
    pub preserve_size_on_move: bool,
    pub defer_resize_until_release: bool,
    pub theme: String,
    pub bindings: Vec<InputSettings>,
}

impl SettingsModel {
    pub fn from_settings(s: &Settings) -> SettingsModel {
        let g = &s.grid;
        SettingsModel {
            splash_screen: s.app.splash_screen,
            grid_x: g.grid_x.clamp(1, MAX_GRID),
            grid_y: g.grid_y.clamp(1, MAX_GRID),
            radial_gradient: g.radial_gradient,
            blur_radius: g.blur_radius.clamp(0.0, 100.0),
            fill_window: g.fill_window,
            opacity: g.opacity.clamp(0.05, 1.0),
            show_selection_hud: g.show_selection_hud,
            show_dimensions: g.show_dimensions,
            show_cell_span: g.show_cell_span,
            show_screen_coverage: g.show_screen_coverage,
            show_aspect_ratio: g.show_aspect_ratio,
            show_mode_indicator: g.show_mode_indicator,
            show_interaction_hints: g.show_interaction_hints,
            anchor: g.anchor(),
            compensate_window_margins: g.compensate_window_margins,
            preserve_size_on_move: g.preserve_size_on_move,
            defer_resize_until_release: g.defer_resize_until_release,
            theme: g.theme_name.clone(),
            // Bindings without a command are not shown (and are dropped on save).
            bindings: s.input.iter().copied().filter(|i| i.command != Command::NONE).collect(),
        }
    }

    /// Writes the model back into `s`, leaving everything the UI does not edit (the list of
    /// themes and their colours, portable flag) untouched.
    pub fn apply_to(&self, s: &mut Settings) {
        s.app.splash_screen = self.splash_screen;
        let g = &mut s.grid;
        g.grid_x = self.grid_x;
        g.grid_y = self.grid_y;
        g.radial_gradient = self.radial_gradient;
        g.blur_radius = self.blur_radius;
        g.fill_window = self.fill_window;
        g.opacity = self.opacity;
        g.show_selection_hud = self.show_selection_hud;
        g.show_dimensions = self.show_dimensions;
        g.show_cell_span = self.show_cell_span;
        g.show_screen_coverage = self.show_screen_coverage;
        g.show_aspect_ratio = self.show_aspect_ratio;
        g.show_mode_indicator = self.show_mode_indicator;
        g.show_interaction_hints = self.show_interaction_hints;
        g.anchor = self.anchor as i32;
        g.compensate_window_margins = self.compensate_window_margins;
        g.preserve_size_on_move = self.preserve_size_on_move;
        g.defer_resize_until_release = self.defer_resize_until_release;
        g.theme_name = self.theme.clone();
        s.input = self.bindings.clone();
    }

    pub fn set_grid(&mut self, x: i32, y: i32) {
        self.grid_x = x.clamp(1, MAX_GRID);
        self.grid_y = y.clamp(1, MAX_GRID);
    }

    pub fn set_blur(&mut self, v: f32) {
        self.blur_radius = v.clamp(0.0, 100.0);
    }

    pub fn set_opacity(&mut self, v: f32) {
        self.opacity = v.clamp(0.05, 1.0);
    }

    pub fn add_binding(&mut self) {
        self.bindings.push(InputSettings { key: KeyCode::SPACE, modifier: Modifiers::NONE, command: Command::RESIZE });
    }

    pub fn remove_binding(&mut self, index: usize) {
        if index < self.bindings.len() {
            self.bindings.remove(index);
        }
    }

    pub fn grid_summary(&self) -> String {
        format!("{} \u{d7} {} ({} cells)", self.grid_x, self.grid_y, self.grid_x * self.grid_y)
    }

    /// Shape of one cell on a screen with the given aspect ratio.
    pub fn cell_shape_summary(&self, screen_ratio: f64) -> String {
        let screen = if screen_ratio > 0.0 { screen_ratio } else { 16.0 / 9.0 };
        let cell = screen * f64::from(self.grid_y) / f64::from(self.grid_x.max(1));
        if (cell - 1.0).abs() <= 0.12 {
            "Square (~1:1)".to_string()
        } else if cell > 1.0 {
            format!("{cell:.1}:1 Wide")
        } else {
            format!("1:{:.1} Tall", 1.0 / cell)
        }
    }
}

fn gcd(mut a: i32, mut b: i32) -> i32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// `"16:9 \u{2022} 3840 \u{d7} 2160"`-style description of the primary screen.
pub fn screen_summary(width: i32, height: i32) -> String {
    let (w, h) = (width.max(1), height.max(1));
    let ratio = f64::from(w) / f64::from(h);
    let near = |t: f64, tol: f64| (ratio - t).abs() < tol;
    let label = if near(16.0 / 9.0, 0.03) {
        "16:9".to_string()
    } else if near(16.0 / 10.0, 0.03) {
        "16:10".to_string()
    } else if near(21.0 / 9.0, 0.05) || near(43.0 / 18.0, 0.05) {
        "21:9".to_string()
    } else if near(32.0 / 9.0, 0.05) {
        "32:9".to_string()
    } else if near(4.0 / 3.0, 0.03) {
        "4:3".to_string()
    } else if near(3.0 / 2.0, 0.03) {
        "3:2".to_string()
    } else {
        let g = gcd(w, h);
        format!("{}:{}", w / g, h / g)
    };
    format!("{label} \u{2022} {w} \u{d7} {h}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_settings() {
        let mut s = Settings::default();
        s.grid.grid_x = 16;
        s.grid.grid_y = 9;
        s.grid.anchor = WindowAnchor::GrabPoint as i32;
        s.grid.theme_name = "dark".into();
        s.app.splash_screen = true;
        let m = SettingsModel::from_settings(&s);
        let mut back = Settings::default();
        m.apply_to(&mut back);
        assert_eq!(back.grid.grid_x, 16);
        assert_eq!(back.grid.anchor(), WindowAnchor::GrabPoint);
        assert_eq!(back.grid.theme_name, "dark");
        assert!(back.app.splash_screen);
        assert_eq!(back.input, s.input);
    }

    #[test]
    fn apply_keeps_what_the_ui_does_not_edit() {
        let mut s = Settings::default();
        s.app.portable = false;
        s.grid.themes.truncate(2);
        SettingsModel::from_settings(&s).apply_to(&mut s);
        assert!(!s.app.portable);
        assert_eq!(s.grid.themes.len(), 2);
    }

    #[test]
    fn values_are_clamped_like_the_legacy_ui() {
        let mut m = SettingsModel::from_settings(&Settings::default());
        m.set_grid(500, 0);
        assert_eq!((m.grid_x, m.grid_y), (64, 1));
        m.set_blur(-3.0);
        assert_eq!(m.blur_radius, 0.0);
        m.set_blur(500.0);
        assert_eq!(m.blur_radius, 100.0);
        m.set_opacity(0.0);
        assert_eq!(m.opacity, 0.05);
        m.set_opacity(7.0);
        assert_eq!(m.opacity, 1.0);
    }

    #[test]
    fn out_of_range_file_values_are_clamped_when_loaded() {
        let mut s = Settings::default();
        s.grid.grid_x = 1000;
        s.grid.opacity = 0.0;
        let m = SettingsModel::from_settings(&s);
        assert_eq!((m.grid_x, m.opacity), (64, 0.05));
    }

    #[test]
    fn bindings_without_a_command_are_dropped() {
        let mut s = Settings::default();
        s.input.push(InputSettings { key: KeyCode::UNASSIGNED, modifier: Modifiers::NONE, command: Command::NONE });
        let m = SettingsModel::from_settings(&s);
        assert_eq!(m.bindings.len(), 4);
    }

    #[test]
    fn binding_list_editing() {
        let mut m = SettingsModel::from_settings(&Settings::default());
        m.add_binding();
        assert_eq!(m.bindings.len(), 5);
        assert_eq!(m.bindings[4], InputSettings { key: KeyCode::SPACE, modifier: Modifiers::NONE, command: Command::RESIZE });
        m.remove_binding(0);
        assert_eq!(m.bindings.len(), 4);
        m.remove_binding(99);
        assert_eq!(m.bindings.len(), 4);
    }

    #[test]
    fn summaries() {
        let m = SettingsModel::from_settings(&Settings::default());
        assert_eq!(m.grid_summary(), "12 \u{d7} 6 (72 cells)");
        // 16:9 screen with a 12x6 grid: cells are 16/9 * 6/12 = 0.89 -> inside the 12% band.
        assert_eq!(m.cell_shape_summary(16.0 / 9.0), "Square (~1:1)");
        // 4x4 cells on 16:9 are 1.78 times wider than tall.
        let mut wide = m.clone();
        wide.set_grid(4, 4);
        assert_eq!(wide.cell_shape_summary(16.0 / 9.0), "1.8:1 Wide");
        let mut tall = m.clone();
        tall.set_grid(24, 4);
        assert_eq!(tall.cell_shape_summary(16.0 / 9.0), "1:3.4 Tall");
        assert_eq!(m.cell_shape_summary(0.0), m.cell_shape_summary(16.0 / 9.0), "invalid ratio falls back to 16:9");
    }

    #[test]
    fn screen_summary_labels() {
        assert_eq!(screen_summary(3840, 2160), "16:9 \u{2022} 3840 \u{d7} 2160");
        assert_eq!(screen_summary(1920, 1200), "16:10 \u{2022} 1920 \u{d7} 1200");
        assert_eq!(screen_summary(3440, 1440), "21:9 \u{2022} 3440 \u{d7} 1440");
        assert_eq!(screen_summary(1024, 768), "4:3 \u{2022} 1024 \u{d7} 768");
        assert_eq!(screen_summary(1000, 700), "10:7 \u{2022} 1000 \u{d7} 700");
        assert_eq!(screen_summary(0, 0), "1:1 \u{2022} 1 \u{d7} 1");
    }
}
