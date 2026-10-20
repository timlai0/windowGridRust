//! The per-drag state machine (ADR 005): input commands, mode flags, and the messages the
//! hook posts to the overlay. All side effects go through [`Host`], so the sequencing can
//! be tested without any real windows.

use crate::grid::{self, Layout, Point, Rect, Selection, Sizes};
use wg_core::ipc::{pack_point, InputMemory, MAX_INPUTS};
use wg_core::keycode::Command;

pub const WM_MOVING: u32 = 0x0216;
pub const WM_SIZING: u32 = 0x0214;
pub const WM_WINDOWPOSCHANGING: u32 = 0x0046;
pub const WM_CAPTURECHANGED: u32 = 0x0215;
pub const WM_NCDESTROY: u32 = 0x0082;
pub const WM_GETMINMAXINFO: u32 = 0x0024;
pub const WM_EXITSIZEMOVE: u32 = 0x0232;

pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;

/// Which overlay message to post.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Msg {
    ShowGrid,
    Area,
    /// `wParam` = the dragged window's handle, so the overlay snaps exactly that window
    /// instead of guessing from the pointer position.
    Target,
}

/// Geometry of the monitor the pointer is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MonitorGeometry {
    pub work: Rect,
    pub monitor: Rect,
}

/// What the window currently looks like.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WindowMetrics {
    /// `GetWindowRect`.
    pub window: Rect,
    /// `DWMWA_EXTENDED_FRAME_BOUNDS`, if available.
    pub frame: Option<Rect>,
    pub maximized: bool,
}

/// Platform services the session needs.
pub trait Host {
    fn cursor_pos(&mut self) -> Point;
    /// `MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)`; 0 on failure.
    fn monitor_from_point(&mut self, pt: Point) -> isize;
    fn monitor_geometry(&mut self, monitor: isize) -> Option<MonitorGeometry>;
    fn window_metrics(&mut self, hwnd: isize) -> WindowMetrics;
    /// `PostMessage(overlay, msg, wparam, lparam)`.
    fn post_overlay(&mut self, overlay: isize, msg: Msg, wparam: usize, lparam: isize);
    fn set_cursor_pos(&mut self, x: i32, y: i32);
    /// `ShowWindow(hwnd, SW_SHOWNORMAL)` followed by `BringWindowToTop(hwnd)`.
    fn show_normal_and_raise(&mut self, hwnd: isize);
}

/// Everything read at the start of a drag.
#[derive(Clone, Copy)]
pub struct Config {
    pub hwnd: isize,
    pub overlay: isize,
    pub grid_x: i32,
    pub grid_y: i32,
    pub defer_resize: bool,
    /// "Show window contents while dragging".
    pub drag_full_windows: bool,
    pub dwm_enabled: bool,
    pub inputs: [InputMemory; MAX_INPUTS],
    pub input_count: usize,
}

/// A side effect requested by the session, run by the caller once it holds no borrow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    None,
    /// Move the pointer vertically by this many pixels.
    Nudge(i32),
    /// Show the window normally and bring it to the top.
    Raise,
}

impl Effect {
    pub fn run(self, host: &mut dyn Host, hwnd: isize) {
        match self {
            Effect::None => {}
            Effect::Nudge(dy) => {
                let p = host.cursor_pos();
                host.set_cursor_pos(p.x, p.y.wrapping_add(dy));
            }
            Effect::Raise => host.show_normal_and_raise(hwnd),
        }
    }
}

/// What the subclass procedure should do after `WM_MOVING`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MovingAction {
    /// Rewrite the message id to `WM_SIZING`.
    pub to_sizing: bool,
    /// Overwrite the `RECT` in `lParam` with this.
    pub rect: Option<Rect>,
}

/// New values for the `WINDOWPOS` structure (`SWP_NOSIZE | SWP_NOMOVE` are cleared).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PosUpdate {
    pub x: i32,
    pub y: i32,
    pub cx: i32,
    pub cy: i32,
}

pub struct Session {
    cfg: Config,
    armed: bool,
    resize_active: bool,
    move_mode: bool,
    cancelled: bool,
    ended: bool,
    /// -1 = not a move drag, 0 = undecided, `WM_MOVING` once a move was seen.
    moving_msg: i32,
    /// Last mode posted (-1 = nothing yet).
    prev_mode: i32,
    target_posted: bool,
    nudge: i32,
    monitor: isize,
    geometry: MonitorGeometry,
    layout: Option<Layout>,
    sizes: Sizes,
    orig_captured: bool,
    margins: Rect,
    start: Point,
    posted_start: Point,
    posted_cur: Point,
    selection: Selection,
}

impl Session {
    pub fn new(cfg: Config) -> Session {
        Session {
            cfg,
            armed: false,
            resize_active: false,
            move_mode: false,
            cancelled: false,
            ended: false,
            moving_msg: 0,
            prev_mode: -1,
            target_posted: false,
            nudge: 1,
            monitor: 0,
            geometry: MonitorGeometry::default(),
            layout: None,
            sizes: Sizes { min: Point::default(), max: Point::new(i32::MAX, i32::MAX), orig: Point::default() },
            orig_captured: false,
            margins: Rect::default(),
            start: Point::new(-1, -1),
            posted_start: Point::new(-1, -1),
            posted_cur: Point::new(-1, -1),
            selection: Selection::default(),
        }
    }

    pub fn hwnd(&self) -> isize {
        self.cfg.hwnd
    }

    /// The subclass procedure intercepts messages only while this is true.
    pub fn intercepting(&self) -> bool {
        !self.cancelled && !self.ended
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    // ---- input ---------------------------------------------------------------------

    /// Called for every key / mouse-button transition seen by the input hooks.
    ///
    /// Returns a side effect for the caller to run *after* releasing its borrow of the
    /// session: raising the window makes Windows call the subclass procedure re-entrantly.
    pub fn on_input(&mut self, key: i32, pressed: bool, released: bool) -> Effect {
        if pressed == released {
            return Effect::None;
        }
        let command = self.cfg.inputs[..self.cfg.input_count.min(MAX_INPUTS)]
            .iter()
            .find(|i| i.input == key)
            .map(|i| i.command);
        let Some(command) = command else { return Effect::None };
        let command = i32::from(command);
        if command != Command::RESIZE.0 && command != Command::MOVE.0 {
            return Effect::None;
        }
        if !pressed {
            self.resize_active = false;
            return Effect::None;
        }
        self.move_mode = command == Command::MOVE.0;
        self.activate()
    }

    fn activate(&mut self) -> Effect {
        self.resize_active = true;
        if !self.armed {
            self.armed = true;
            self.monitor = 0;
        }
        if self.cfg.drag_full_windows {
            // Re-raising the window forces a WM_WINDOWPOSCHANGING that we rewrite.
            Effect::Raise
        } else {
            // Outline dragging: move the pointer by one pixel to provoke a WM_MOVING.
            self.nudge = -self.nudge;
            Effect::Nudge(self.nudge)
        }
    }

    // ---- window messages -----------------------------------------------------------

    pub fn on_moving(&mut self, host: &mut dyn Host) -> MovingAction {
        if self.moving_msg != -1 {
            self.moving_msg = WM_MOVING as i32;
        }
        let mut action = MovingAction::default();
        if !self.armed || self.moving_msg != WM_MOVING as i32 {
            return action;
        }
        if !self.cfg.defer_resize {
            action.to_sizing = true;
        }
        if !self.cfg.drag_full_windows {
            self.update_grid(host);
            if !self.cfg.defer_resize {
                action.rect = Some(self.selection.pixels);
            }
        }
        action
    }

    pub fn on_window_pos_changing(&mut self, host: &mut dyn Host) -> Option<PosUpdate> {
        if !self.armed || self.moving_msg != WM_MOVING as i32 {
            return None;
        }
        self.update_grid(host);
        if self.cfg.defer_resize {
            return None;
        }
        let p = self.selection.pixels;
        let m = self.margins;
        let (left, top) = (p.left.wrapping_add(m.left), p.top.wrapping_add(m.top));
        let (right, bottom) = (p.right.wrapping_add(m.right), p.bottom.wrapping_add(m.bottom));
        Some(PosUpdate { x: left, y: top, cx: right.wrapping_sub(left), cy: bottom.wrapping_sub(top) })
    }

    pub fn on_capture_changed(&mut self) {
        if !self.armed {
            self.moving_msg = 0;
        }
    }

    /// `WM_GETMINMAXINFO` results (after the window's own procedure ran).
    pub fn on_min_max_info(&mut self, min: Point, max: Point) {
        let fix = |v: i32| if v <= 0 { i32::MAX } else { v };
        self.sizes.min = min;
        self.sizes.max = Point::new(fix(max.x), fix(max.y));
    }

    /// Ends the session; posts "drag ended" to the overlay. Safe to call repeatedly.
    pub fn end(&mut self, host: &mut dyn Host) {
        if self.ended {
            return;
        }
        self.ended = true;
        if self.cfg.overlay != 0 {
            host.post_overlay(self.cfg.overlay, Msg::ShowGrid, 0, 0);
        }
        self.moving_msg = -1;
    }

    // ---- grid update ---------------------------------------------------------------

    pub fn update_grid(&mut self, host: &mut dyn Host) {
        let pt = host.cursor_pos();

        let refresh = self.resize_active || self.move_mode || self.monitor == 0;
        let mut monitor_changed = false;
        if refresh {
            let hmon = host.monitor_from_point(pt);
            if hmon != 0 && hmon != self.monitor {
                self.monitor = hmon;
                self.geometry = host.monitor_geometry(hmon).unwrap_or_default();
                self.layout = Layout::new(self.cfg.grid_x, self.cfg.grid_y, self.geometry.work, self.geometry.monitor);
                monitor_changed = true;
            }
        }

        self.refresh_window(host);

        let Some(layout) = self.layout else { return };
        self.selection = grid::select(&layout, pt, self.start, refresh, self.move_mode, &self.sizes);
        self.start = self.selection.start;

        if self.cfg.overlay != 0 {
            self.post_updates(host, &layout, monitor_changed);
        }
    }

    fn refresh_window(&mut self, host: &mut dyn Host) {
        let m = host.window_metrics(self.cfg.hwnd);
        self.margins = match (self.cfg.dwm_enabled, m.frame) {
            (true, Some(f)) => Rect::new(
                m.window.left.wrapping_sub(f.left),
                m.window.top.wrapping_sub(f.top),
                m.window.right.wrapping_sub(f.right),
                m.window.bottom.wrapping_sub(f.bottom),
            ),
            _ => Rect::default(),
        };
        if !self.orig_captured && !m.maximized {
            self.orig_captured = true;
            self.sizes.orig = Point::new(m.window.width(), m.window.height());
        }
    }

    fn post_updates(&mut self, host: &mut dyn Host, layout: &Layout, monitor_changed: bool) {
        let mode = i32::from(self.move_mode);
        let mode_changed = mode != self.prev_mode;
        self.prev_mode = mode;
        let area_changed = self.selection.start != self.posted_start || self.selection.cur != self.posted_cur;
        if area_changed {
            self.posted_start = self.selection.start;
            self.posted_cur = self.selection.cur;
        }
        let overlay = self.cfg.overlay;

        if !self.target_posted {
            // Posted ahead of the first ShowGrid (same queue, so it arrives first).
            self.target_posted = true;
            host.post_overlay(overlay, Msg::Target, self.cfg.hwnd as u32 as usize, 0);
        }

        if monitor_changed || mode_changed {
            let c = layout.center();
            host.post_overlay(overlay, Msg::ShowGrid, mode as usize + 1, sign_extend(pack_point(c.x, c.y)));
        } else if !area_changed {
            return;
        }

        let (a, b) = if self.move_mode {
            let c = self.selection.cells;
            (pack_point(c.left, c.top), pack_point(c.right, c.bottom))
        } else {
            (
                pack_point(self.selection.start.x, self.selection.start.y),
                pack_point(self.selection.cur.x, self.selection.cur.y),
            )
        };
        host.post_overlay(overlay, Msg::Area, sign_extend(a) as usize, sign_extend(b));
    }
}

/// Packed points are 32-bit values that the original sign-extended to pointer width.
fn sign_extend(v: usize) -> isize {
    v as u32 as i32 as isize
}

#[cfg(test)]
mod tests {
    use super::*;
    use wg_core::ipc::MAX_INPUTS;
    use wg_core::keycode::KeyCode;

    #[derive(Default)]
    struct FakeHost {
        cursor: Point,
        window: Rect,
        frame: Option<Rect>,
        posted: std::vec::Vec<(Msg, usize, isize)>,
        targets: std::vec::Vec<usize>,
        cursor_sets: std::vec::Vec<(i32, i32)>,
        raised: u32,
    }

    impl Host for FakeHost {
        fn cursor_pos(&mut self) -> Point {
            self.cursor
        }
        fn monitor_from_point(&mut self, _pt: Point) -> isize {
            1
        }
        fn monitor_geometry(&mut self, _m: isize) -> Option<MonitorGeometry> {
            let r = Rect::new(0, 0, 1920, 1080);
            Some(MonitorGeometry { work: r, monitor: r })
        }
        fn window_metrics(&mut self, _h: isize) -> WindowMetrics {
            WindowMetrics { window: self.window, frame: self.frame, maximized: false }
        }
        fn post_overlay(&mut self, _o: isize, msg: Msg, w: usize, l: isize) {
            if msg == Msg::Target {
                self.targets.push(w);
                return;
            }
            self.posted.push((msg, w, l));
        }
        fn set_cursor_pos(&mut self, x: i32, y: i32) {
            self.cursor_sets.push((x, y));
        }
        fn show_normal_and_raise(&mut self, _h: isize) {
            self.raised += 1;
        }
    }

    fn config(defer: bool, drag_full: bool) -> Config {
        let mut inputs = [InputMemory::default(); MAX_INPUTS];
        let table = [
            (KeyCode::SPACE.0, 1),
            (KeyCode::LCONTROL.0, 2),
            (KeyCode::MOUSE_RBUTTON.0, 1),
            (KeyCode::MOUSE_MBUTTON.0, 2),
        ];
        for (i, (k, c)) in table.iter().enumerate() {
            inputs[i] = InputMemory { input: *k, modifier: 0, command: *c };
        }
        Config {
            hwnd: 100,
            overlay: 200,
            grid_x: 12,
            grid_y: 6,
            defer_resize: defer,
            drag_full_windows: drag_full,
            dwm_enabled: true,
            inputs,
            input_count: 4,
        }
    }

    fn input(s: &mut Session, h: &mut FakeHost, key: i32, pressed: bool, released: bool) {
        let effect = s.on_input(key, pressed, released);
        effect.run(h, s.hwnd());
    }

    fn host_at(x: i32, y: i32) -> FakeHost {
        FakeHost {
            cursor: Point::new(x, y),
            window: Rect::new(100, 100, 900, 700),
            frame: Some(Rect::new(107, 100, 893, 693)),
            ..FakeHost::default()
        }
    }

    const SPACE: i32 = 57;
    const CTRL: i32 = 29;

    #[test]
    fn nothing_happens_before_activation() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(500, 500);
        assert_eq!(s.on_moving(&mut h), MovingAction::default());
        assert_eq!(s.on_window_pos_changing(&mut h), None);
        assert!(h.posted.is_empty());
    }

    #[test]
    fn input_requires_a_real_transition_and_a_bound_key() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(500, 500);
        input(&mut s, &mut h, SPACE, false, false);
        input(&mut s, &mut h, SPACE, true, true);
        input(&mut s, &mut h, 12345, true, false);
        assert!(!s.is_armed());
        input(&mut s, &mut h, SPACE, true, false);
        assert!(s.is_armed());
    }

    #[test]
    fn activation_nudges_the_cursor_in_outline_mode_and_raises_in_full_drag_mode() {
        let mut outline = Session::new(config(false, false));
        let mut h = host_at(500, 500);
        input(&mut outline, &mut h, SPACE, true, false);
        input(&mut outline, &mut h, SPACE, false, true);
        input(&mut outline, &mut h, SPACE, true, false);
        assert_eq!(h.cursor_sets, [(500, 499), (500, 501)], "sign alternates, first is -1");
        assert_eq!(h.raised, 0);

        let mut full = Session::new(config(false, true));
        let mut h = host_at(500, 500);
        input(&mut full, &mut h, SPACE, true, false);
        assert_eq!(h.raised, 1);
        assert!(h.cursor_sets.is_empty());
    }

    #[test]
    fn resize_flow_in_full_drag_mode_rewrites_windowpos() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(1000, 400); // cell (6, 2)
        s.on_moving(&mut h); // before activation: just records the move drag
        input(&mut s, &mut h, SPACE, true, false);

        // WM_MOVING in full-drag mode only converts the message; the work is in WINDOWPOSCHANGING.
        assert_eq!(s.on_moving(&mut h), MovingAction { to_sizing: true, rect: None });
        let pos = s.on_window_pos_changing(&mut h).unwrap();
        // Single cell (6,2): pixels (960,360)-(1120,540) + margins (-7,0,+7,+7)
        assert_eq!(pos, PosUpdate { x: 953, y: 360, cx: 174, cy: 187 });

        let kinds: std::vec::Vec<_> = h.posted.iter().map(|p| p.0).collect();
        assert_eq!(kinds, [Msg::ShowGrid, Msg::Area]);
        assert_eq!(h.posted[0].1, 1, "resize mode = 1");
        assert_eq!(h.posted[0].2, pack_point(960, 540) as isize, "centre of the work area");
        assert_eq!(h.posted[1].1, pack_point(6, 2), "start == current while the key is held");
        assert_eq!(h.posted[1].2, pack_point(6, 2) as isize);
    }

    #[test]
    fn releasing_the_key_freezes_the_start_cell() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(1000, 400); // cell (6, 2)
        s.on_moving(&mut h);
        input(&mut s, &mut h, SPACE, true, false);
        s.on_window_pos_changing(&mut h);
        input(&mut s, &mut h, SPACE, false, true);
        assert!(s.is_armed(), "stays armed until the drag ends");

        h.cursor = Point::new(1500, 900); // cell (9, 5)
        h.posted.clear();
        let pos = s.on_window_pos_changing(&mut h).unwrap();
        assert_eq!((pos.x, pos.y), (960 - 7, 360));
        assert_eq!(h.posted.len(), 1, "only an Area update, no new ShowGrid");
        assert_eq!(h.posted[0].0, Msg::Area);
        assert_eq!(h.posted[0].1, pack_point(6, 2));
        assert_eq!(h.posted[0].2, pack_point(9, 5) as isize);
    }

    #[test]
    fn the_dragged_window_is_announced_once_before_the_first_grid_message() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(1000, 400);
        s.on_moving(&mut h);
        input(&mut s, &mut h, SPACE, true, false);
        s.on_window_pos_changing(&mut h);
        h.cursor = Point::new(1500, 900);
        s.on_window_pos_changing(&mut h);
        input(&mut s, &mut h, CTRL, true, false);
        s.on_window_pos_changing(&mut h);
        assert_eq!(h.targets, [100], "once per drag, carrying the hwnd");
    }

    #[test]
    fn unchanged_cells_post_nothing() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(1000, 400);
        s.on_moving(&mut h);
        input(&mut s, &mut h, SPACE, true, false);
        s.on_window_pos_changing(&mut h);
        input(&mut s, &mut h, SPACE, false, true);
        h.posted.clear();
        s.on_window_pos_changing(&mut h);
        assert!(h.posted.is_empty());
    }

    #[test]
    fn move_mode_stays_active_after_release_and_reports_the_cell_rect() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(960, 540); // cell (6, 3); window is 800x600 -> 5x3 cells
        s.on_moving(&mut h);
        input(&mut s, &mut h, CTRL, true, false);
        s.on_window_pos_changing(&mut h);
        input(&mut s, &mut h, CTRL, false, true);

        assert_eq!(h.posted[0].1, 2, "move mode = 2");
        assert_eq!(h.posted[1].1, pack_point(4, 2));
        assert_eq!(h.posted[1].2, pack_point(8, 4) as isize);

        // Pointer moves; since move_mode is still set the selection keeps following it.
        h.cursor = Point::new(400, 200);
        h.posted.clear();
        s.on_window_pos_changing(&mut h);
        assert_eq!(h.posted.len(), 1);
        assert_eq!(h.posted[0].0, Msg::Area);
    }

    #[test]
    fn switching_modes_reposts_show_grid() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(960, 540);
        s.on_moving(&mut h);
        input(&mut s, &mut h, SPACE, true, false);
        s.on_window_pos_changing(&mut h);
        input(&mut s, &mut h, CTRL, true, false);
        h.posted.clear();
        s.on_window_pos_changing(&mut h);
        assert_eq!(h.posted[0].0, Msg::ShowGrid);
        assert_eq!(h.posted[0].1, 2);
    }

    #[test]
    fn deferral_posts_updates_but_never_touches_the_window() {
        let mut s = Session::new(config(true, true));
        let mut h = host_at(1000, 400);
        s.on_moving(&mut h);
        input(&mut s, &mut h, SPACE, true, false);
        assert_eq!(s.on_moving(&mut h), MovingAction::default(), "full-drag: nothing in WM_MOVING");
        assert_eq!(s.on_window_pos_changing(&mut h), None);
        assert_eq!(h.posted.len(), 2, "overlay still gets ShowGrid + Area");

        let mut s = Session::new(config(true, false));
        let mut h = host_at(1000, 400);
        s.on_moving(&mut h);
        input(&mut s, &mut h, SPACE, true, false);
        assert_eq!(s.on_moving(&mut h), MovingAction::default(), "outline: updates but no rect/message change");
        assert_eq!(h.posted.len(), 2);
    }

    #[test]
    fn outline_mode_supplies_the_rect_when_not_deferring() {
        let mut s = Session::new(config(false, false));
        let mut h = host_at(1000, 400);
        s.on_moving(&mut h);
        input(&mut s, &mut h, SPACE, true, false);
        let a = s.on_moving(&mut h);
        assert!(a.to_sizing);
        assert_eq!(a.rect, Some(Rect::new(960, 360, 1120, 540)), "no margins in outline mode");
    }

    #[test]
    fn edge_resize_drags_are_left_alone() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(1000, 400);
        input(&mut s, &mut h, SPACE, true, false);
        // No WM_MOVING was ever seen (user is resizing an edge): WINDOWPOSCHANGING is ignored.
        // moving_msg starts at 0 and only WM_MOVING sets it.
        assert_eq!(s.on_window_pos_changing(&mut h), None);
        assert!(h.posted.is_empty());
    }

    #[test]
    fn capture_changed_resets_only_when_not_armed() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(1000, 400);
        s.on_moving(&mut h);
        input(&mut s, &mut h, SPACE, true, false);
        s.on_capture_changed();
        assert!(s.on_window_pos_changing(&mut h).is_some(), "armed sessions keep their move state");

        // Before activation a capture change forgets that a move was seen.
        let mut s = Session::new(config(false, true));
        s.on_moving(&mut h);
        s.on_capture_changed();
        input(&mut s, &mut h, SPACE, true, false);
        assert_eq!(s.on_window_pos_changing(&mut h), None);
        s.on_moving(&mut h);
        assert!(s.on_window_pos_changing(&mut h).is_some());
    }

    #[test]
    fn min_max_info_bounds_the_selection() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(1000, 400);
        s.on_min_max_info(Point::new(500, 400), Point::new(0, 0));
        s.on_moving(&mut h);
        input(&mut s, &mut h, SPACE, true, false);
        s.on_window_pos_changing(&mut h);
        // min 500x400 -> 4x3 cells, so the Area is at least that large.
        let (a, b) = (h.posted[1].1, h.posted[1].2);
        let (sx, sy) = wg_core::ipc::unpack_point(a);
        let (ex, ey) = wg_core::ipc::unpack_point(b as usize);
        assert_eq!((sx, sy), (ex, ey), "resize-mode Area reports start and current cell");
        assert!(s.selection.cells.width() + 1 >= 4 && s.selection.cells.height() + 1 >= 3);
    }

    #[test]
    fn end_posts_drag_ended_once() {
        let mut s = Session::new(config(false, true));
        let mut h = host_at(1000, 400);
        s.end(&mut h);
        s.end(&mut h);
        assert_eq!(h.posted, [(Msg::ShowGrid, 0, 0)]);
        assert!(!s.intercepting());
    }

    #[test]
    fn escape_stops_interception_without_ending() {
        let mut s = Session::new(config(false, true));
        assert!(s.intercepting());
        s.cancel();
        assert!(!s.intercepting());
    }

    #[test]
    fn without_an_overlay_nothing_is_posted() {
        let mut cfg = config(false, true);
        cfg.overlay = 0;
        let mut s = Session::new(cfg);
        let mut h = host_at(1000, 400);
        s.on_moving(&mut h);
        input(&mut s, &mut h, SPACE, true, false);
        assert!(s.on_window_pos_changing(&mut h).is_some());
        s.end(&mut h);
        assert!(h.posted.is_empty());
    }
}
