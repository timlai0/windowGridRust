//! Colours for the grid, selection and HUD.
//!
//! The legacy overlay used WPF colour arithmetic, so the darker centre of the radial
//! selection gradient is `colorStart * 0.7` in scRGB (linear) space with alpha scaled too;
//! that is reproduced here.

use wg_core::ipc::GridWindowMemory;

/// Non-premultiplied sRGB colour with float channels in `0..=1`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub fn from_argb8(a: u8, r: u8, g: u8, b: u8) -> Color {
        Color { r: f32::from(r) / 255.0, g: f32::from(g) / 255.0, b: f32::from(b) / 255.0, a: f32::from(a) / 255.0 }
    }

    /// From the packed `0xAARRGGBB` int stored in shared memory.
    pub fn from_packed(v: i32) -> Color {
        let [a, r, g, b] = (v as u32).to_be_bytes();
        Color::from_argb8(a, r, g, b)
    }

    pub fn alpha8(&self) -> u8 {
        (self.a * 255.0).round() as u8
    }

    pub fn with_alpha(mut self, a: f32) -> Color {
        self.a = a;
        self
    }

    /// WPF's `Color * factor`: scales the linear RGB channels and the alpha.
    pub fn scaled_linear(&self, factor: f32) -> Color {
        Color {
            r: linear_to_srgb(srgb_to_linear(self.r) * factor),
            g: linear_to_srgb(srgb_to_linear(self.g) * factor),
            b: linear_to_srgb(srgb_to_linear(self.b) * factor),
            a: (self.a * factor).clamp(0.0, 1.0),
        }
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Resolved appearance of one grid session.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Palette {
    pub line: Color,
    pub start: Color,
    pub end: Color,
    pub radial: bool,
    pub fill: bool,
    /// Whole-overlay opacity.
    pub opacity: f32,
    /// Selection shadow blur in device-independent pixels (0 = no shadow).
    pub shadow_blur: f32,
}

impl Palette {
    pub fn from_memory(m: &GridWindowMemory) -> Palette {
        let packed = |v: i32, default: Color| if v == 0 { default } else { Color::from_packed(v) };
        let mut line = packed(m.color_line, Color::from_argb8(220, 0, 210, 230));
        let mut start = packed(m.color_start, Color::from_argb8(200, 0, 151, 167));
        let mut end = packed(m.color_end, Color::from_argb8(200, 178, 235, 242));
        if line.alpha8() < 120 {
            line.a = 220.0 / 255.0;
        }
        for c in [&mut start, &mut end] {
            if c.alpha8() < 120 {
                c.a = 190.0 / 255.0;
            }
        }
        let opacity = if m.opacity < 0.3 { 0.85 } else { m.opacity.min(1.0) };
        Palette {
            line,
            start,
            end,
            radial: m.radial_gradient != 0,
            fill: m.fill_window != 0,
            opacity,
            // 3 (the default) reproduces the legacy 18 px shadow.
            shadow_blur: (m.blur_radius.max(0.0)) * 6.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_colours_round_trip() {
        let c = Color::from_packed(0xFF00_BDD4u32 as i32);
        assert_eq!((c.alpha8(), (c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8), (255, 0, 189, 212));
    }

    #[test]
    fn wpf_style_scaling_darkens_in_linear_space_and_scales_alpha() {
        let c = Color::from_argb8(200, 0, 151, 167).scaled_linear(0.7);
        assert!((c.a - 200.0 / 255.0 * 0.7).abs() < 1e-6);
        // 151 -> linear 0.309 -> * 0.7 -> 0.216 -> sRGB ~ 0.5 (127.. not 0.7 * 151 = 106)
        let g = (c.g * 255.0).round() as u8;
        assert!((120..=135).contains(&g), "got {g}");
        assert_eq!(c.r, 0.0);
    }

    #[test]
    fn palette_applies_legacy_defaults_and_alpha_floors() {
        let mut m = GridWindowMemory {
            opacity: 0.1,
            blur_radius: 3.0,
            radial_gradient: 1,
            fill_window: 1,
            ..GridWindowMemory::default()
        };
        let p = Palette::from_memory(&m);
        assert_eq!(p.line.alpha8(), 220);
        assert_eq!(p.start.alpha8(), 200);
        assert_eq!(p.opacity, 0.85, "very low opacity is replaced by 0.85");
        assert_eq!(p.shadow_blur, 18.0);

        m.color_line = 0x2000_FF00u32 as i32; // alpha 0x20 < 120 -> 220
        m.color_start = 0x1000_8000u32 as i32; // alpha 0x10 < 120 -> 190
        m.opacity = 0.75;
        let p = Palette::from_memory(&m);
        assert_eq!(p.line.alpha8(), 220);
        assert_eq!(p.start.alpha8(), 190);
        assert_eq!(p.opacity, 0.75);
    }
}
