//! Bytes captured read-only from the shared-memory sections of a running C# WindowGrid
//! (2026-10-14, grid 14x6, default bindings). They pin the IPC layout to what the shipping
//! app really writes, not just to what we believe it writes.

use wg_core::ipc::{GridWindowMemory, InputMemory, SettingsMemory, Wire, DEFER_FLAG, MAX_INPUTS};

fn bytes(hex: &str) -> Vec<u8> {
    hex.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
}

#[test]
fn hook_section() {
    let raw = bytes("3e 12 b5 00 0e 00 01 00 06 00 00 00 04 00 00 00");
    let m = SettingsMemory::from_bytes(&raw).unwrap();
    assert_eq!({ m.grid_window }, 0x00B5_123E, "overlay HWND written by the overlay");
    assert_eq!({ m.grid_x }, 14 | DEFER_FLAG);
    assert_eq!(m.columns(), 14);
    assert!(m.defer_resize());
    assert_eq!({ m.grid_y }, 6);
    assert_eq!({ m.input_size }, 4);
}

#[test]
fn input_section() {
    let raw = bytes(
        "39 00 00 00 00 00 01 00  1d 00 00 00 00 00 02 00  02 01 00 00 00 00 01 00  03 01 00 00 00 00 02 00",
    );
    let mut out = [InputMemory::default(); MAX_INPUTS];
    let n = InputMemory::read_section(&raw, 4, &mut out);
    assert_eq!(n, 4);
    let got: Vec<(i32, i16, i16)> = out[..n].iter().map(|e| (e.input, e.modifier, e.command)).collect();
    // SPACE -> resize, LCONTROL -> move, RBUTTON -> resize, MBUTTON -> move; no modifiers.
    assert_eq!(got, [(57, 0, 1), (29, 0, 2), (258, 0, 1), (259, 0, 2)]);
}

#[test]
fn grid_window_section() {
    let raw = bytes(
        "0e 00 00 00 06 00 00 00 01 00 00 40 40 01 00 00 40 3f d4 bd 00 ff a7 97 00 ff f2 eb b2 ff \
         01 01 01 01 01 01 01 00 00 00 00 01 01 01 00 00 00 00 00 00",
    );
    assert!(raw.len() >= GridWindowMemory::SIZE);
    let m = GridWindowMemory::from_bytes(&raw).unwrap();
    assert_eq!(GridWindowMemory::SIZE, 44);
    assert_eq!(({ m.grid_x }, { m.grid_y }), (14, 6));
    assert_eq!({ m.radial_gradient }, 1);
    assert_eq!({ m.blur_radius }, 3.0);
    assert_eq!({ m.fill_window }, 1);
    assert_eq!({ m.opacity }, 0.75);
    assert_eq!({ m.color_line } as u32, 0xFF00_BDD4);
    assert_eq!({ m.color_start } as u32, 0xFF00_97A7);
    assert_eq!({ m.color_end } as u32, 0xFFB2_EBF2);
    let huds = [
        m.show_selection_hud,
        m.show_dimensions,
        m.show_cell_span,
        m.show_screen_coverage,
        m.show_aspect_ratio,
        m.show_mode_indicator,
        m.show_interaction_hints,
    ];
    assert_eq!(huds, [1; 7]);
    assert_eq!({ m.anchor }, 0);
    assert_eq!(({ m.compensate_window_margins }, { m.preserve_size_on_move }, { m.defer_resize_until_release }), (1, 1, 1));
}
