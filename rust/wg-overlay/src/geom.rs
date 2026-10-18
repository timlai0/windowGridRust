//! Small geometry types and the cell/pixel conversions shared by the overlay's logic.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Point {
        Point { x, y }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// Floating-point rectangle in the overlay's pixel space.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl RectF {
    pub fn right(&self) -> f32 {
        self.left + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.top + self.height
    }
}

/// Inclusive cell selection, normalised so `min <= max`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cells {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

impl Cells {
    /// Normalises two corner cells.
    pub fn from_corners(sx: i32, sy: i32, ex: i32, ey: i32) -> Cells {
        Cells { min_x: sx.min(ex), min_y: sy.min(ey), max_x: sx.max(ex), max_y: sy.max(ey) }
    }

    pub fn span_x(&self) -> i32 {
        self.max_x - self.min_x + 1
    }

    pub fn span_y(&self) -> i32 {
        self.max_y - self.min_y + 1
    }
}

/// Rounds like .NET's `Math.Round(double)`: half to even.
pub fn round_even(v: f64) -> f64 {
    v.round_ties_even()
}

/// Pixel x of the boundary before column `i` (`0..=cols`), rounded as the legacy overlay did.
pub fn boundary(extent: f64, i: i32, n: i32) -> f64 {
    round_even(extent * f64::from(i) / f64::from(n))
}

/// The selection's rectangle in overlay pixels, at least 10x10, from boundary rounding.
pub fn selection_rect(cells: Cells, cols: i32, rows: i32, width: f64, height: f64) -> RectF {
    let left = boundary(width, cells.min_x, cols);
    let top = boundary(height, cells.min_y, rows);
    let right = boundary(width, cells.max_x + 1, cols);
    let bottom = boundary(height, cells.max_y + 1, rows);
    RectF {
        left: left as f32,
        top: top as f32,
        width: (right - left).max(10.0) as f32,
        height: (bottom - top).max(10.0) as f32,
    }
}

/// Cell under a monitor-relative position, clamped to the grid (used for the initial selection).
pub fn cell_at(rel: Point, work_w: i32, work_h: i32, cols: i32, rows: i32) -> Point {
    if work_w <= 0 || work_h <= 0 {
        return Point::new(0, 0);
    }
    let x = (f64::from(rel.x) * f64::from(cols) / f64::from(work_w)) as i32;
    let y = (f64::from(rel.y) * f64::from(rows) / f64::from(work_h)) as i32;
    Point::new(x.clamp(0, (cols - 1).max(0)), y.clamp(0, (rows - 1).max(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_is_half_to_even() {
        assert_eq!(round_even(0.5), 0.0);
        assert_eq!(round_even(1.5), 2.0);
        assert_eq!(round_even(2.5), 2.0);
        assert_eq!(round_even(-0.5), -0.0);
    }

    #[test]
    fn selection_rect_matches_legacy_rounding() {
        // 1920 wide, 12 columns -> 160 px; 1080 high, 6 rows -> 180 px.
        let r = selection_rect(Cells::from_corners(2, 1, 5, 3), 12, 6, 1920.0, 1080.0);
        assert_eq!((r.left, r.top, r.width, r.height), (320.0, 180.0, 640.0, 540.0));
        // Odd split: 1000 / 7 = 142.857..: boundaries round individually, so cells differ by 1 px.
        let a = selection_rect(Cells::from_corners(0, 0, 0, 0), 7, 7, 1000.0, 1000.0);
        let b = selection_rect(Cells::from_corners(1, 0, 1, 0), 7, 7, 1000.0, 1000.0);
        assert_eq!(a.width, 143.0);
        assert_eq!(b.left, 143.0);
        assert_eq!(b.width, 143.0);
    }

    #[test]
    fn selection_has_a_minimum_size() {
        let r = selection_rect(Cells::from_corners(0, 0, 0, 0), 500, 500, 600.0, 600.0);
        assert_eq!((r.width, r.height), (10.0, 10.0));
    }

    #[test]
    fn corners_are_normalised() {
        let c = Cells::from_corners(5, 4, 2, 7);
        assert_eq!((c.min_x, c.min_y, c.max_x, c.max_y), (2, 4, 5, 7));
        assert_eq!((c.span_x(), c.span_y()), (4, 4));
    }

    #[test]
    fn cell_at_clamps() {
        assert_eq!(cell_at(Point::new(960, 540), 1920, 1080, 12, 6), Point::new(6, 3));
        assert_eq!(cell_at(Point::new(5000, -10), 1920, 1080, 12, 6), Point::new(11, 0));
        assert_eq!(cell_at(Point::new(10, 10), 0, 0, 12, 6), Point::new(0, 0));
    }
}
