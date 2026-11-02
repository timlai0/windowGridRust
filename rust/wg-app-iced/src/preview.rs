//! The live preview: a monitor with the grid and a sample selection in the chosen theme.

use crate::memo::Memo;
use crate::style::with_alpha;
use iced::mouse;
use iced::widget::canvas::{self, gradient, Frame, Path, Stroke};
use iced::{color, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};

const BEZEL: f32 = 4.0;
/// How many concentric ellipses draw the radial gradient.
const RADIAL_STEPS: u32 = 24;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridPreview {
    pub cols: i32,
    pub rows: i32,
    /// Width over height of the screen being previewed.
    pub ratio: f32,
    pub line: Color,
    pub start: Color,
    pub end: Color,
    pub radial: bool,
    pub fill: bool,
    pub opacity: f32,
}

/// The sample selection, roughly the middle half of the grid: first and one-past-last column
/// and row.
pub fn selection(cols: i32, rows: i32) -> (i32, i32, i32, i32) {
    let span = |n: i32| {
        let first = n / 4;
        (first, (first + 1).max((n * 3 + 3) / 4))
    };
    let (c0, c1) = span(cols.max(1));
    let (r0, r1) = span(rows.max(1));
    (c0, c1, r0, r1)
}

/// The largest rectangle of the given aspect ratio that fits in `size`, centred.
pub fn fit(size: Size, ratio: f32) -> Rectangle {
    let ratio = if ratio > 0.0 { ratio } else { 16.0 / 9.0 };
    let (w, h) = if size.width / size.height.max(1.0) > ratio { (size.height * ratio, size.height) } else { (size.width, size.width / ratio) };
    Rectangle { x: (size.width - w) / 2.0, y: (size.height - h) / 2.0, width: w, height: h }
}

fn darker(c: Color, factor: f32) -> Color {
    Color { r: c.r / (1.0 + factor), g: c.g / (1.0 + factor), b: c.b / (1.0 + factor), a: c.a }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
}

impl GridPreview {
    pub fn view<'a, M: 'a>(self, height: f32) -> Element<'a, M> {
        canvas::Canvas::new(self).width(Fill).height(height).into()
    }

    fn paint(&self, frame: &mut Frame) {
        let a = |c: Color| with_alpha(c, self.opacity);
        let screen = fit(frame.size(), self.ratio);
        frame.fill(&Path::rounded_rectangle(screen.position(), screen.size(), 4.0.into()), a(color!(0x2b333c)));
        let inner = Rectangle {
            x: screen.x + BEZEL,
            y: screen.y + BEZEL,
            width: (screen.width - 2.0 * BEZEL).max(1.0),
            height: (screen.height - 2.0 * BEZEL).max(1.0),
        };
        frame.fill_rectangle(inner.position(), inner.size(), a(color!(0x11161c)));

        let (cols, rows) = (self.cols.max(1) as f32, self.rows.max(1) as f32);
        let line = a(with_alpha(self.line, 0.8));
        for i in 1..self.cols.max(1) {
            frame.fill_rectangle(Point::new(inner.x + inner.width * i as f32 / cols - 0.5, inner.y), Size::new(1.0, inner.height), line);
        }
        for j in 1..self.rows.max(1) {
            frame.fill_rectangle(Point::new(inner.x, inner.y + inner.height * j as f32 / rows - 0.5), Size::new(inner.width, 1.0), line);
        }

        let (c0, c1, r0, r1) = selection(self.cols, self.rows);
        let sel = Rectangle {
            x: inner.x + inner.width * c0 as f32 / cols,
            y: inner.y + inner.height * r0 as f32 / rows,
            width: inner.width * (c1 - c0) as f32 / cols,
            height: inner.height * (r1 - r0) as f32 / rows,
        };
        if self.fill {
            if self.radial {
                self.paint_radial(frame, sel);
            } else {
                let g = gradient::Linear::new(sel.position(), Point::new(sel.x + sel.width, sel.y + sel.height))
                    .add_stop(0.0, a(self.start))
                    .add_stop(1.0, a(self.end));
                frame.fill(&Path::rounded_rectangle(sel.position(), sel.size(), 3.0.into()), canvas::Fill::from(canvas::Gradient::Linear(g)));
            }
        }
        let outline = Path::rounded_rectangle(
            Point::new(sel.x + 1.0, sel.y + 1.0),
            Size::new((sel.width - 2.0).max(1.0), (sel.height - 2.0).max(1.0)),
            3.0.into(),
        );
        frame.stroke(&outline, Stroke::default().with_width(2.0).with_color(a(self.line)));
    }

    /// Ellipses centred in the selection, from the start colour at the edge to a darker one
    /// in the middle. (iced's canvas has linear gradients only.)
    fn paint_radial(&self, frame: &mut Frame, sel: Rectangle) {
        let (start, dark) = (with_alpha(self.start, self.opacity), darker(with_alpha(self.start, self.opacity), 0.35));
        for k in 0..RADIAL_STEPS {
            let t = 1.0 - k as f32 / RADIAL_STEPS as f32;
            let points = ring(sel, t);
            let path = Path::new(|b| {
                b.move_to(points[0]);
                for p in &points[1..] {
                    b.line_to(*p);
                }
                b.close();
            });
            frame.fill(&path, mix(dark, start, t));
        }
    }
}

/// The outline of one step of the radial gradient: the ellipse that touches the corners of
/// `sel` when `t` is 1, scaled by `t`, and kept inside `sel`. (`Frame::with_clip` does not
/// clip in iced's software renderer, so the clipping is done here.)
pub fn ring(sel: Rectangle, t: f32) -> Vec<Point> {
    const SAMPLES: usize = 96;
    let (hw, hh) = (sel.width / 2.0, sel.height / 2.0);
    let (rx, ry) = (hw * std::f32::consts::SQRT_2 * t, hh * std::f32::consts::SQRT_2 * t);
    let center = Point::new(sel.x + hw, sel.y + hh);
    (0..SAMPLES)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / SAMPLES as f32;
            Point::new(center.x + (rx * a.cos()).clamp(-hw, hw), center.y + (ry * a.sin()).clamp(-hh, hh))
        })
        .collect()
}

impl<M> canvas::Program<M> for GridPreview {
    /// Drawn again only when one of the preview's own inputs changes (see [`Memo`]): drawing it
    /// fresh every frame made every hover anywhere in the window repaint the preview.
    type State = Memo<GridPreview>;

    fn draw(&self, memo: &Self::State, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<canvas::Geometry> {
        vec![memo.draw(*self, renderer, bounds.size(), |frame| self.paint(frame))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_is_the_middle_half() {
        assert_eq!(selection(12, 6), (3, 9, 1, 5));
        assert_eq!(selection(4, 4), (1, 3, 1, 3));
    }

    #[test]
    fn selection_is_never_empty_and_stays_inside() {
        for n in 1..=64 {
            let (c0, c1, _, _) = selection(n, 1);
            assert!(c0 < c1 && c1 <= n, "{n}: {c0}..{c1}");
        }
        assert_eq!(selection(1, 1), (0, 1, 0, 1));
        assert_eq!(selection(0, 0), (0, 1, 0, 1), "a degenerate grid is treated as 1x1");
    }

    #[test]
    fn fit_letterboxes_to_the_ratio() {
        // Wider than 16:9: bars left and right.
        let r = fit(Size::new(800.0, 300.0), 16.0 / 9.0);
        assert!((r.height - 300.0).abs() < 1e-3 && (r.width - 300.0 * 16.0 / 9.0).abs() < 1e-3);
        assert!((r.x - (800.0 - r.width) / 2.0).abs() < 1e-3 && r.y == 0.0);
        // Taller: bars above and below.
        let r = fit(Size::new(320.0, 400.0), 2.0);
        assert!((r.width - 320.0).abs() < 1e-3 && (r.height - 160.0).abs() < 1e-3);
        assert!((r.y - 120.0).abs() < 1e-3 && r.x == 0.0);
        // A nonsense ratio falls back to 16:9.
        assert_eq!(fit(Size::new(160.0, 90.0), 0.0).width, 160.0);
    }

    #[test]
    fn radial_rings_stay_inside_the_selection_and_the_outer_one_fills_it() {
        let sel = Rectangle { x: 100.0, y: 50.0, width: 200.0, height: 80.0 };
        let inside = |p: &Point| p.x >= sel.x - 1e-3 && p.x <= sel.x + sel.width + 1e-3 && p.y >= sel.y - 1e-3 && p.y <= sel.y + sel.height + 1e-3;
        for k in 0..=RADIAL_STEPS {
            let t = 1.0 - k as f32 / RADIAL_STEPS as f32;
            assert!(ring(sel, t).iter().all(inside), "ring {t} leaves the selection");
        }
        // The outermost ring reaches every side of the selection (the corners are clamped to it).
        let outer = ring(sel, 1.0);
        let min_x = outer.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        let max_x = outer.iter().map(|p| p.x).fold(f32::MIN, f32::max);
        let min_y = outer.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        let max_y = outer.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        assert_eq!((min_x, max_x, min_y, max_y), (100.0, 300.0, 50.0, 130.0));
        // Smaller rings shrink toward the middle of the selection.
        let small = ring(sel, 0.25);
        assert!(small.iter().all(|p| (p.x - 200.0).abs() < 80.0 && (p.y - 90.0).abs() < 30.0));
    }

    #[test]
    fn colour_helpers() {
        let c = Color::from_rgb(0.5, 0.5, 0.5);
        assert!((darker(c, 0.35).r - 0.5 / 1.35).abs() < 1e-6);
        assert_eq!(mix(Color::BLACK, Color::WHITE, 0.5).r, 0.5);
        assert_eq!(mix(Color::BLACK, Color::WHITE, 1.0), Color::WHITE);
    }
}
