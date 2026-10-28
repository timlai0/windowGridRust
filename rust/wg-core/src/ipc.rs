//! The wire contract between the app, the overlay and the hook DLLs.
//!
//! Everything here is `repr(C, packed)` plain data written into named, pagefile-backed
//! shared memory sections (always at offset 0), plus three registered window messages.
//! Bools are single bytes: that is the layout `MemoryMappedViewAccessor.Write<T>` produced
//! in the original C# WindowGrid, which these structs still match byte for byte; only the
//! section and message names differ. (`GridWindowMemory` is only exchanged between app and
//! overlay, so its layout can change freely, but it must change on both sides together.)

use core::mem::size_of;

#[cfg(feature = "std")]
use crate::settings::{GridSettings, InputSettings, Settings};

pub const MAP_HOOK: &str = "WindowGridRustMapping_Hook";
pub const MAP_INPUT: &str = "WindowGridRustMapping_Input";
pub const MAP_GRID_WINDOW: &str = "WindowGridRustMapping_GridWindow";
pub const MAP_GRID_WINDOW_HANDLE: &str = "WindowGridRustMapping_GridWindowHandle";

pub const MSG_SHOW_GRID: &str = "WindowGridRustShowGrid";
pub const MSG_AREA: &str = "WindowGridRustArea";
pub const MSG_TARGET: &str = "WindowGridRustTarget";
pub const MSG_STOP: &str = "WindowGridRustStop";

pub const LOG_MUTEX: &str = "Global\\WindowGridRustLog";

/// Bit 16 of `SettingsMemory::grid_x`: tells the hook to defer window resizing until the
/// drag ends. The hook masks it out before doing cell maths; the overlay never sees it.
pub const DEFER_FLAG: i32 = 0x10000;

/// The input map is a fixed 64-byte section holding up to this many [`InputMemory`] entries.
pub const INPUT_MAP_SIZE: usize = 64;
pub const MAX_INPUTS: usize = INPUT_MAP_SIZE / size_of::<InputMemory>();

/// Plain-old-data that can be copied to and from shared memory as raw bytes.
///
/// # Safety
/// Implementors must be `repr(C)`/`repr(C, packed)` with only integer or float fields, so
/// that every bit pattern is a valid value.
pub unsafe trait Wire: Copy + Sized {
    const SIZE: usize = size_of::<Self>();

    /// Copies the raw bytes into `out`; returns `false` if `out` is too small.
    fn write_to(&self, out: &mut [u8]) -> bool {
        if out.len() < Self::SIZE {
            return false;
        }
        // SAFETY: `Self` is plain data (trait contract) and `out` has room for `SIZE` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(self as *const Self as *const u8, out.as_mut_ptr(), Self::SIZE)
        };
        true
    }

    #[cfg(feature = "std")]
    fn to_bytes(&self) -> Vec<u8> {
        let mut v = vec![0u8; Self::SIZE];
        self.write_to(&mut v);
        v
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::SIZE {
            return None;
        }
        // SAFETY: length checked above; any bit pattern is valid (trait contract).
        Some(unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const Self) })
    }
}

// The window-handle and instance-PID sections hold one 32-bit integer each.
unsafe impl Wire for i32 {}

/// `WindowGridRustMapping_Hook` (16 bytes). Written by the app, read by the hooks; the overlay
/// stores its window handle in `grid_window` so the hooks know where to post messages.
#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct SettingsMemory {
    /// Overlay `HWND`, truncated to 32 bits.
    pub grid_window: i32,
    /// Columns, with [`DEFER_FLAG`] possibly set.
    pub grid_x: i32,
    pub grid_y: i32,
    /// Number of valid entries in the input map.
    pub input_size: i32,
}
unsafe impl Wire for SettingsMemory {}

impl SettingsMemory {
    #[cfg(feature = "std")]
    pub fn from_settings(s: &Settings, grid_window: i32) -> SettingsMemory {
        let defer = if s.grid.defer_resize_until_release { DEFER_FLAG } else { 0 };
        SettingsMemory {
            grid_window,
            grid_x: s.grid.grid_x | defer,
            grid_y: s.grid.grid_y,
            input_size: s.input.len().min(MAX_INPUTS) as i32,
        }
    }

    /// Column count with the deferral flag removed.
    pub fn columns(&self) -> i32 {
        self.grid_x & !DEFER_FLAG
    }

    pub fn defer_resize(&self) -> bool {
        self.grid_x & DEFER_FLAG != 0
    }
}

/// One entry of `WindowGridRustMapping_Input` (8 bytes each).
#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct InputMemory {
    /// [`crate::keycode::KeyCode`] value.
    pub input: i32,
    /// WPF `ModifierKeys` flags.
    pub modifier: i16,
    /// 1 = resize, 2 = move.
    pub command: i16,
}
unsafe impl Wire for InputMemory {}

impl InputMemory {
    #[cfg(feature = "std")]
    pub fn from_input(i: &InputSettings) -> InputMemory {
        InputMemory { input: i.key.0, modifier: i.modifier.0 as i16, command: i.command.0 as i16 }
    }

    /// The full 64-byte section contents for a set of bindings (extra bindings are dropped).
    #[cfg(feature = "std")]
    pub fn section_bytes(inputs: &[InputSettings]) -> [u8; INPUT_MAP_SIZE] {
        let mut out = [0u8; INPUT_MAP_SIZE];
        for (i, input) in inputs.iter().take(MAX_INPUTS).enumerate() {
            let bytes = InputMemory::from_input(input).to_bytes();
            out[i * InputMemory::SIZE..(i + 1) * InputMemory::SIZE].copy_from_slice(&bytes);
        }
        out
    }

    /// Reads up to `count` entries from a section into `out` and returns how many were read.
    /// Allocation-free so the hook DLLs can use it.
    pub fn read_section(bytes: &[u8], count: usize, out: &mut [InputMemory; MAX_INPUTS]) -> usize {
        let mut n = 0;
        while n < count.min(MAX_INPUTS) {
            match bytes.get(n * Self::SIZE..).and_then(InputMemory::from_bytes) {
                Some(entry) => out[n] = entry,
                None => break,
            }
            n += 1;
        }
        n
    }

    #[cfg(feature = "std")]
    pub fn parse_section(bytes: &[u8], count: usize) -> Vec<InputMemory> {
        let mut out = [InputMemory::default(); MAX_INPUTS];
        let n = Self::read_section(bytes, count, &mut out);
        out[..n].to_vec()
    }
}

/// `WindowGridRustMapping_GridWindow` (44 bytes). Written by the app, read by the overlay.
/// Colours are packed `0xAARRGGBB`; `bool`s are one byte, 0 or 1.
#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct GridWindowMemory {
    pub grid_x: i32,
    pub grid_y: i32,
    pub radial_gradient: u8,
    pub blur_radius: f32,
    pub fill_window: u8,
    pub opacity: f32,
    pub color_line: i32,
    pub color_start: i32,
    pub color_end: i32,
    pub show_selection_hud: u8,
    pub show_dimensions: u8,
    pub show_cell_span: u8,
    pub show_screen_coverage: u8,
    pub show_aspect_ratio: u8,
    pub show_mode_indicator: u8,
    pub show_interaction_hints: u8,
    /// 0 = top-left, 1 = centre, 2 = grab point.
    pub anchor: i32,
    pub compensate_window_margins: u8,
    pub preserve_size_on_move: u8,
    pub defer_resize_until_release: u8,
}
unsafe impl Wire for GridWindowMemory {}

impl GridWindowMemory {
    #[cfg(feature = "std")]
    pub fn from_settings(g: &GridSettings) -> GridWindowMemory {
        let theme = g.active_theme();
        GridWindowMemory {
            grid_x: g.grid_x,
            grid_y: g.grid_y,
            radial_gradient: g.radial_gradient as u8,
            blur_radius: g.blur_radius,
            fill_window: g.fill_window as u8,
            opacity: g.opacity,
            color_line: theme.color_line.to_argb_i32(),
            color_start: theme.color_start.to_argb_i32(),
            color_end: theme.color_end.to_argb_i32(),
            show_selection_hud: g.show_selection_hud as u8,
            show_dimensions: g.show_dimensions as u8,
            show_cell_span: g.show_cell_span as u8,
            show_screen_coverage: g.show_screen_coverage as u8,
            show_aspect_ratio: g.show_aspect_ratio as u8,
            show_mode_indicator: g.show_mode_indicator as u8,
            show_interaction_hints: g.show_interaction_hints as u8,
            anchor: g.anchor,
            compensate_window_margins: g.compensate_window_margins as u8,
            preserve_size_on_move: g.preserve_size_on_move as u8,
            defer_resize_until_release: g.defer_resize_until_release as u8,
        }
    }
}

/// What the overlay is doing, carried in `wParam` of [`MSG_SHOW_GRID`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The drag ended: snap (or hide) now.
    End = 0,
    Resize = 1,
    Move = 2,
}

impl Mode {
    pub fn from_wparam(w: usize) -> Option<Mode> {
        match w {
            0 => Some(Mode::End),
            1 => Some(Mode::Resize),
            2 => Some(Mode::Move),
            _ => None,
        }
    }
}

/// Packs two signed 16-bit values into a message parameter: `x` low word, `y` high word.
pub fn pack_point(x: i32, y: i32) -> usize {
    (((y as u16 as u32) << 16) | (x as u16 as u32)) as usize
}

/// Inverse of [`pack_point`]; only the low 32 bits of the parameter are used, and each
/// half is sign-extended (monitor centres on screens left of / above the primary are negative).
pub fn unpack_point(param: usize) -> (i32, i32) {
    let v = param as u32;
    ((v & 0xFFFF) as u16 as i16 as i32, (v >> 16) as u16 as i16 as i32)
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use std::mem::offset_of;

    #[test]
    fn struct_sizes_are_part_of_the_contract() {
        assert_eq!(SettingsMemory::SIZE, 16);
        assert_eq!(InputMemory::SIZE, 8);
        assert_eq!(GridWindowMemory::SIZE, 44);
        assert_eq!(MAX_INPUTS, 8);
    }

    #[test]
    fn grid_window_offsets() {
        assert_eq!(offset_of!(GridWindowMemory, radial_gradient), 8);
        assert_eq!(offset_of!(GridWindowMemory, blur_radius), 9);
        assert_eq!(offset_of!(GridWindowMemory, fill_window), 13);
        assert_eq!(offset_of!(GridWindowMemory, opacity), 14);
        assert_eq!(offset_of!(GridWindowMemory, color_line), 18);
        assert_eq!(offset_of!(GridWindowMemory, show_selection_hud), 30);
        assert_eq!(offset_of!(GridWindowMemory, show_interaction_hints), 36);
        assert_eq!(offset_of!(GridWindowMemory, anchor), 37);
        assert_eq!(offset_of!(GridWindowMemory, defer_resize_until_release), 43);
    }

    #[test]
    fn hook_memory_carries_the_deferral_flag() {
        let mut s = Settings::default();
        let on = SettingsMemory::from_settings(&s, 0x1234);
        assert_eq!({ on.grid_x }, 0x1000C);
        assert_eq!(on.columns(), 12);
        assert!(on.defer_resize());
        assert_eq!({ on.grid_window }, 0x1234);
        assert_eq!({ on.input_size }, 4);

        s.grid.defer_resize_until_release = false;
        let off = SettingsMemory::from_settings(&s, 0);
        assert_eq!({ off.grid_x }, 12);
        assert!(!off.defer_resize());
    }

    #[test]
    fn input_section_layout() {
        let bytes = InputMemory::section_bytes(&Settings::default_input());
        assert_eq!(bytes.len(), 64);
        // KEY_SPACE (57), no modifier, Resize (1)
        assert_eq!(&bytes[0..8], &[57, 0, 0, 0, 0, 0, 1, 0]);
        // second entry: KEY_LCONTROL (29), Move (2)
        assert_eq!(&bytes[8..16], &[29, 0, 0, 0, 0, 0, 2, 0]);
        // MOUSE_RBUTTON (258 = 0x102)
        assert_eq!(&bytes[16..24], &[2, 1, 0, 0, 0, 0, 1, 0]);
        assert!(bytes[32..].iter().all(|b| *b == 0));

        let parsed = InputMemory::parse_section(&bytes, 4);
        assert_eq!(parsed.len(), 4);
        assert_eq!({ parsed[3].input }, 259);
        assert_eq!({ parsed[3].command }, 2);
    }

    #[test]
    fn grid_window_memory_from_default_settings() {
        let m = GridWindowMemory::from_settings(&GridSettings::default());
        let bytes = m.to_bytes();
        assert_eq!(bytes.len(), 44);
        assert_eq!({ m.color_line }, 0xFF00BDD4u32 as i32);
        assert_eq!({ m.color_start }, 0xFF0097A7u32 as i32);
        let back = GridWindowMemory::from_bytes(&bytes).unwrap();
        assert_eq!({ back.opacity }, 0.75);
        assert_eq!({ back.defer_resize_until_release }, 1);
        assert!(GridWindowMemory::from_bytes(&bytes[..43]).is_none());
    }

    #[test]
    fn point_packing_round_trips_including_negatives() {
        for (x, y) in [(0, 0), (960, 540), (-1920, 1080), (-1, -1), (32767, -32768)] {
            assert_eq!(unpack_point(pack_point(x, y)), (x, y));
        }
        // A 64-bit parameter with garbage in the upper half still decodes.
        assert_eq!(unpack_point(0xDEAD_BEEF_0010_0020), (0x20, 0x10));
    }

    #[test]
    fn mode_decoding() {
        assert_eq!(Mode::from_wparam(0), Some(Mode::End));
        assert_eq!(Mode::from_wparam(2), Some(Mode::Move));
        assert_eq!(Mode::from_wparam(3), None);
    }
}
