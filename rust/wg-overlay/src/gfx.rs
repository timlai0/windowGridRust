//! Direct2D / DirectWrite rendering of the overlay scene.
//!
//! The renderer draws into whatever bitmap is currently set as the device context's target:
//! a DirectComposition swap-chain buffer in the real overlay, or an offscreen bitmap for the
//! tests and PNG dumps. Everything is GPU-side; nothing is read back except by the tests.

use crate::geom::{round_even, RectF};
use crate::hud::{self, HintBar, HudText};
use crate::theme::{Color, Palette};
use windows::core::{w, Interface, PCWSTR, Result};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

/// Colours and metrics of the HUD chrome, in the legacy overlay's device-independent pixels.
const PILL_BG: Color = Color { r: 24.0 / 255.0, g: 24.0 / 255.0, b: 24.0 / 255.0, a: 242.0 / 255.0 };
const PILL_BORDER: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 102.0 / 255.0 };
const ACCENT: Color = Color { r: 76.0 / 255.0, g: 194.0 / 255.0, b: 1.0, a: 1.0 };

fn gray(v: u8) -> Color {
    Color::from_argb8(255, v, v, v)
}

fn d2d(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}

fn rect_f(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: l, top: t, right: r, bottom: b }
}

fn rounded(l: f32, t: f32, r: f32, b: f32, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: rect_f(l, t, r, b), radiusX: radius, radiusY: radius }
}

/// What to draw. Sizes are in overlay pixels; `scale` is the DPI scale (1.0 = 96 dpi).
pub struct Scene<'a> {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub cols: i32,
    pub rows: i32,
    pub palette: Palette,
    pub selection: Option<SelectionScene<'a>>,
    pub hint: Option<HintBar>,
}

pub struct SelectionScene<'a> {
    pub rect: RectF,
    pub hud: Option<&'a HudText>,
}

/// GPU devices shared by every renderer target.
pub struct Devices {
    pub d3d: ID3D11Device,
    pub dxgi: IDXGIDevice,
    pub d2d_device: ID2D1Device,
    pub ctx: ID2D1DeviceContext,
    pub dwrite: IDWriteFactory,
}

impl Devices {
    pub fn new() -> Result<Devices> {
        let d3d = create_d3d_device(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| create_d3d_device(D3D_DRIVER_TYPE_WARP))?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        // SAFETY: plain factory/device creation with valid arguments.
        unsafe {
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = factory.CreateDevice(&dxgi)?;
            let ctx = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Devices { d3d, dxgi, d2d_device, ctx, dwrite })
        }
    }
}

fn create_d3d_device(driver: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    // SAFETY: out pointer is valid; no software module is used.
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    device.ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(0x8000_4005u32 as i32)))
}

/// A run of HUD text, measured and ready to draw.
struct Run {
    layout: IDWriteTextLayout,
    width: f32,
    height: f32,
    color: Color,
    /// Space after the run, in pixels.
    gap: f32,
}

pub struct Renderer {
    pub dev: Devices,
}

impl Renderer {
    pub fn new() -> Result<Renderer> {
        Ok(Renderer { dev: Devices::new()? })
    }

    /// Draws the scene into the context's current target.
    pub fn draw(&self, scene: &Scene) -> Result<()> {
        let ctx = &self.dev.ctx;
        // SAFETY: all calls are made between BeginDraw/EndDraw on a valid context with a
        // target set by the caller; interface pointers passed are alive for the call.
        unsafe {
            ctx.SetDpi(96.0, 96.0);
            ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            ctx.BeginDraw();
            ctx.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            let result = self.draw_inner(scene);
            let end = ctx.EndDraw(None, None);
            result.and(end)
        }
    }

    unsafe fn draw_inner(&self, scene: &Scene) -> Result<()> {
        let ctx = &self.dev.ctx;
        let s = scene.scale;
        let layer = D2D1_LAYER_PARAMETERS1 {
            contentBounds: rect_f(f32::MIN, f32::MIN, f32::MAX, f32::MAX),
            geometricMask: std::mem::ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: identity(),
            opacity: scene.palette.opacity,
            opacityBrush: std::mem::ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        };
        ctx.PushLayer(&layer, None);

        // Dim the whole work area slightly (#26000000 in the legacy overlay).
        let dim = ctx.CreateSolidColorBrush(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 38.0 / 255.0 }, None)?;
        ctx.FillRectangle(&rect_f(0.0, 0.0, scene.width, scene.height), &dim);

        self.draw_grid(scene)?;
        if let Some(sel) = &scene.selection {
            self.draw_selection(scene, sel)?;
            if let Some(text) = sel.hud {
                self.draw_hud(scene, sel.rect, text)?;
            }
        }
        if let Some(bar) = scene.hint {
            self.draw_hint_bar(scene, bar)?;
        }
        ctx.PopLayer();
        let _ = s;
        Ok(())
    }

    /// Grid lines: two crisp pixels per line, plus a one-pixel frame.
    unsafe fn draw_grid(&self, scene: &Scene) -> Result<()> {
        let ctx = &self.dev.ctx;
        let brush = ctx.CreateSolidColorBrush(&d2d(scene.palette.line), None)?;
        ctx.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        let (w, h) = (scene.width, scene.height);
        for i in 1..scene.cols {
            let x = round_even(f64::from(w) * f64::from(i) / f64::from(scene.cols)) as f32;
            ctx.FillRectangle(&rect_f(x - 1.0, 0.0, x + 1.0, h), &brush);
        }
        for j in 1..scene.rows {
            let y = round_even(f64::from(h) * f64::from(j) / f64::from(scene.rows)) as f32;
            ctx.FillRectangle(&rect_f(0.0, y - 1.0, w, y + 1.0), &brush);
        }
        ctx.FillRectangle(&rect_f(0.0, 0.0, w, 1.0), &brush);
        ctx.FillRectangle(&rect_f(0.0, h - 1.0, w, h), &brush);
        ctx.FillRectangle(&rect_f(0.0, 0.0, 1.0, h), &brush);
        ctx.FillRectangle(&rect_f(w - 1.0, 0.0, w, h), &brush);
        ctx.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        Ok(())
    }

    unsafe fn draw_selection(&self, scene: &Scene, sel: &SelectionScene) -> Result<()> {
        let ctx = &self.dev.ctx;
        let s = scene.scale;
        let r = sel.rect;
        let radius = 6.0 * s;
        let rr = rounded(r.left, r.top, r.right(), r.bottom(), radius);

        if scene.palette.shadow_blur > 0.0 {
            if scene.palette.fill {
                self.soft_shadow(&rr, scene.palette.shadow_blur * s, 2.0 * s, 0.75)?;
            } else {
                // No fill: the shadow must not darken the (empty) interior.
                self.shadow_ring(&rr, scene.palette.shadow_blur * s, 2.0 * s, 0.75)?;
            }
        }

        if scene.palette.fill {
            let brush = self.selection_brush(&scene.palette, r)?;
            ctx.FillRoundedRectangle(&rr, &brush);
        }

        // 2 px border, drawn inside the box.
        let bw = 2.0 * s;
        let line = ctx.CreateSolidColorBrush(&d2d(scene.palette.line), None)?;
        let inset = bw / 2.0;
        let br = rounded(r.left + inset, r.top + inset, r.right() - inset, r.bottom() - inset, (radius - inset).max(0.0));
        ctx.DrawRoundedRectangle(&br, &line, bw, None);
        Ok(())
    }

    /// Radial (dark centre) or diagonal-linear gradient over the selection, as in the legacy
    /// overlay: stops are defined on the unit square and mapped onto the rectangle.
    unsafe fn selection_brush(&self, p: &Palette, r: RectF) -> Result<ID2D1Brush> {
        let ctx = &self.dev.ctx;
        let (c0, c1) = if p.radial { (p.start.scaled_linear(0.7), p.start) } else { (p.start, p.end) };
        let stops = [
            D2D1_GRADIENT_STOP { position: 0.0, color: d2d(c0) },
            D2D1_GRADIENT_STOP { position: 1.0, color: d2d(c1) },
        ];
        let collection = ctx.CreateGradientStopCollection(
            &stops,
            D2D1_COLOR_SPACE_SRGB,
            D2D1_COLOR_SPACE_SRGB,
            D2D1_BUFFER_PRECISION_8BPC_UNORM,
            D2D1_EXTEND_MODE_CLAMP,
            D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
        )?;
        let props = D2D1_BRUSH_PROPERTIES {
            opacity: 1.0,
            transform: windows_numerics::Matrix3x2 { M11: r.width, M12: 0.0, M21: 0.0, M22: r.height, M31: r.left, M32: r.top },
        };
        if p.radial {
            let radial = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                center: windows_numerics::Vector2 { X: 0.5, Y: 0.5 },
                gradientOriginOffset: windows_numerics::Vector2 { X: 0.0, Y: 0.0 },
                radiusX: 0.5,
                radiusY: 0.5,
            };
            ctx.CreateRadialGradientBrush(&radial, Some(&props), &collection)?.cast()
        } else {
            let linear = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: windows_numerics::Vector2 { X: 0.0, Y: 0.0 },
                endPoint: windows_numerics::Vector2 { X: 1.0, Y: 1.0 },
            };
            ctx.CreateLinearGradientBrush(&linear, Some(&props), &collection)?.cast()
        }
    }

    /// A drop shadow built from stacked, progressively larger translucent rounded rectangles
    /// (outermost first), which avoids an effect graph and any intermediate bitmaps.
    unsafe fn soft_shadow(&self, rr: &D2D1_ROUNDED_RECT, blur: f32, dy: f32, opacity: f32) -> Result<()> {
        let ctx = &self.dev.ctx;
        let steps = ((blur / 2.0).ceil() as i32).clamp(3, 16);
        let black = ctx.CreateSolidColorBrush(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }, None)?;
        // Cumulative coverage wanted at layer k (k = steps is the outermost ring).
        let coverage = |k: i32| -> f32 {
            if k == 0 {
                opacity
            } else {
                let t = (k as f32 - 0.5) / steps as f32;
                opacity * 0.5 * (1.0 - t) * (1.0 - t)
            }
        };
        let mut outer_cov = 0.0f32;
        for k in (0..=steps).rev() {
            let target = coverage(k);
            let alpha = (1.0 - (1.0 - target) / (1.0 - outer_cov)).clamp(0.0, 1.0);
            outer_cov = target;
            let d = blur * k as f32 / steps as f32;
            black.SetOpacity(alpha);
            let g = D2D1_ROUNDED_RECT {
                rect: rect_f(rr.rect.left - d, rr.rect.top - d + dy, rr.rect.right + d, rr.rect.bottom + d + dy),
                radiusX: rr.radiusX + d,
                radiusY: rr.radiusY + d,
            };
            ctx.FillRoundedRectangle(&g, &black);
        }
        Ok(())
    }

    /// Outer-only glow: concentric strokes whose opacity falls off with distance.
    unsafe fn shadow_ring(&self, rr: &D2D1_ROUNDED_RECT, blur: f32, dy: f32, opacity: f32) -> Result<()> {
        let ctx = &self.dev.ctx;
        let steps = ((blur / 2.0).ceil() as i32).clamp(3, 16);
        let black = ctx.CreateSolidColorBrush(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }, None)?;
        let width = blur / steps as f32;
        for k in 1..=steps {
            let t = (k as f32 - 0.5) / steps as f32;
            black.SetOpacity(opacity * 0.5 * (1.0 - t) * (1.0 - t));
            let d = width * (k as f32 - 0.5);
            let g = D2D1_ROUNDED_RECT {
                rect: rect_f(rr.rect.left - d, rr.rect.top - d + dy, rr.rect.right + d, rr.rect.bottom + d + dy),
                radiusX: rr.radiusX + d,
                radiusY: rr.radiusY + d,
            };
            ctx.DrawRoundedRectangle(&g, &black, width + 0.5, None);
        }
        Ok(())
    }

    fn format(&self, size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
        // SAFETY: valid factory and NUL-terminated wide literals.
        unsafe {
            let f = self.dev.dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            )?;
            f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(f)
        }
    }

    fn run(&self, text: &str, size: f32, weight: DWRITE_FONT_WEIGHT, color: Color, gap: f32) -> Result<Run> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let format = self.format(size, weight)?;
        // SAFETY: the slice and format are valid for the call.
        unsafe {
            let layout = self.dev.dwrite.CreateTextLayout(&wide, &format, 10_000.0, 1_000.0)?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            Ok(Run { layout, width: m.widthIncludingTrailingWhitespace, height: m.height, color, gap })
        }
    }

    unsafe fn draw_run(&self, run: &Run, x: f32, y: f32) -> Result<()> {
        let brush = self.dev.ctx.CreateSolidColorBrush(&d2d(run.color), None)?;
        self.dev.ctx.DrawTextLayout(windows_numerics::Vector2 { X: x, Y: y }, &run.layout, &brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
        Ok(())
    }

    /// Dark rounded pill with a 1 px light border and a soft shadow.
    unsafe fn draw_pill(&self, l: f32, t: f32, w: f32, h: f32, radius: f32, s: f32) -> Result<()> {
        let ctx = &self.dev.ctx;
        let rr = rounded(l, t, l + w, t + h, radius);
        self.soft_shadow(&rr, 14.0 * s, 2.0 * s, 0.6)?;
        let bg = ctx.CreateSolidColorBrush(&d2d(PILL_BG), None)?;
        ctx.FillRoundedRectangle(&rr, &bg);
        let bw = (1.0 * s).max(1.0);
        let border = ctx.CreateSolidColorBrush(&d2d(PILL_BORDER), None)?;
        let half = bw / 2.0;
        ctx.DrawRoundedRectangle(&rounded(l + half, t + half, l + w - half, t + h - half, (radius - half).max(0.0)), &border, bw, None);
        Ok(())
    }

    unsafe fn draw_hud(&self, scene: &Scene, sel: RectF, text: &HudText) -> Result<()> {
        let s = scene.scale;
        let mut runs: Vec<Run> = Vec::new();
        if let Some(t) = &text.dimensions {
            runs.push(self.run(t, 13.0 * s, DWRITE_FONT_WEIGHT_BOLD, gray(255), 10.0 * s)?);
        }
        if let Some(t) = &text.cell_span {
            runs.push(self.run(t, 12.0 * s, DWRITE_FONT_WEIGHT_NORMAL, gray(0xCC), 10.0 * s)?);
        }
        if let Some(t) = &text.coverage {
            runs.push(self.run(t, 12.0 * s, DWRITE_FONT_WEIGHT_NORMAL, gray(0xAA), 10.0 * s)?);
        }
        if let Some(t) = &text.aspect {
            runs.push(self.run(t, 12.0 * s, DWRITE_FONT_WEIGHT_SEMI_BOLD, ACCENT, 14.0 * s)?);
        }
        let sep_gap = 14.0 * s;
        let trailing = [
            self.run(&text.grid_info, 11.0 * s, DWRITE_FONT_WEIGHT_NORMAL, gray(0x99), 8.0 * s)?,
            self.run(&text.screen_info, 11.0 * s, DWRITE_FONT_WEIGHT_NORMAL, gray(0x99), 0.0)?,
        ];

        let sep_w = (1.0 * s).max(1.0);
        let mut content_w: f32 = runs.iter().map(|r| r.width + r.gap).sum();
        content_w += sep_w + sep_gap;
        content_w += trailing.iter().map(|r| r.width + r.gap).sum::<f32>();
        let content_h = runs.iter().chain(trailing.iter()).map(|r| r.height).fold(0.0f32, f32::max);

        let (pad_x, pad_y, border) = (14.0 * s, 7.0 * s, (1.0 * s).max(1.0));
        let pill_w = content_w + 2.0 * (pad_x + border);
        let pill_h = content_h + 2.0 * (pad_y + border);
        let (px, py) = hud::place_hud(sel, pill_w, pill_h, scene.width, scene.height, s);
        self.draw_pill(px, py, pill_w, pill_h, 8.0 * s, s)?;

        let mut x = px + border + pad_x;
        let cy = py + pill_h / 2.0;
        for r in &runs {
            self.draw_run(r, x, cy - r.height / 2.0)?;
            x += r.width + r.gap;
        }
        // Thin divider, inset 3 px top and bottom.
        let div = self.dev.ctx.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 77.0 / 255.0 }, None)?;
        let inset = 3.0 * s;
        self.dev.ctx.FillRectangle(&rect_f(x, cy - content_h / 2.0 + inset, x + sep_w, cy + content_h / 2.0 - inset), &div);
        x += sep_w + sep_gap;
        for r in &trailing {
            self.draw_run(r, x, cy - r.height / 2.0)?;
            x += r.width + r.gap;
        }
        Ok(())
    }

    unsafe fn draw_hint_bar(&self, scene: &Scene, bar: HintBar) -> Result<()> {
        let s = scene.scale;
        let tag = match bar.tag {
            Some(t) => Some(self.run(t, 11.0 * s, DWRITE_FONT_WEIGHT_BOLD, ACCENT, 10.0 * s)?),
            None => None,
        };
        let hint = match bar.hint {
            Some(t) => Some(self.run(t, 12.0 * s, DWRITE_FONT_WEIGHT_NORMAL, gray(0xE0), 0.0)?),
            None => None,
        };
        let (tag_pad_x, tag_pad_y) = (6.0 * s, 2.0 * s);
        let mut content_w = 0.0;
        let mut content_h: f32 = 0.0;
        if let Some(t) = &tag {
            content_w += t.width + 2.0 * tag_pad_x + t.gap;
            content_h = content_h.max(t.height + 2.0 * tag_pad_y);
        }
        if let Some(h) = &hint {
            content_w += h.width;
            content_h = content_h.max(h.height);
        }
        let (pad_x, pad_y, border) = (16.0 * s, 7.0 * s, (1.0 * s).max(1.0));
        let w = content_w + 2.0 * (pad_x + border);
        let h = content_h + 2.0 * (pad_y + border);
        let l = ((scene.width - w) / 2.0).round();
        let t = 28.0 * s;
        self.draw_pill(l, t, w, h, 14.0 * s, s)?;

        let cy = t + h / 2.0;
        let mut x = l + border + pad_x;
        if let Some(tg) = &tag {
            let tw = tg.width + 2.0 * tag_pad_x;
            let th = tg.height + 2.0 * tag_pad_y;
            let chip = self.dev.ctx.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 51.0 / 255.0 }, None)?;
            self.dev.ctx.FillRoundedRectangle(&rounded(x, cy - th / 2.0, x + tw, cy + th / 2.0, 4.0 * s), &chip);
            self.draw_run(tg, x + tag_pad_x, cy - tg.height / 2.0)?;
            x += tw + tg.gap;
        }
        if let Some(hn) = &hint {
            self.draw_run(hn, x, cy - hn.height / 2.0)?;
        }
        Ok(())
    }

    // ---- targets -----------------------------------------------------------------------

    /// Points the context at a DXGI surface (a swap-chain buffer).
    pub fn set_target_surface(&self, surface: &IDXGISurface) -> Result<()> {
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        // SAFETY: the surface and properties are valid for the call.
        unsafe {
            let bitmap = self.dev.ctx.CreateBitmapFromDxgiSurface(surface, Some(&props))?;
            self.dev.ctx.SetTarget(&bitmap);
        }
        Ok(())
    }

    pub fn clear_target(&self) {
        // SAFETY: setting no target is always valid.
        unsafe { self.dev.ctx.SetTarget(None) };
    }

    /// Renders `scene` offscreen and returns straight-alpha RGBA, composited over a neutral
    /// checkerboard so translucency is visible in the PNG.
    pub fn render_to_rgba(&self, scene: &Scene, checker: bool) -> Result<Vec<u8>> {
        let (w, h) = (scene.width as u32, scene.height as u32);
        let ctx = &self.dev.ctx;
        // SAFETY: bitmaps are created with matching sizes/formats and mapped only while alive.
        unsafe {
            let size = D2D_SIZE_U { width: w, height: h };
            let fmt = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED };
            let target = ctx.CreateBitmap(
                size,
                None,
                0,
                &D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: fmt,
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                    colorContext: std::mem::ManuallyDrop::new(None),
                },
            )?;
            let staging = ctx.CreateBitmap(
                size,
                None,
                0,
                &D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: fmt,
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    colorContext: std::mem::ManuallyDrop::new(None),
                },
            )?;
            ctx.SetTarget(&target);
            self.draw(scene)?;
            ctx.SetTarget(None);
            staging.CopyFromBitmap(None, &target, None)?;
            let mapped = staging.Map(D2D1_MAP_OPTIONS_READ)?;
            let mut out = vec![0u8; (w * h * 4) as usize];
            for y in 0..h as usize {
                let row = std::slice::from_raw_parts(mapped.bits.add(y * mapped.pitch as usize), (w * 4) as usize);
                for x in 0..w as usize {
                    let (b, g, r, a) = (row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]);
                    let bg = if !checker {
                        0.0
                    } else if ((x / 16) + (y / 16)) % 2 == 0 {
                        0x3C as f32
                    } else {
                        0x50 as f32
                    };
                    let inv = 1.0 - f32::from(a) / 255.0;
                    // Premultiplied source over an opaque grey background.
                    let blend = |c: u8| (f32::from(c) + bg * inv).round().clamp(0.0, 255.0) as u8;
                    let o = (y * w as usize + x) * 4;
                    out[o] = blend(r);
                    out[o + 1] = blend(g);
                    out[o + 2] = blend(b);
                    out[o + 3] = 255;
                }
            }
            staging.Unmap()?;
            Ok(out)
        }
    }
}

fn identity() -> windows_numerics::Matrix3x2 {
    windows_numerics::Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 }
}

pub use wg_win::wide;

#[allow(dead_code)]
fn _unused(_: PCWSTR) {}
