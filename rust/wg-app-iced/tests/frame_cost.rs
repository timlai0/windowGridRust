//! What the settings window costs per frame, and what a hover costs. Headless: it builds the real
//! view and runs it through iced's layout, drawing, damage tracking and software rasterizer
//! in-process, the same stages the window goes through, with no window, tray or helper process.
//! So it can run any time, next to a running WindowGridRust.
//!
//! ```text
//! cargo test -p wg-app-iced --release --test frame_cost                               # the checks
//! cargo test -p wg-app-iced --release --test frame_cost -- --ignored --nocapture      # the timing report
//! ```
//!
//! The checks do not use clocks, so they do not flake: they count the pixels a hover damages.
//! That is what went wrong twice. Canvases drawn fresh every frame (no `canvas::Cache`) counted as
//! changed every time, and idle navigation cells that drew no quad shifted every later quad when
//! hovered; either way a hover repainted far more than the cell and took ~100 ms. See `memo.rs`
//! and `style::nav_cell`.

use iced::{window, Color, Event, Font, Pixels, Point, Rectangle, Size};
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_tiny_skia::core::{clipboard, mouse, renderer};
use iced_tiny_skia::graphics::{damage, Viewport};
use iced_tiny_skia::{window::compositor, Layer, Renderer};
use std::time::{Duration, Instant};
use wg_app::config::Config;
use wg_app_iced::settings::{Msg, Page, SettingsWindow};
use wg_app_iced::{style, AppState, Paths};
use wg_core::settings::Settings;

const SIZE: (u32, u32) = (960, 680);

const PAGES: [(&str, Page); 7] = [
    ("grid", Page::Grid),
    ("appearance", Page::Appearance),
    ("behavior", Page::Behavior),
    ("hud", Page::Hud),
    ("shortcuts", Page::Shortcuts),
    ("info", Page::Info),
    ("hood", Page::Hood),
];

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn worst(v: &[f64]) -> f64 {
    v.iter().copied().fold(0.0, f64::max)
}

fn window_on(page: Page) -> SettingsWindow {
    let dir = std::env::temp_dir();
    let mut state = AppState { config: Config { settings: Settings::default(), path: dir.join("wg-iced-frame-cost.settings") }, shared: None };
    let paths = Paths { work_dir: dir.clone(), log_file: dir.join("none.log"), version: "test".into() };
    let mut w = SettingsWindow::new(window::Id::unique(), &state, &paths);
    w.update(Msg::Page(page), &mut state);
    w
}

struct Probe {
    size: (u32, u32),
    renderer: Renderer,
    viewport: Viewport,
    theme: iced::Theme,
    pixels: Vec<u8>,
    mask: tiny_skia::Mask,
}

impl Probe {
    fn new() -> Probe {
        Probe::sized(SIZE)
    }

    fn sized((w, h): (u32, u32)) -> Probe {
        Probe {
            size: (w, h),
            renderer: Renderer::new(Font::with_name("Segoe UI"), Pixels(16.0)),
            viewport: Viewport::with_physical_size(Size::new(w, h), 1.0),
            theme: style::theme(true),
            pixels: vec![0; (w * h * 4) as usize],
            mask: tiny_skia::Mask::new(w, h).unwrap(),
        }
    }

    fn bounds(&self) -> Size {
        Size::new(self.size.0 as f32, self.size.1 as f32)
    }

    fn style(&self) -> renderer::Style {
        renderer::Style { text_color: self.theme.palette().text }
    }

    /// The window background: the page colour, as in the window.
    fn background(&self) -> Color {
        self.theme.palette().background
    }

    /// Rasterizes only `damage`, as the window does.
    fn raster(&mut self, damage: &[Rectangle]) {
        let (w, h) = self.size;
        let background = self.background();
        let mut pixmap = tiny_skia::PixmapMut::from_bytes(&mut self.pixels, w, h).unwrap();
        self.renderer.draw(&mut pixmap, &mut self.mask, &self.viewport, damage, background);
    }
}

fn area(rects: &[Rectangle]) -> f32 {
    rects.iter().map(|r| r.width * r.height).sum()
}

/// What moving the pointer down the navigation does, move by move.
struct Sweep {
    updates: Vec<f64>,
    draws: Vec<f64>,
    diffs: Vec<f64>,
    rasters: Vec<f64>,
    totals: Vec<f64>,
    /// Damaged area of each move, in pixels.
    damaged: Vec<f64>,
    /// How many separate rectangles each move damaged.
    rects: Vec<usize>,
    /// The rectangles themselves, move by move.
    shapes: Vec<Vec<Rectangle>>,
}

/// Lays the page out, then moves the pointer `moves` times down the navigation (14 px apart),
/// redrawing after each move the way the window does: update, draw, diff against the last frame,
/// rasterize only the damage.
fn hover_sweep(page: Page, moves: usize) -> Sweep {
    let window = window_on(page);
    let mut p = Probe::new();
    let (bounds, style) = (p.bounds(), p.style());
    // Warm-up: the first layout loads fonts and shapes text.
    drop(UserInterface::build(window.view(), bounds, Cache::default(), &mut p.renderer));
    let mut ui = UserInterface::build(window.view(), bounds, Cache::default(), &mut p.renderer);
    // Settle the first frame: widgets work out their status (hovered, pressed, ...) on their first
    // `RedrawRequested`, and the window always delivers one before it draws. Without this the first
    // move would be compared with a frame no window ever shows, and damage the buttons and sliders.
    let mut messages: Vec<Msg> = Vec::new();
    let away = mouse::Cursor::Unavailable;
    ui.update(&[Event::Window(window::Event::RedrawRequested(Instant::now()))], away, &mut p.renderer, &mut clipboard::Null, &mut messages);
    ui.draw(&mut p.renderer, &p.theme, &style, away);
    let mut last = p.renderer.layers().to_vec();
    p.raster(&[Rectangle::with_size(bounds)]);

    let mut s = Sweep { updates: vec![], draws: vec![], diffs: vec![], rasters: vec![], totals: vec![], damaged: vec![], rects: vec![], shapes: vec![] };
    for step in 0..moves {
        let position = Point::new(100.0, 90.0 + (step % 20) as f32 * 14.0);
        let cursor = mouse::Cursor::Available(position);
        let t0 = Instant::now();
        ui.update(&[Event::Mouse(mouse::Event::CursorMoved { position })], cursor, &mut p.renderer, &mut clipboard::Null, &mut messages);
        ui.update(&[Event::Window(window::Event::RedrawRequested(Instant::now()))], cursor, &mut p.renderer, &mut clipboard::Null, &mut messages);
        let up = t0.elapsed();
        let t = Instant::now();
        ui.draw(&mut p.renderer, &p.theme, &style, cursor);
        let draw = t.elapsed();
        let t = Instant::now();
        let now = p.renderer.layers().to_vec();
        let dmg = damage::diff(&last, &now, |l: &Layer| vec![l.bounds], Layer::damage);
        let dmg = damage::group(dmg, Rectangle::with_size(bounds));
        let diff = t.elapsed();
        let t = Instant::now();
        if !dmg.is_empty() {
            p.raster(&dmg);
        }
        let raster = t.elapsed();
        last = now;
        s.updates.push(ms(up));
        s.draws.push(ms(draw));
        s.diffs.push(ms(diff));
        s.rasters.push(ms(raster));
        s.totals.push(ms(t0.elapsed()));
        s.damaged.push(f64::from(area(&dmg)));
        s.rects.push(dmg.len());
        s.shapes.push(dmg);
    }
    s
}

#[test]
fn a_hover_repaints_only_the_cells_that_changed() {
    let window_px = f64::from(SIZE.0 * SIZE.1);
    for (name, page) in PAGES {
        let s = hover_sweep(page, 40);
        // Sweeping the pointer across seven navigation cells must show something: the hover.
        assert!(s.damaged.iter().any(|d| *d > 0.0), "{name}: hovering a navigation cell changes nothing on screen");
        // And what it repaints is two cells at most (the one left and the one entered, each about
        // 224 x 44 px, 1.5% of this window), not the navigation or the window. Before the fix a
        // hover damaged the whole navigation column (17%) or the whole window (96%).
        let most = worst(&s.damaged);
        assert!(most < 0.05 * window_px, "{name}: one hover damaged {most:.0} px ({:.1}% of the window)", 100.0 * most / window_px);
        // In few rectangles: every extra rectangle makes the rasterizer walk all the primitives again.
        let rects = s.rects.iter().copied().max().unwrap();
        assert!(rects <= 4, "{name}: one hover damaged {rects} separate rectangles");
    }
}

#[test]
fn switching_pages_is_one_repaint_not_dozens() {
    // Every page to every other page, in both directions: the pages are told apart by tiny
    // differences in their layer's bounds, so neighbours are not enough.
    let mut order = Vec::new();
    for (_, from) in PAGES {
        for (_, to) in PAGES {
            order.push(from);
            order.push(to);
        }
    }
    let frames = frames_after(Page::Grid, &order.iter().map(|p| Msg::Page(*p)).collect::<Vec<_>>());
    let mut previous = Page::Grid;
    for (page, f) in order.iter().zip(&frames) {
        assert!(f.rects <= 4, "switching from {previous:?} to {page:?} damaged {} separate rectangles", f.rects);
        previous = *page;
    }
}

#[test]
fn moves_inside_one_cell_repaint_nothing() {
    for (name, page) in PAGES {
        let s = hover_sweep(page, 30);
        // The pointer changes cells only every few moves; the moves in between change nothing.
        let still = s.damaged.iter().filter(|d| **d == 0.0).count();
        assert!(still >= 10, "{name}: moves inside one cell still repaint ({:?})", s.damaged);
    }
}

/// What it costs to show the result of one message: rebuild the view, lay it out reusing the
/// widget state, draw, diff against the previous frame, rasterize the damage.
struct Frame {
    total: f64,
    layout: f64,
    raster: f64,
    damaged: f64,
    rects: usize,
    /// The damaged rectangles, and the bounds of each layer drawn.
    shapes: Vec<Rectangle>,
    layers: Vec<Rectangle>,
}

/// Applies each message in turn and measures the frame that shows it.
fn frames_after(page: Page, messages: &[Msg]) -> Vec<Frame> {
    let dir = std::env::temp_dir();
    let mut state = AppState { config: Config { settings: Settings::default(), path: dir.join("wg-iced-frame-cost.settings") }, shared: None };
    let paths = Paths { work_dir: dir.clone(), log_file: dir.join("none.log"), version: "test".into() };
    let mut window = SettingsWindow::new(window::Id::unique(), &state, &paths);
    window.update(Msg::Page(page), &mut state);
    let mut p = Probe::new();
    let (bounds, style) = (p.bounds(), p.style());
    let away = mouse::Cursor::Unavailable;
    let redraw = || [Event::Window(window::Event::RedrawRequested(Instant::now()))];
    let mut sink: Vec<Msg> = Vec::new();

    drop(UserInterface::build(window.view(), bounds, Cache::default(), &mut p.renderer)); // warm-up
    let mut first = UserInterface::build(window.view(), bounds, Cache::default(), &mut p.renderer);
    first.update(&redraw(), away, &mut p.renderer, &mut clipboard::Null, &mut sink);
    first.draw(&mut p.renderer, &p.theme, &style, away);
    let mut last = p.renderer.layers().to_vec();
    p.raster(&[Rectangle::with_size(bounds)]);
    // The built UI borrows the window, so it is turned into its cache (the widget state, which
    // is what a real window keeps between frames) before the window is changed.
    let mut cache = first.into_cache();

    let mut frames = Vec::new();
    for message in messages {
        window.update(message.clone(), &mut state);
        let t0 = Instant::now();
        let mut ui = UserInterface::build(window.view(), bounds, cache, &mut p.renderer);
        let layout = t0.elapsed();
        ui.update(&redraw(), away, &mut p.renderer, &mut clipboard::Null, &mut sink);
        ui.draw(&mut p.renderer, &p.theme, &style, away);
        let now = p.renderer.layers().to_vec();
        let dmg = damage::diff(&last, &now, |l: &Layer| vec![l.bounds], Layer::damage);
        let dmg = damage::group(dmg, Rectangle::with_size(bounds));
        let t = Instant::now();
        if !dmg.is_empty() {
            p.raster(&dmg);
        }
        let raster = t.elapsed();
        last = now.clone();
        frames.push(Frame {
            total: ms(t0.elapsed()),
            layout: ms(layout),
            raster: ms(raster),
            damaged: f64::from(area(&dmg)),
            rects: dmg.len(),
            layers: now.iter().map(|l| l.bounds).collect(),
            shapes: dmg,
        });
        cache = ui.into_cache();
    }
    frames
}

#[test]
#[ignore = "prints a timing report; run with --nocapture"]
fn interaction_report() {
    println!("
{:<34} {:>8} {:>8} {:>9} {:>6} {:>8}", "interaction (one message)", "layout", "raster", "damage px", "rects", "frame");
    let sweeps: [(&str, Page, Vec<Msg>); 6] = [
        ("stepper: columns +1", Page::Grid, (13..=24).map(Msg::Columns).collect()),
        ("preset button", Page::Grid, [1usize, 2, 3, 4, 5, 6, 0].map(Msg::Preset).to_vec()),
        ("slider: overlay opacity", Page::Appearance, (0..20).map(|i| Msg::Opacity(0.3 + i as f32 * 0.03)).collect()),
        ("slider: shadow blur", Page::Appearance, (0..20).map(|i| Msg::Blur(i as f32 * 2.0)).collect()),
        ("toggle: radial gradient", Page::Appearance, [false, true, false, true, false, true].map(Msg::Radial).to_vec()),
        ("toggle: HUD (dims 4 rows)", Page::Hud, [false, true, false, true].map(Msg::Hud).to_vec()),
    ];
    for (name, page, messages) in sweeps {
        let frames = frames_after(page, &messages);
        let w = |f: fn(&Frame) -> f64| worst(&frames.iter().map(f).collect::<Vec<_>>());
        println!("{:<34} {:>8.2} {:>8.2} {:>9.0} {:>6} {:>8.2}", name, w(|f| f.layout), w(|f| f.raster), w(|f| f.damaged), frames.iter().map(|f| f.rects).max().unwrap(), w(|f| f.total));
    }
    // Switching pages repaints the whole page area: the cost to watch is the full frame.
    let pages = [Page::Appearance, Page::Behavior, Page::Hud, Page::Shortcuts, Page::Info, Page::Hood, Page::Grid];
    let frames = frames_after(Page::Grid, &pages.map(Msg::Page));
    let w = |f: fn(&Frame) -> f64| worst(&frames.iter().map(f).collect::<Vec<_>>());
    println!("{:<34} {:>8.2} {:>8.2} {:>9.0} {:>6} {:>8.2}", "switch page", w(|f| f.layout), w(|f| f.raster), w(|f| f.damaged), frames.iter().map(|f| f.rects).max().unwrap(), w(|f| f.total));
    println!("(milliseconds, worst of the sweep)");
}

/// Prints the layers and the damaged rectangles of switching pages, for finding what fragments it.
#[test]
#[ignore = "diagnostic; run with --nocapture"]
fn page_switch_shapes() {
    let fmt = |r: &Rectangle| format!("[{:.0},{:.0} {:.0}x{:.0}]", r.x, r.y, r.width, r.height);
    let order = [Page::Appearance, Page::Behavior, Page::Grid];
    for (page, f) in order.iter().zip(frames_after(Page::Grid, &order.map(Msg::Page))) {
        println!("
switch to {page:?}: {} rects, layers: {}", f.rects, f.layers.iter().map(fmt).collect::<Vec<_>>().join(" "));
        println!("  damage: {}", f.shapes.iter().map(fmt).collect::<Vec<_>>().join(" "));
    }
}

/// Prints which rectangles each hover damages, for finding what else changes between frames.
#[test]
#[ignore = "diagnostic; run with --nocapture"]
fn damaged_rectangles() {
    for (name, page) in PAGES {
        let s = hover_sweep(page, 24);
        println!("
{name}:");
        for (step, shapes) in s.shapes.iter().enumerate().filter(|(_, r)| !r.is_empty()) {
            let list: Vec<String> = shapes.iter().map(|r| format!("[{:.0},{:.0} {:.0}x{:.0}]", r.x, r.y, r.width, r.height)).collect();
            println!("  move {step:>2}: {}", list.join(" "));
        }
    }
}

#[test]
#[ignore = "prints a timing report; run with --nocapture"]
fn frame_cost_report() {
    println!("\nsettings window, {} x {} px", SIZE.0, SIZE.1);
    println!(
        "{:<12} {:>8} {:>8} {:>7} | {:>9} {:>8} {:>8} {:>8} {:>9} {:>6} {:>8}",
        "page", "layout", "full", "layers", "hover:up", "draw", "diff", "raster", "damage px", "rects", "total"
    );
    for (name, page) in PAGES {
        let window = window_on(page);
        let mut p = Probe::new();
        let (bounds, style) = (p.bounds(), p.style());
        drop(UserInterface::build(window.view(), bounds, Cache::default(), &mut p.renderer));
        let (mut layouts, mut fulls) = (vec![], vec![]);
        let mut layers = 0;
        for _ in 0..7 {
            let t = Instant::now();
            let mut ui = UserInterface::build(window.view(), bounds, Cache::default(), &mut p.renderer);
            layouts.push(ms(t.elapsed()));
            ui.draw(&mut p.renderer, &p.theme, &style, mouse::Cursor::Unavailable);
            layers = p.renderer.layers().len();
            let t = Instant::now();
            let background = p.background();
            let _ = compositor::screenshot(&mut p.renderer, &p.viewport, background);
            fulls.push(ms(t.elapsed()));
        }
        let s = hover_sweep(page, 40);
        println!(
            "{:<12} {:>8.2} {:>8.2} {:>7} | {:>9.2} {:>8.2} {:>8.2} {:>8.2} {:>9.0} {:>6} {:>8.2}",
            name,
            median(layouts),
            median(fulls),
            layers,
            worst(&s.updates),
            worst(&s.draws),
            worst(&s.diffs),
            worst(&s.rasters),
            worst(&s.damaged),
            s.rects.iter().max().unwrap(),
            worst(&s.totals),
        );
    }
    println!("(milliseconds; the hover columns are the worst of 40 pointer moves down the navigation)");
}

/// What resizing the window costs: every size change lays the page out again and repaints all of
/// it, so this is the frame cost while dragging a window edge or maximizing.
#[test]
#[ignore = "prints a timing report; run with --nocapture"]
fn resize_report() {
    println!("
{:<14} {:>10} {:>9} {:>9} {:>9}", "window size", "pixels", "layout", "raster", "frame");
    for size in [(640, 480), (960, 680), (1280, 800), (1920, 1080), (2560, 1440)] {
        let window = window_on(Page::Grid);
        let mut p = Probe::sized(size);
        let (bounds, style) = (p.bounds(), p.style());
        drop(UserInterface::build(window.view(), bounds, Cache::default(), &mut p.renderer));
        let (mut layouts, mut rasters) = (vec![], vec![]);
        for _ in 0..9 {
            let t = Instant::now();
            let mut ui = UserInterface::build(window.view(), bounds, Cache::default(), &mut p.renderer);
            layouts.push(ms(t.elapsed()));
            ui.draw(&mut p.renderer, &p.theme, &style, mouse::Cursor::Unavailable);
            let t = Instant::now();
            p.raster(&[Rectangle::with_size(bounds)]);
            rasters.push(ms(t.elapsed()));
        }
        let (l, r) = (median(layouts), median(rasters));
        println!("{:<14} {:>10} {:>7.2}ms {:>7.2}ms {:>7.2}ms", format!("{} x {}", size.0, size.1), size.0 * size.1, l, r, l + r);
    }
    println!("(median of 9 full repaints)");
}

/// The first frame of a process, stage by stage: iced loads the system fonts the first time text is
/// measured, and shapes and rasterizes each glyph the first time it is drawn. Run it alone, since
/// whatever runs before it in the same process has already paid:
/// `-- --ignored --nocapture --exact first_paint_report`.
#[test]
#[ignore = "prints a timing report; run alone with --exact"]
fn first_paint_report() {
    let t = Instant::now();
    drop(iced_tiny_skia::graphics::text::font_system().read().unwrap());
    let fonts = ms(t.elapsed());

    let window = window_on(Page::Grid);
    let mut p = Probe::new();
    let (bounds, style) = (p.bounds(), p.style());
    let mut stages = Vec::new();
    for pass in ["first", "second"] {
        let t = Instant::now();
        let mut ui = UserInterface::build(window.view(), bounds, Cache::default(), &mut p.renderer);
        let layout = ms(t.elapsed());
        ui.draw(&mut p.renderer, &p.theme, &style, mouse::Cursor::Unavailable);
        let t = Instant::now();
        p.raster(&[Rectangle::with_size(bounds)]);
        stages.push((pass, layout, ms(t.elapsed())));
    }
    println!("
system fonts loaded: {fonts:.1} ms");
    for (pass, layout, raster) in stages {
        println!("{pass:<7} frame: layout {layout:>6.1} ms, raster {raster:>6.1} ms");
    }
}
