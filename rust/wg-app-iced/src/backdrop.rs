//! The grid behind the settings pages: the app's own subject, as a faint field the cards sit
//! on. It stays put while the page scrolls over it.

use crate::memo::Memo;
use crate::style::tokens;
use iced::mouse;
use iced::widget::canvas;
use iced::{Element, Fill, Point, Rectangle, Renderer, Size, Theme};

/// Every `MAJOR`th line is drawn a little stronger.
const MAJOR: usize = 4;

#[derive(Clone, Copy, Debug)]
pub struct Backdrop {
    /// Distance between lines, in logical pixels.
    pub step: f32,
}

impl Backdrop {
    pub fn view<'a, M: 'a>(self) -> Element<'a, M> {
        canvas::Canvas::new(self).width(Fill).height(Fill).into()
    }
}

/// The positions of the lines from 0 (inclusive) to `extent` (exclusive), and whether each
/// one is a major line.
pub fn lines(extent: f32, step: f32) -> Vec<(f32, bool)> {
    if step < 1.0 {
        return Vec::new();
    }
    (0..)
        .map(|i| (i as f32 * step, i % MAJOR == 0))
        .take_while(|(at, _)| *at < extent)
        .collect()
}

impl<M> canvas::Program<M> for Backdrop {
    /// Drawn once, and again only when the step or the theme changes (or the size, which the
    /// cache checks itself): see [`Memo`] for why this must not be redrawn every frame.
    type State = Memo<(u32, bool)>;

    fn draw(&self, memo: &Self::State, renderer: &Renderer, theme: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<canvas::Geometry> {
        let t = tokens(theme);
        vec![memo.draw((self.step.to_bits(), t.dark), renderer, bounds.size(), |frame| {
            let colour = |major: bool| if major { t.grid_major() } else { t.grid() };
            for (x, major) in lines(bounds.width, self.step) {
                frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, bounds.height), colour(major));
            }
            for (y, major) in lines(bounds.height, self.step) {
                frame.fill_rectangle(Point::new(0.0, y), Size::new(bounds.width, 1.0), colour(major));
            }
        })]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_start_at_the_edge_and_every_fourth_is_major() {
        let l = lines(200.0, 32.0);
        assert_eq!(l.iter().map(|(at, _)| *at).collect::<Vec<_>>(), vec![0.0, 32.0, 64.0, 96.0, 128.0, 160.0, 192.0]);
        assert_eq!(l.iter().filter(|(_, major)| *major).map(|(at, _)| *at).collect::<Vec<_>>(), vec![0.0, 128.0]);
    }

    #[test]
    fn degenerate_input_draws_nothing_and_terminates() {
        assert!(lines(100.0, 0.0).is_empty());
        assert!(lines(0.0, 32.0).is_empty());
        assert!(lines(-5.0, 32.0).is_empty());
    }
}
