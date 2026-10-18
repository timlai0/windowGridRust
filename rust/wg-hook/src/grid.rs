//! The hook's grid maths: pointer position -> cell, selection fitting, cell -> pixel rect.
//!
//! Pure functions with no platform dependency, specified by ADR 005. Integer division
//! truncates (like `idiv`), cell/size conversion divides first and then multiplies in
//! `f64` (like the original), so rounding at exact cell boundaries matches.

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
        self.right.wrapping_sub(self.left)
    }

    pub fn height(&self) -> i32 {
        self.bottom.wrapping_sub(self.top)
    }
}

/// The grid laid over one monitor's work area.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Layout {
    pub cols: i32,
    pub rows: i32,
    pub x0: i32,
    pub y0: i32,
    pub w: i32,
    pub h: i32,
}

impl Layout {
    /// `grid_x`/`grid_y` are the configured columns/rows. On a portrait monitor
    /// (width < height) the two are swapped. Returns `None` for unusable geometry.
    pub fn new(grid_x: i32, grid_y: i32, work: Rect, monitor: Rect) -> Option<Layout> {
        let landscape = monitor.width() >= monitor.height();
        let (cols, rows) = if landscape { (grid_x, grid_y) } else { (grid_y, grid_x) };
        let (w, h) = (work.width(), work.height());
        if cols < 1 || rows < 1 || w < 1 || h < 1 {
            return None;
        }
        Some(Layout { cols, rows, x0: work.left, y0: work.top, w, h })
    }

    /// Centre of the work area, sent to the overlay so it can find the monitor.
    pub fn center(&self) -> Point {
        Point::new(self.x0.wrapping_add(self.w / 2), self.y0.wrapping_add(self.h / 2))
    }
}

/// Window sizes in pixels that bound the selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Sizes {
    /// `ptMinTrackSize`.
    pub min: Point,
    /// `ptMaxTrackSize` (components `<= 0` already replaced by `i32::MAX`).
    pub max: Point,
    /// The window's size when the session started.
    pub orig: Point,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    pub start: Point,
    pub cur: Point,
    /// Inclusive cell rectangle.
    pub cells: Rect,
    /// Pixel rectangle of those cells (before DWM margin compensation).
    pub pixels: Rect,
}

fn trunc(x: f64) -> f64 {
    if x >= 2147483647.0 {
        2147483647.0
    } else if x <= -2147483648.0 {
        -2147483648.0
    } else {
        (x as i32) as f64
    }
}

fn ceil(x: f64) -> f64 {
    let t = trunc(x);
    if t < x {
        t + 1.0
    } else {
        t
    }
}

fn floor(x: f64) -> f64 {
    let t = trunc(x);
    if t > x {
        t - 1.0
    } else {
        t
    }
}

/// Rounds half away from zero.
fn round(x: f64) -> f64 {
    let t = trunc(x);
    let d = x - t;
    if d >= 0.5 {
        t + 1.0
    } else if d <= -0.5 {
        t - 1.0
    } else {
        t
    }
}

fn convert(px: i32, size: i32, n: i32, f: fn(f64) -> f64) -> i32 {
    f((px as f64 / size as f64) * n as f64) as i32
}

fn sizes_in_cells(layout: &Layout, v: Point, f: fn(f64) -> f64) -> Point {
    Point::new(convert(v.x, layout.w, layout.cols, f), convert(v.y, layout.h, layout.rows, f))
}

/// Grid cell under `pt`, clamped to the grid.
pub fn locate(layout: &Layout, pt: Point) -> Point {
    let cx = pt.x.wrapping_sub(layout.x0).wrapping_mul(layout.cols) / layout.w;
    let cy = pt.y.wrapping_sub(layout.y0).wrapping_mul(layout.rows) / layout.h;
    Point::new(cx.clamp(0, layout.cols - 1), cy.clamp(0, layout.rows - 1))
}

/// Grows or shrinks `[left, right]` symmetrically (extra cell on the right) so that its
/// span fits `[min-1, max-1]`.
fn fit_axis(left: &mut i32, right: &mut i32, min: i32, max: i32) {
    let span = *right - *left;
    let lower = min.wrapping_sub(1);
    let upper = max.wrapping_sub(1);
    let target = if span < lower {
        lower
    } else if span > upper {
        upper
    } else {
        return;
    };
    let delta = target - span;
    let half = delta / 2;
    *left -= half;
    *right += delta - half;
}

/// Moves/shrinks `[left, right]` back inside `0..n`.
fn clamp_axis(left: &mut i32, right: &mut i32, n: i32) {
    let span = *right - *left;
    let last = n - 1;
    if span > last {
        *left = 0;
        *right = last;
    } else if *right > last {
        *right = last;
        *left = last - span;
    } else if *left < 0 {
        *left = 0;
        *right = span;
    }
}

/// Computes the selection for the pointer position.
///
/// * `prev_start` is the start cell kept from the previous update; it is replaced by the
///   current cell when `reset_start` is set.
/// * In `move_mode` the selection is forced to the window's own size in cells.
pub fn select(
    layout: &Layout,
    pt: Point,
    prev_start: Point,
    reset_start: bool,
    move_mode: bool,
    sizes: &Sizes,
) -> Selection {
    let cur = locate(layout, pt);
    let start = if reset_start { cur } else { prev_start };

    let mut min = sizes_in_cells(layout, sizes.min, ceil);
    let mut max = sizes_in_cells(layout, sizes.max, floor);
    if move_mode {
        let size = sizes_in_cells(layout, sizes.orig, round);
        min = Point::new(min.x.max(size.x), min.y.max(size.y));
        max = Point::new(max.x.min(size.x), max.y.min(size.y));
    }

    let (mut left, mut right) = if start.x <= cur.x { (start.x, cur.x) } else { (cur.x, start.x) };
    let (mut top, mut bottom) = if start.y <= cur.y { (start.y, cur.y) } else { (cur.y, start.y) };

    fit_axis(&mut left, &mut right, min.x, max.x);
    fit_axis(&mut top, &mut bottom, min.y, max.y);
    clamp_axis(&mut left, &mut right, layout.cols);
    clamp_axis(&mut top, &mut bottom, layout.rows);

    let cells = Rect::new(left, top, right, bottom);
    let pixels = Rect::new(
        left.wrapping_mul(layout.w) / layout.cols + layout.x0,
        top.wrapping_mul(layout.h) / layout.rows + layout.y0,
        (right + 1).wrapping_mul(layout.w) / layout.cols + layout.x0,
        (bottom + 1).wrapping_mul(layout.h) / layout.rows + layout.y0,
    );
    Selection { start, cur, cells, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::new(12, 6, Rect::new(0, 0, 1920, 1080), Rect::new(0, 0, 1920, 1080)).unwrap()
    }

    fn free_sizes() -> Sizes {
        Sizes { min: Point::new(0, 0), max: Point::new(i32::MAX, i32::MAX), orig: Point::new(800, 600) }
    }

    #[test]
    fn rounding_helpers() {
        assert_eq!((ceil(2.2), ceil(2.5), ceil(4.0), ceil(-0.5)), (3.0, 3.0, 4.0, 0.0));
        assert_eq!((floor(2.7), floor(3.5), floor(4.0), floor(-0.5)), (2.0, 3.0, 4.0, -1.0));
        assert_eq!((round(2.2), round(2.5), round(2.7), round(-0.5)), (2.0, 3.0, 3.0, -1.0));
    }

    #[test]
    fn portrait_monitor_swaps_grid() {
        let l = Layout::new(12, 6, Rect::new(0, 0, 1080, 1920), Rect::new(0, 0, 1080, 1920)).unwrap();
        assert_eq!((l.cols, l.rows), (6, 12));
        let square = Layout::new(12, 6, Rect::new(0, 0, 1000, 1000), Rect::new(0, 0, 1000, 1000)).unwrap();
        assert_eq!((square.cols, square.rows), (12, 6), "square counts as landscape");
    }

    #[test]
    fn unusable_geometry_is_rejected() {
        let r = Rect::new(0, 0, 100, 100);
        assert!(Layout::new(0, 6, r, r).is_none());
        assert!(Layout::new(12, 6, Rect::new(0, 0, 0, 100), r).is_none());
    }

    #[test]
    fn center_uses_work_area() {
        let l = Layout::new(12, 6, Rect::new(-1920, 0, 0, 1040), Rect::new(-1920, 0, 0, 1080)).unwrap();
        assert_eq!(l.center(), Point::new(-960, 520));
    }

    #[test]
    fn locate_clamps_to_grid() {
        let l = layout();
        assert_eq!(locate(&l, Point::new(0, 0)), Point::new(0, 0));
        assert_eq!(locate(&l, Point::new(960, 540)), Point::new(6, 3));
        assert_eq!(locate(&l, Point::new(1919, 1079)), Point::new(11, 5));
        assert_eq!(locate(&l, Point::new(5000, -50)), Point::new(11, 0));
        assert_eq!(locate(&l, Point::new(-50, 5000)), Point::new(0, 5));
    }

    #[test]
    fn locate_respects_work_area_offset() {
        let l = Layout::new(10, 4, Rect::new(1920, 100, 3840, 1100), Rect::new(1920, 0, 3840, 1200)).unwrap();
        assert_eq!(locate(&l, Point::new(1920, 100)), Point::new(0, 0));
        assert_eq!(locate(&l, Point::new(2112, 350)), Point::new(1, 1));
    }

    #[test]
    fn resize_selection_between_two_cells() {
        let l = layout();
        let s = select(&l, Point::new(1000, 400), Point::new(2, 1), false, false, &free_sizes());
        assert_eq!(s.start, Point::new(2, 1));
        assert_eq!(s.cur, Point::new(6, 2));
        assert_eq!(s.cells, Rect::new(2, 1, 6, 2));
        assert_eq!(s.pixels, Rect::new(320, 180, 1120, 540));
    }

    #[test]
    fn selection_is_normalised_when_dragging_up_left() {
        let l = layout();
        let s = select(&l, Point::new(100, 100), Point::new(5, 3), false, false, &free_sizes());
        assert_eq!(s.cells, Rect::new(0, 0, 5, 3));
    }

    #[test]
    fn holding_the_key_collapses_selection_to_one_cell() {
        let l = layout();
        let s = select(&l, Point::new(1000, 400), Point::new(0, 0), true, false, &free_sizes());
        assert_eq!(s.start, s.cur);
        assert_eq!(s.cells, Rect::new(6, 2, 6, 2));
    }

    #[test]
    fn move_mode_keeps_window_size_in_cells() {
        // 800x600 window on a 12x6 grid of 160x180 cells: round(5.0) x round(3.33) = 5 x 3.
        let s = select(&layout(), Point::new(960, 540), Point::new(0, 0), true, true, &free_sizes());
        assert_eq!(s.cells.width() + 1, 5);
        assert_eq!(s.cells.height() + 1, 3);
        // Centred on the pointer cell (6, 3): grows by 2 left, 2 right and 1 up, 1 down.
        assert_eq!(s.cells, Rect::new(4, 2, 8, 4));
    }

    #[test]
    fn move_mode_clamps_against_grid_edges() {
        let s = select(&layout(), Point::new(1919, 1079), Point::new(0, 0), true, true, &free_sizes());
        assert_eq!(s.cells, Rect::new(7, 3, 11, 5));
        assert_eq!(s.pixels, Rect::new(1120, 540, 1920, 1080));
        let s = select(&layout(), Point::new(0, 0), Point::new(0, 0), true, true, &free_sizes());
        assert_eq!(s.cells, Rect::new(0, 0, 4, 2));
    }

    #[test]
    fn minimum_size_expands_the_selection() {
        // min track 500x400 -> ceil(3.125)=4 cols, ceil(2.22)=3 rows.
        let sizes = Sizes { min: Point::new(500, 400), ..free_sizes() };
        let s = select(&layout(), Point::new(400, 100), Point::new(2, 0), false, false, &sizes);
        // pointer cell (2,0), start (2,0): one cell wide -> grown to 4x3 from (2,0)
        assert_eq!(s.cells.width() + 1, 4);
        assert_eq!(s.cells.height() + 1, 3);
    }

    #[test]
    fn maximum_size_shrinks_the_selection() {
        // max track 400x300 -> floor(2.5)=2 cols, floor(1.67)=1 row.
        let sizes = Sizes { max: Point::new(400, 300), ..free_sizes() };
        let s = select(&layout(), Point::new(1900, 1000), Point::new(0, 0), false, false, &sizes);
        assert_eq!(s.cells.width() + 1, 2);
        assert_eq!(s.cells.height() + 1, 1);
    }

    #[test]
    fn odd_extra_cell_goes_to_the_right() {
        // Grow a 1-cell selection to 4 cells: delta 3 -> half 1 -> left-1, right+2.
        let sizes = Sizes { min: Point::new(640, 0), ..free_sizes() };
        let s = select(&layout(), Point::new(960, 540), Point::new(0, 0), true, false, &sizes);
        assert_eq!((s.cells.left, s.cells.right), (5, 8));
    }

    #[test]
    fn pixel_rects_tile_without_gaps() {
        let l = Layout::new(7, 5, Rect::new(0, 0, 1000, 1000), Rect::new(0, 0, 1000, 1000)).unwrap();
        for col in 0..6 {
            let a = select(&l, Point::new(col * 143, 0), Point::new(0, 0), true, false, &free_sizes());
            let b = select(&l, Point::new((col + 1) * 143 + 20, 0), Point::new(0, 0), true, false, &free_sizes());
            if b.cells.left == a.cells.right + 1 {
                assert_eq!(a.pixels.right, b.pixels.left);
            }
        }
    }
}
