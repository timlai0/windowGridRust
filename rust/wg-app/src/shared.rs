//! The shared-memory sections the app owns for the hook DLLs and the overlay.
//!
//! The app creates them at startup and keeps them alive: the hooks open them read-only for
//! every drag, the overlay reads its settings from one and writes its window handle into two.

use crate::config::publish;
use wg_core::ipc::{GridWindowMemory, SettingsMemory, Wire, INPUT_MAP_SIZE};
use wg_core::names;
use wg_core::settings::Settings;
use wg_win::shm::Section;
use windows::core::Result;

pub struct Shared {
    hook: Section,
    input: Section,
    grid_window: Section,
}

impl Shared {
    pub fn create() -> Result<Shared> {
        Ok(Shared {
            hook: Section::open_or_create(names::MAP_HOOK, SettingsMemory::SIZE)?,
            input: Section::open_or_create(names::MAP_INPUT, INPUT_MAP_SIZE)?,
            grid_window: Section::open_or_create(names::MAP_GRID_WINDOW, GridWindowMemory::SIZE)?,
        })
    }

    /// Publishes `settings`. The overlay's window handle (the first field of the hook
    /// section) is the overlay's to write, so only the fields after it are touched.
    pub fn publish(&self, settings: &Settings) {
        let p = publish(settings);
        let mut hook = [0u8; SettingsMemory::SIZE];
        p.hook.write_to(&mut hook);
        self.hook.write_bytes(4, &hook[4..]);
        self.input.write_bytes(0, &p.input);
        self.grid_window.write_bytes(0, &p.grid_window.to_bytes());
    }

    /// The overlay's window handle as currently recorded in the hook section (0 = none yet).
    pub fn overlay_window(&self) -> i32 {
        self.hook.read::<SettingsMemory>().grid_window
    }
}
