//! Where the target window goes when the drag ends (ported from the legacy overlay's
//! `SnapTargetWindow`, plus the previously unimplemented `GrabPoint` anchor).

use crate::geom::{round_even, Point, Rect};
use wg_core::settings::WindowAnchor;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// No drag in progress.
    Idle,
    Resize,
    Move,
}

impl Mode {
    pub fn from_wparam(w: usize) -> Option<Mode> {
        match w {
            0 => Some(Mode::Idle),
            1 => Some(Mode::Resize),
            2 => Some(Mode::Move),
            _ => None,
        }
    }
}

/// Everything the snap needs; no platform calls.
#[derive(Clone, Copy, Debug)]
pub struct SnapParams {
    /// Monitor work area, physical pixels.
    pub work: Rect,
    pub cols: i32,
    pub rows: i32,
    /// Selected cells (inclusive).
    pub min: Point,
    pub max: Point,
    pub previous: Mode,
    pub current: Mode,
    /// The target window's current rectangle (`GetWindowRect`), if known.
    pub window: Option<Rect>,
    /// The target's visible frame (`DWMWA_EXTENDED_FRAME_BOUNDS`), if known.
    pub frame: Option<Rect>,
    pub anchor: WindowAnchor,
    pub compensate_margins: bool,
    pub preserve_size_on_move: bool,
    /// Pointer position, used by the `GrabPoint` anchor.
    pub cursor: Option<Point>,
}

/// Position and size to hand to `SetWindowPos`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SnapRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Returns `None` when there is nothing valid to snap to.
pub fn compute(p: &SnapParams) -> Option<SnapRect> {
    if p.min.x < 0 || p.min.y < 0 || p.cols <= 0 || p.rows <= 0 {
        return None;
    }
    let cell_w = f64::from(p.work.width()) / f64::from(p.cols);
    let cell_h = f64::from(p.work.height()) / f64::from(p.rows);
    let sel_x = round_even(f64::from(p.work.left) + f64::from(p.min.x) * cell_w) as i32;
    let sel_y = round_even(f64::from(p.work.top) + f64::from(p.min.y) * cell_h) as i32;
    let sel_w = round_even(f64::from(p.max.x - p.min.x + 1) * cell_w) as i32;
    let sel_h = round_even(f64::from(p.max.y - p.min.y + 1) * cell_h) as i32;

    let (mut x, mut y, mut w, mut h) = (sel_x, sel_y, sel_w, sel_h);

    // The invisible resize border around the visible frame (DWM). When it is compensated
    // below, sizes must be measured on the *visible* frame, otherwise preserving a window's
    // size would add the border to it again on every snap (the legacy overlay grew a moved
    // window by about 14 x 7 px each time).
    let margins = if p.compensate_margins {
        p.window.zip(p.frame).and_then(|(win, frame)| {
            let m = (frame.left - win.left, frame.top - win.top, win.right - frame.right, win.bottom - frame.bottom);
            (m.0 >= 0 && m.1 >= 0 && m.2 >= 0 && m.3 >= 0).then_some(m)
        })
    } else {
        None
    };
    let current = if margins.is_some() { p.frame } else { p.window };
    let (cur_w, cur_h) = current.map_or((w, h), |r| (r.width(), r.height()));

    let is_resize = p.current == Mode::Resize || p.previous == Mode::Resize;
    if !is_resize && p.previous == Mode::Move && p.preserve_size_on_move && p.window.is_some() {
        w = cur_w;
        h = cur_h;
    }

    if !is_resize && p.window.is_some() {
        match p.anchor {
            WindowAnchor::TopLeft => {}
            WindowAnchor::Center => {
                if cur_w > w {
                    x -= (cur_w - w) / 2;
                }
                if cur_h > h {
                    y -= (cur_h - h) / 2;
                }
            }
            WindowAnchor::GrabPoint => {
                // The oversized window extends away from the pointer: with the pointer at the
                // left edge of the selection it grows to the right, at the right edge to the
                // left, in between proportionally.
                if let Some(c) = p.cursor {
                    if cur_w > w {
                        x -= overflow_share(cur_w - w, c.x - sel_x, sel_w);
                    }
                    if cur_h > h {
                        y -= overflow_share(cur_h - h, c.y - sel_y, sel_h);
                    }
                }
            }
        }
    }

    if let Some((left, top, right, bottom)) = margins {
        x -= left;
        y -= top;
        w += left + right;
        h += top + bottom;
    }

    Some(SnapRect { x, y, w, h })
}

/// How the overlay checks that a snap really landed.
///
/// The snap is a fire-and-forget `SWP_ASYNCWINDOWPOS`, and plenty can undo it afterwards: the
/// target's own handling of `WM_EXITSIZEMOVE`, an Aero Snap or maximize triggered by the
/// pointer sitting at the screen edge, a `WM_DPICHANGED` resize after crossing to a monitor
/// with another scale. So the overlay polls the window for a short while and re-applies the
/// rectangle until it sticks.
pub const VERIFY_TICK_MS: u32 = 40;
/// Give up after this many polls (about half a second).
pub const VERIFY_MAX_TICKS: u32 = 14;
/// Consecutive matching polls that count as "it stuck".
pub const VERIFY_STABLE_TICKS: u32 = 3;
/// Re-applications before accepting the window's own idea (e.g. a minimum size).
pub const VERIFY_MAX_REAPPLY: u32 = 3;
const VERIFY_TOLERANCE: i32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerifyStep {
    /// Look again on the next poll.
    Wait,
    /// The window is where it should be and stayed there.
    Done,
    /// It is somewhere else: apply the rectangle again.
    Reapply,
    /// Out of attempts or time; the window refuses (or is still being dragged).
    GiveUp,
}

#[derive(Clone, Copy, Debug)]
pub struct Verify {
    pub want: SnapRect,
    ticks: u32,
    stable: u32,
    reapplied: u32,
    /// Polls to skip after a re-application so the async request can be processed.
    cooldown: u32,
}

impl Verify {
    pub fn new(want: SnapRect) -> Verify {
        Verify { want, ticks: 0, stable: 0, reapplied: 0, cooldown: 0 }
    }

    pub fn reapplied(&self) -> u32 {
        self.reapplied
    }

    /// One poll. `actual` is `GetWindowRect`, `in_loop` whether the target is (again) inside a
    /// move/size loop.
    pub fn tick(&mut self, actual: Rect, zoomed: bool, in_loop: bool) -> VerifyStep {
        self.ticks += 1;
        if in_loop {
            // A new drag took the window over: stay out of its way.
            return VerifyStep::GiveUp;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return if self.ticks >= VERIFY_MAX_TICKS { VerifyStep::GiveUp } else { VerifyStep::Wait };
        }
        let w = self.want;
        let close = |a: i32, b: i32| (a - b).abs() <= VERIFY_TOLERANCE;
        let matches = !zoomed
            && close(actual.left, w.x)
            && close(actual.top, w.y)
            && close(actual.right, w.x + w.w)
            && close(actual.bottom, w.y + w.h);
        if matches {
            self.stable += 1;
            return if self.stable >= VERIFY_STABLE_TICKS { VerifyStep::Done } else { VerifyStep::Wait };
        }
        self.stable = 0;
        if self.reapplied >= VERIFY_MAX_REAPPLY || self.ticks >= VERIFY_MAX_TICKS {
            return VerifyStep::GiveUp;
        }
        self.reapplied += 1;
        self.cooldown = 1;
        VerifyStep::Reapply
    }
}

/// `overflow * clamp(offset / extent, 0, 1)`, rounded.
fn overflow_share(overflow: i32, offset: i32, extent: i32) -> i32 {
    if extent <= 0 {
        return overflow / 2;
    }
    let f = (f64::from(offset) / f64::from(extent)).clamp(0.0, 1.0);
    round_even(f64::from(overflow) * f) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SnapParams {
        SnapParams {
            work: Rect::new(0, 0, 1920, 1080),
            cols: 12,
            rows: 6,
            min: Point::new(2, 1),
            max: Point::new(5, 3),
            previous: Mode::Resize,
            current: Mode::Idle,
            window: None,
            frame: None,
            anchor: WindowAnchor::TopLeft,
            compensate_margins: true,
            preserve_size_on_move: true,
            cursor: None,
        }
    }

    fn want() -> SnapRect {
        SnapRect { x: 100, y: 50, w: 400, h: 300 }
    }

    fn at_want() -> Rect {
        Rect::new(100, 50, 500, 350)
    }

    #[test]
    fn verify_finishes_once_the_window_stays_put() {
        let mut v = Verify::new(want());
        assert_eq!(v.tick(at_want(), false, false), VerifyStep::Wait);
        assert_eq!(v.tick(Rect::new(101, 50, 500, 349), false, false), VerifyStep::Wait, "1px of slack");
        assert_eq!(v.tick(at_want(), false, false), VerifyStep::Done);
        assert_eq!(v.reapplied(), 0);
    }

    #[test]
    fn verify_reapplies_when_the_window_was_moved_back() {
        let mut v = Verify::new(want());
        assert_eq!(v.tick(at_want(), false, false), VerifyStep::Wait);
        let stale = Rect::new(300, 300, 700, 600);
        assert_eq!(v.tick(stale, false, false), VerifyStep::Reapply);
        assert_eq!(v.tick(stale, false, false), VerifyStep::Wait, "cool-down while the async request is in flight");
        assert_eq!(v.tick(at_want(), false, false), VerifyStep::Wait);
        assert_eq!(v.tick(at_want(), false, false), VerifyStep::Wait);
        assert_eq!(v.tick(at_want(), false, false), VerifyStep::Done);
        assert_eq!(v.reapplied(), 1);
    }

    #[test]
    fn verify_treats_a_maximized_window_as_not_snapped() {
        let mut v = Verify::new(want());
        assert_eq!(v.tick(at_want(), true, false), VerifyStep::Reapply);
    }

    #[test]
    fn verify_gives_up_on_a_window_that_refuses() {
        let mut v = Verify::new(want());
        let stubborn = Rect::new(100, 50, 700, 350); // minimum width wins
        let mut steps = Vec::new();
        for _ in 0..30 {
            let step = v.tick(stubborn, false, false);
            steps.push(step);
            if step == VerifyStep::GiveUp {
                break;
            }
        }
        assert_eq!(steps.last(), Some(&VerifyStep::GiveUp));
        assert_eq!(v.reapplied(), VERIFY_MAX_REAPPLY);
        assert!(steps.len() <= VERIFY_MAX_TICKS as usize);
    }

    #[test]
    fn verify_backs_off_when_a_new_drag_starts() {
        let mut v = Verify::new(want());
        assert_eq!(v.tick(Rect::new(0, 0, 10, 10), false, true), VerifyStep::GiveUp);
    }

    #[test]
    fn plain_resize_fills_the_selected_cells() {
        let r = compute(&base()).unwrap();
        assert_eq!(r, SnapRect { x: 320, y: 180, w: 640, h: 540 });
    }

    #[test]
    fn invalid_selection_is_ignored() {
        let mut p = base();
        p.min = Point::new(-1, -1);
        assert_eq!(compute(&p), None);
        let mut p = base();
        p.cols = 0;
        assert_eq!(compute(&p), None);
    }

    #[test]
    fn work_area_offset_is_applied() {
        let mut p = base();
        p.work = Rect::new(1920, 40, 3840, 1080);
        let r = compute(&p).unwrap();
        assert_eq!((r.x, r.y), (1920 + 320, 40 + 173));
    }

    #[test]
    fn dwm_margins_expand_the_rect() {
        let mut p = base();
        p.window = Some(Rect::new(100, 100, 900, 700));
        p.frame = Some(Rect::new(107, 100, 893, 693));
        let r = compute(&p).unwrap();
        // left/top margins are 7 and 0, right/bottom 7 and 7.
        assert_eq!(r, SnapRect { x: 320 - 7, y: 180, w: 640 + 14, h: 540 + 7 });
    }

    #[test]
    fn unexpected_margin_signs_are_ignored() {
        let mut p = base();
        p.window = Some(Rect::new(100, 100, 900, 700));
        p.frame = Some(Rect::new(90, 100, 893, 693)); // frame left of window: negative margin
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 320, y: 180, w: 640, h: 540 });
        p.compensate_margins = false;
        p.frame = Some(Rect::new(107, 107, 893, 693));
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 320, y: 180, w: 640, h: 540 });
    }

    #[test]
    fn move_keeps_the_window_size_when_configured() {
        let mut p = base();
        p.previous = Mode::Move;
        p.window = Some(Rect::new(0, 0, 800, 600));
        p.compensate_margins = false;
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 320, y: 180, w: 800, h: 600 });
        p.preserve_size_on_move = false;
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 320, y: 180, w: 640, h: 540 });
    }

    #[test]
    fn anchors_apply_to_oversized_windows_outside_resize() {
        let mut p = base();
        p.previous = Mode::Move;
        p.preserve_size_on_move = false;
        p.compensate_margins = false;
        p.window = Some(Rect::new(0, 0, 800, 700)); // bigger than 640 x 540
        p.anchor = WindowAnchor::Center;
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 320 - 80, y: 180 - 80, w: 640, h: 540 });
        p.anchor = WindowAnchor::TopLeft;
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 320, y: 180, w: 640, h: 540 });
    }

    #[test]
    fn grab_point_extends_away_from_the_pointer() {
        let mut p = base();
        p.previous = Mode::Move;
        p.preserve_size_on_move = false;
        p.compensate_margins = false;
        p.window = Some(Rect::new(0, 0, 840, 640)); // overflow 200 x 100
        p.anchor = WindowAnchor::GrabPoint;
        // Pointer at the selection's left edge: grows right, nothing shifts.
        p.cursor = Some(Point::new(320, 180));
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 320, y: 180, w: 640, h: 540 });
        // Pointer at the far right/bottom: grows left/up by the full overflow.
        p.cursor = Some(Point::new(960, 720));
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 120, y: 80, w: 640, h: 540 });
        // Pointer in the middle: half the overflow.
        p.cursor = Some(Point::new(640, 450));
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 220, y: 130, w: 640, h: 540 });
        // Pointer outside the selection is clamped to its edges.
        p.cursor = Some(Point::new(5000, -5000));
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 120, y: 180, w: 640, h: 540 });
    }

    #[test]
    fn anchors_do_not_apply_in_resize_mode() {
        let mut p = base();
        p.window = Some(Rect::new(0, 0, 800, 700));
        p.anchor = WindowAnchor::Center;
        p.compensate_margins = false;
        assert_eq!(compute(&p).unwrap(), SnapRect { x: 320, y: 180, w: 640, h: 540 });
    }
}

#[cfg(test)]
mod preserve_size_tests {
    use super::*;

    /// A window whose rectangle includes 7 px invisible borders left, right and bottom.
    fn moved_window() -> SnapParams {
        SnapParams {
            work: Rect::new(0, 0, 1920, 1080),
            cols: 12,
            rows: 6,
            min: Point::new(8, 3),
            max: Point::new(9, 3),
            previous: Mode::Move,
            current: Mode::Idle,
            window: Some(Rect::new(100, 100, 700, 500)), // 600 x 400 rectangle
            frame: Some(Rect::new(107, 100, 693, 493)),  // 586 x 393 visible
            anchor: WindowAnchor::TopLeft,
            compensate_margins: true,
            preserve_size_on_move: true,
            cursor: None,
        }
    }

    #[test]
    fn moving_a_window_does_not_change_its_visible_size() {
        let p = moved_window();
        let r = compute(&p).unwrap();
        // Visible frame after the move = rectangle minus the same margins as before.
        let (left, top, right, bottom) = (7, 0, 7, 7);
        let visible = (r.w - left - right, r.h - top - bottom);
        assert_eq!(visible, (586, 393), "size must be preserved, not grown by the margins");
        // And the visible frame starts exactly at the selected cell.
        assert_eq!((r.x + left, r.y + top), (8 * 160, 3 * 180));
    }
}
