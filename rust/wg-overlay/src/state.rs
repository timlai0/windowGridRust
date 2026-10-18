//! The overlay's session state machine, free of platform calls. It turns the hook's
//! messages and the safety-net timer ticks into a list of effects for the platform layer.
//!
//! Compared with the legacy overlay, the safety-net timer (15 ms) runs in both Resize and
//! Move mode, so Esc also cancels a Move drag instead of the window still snapping when the
//! drag ends. Releasing the primary mouse button for [`RELEASE_TICKS`] consecutive ticks
//! snaps even if the hook's "drag ended" message was lost.

use crate::geom::{Cells, Point};
use crate::snap::Mode;

/// Timer period of the safety net.
pub const TICK_MS: u32 = 15;
/// Consecutive ticks with the primary button up before the fallback snap (about 75 ms).
pub const RELEASE_TICKS: u32 = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    /// A session begins: find the target window and monitor, size and show the overlay,
    /// draw the grid, then feed the initial selection to [`Machine::on_area`].
    Begin { mode: Mode, center: Point },
    /// The selection changed: redraw it and the HUD.
    Select(Cells),
    /// Hide the overlay (visuals only, the window stays alive).
    Hide,
    /// Move the target window to the last selection (`None` = nothing was selected).
    Snap { previous: Mode, current: Mode, cells: Option<Cells> },
    StartTimer,
    StopTimer,
    /// Forget the remembered target window.
    ClearTarget,
}

#[derive(Debug)]
pub struct Machine {
    mode: Mode,
    last: Option<Cells>,
    up_ticks: u32,
}

impl Default for Machine {
    fn default() -> Self {
        Machine { mode: Mode::Idle, last: None, up_ticks: 0 }
    }
}

impl Machine {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn last_cells(&self) -> Option<Cells> {
        self.last
    }

    /// `WindowGridShowGrid`: a mode switch, or `Idle` when the drag has ended.
    pub fn on_show_grid(&mut self, mode: Mode, center: Point, esc_down: bool) -> Vec<Effect> {
        if mode == Mode::Idle {
            let previous = self.mode;
            return self.finish(previous, esc_down);
        }
        self.mode = mode;
        self.up_ticks = 0;
        self.last = None;
        vec![Effect::StartTimer, Effect::Begin { mode, center }]
    }

    /// `WindowGridArea`: the selection corners in cells. Ignored while idle or unchanged.
    pub fn on_area(&mut self, cells: Cells) -> Option<Effect> {
        if self.mode == Mode::Idle || self.last == Some(cells) {
            return None;
        }
        self.last = Some(cells);
        Some(Effect::Select(cells))
    }

    /// One tick of the safety-net timer.
    pub fn on_tick(&mut self, esc_down: bool, primary_down: bool) -> Vec<Effect> {
        if self.mode == Mode::Idle {
            self.up_ticks = 0;
            return vec![Effect::StopTimer];
        }
        if esc_down {
            self.mode = Mode::Idle;
            self.last = None;
            self.up_ticks = 0;
            return vec![Effect::StopTimer, Effect::Hide, Effect::ClearTarget];
        }
        if primary_down {
            self.up_ticks = 0;
            return Vec::new();
        }
        self.up_ticks += 1;
        if self.up_ticks >= RELEASE_TICKS {
            let previous = self.mode;
            return self.finish(previous, false);
        }
        Vec::new()
    }

    /// Ends the session, snapping unless cancelled.
    fn finish(&mut self, previous: Mode, cancelled: bool) -> Vec<Effect> {
        let cells = self.last.take();
        self.mode = Mode::Idle;
        self.up_ticks = 0;
        let mut effects = vec![Effect::StopTimer, Effect::Hide];
        if !cancelled {
            effects.push(Effect::Snap { previous, current: Mode::Idle, cells });
        }
        effects.push(Effect::ClearTarget);
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTER: Point = Point { x: 960, y: 540 };

    fn cells(a: i32, b: i32, c: i32, d: i32) -> Cells {
        Cells::from_corners(a, b, c, d)
    }

    #[test]
    fn showing_the_grid_starts_the_timer_and_a_session() {
        let mut m = Machine::default();
        let e = m.on_show_grid(Mode::Resize, CENTER, false);
        assert_eq!(e, vec![Effect::StartTimer, Effect::Begin { mode: Mode::Resize, center: CENTER }]);
        assert_eq!(m.mode(), Mode::Resize);
    }

    #[test]
    fn area_updates_are_deduplicated_and_ignored_while_idle() {
        let mut m = Machine::default();
        assert_eq!(m.on_area(cells(0, 0, 1, 1)), None, "idle");
        m.on_show_grid(Mode::Resize, CENTER, false);
        assert_eq!(m.on_area(cells(2, 1, 3, 3)), Some(Effect::Select(cells(2, 1, 3, 3))));
        assert_eq!(m.on_area(cells(3, 3, 2, 1)), None, "same cells, corners swapped");
        assert_eq!(m.on_area(cells(2, 1, 4, 3)), Some(Effect::Select(cells(2, 1, 4, 3))));
    }

    #[test]
    fn drag_end_snaps_to_the_last_selection() {
        let mut m = Machine::default();
        m.on_show_grid(Mode::Resize, CENTER, false);
        m.on_area(cells(2, 1, 5, 3));
        let e = m.on_show_grid(Mode::Idle, CENTER, false);
        assert_eq!(
            e,
            vec![
                Effect::StopTimer,
                Effect::Hide,
                Effect::Snap { previous: Mode::Resize, current: Mode::Idle, cells: Some(cells(2, 1, 5, 3)) },
                Effect::ClearTarget,
            ]
        );
        assert_eq!(m.mode(), Mode::Idle);
        assert_eq!(m.last_cells(), None);
    }

    #[test]
    fn drag_end_with_escape_held_does_not_snap() {
        let mut m = Machine::default();
        m.on_show_grid(Mode::Move, CENTER, false);
        m.on_area(cells(1, 1, 1, 1));
        let e = m.on_show_grid(Mode::Idle, CENTER, true);
        assert_eq!(e, vec![Effect::StopTimer, Effect::Hide, Effect::ClearTarget]);
    }

    #[test]
    fn drag_end_without_a_session_asks_for_an_empty_snap() {
        let mut m = Machine::default();
        let e = m.on_show_grid(Mode::Idle, CENTER, false);
        assert!(e.contains(&Effect::Snap { previous: Mode::Idle, current: Mode::Idle, cells: None }));
    }

    #[test]
    fn escape_on_a_tick_cancels_in_both_modes() {
        for mode in [Mode::Resize, Mode::Move] {
            let mut m = Machine::default();
            m.on_show_grid(mode, CENTER, false);
            m.on_area(cells(1, 1, 2, 2));
            let e = m.on_tick(true, true);
            assert_eq!(e, vec![Effect::StopTimer, Effect::Hide, Effect::ClearTarget]);
            assert_eq!(m.mode(), Mode::Idle);
            // The hook's drag-end message that follows finds nothing to snap.
            let e = m.on_show_grid(Mode::Idle, CENTER, false);
            assert!(e.contains(&Effect::Snap { previous: Mode::Idle, current: Mode::Idle, cells: None }));
        }
    }

    #[test]
    fn fallback_snap_needs_five_consecutive_ticks_with_the_button_up() {
        let mut m = Machine::default();
        m.on_show_grid(Mode::Resize, CENTER, false);
        m.on_area(cells(0, 0, 3, 2));
        for _ in 0..4 {
            assert!(m.on_tick(false, false).is_empty());
        }
        // A dropped reading resets the debounce.
        assert!(m.on_tick(false, true).is_empty());
        for _ in 0..4 {
            assert!(m.on_tick(false, false).is_empty());
        }
        let e = m.on_tick(false, false);
        assert_eq!(
            e,
            vec![
                Effect::StopTimer,
                Effect::Hide,
                Effect::Snap { previous: Mode::Resize, current: Mode::Idle, cells: Some(cells(0, 0, 3, 2)) },
                Effect::ClearTarget,
            ]
        );
    }

    #[test]
    fn button_held_never_triggers_the_fallback() {
        let mut m = Machine::default();
        m.on_show_grid(Mode::Move, CENTER, false);
        for _ in 0..100 {
            assert!(m.on_tick(false, true).is_empty());
        }
        assert_eq!(m.mode(), Mode::Move);
    }

    #[test]
    fn ticks_while_idle_just_stop_the_timer() {
        let mut m = Machine::default();
        assert_eq!(m.on_tick(false, false), vec![Effect::StopTimer]);
    }

    #[test]
    fn switching_mode_restarts_the_selection() {
        let mut m = Machine::default();
        m.on_show_grid(Mode::Resize, CENTER, false);
        m.on_area(cells(0, 0, 1, 1));
        let e = m.on_show_grid(Mode::Move, CENTER, false);
        assert!(matches!(e.last(), Some(Effect::Begin { mode: Mode::Move, .. })));
        assert_eq!(m.last_cells(), None);
        assert!(m.on_area(cells(0, 0, 1, 1)).is_some(), "same cells are news again after a new Begin");
    }
}
