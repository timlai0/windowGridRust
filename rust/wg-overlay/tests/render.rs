//! Renders overlay scenes offscreen (GPU or WARP) and writes PNGs for visual review into
//! `target/tmp/`. The assertions only check that drawing happened where it should.

#![cfg(windows)]

use std::path::PathBuf;
use wg_core::ipc::GridWindowMemory;
use wg_core::settings::GridTheme;
use wg_overlay::geom::{selection_rect, Cells};
use wg_overlay::gfx::{Renderer, Scene, SelectionScene};
use wg_overlay::hud::{hint_bar, hud_text, HudOptions, HudSubject};
use wg_overlay::png;
use wg_overlay::snap::Mode;
use wg_overlay::theme::Palette;

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn memory(theme: &GridTheme, radial: bool, fill: bool) -> GridWindowMemory {
    GridWindowMemory {
        grid_x: 12,
        grid_y: 6,
        radial_gradient: radial as u8,
        blur_radius: 3.0,
        fill_window: fill as u8,
        opacity: 0.75,
        color_line: theme.color_line.to_argb_i32(),
        color_start: theme.color_start.to_argb_i32(),
        color_end: theme.color_end.to_argb_i32(),
        ..GridWindowMemory::default()
    }
}

struct Case {
    name: &'static str,
    width: u32,
    height: u32,
    scale: f32,
    cells: Cells,
    theme: GridTheme,
    radial: bool,
    fill: bool,
    mode: Mode,
}

fn render(case: &Case) -> Vec<u8> {
    let renderer = Renderer::new().expect("a D3D11 device (hardware or WARP)");
    let palette = Palette::from_memory(&memory(&case.theme, case.radial, case.fill));
    let (w, h) = (case.width as f32, case.height as f32);
    let rect = selection_rect(case.cells, 12, 6, f64::from(w), f64::from(h));
    let text = hud_text(
        HudOptions { show_dimensions: true, show_cell_span: true, show_coverage: true, show_aspect_ratio: true },
        HudSubject {
            px_w: rect.width.round() as i32,
            px_h: rect.height.round() as i32,
            cells: case.cells,
            cols: 12,
            rows: 6,
            screen_w: case.width as i32,
            screen_h: case.height as i32,
        },
    );
    let scene = Scene {
        width: w,
        height: h,
        scale: case.scale,
        cols: 12,
        rows: 6,
        palette,
        selection: Some(SelectionScene { rect, hud: Some(&text) }),
        hint: hint_bar(case.mode, true, true),
    };
    let rgba = renderer.render_to_rgba(&scene, true).expect("render");
    let file = out_dir().join(format!("overlay-{}.png", case.name));
    std::fs::write(&file, png::encode_rgba(case.width, case.height, &rgba)).unwrap();
    rgba
}

fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let o = ((y * width + x) * 4) as usize;
    [rgba[o], rgba[o + 1], rgba[o + 2]]
}

#[test]
fn default_theme_resize() {
    let case = Case {
        name: "default-resize",
        width: 1280,
        height: 720,
        scale: 1.0,
        cells: Cells::from_corners(2, 1, 5, 3),
        theme: GridTheme::default_theme(),
        radial: true,
        fill: true,
        mode: Mode::Resize,
    };
    let img = render(&case);
    // Inside the selection the cyan fill must dominate the checkerboard.
    let inside = pixel(&img, case.width, 320, 250);
    assert!(inside[2] > inside[0] + 40, "expected a cyan-ish fill, got {inside:?}");
    // Far outside the selection and off grid lines it is just the dimmed checkerboard.
    let outside = pixel(&img, case.width, 1250, 700);
    assert!((outside[0] as i32 - outside[2] as i32).abs() < 8, "expected neutral grey, got {outside:?}");
}

#[test]
fn linear_gradient_dark_theme_move() {
    render(&Case {
        name: "dark-linear-move",
        width: 1280,
        height: 720,
        scale: 1.0,
        cells: Cells::from_corners(6, 2, 9, 4),
        theme: GridTheme::builtin_themes().into_iter().find(|t| t.name == "red").unwrap(),
        radial: false,
        fill: true,
        mode: Mode::Move,
    });
}

#[test]
fn outline_only_at_high_dpi() {
    render(&Case {
        name: "outline-1_5x",
        width: 1920,
        height: 1080,
        scale: 1.5,
        cells: Cells::from_corners(0, 0, 1, 1),
        theme: GridTheme::builtin_themes().into_iter().find(|t| t.name == "green").unwrap(),
        radial: true,
        fill: false,
        mode: Mode::Resize,
    });
}
