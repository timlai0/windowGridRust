//! A canvas cache that knows what it was drawn for.
//!
//! This is what keeps the windows responsive. iced's software renderer redraws only what changed
//! since the last frame, but it can only tell that a canvas has not changed when the canvas
//! hands it the *same cached geometry*: geometry drawn fresh each frame always counts as
//! changed, so every hover or click anywhere then repaints the whole canvas area, and the
//! many separate rectangles that produces made a hover take ~100 ms. A `canvas::Cache` fixes
//! that, but it must be told when its inputs change; this remembers them.

use iced::widget::canvas::{Cache, Frame, Geometry};
use iced::{Renderer, Size};
use std::cell::Cell;

/// The cache and the key it was drawn for. It is a canvas program's `State`, which lives as long
/// as the canvas widget and so survives rebuilding the view.
pub struct Memo<K> {
    cache: Cache,
    key: Cell<Option<K>>,
}

impl<K> Default for Memo<K> {
    fn default() -> Self {
        Memo { cache: Cache::default(), key: Cell::new(None) }
    }
}

impl<K: Copy + PartialEq> Memo<K> {
    /// The geometry for `key` at `size`: the cached one, unless the key (or, as the cache itself
    /// checks, the size) is not what it was drawn for, in which case `draw` runs again.
    pub fn draw(&self, key: K, renderer: &Renderer, size: Size, draw: impl FnOnce(&mut Frame)) -> Geometry {
        if self.key.get() != Some(key) {
            self.cache.clear();
            self.key.set(Some(key));
        }
        self.cache.draw(renderer, size, draw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::{Font, Pixels};

    fn renderer() -> Renderer {
        Renderer::new(Font::DEFAULT, Pixels(16.0))
    }

    #[test]
    fn draws_again_only_when_the_key_or_the_size_changes() {
        let (renderer, memo) = (renderer(), Memo::<u32>::default());
        let draws = Cell::new(0);
        let frame = |key: u32, size: Size| {
            let _ = memo.draw(key, &renderer, size, |_| draws.set(draws.get() + 1));
        };
        let size = Size::new(100.0, 80.0);
        frame(1, size);
        frame(1, size);
        frame(1, size);
        assert_eq!(draws.get(), 1, "the same key and size is drawn once");
        frame(2, size);
        assert_eq!(draws.get(), 2, "a new key draws again");
        frame(2, size);
        assert_eq!(draws.get(), 2);
        frame(2, Size::new(120.0, 80.0));
        assert_eq!(draws.get(), 3, "a new size draws again");
        frame(1, Size::new(120.0, 80.0));
        assert_eq!(draws.get(), 4, "going back to an earlier key draws again");
    }
}
