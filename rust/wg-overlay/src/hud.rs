//! Text and placement for the selection HUD and the hint bar (no drawing here).

use crate::geom::{round_even, Cells, RectF};
use crate::snap::Mode;

fn gcd(mut a: i32, mut b: i32) -> i32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// `(16:9)`-style label; common ratios are matched when the reduced form is unwieldy.
pub fn aspect_ratio(width: i32, height: i32) -> String {
    if width <= 0 || height <= 0 {
        return "(16:9)".to_string();
    }
    let g = gcd(width, height);
    let (a, b) = (width / g, height / g);
    if a > 32 || b > 32 {
        let ratio = f64::from(width) / f64::from(height);
        const KNOWN: [(f64, &str); 6] = [
            (16.0 / 9.0, "(16:9)"),
            (16.0 / 10.0, "(16:10)"),
            (4.0 / 3.0, "(4:3)"),
            (21.0 / 9.0, "(21:9)"),
            (32.0 / 9.0, "(32:9)"),
            (1.0, "(1:1)"),
        ];
        for (r, label) in KNOWN {
            if (ratio - r).abs() < 0.05 {
                return label.to_string();
            }
        }
        return format!("({ratio:.1}:1)");
    }
    format!("({a}:{b})")
}

/// Share of the grid covered by the selection, in whole percent.
pub fn coverage_percent(cells: Cells, cols: i32, rows: i32) -> i32 {
    let area = f64::from(cells.span_x()) * f64::from(cells.span_y());
    round_even(area / (f64::from(cols) * f64::from(rows)) * 100.0) as i32
}

#[derive(Clone, Copy, Debug)]
pub struct HudOptions {
    pub show_dimensions: bool,
    pub show_cell_span: bool,
    pub show_coverage: bool,
    pub show_aspect_ratio: bool,
}

/// The HUD's text runs. Hidden items are `None`; the grid and screen labels always show.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HudText {
    pub dimensions: Option<String>,
    pub cell_span: Option<String>,
    pub coverage: Option<String>,
    pub aspect: Option<String>,
    pub grid_info: String,
    pub screen_info: String,
}

/// What the HUD describes: the selection and the grid and screen it sits on.
#[derive(Clone, Copy, Debug)]
pub struct HudSubject {
    /// Selection size in pixels.
    pub px_w: i32,
    pub px_h: i32,
    pub cells: Cells,
    pub cols: i32,
    pub rows: i32,
    /// Work-area size in pixels.
    pub screen_w: i32,
    pub screen_h: i32,
}

pub fn hud_text(opts: HudOptions, subject: HudSubject) -> HudText {
    let HudSubject { px_w, px_h, cells, cols, rows, screen_w, screen_h } = subject;
    HudText {
        dimensions: opts.show_dimensions.then(|| format!("{px_w} \u{d7} {px_h} px")),
        cell_span: opts
            .show_cell_span
            .then(|| format!("{} \u{d7} {} cells", cells.span_x(), cells.span_y())),
        coverage: opts.show_coverage.then(|| format!("{}%", coverage_percent(cells, cols, rows))),
        aspect: opts.show_aspect_ratio.then(|| aspect_ratio(px_w, px_h)),
        grid_info: format!("{cols}\u{d7}{rows} Grid"),
        screen_info: format!("{screen_w}\u{d7}{screen_h} Screen"),
    }
}

/// Top-left of the HUD: centred above the selection, below it if there is no room above,
/// inside it if there is no room below either; always at least `10 * scale` from the edges.
pub fn place_hud(sel: RectF, hud_w: f32, hud_h: f32, area_w: f32, area_h: f32, scale: f32) -> (f32, f32) {
    let margin = 10.0 * scale;
    let gap = 12.0 * scale;
    let max_x = (area_w - hud_w - margin).max(margin);
    let x = (sel.left + (sel.width - hud_w) / 2.0).clamp(margin, max_x);
    let mut y = sel.top - hud_h - gap;
    if y < margin {
        y = sel.bottom() + gap;
        if y + hud_h > area_h - margin {
            y = sel.top + (sel.height - hud_h) / 2.0;
        }
    }
    (x, y)
}

/// What the hint bar shows: the mode tag and/or the hint line. `None` hides the bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HintBar {
    pub tag: Option<&'static str>,
    pub hint: Option<&'static str>,
}

pub fn hint_bar(mode: Mode, show_mode: bool, show_hints: bool) -> Option<HintBar> {
    if !show_mode && !show_hints {
        return None;
    }
    let resize = mode == Mode::Resize;
    Some(HintBar {
        tag: show_mode.then_some(if resize { "RESIZE" } else { "MOVE" }),
        hint: show_hints.then_some(if resize {
            "Drag to select grid cells  \u{2022}  Release mouse to snap  \u{2022}  Esc to cancel"
        } else {
            "Position window on grid  \u{2022}  Release mouse to snap  \u{2022}  Esc to cancel"
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_ratio_reduces_or_matches_known() {
        assert_eq!(aspect_ratio(1920, 1080), "(16:9)");
        assert_eq!(aspect_ratio(640, 540), "(32:27)");
        assert_eq!(aspect_ratio(800, 600), "(4:3)");
        assert_eq!(aspect_ratio(1000, 1000), "(1:1)");
        // 1366x768 reduces to 683:384 -> too large, matches 16:9 within tolerance
        assert_eq!(aspect_ratio(1366, 768), "(16:9)");
        assert_eq!(aspect_ratio(2560, 1080), "(21:9)");
        assert_eq!(aspect_ratio(1000, 713), "(1.4:1)");
        assert_eq!(aspect_ratio(0, 5), "(16:9)");
    }

    #[test]
    fn coverage_uses_half_even_rounding() {
        let c = Cells::from_corners(0, 0, 5, 2);
        assert_eq!(coverage_percent(c, 12, 6), 25);
        // 1 of 8 cells = 12.5% -> 12 (half to even)
        assert_eq!(coverage_percent(Cells::from_corners(0, 0, 0, 0), 4, 2), 12);
        assert_eq!(coverage_percent(Cells::from_corners(0, 0, 1, 0), 4, 2), 25);
    }

    #[test]
    fn hud_text_honours_visibility() {
        let all = HudOptions { show_dimensions: true, show_cell_span: true, show_coverage: true, show_aspect_ratio: true };
        let t = hud_text(all, HudSubject { px_w: 1920, px_h: 1080, cells: Cells::from_corners(0, 0, 5, 2), cols: 12, rows: 6, screen_w: 2560, screen_h: 1440 });
        assert_eq!(t.dimensions.as_deref(), Some("1920 \u{d7} 1080 px"));
        assert_eq!(t.cell_span.as_deref(), Some("6 \u{d7} 3 cells"));
        assert_eq!(t.coverage.as_deref(), Some("25%"));
        assert_eq!(t.aspect.as_deref(), Some("(16:9)"));
        assert_eq!(t.grid_info, "12\u{d7}6 Grid");
        assert_eq!(t.screen_info, "2560\u{d7}1440 Screen");

        let none = HudOptions { show_dimensions: false, show_cell_span: false, show_coverage: false, show_aspect_ratio: false };
        let t = hud_text(none, HudSubject { px_w: 1, px_h: 1, cells: Cells::from_corners(0, 0, 0, 0), cols: 2, rows: 2, screen_w: 10, screen_h: 10 });
        assert!(t.dimensions.is_none() && t.cell_span.is_none() && t.coverage.is_none() && t.aspect.is_none());
        assert_eq!(t.grid_info, "2\u{d7}2 Grid");
    }

    #[test]
    fn hud_is_placed_above_below_or_inside() {
        let area = (1920.0, 1080.0);
        let hud = (360.0, 36.0);
        // Room above: centred over the selection.
        let sel = RectF { left: 500.0, top: 400.0, width: 400.0, height: 200.0 };
        assert_eq!(place_hud(sel, hud.0, hud.1, area.0, area.1, 1.0), (520.0, 400.0 - 36.0 - 12.0));
        // No room above: goes below.
        let sel = RectF { left: 500.0, top: 20.0, width: 400.0, height: 200.0 };
        assert_eq!(place_hud(sel, hud.0, hud.1, area.0, area.1, 1.0), (520.0, 232.0));
        // No room above or below: inside, vertically centred.
        let sel = RectF { left: 0.0, top: 0.0, width: 1920.0, height: 1080.0 };
        assert_eq!(place_hud(sel, hud.0, hud.1, area.0, area.1, 1.0), (780.0, 522.0));
        // Horizontal clamping to 10 px from either edge.
        let sel = RectF { left: 0.0, top: 400.0, width: 50.0, height: 100.0 };
        assert_eq!(place_hud(sel, hud.0, hud.1, area.0, area.1, 1.0).0, 10.0);
        let sel = RectF { left: 1900.0, top: 400.0, width: 20.0, height: 100.0 };
        assert_eq!(place_hud(sel, hud.0, hud.1, area.0, area.1, 1.0).0, 1920.0 - 360.0 - 10.0);
        // The margins scale with DPI.
        let sel = RectF { left: 0.0, top: 400.0, width: 50.0, height: 100.0 };
        assert_eq!(place_hud(sel, hud.0, hud.1, area.0, area.1, 2.0).0, 20.0);
    }

    #[test]
    fn hud_wider_than_the_screen_does_not_panic() {
        let sel = RectF { left: 0.0, top: 400.0, width: 50.0, height: 100.0 };
        let (x, _) = place_hud(sel, 500.0, 36.0, 400.0, 300.0, 1.0);
        assert_eq!(x, 10.0);
    }

    #[test]
    fn hint_bar_content() {
        assert_eq!(hint_bar(Mode::Resize, false, false), None);
        let b = hint_bar(Mode::Resize, true, true).unwrap();
        assert_eq!(b.tag, Some("RESIZE"));
        assert!(b.hint.unwrap().starts_with("Drag to select grid cells"));
        let b = hint_bar(Mode::Move, true, false).unwrap();
        assert_eq!((b.tag, b.hint), (Some("MOVE"), None));
        let b = hint_bar(Mode::Move, false, true).unwrap();
        assert!(b.tag.is_none() && b.hint.unwrap().starts_with("Position window on grid"));
    }
}
