//! Colours and widget styles. The look is built on the grid the app is about: square cells
//! with hairline borders, tables of label and control cells, and the overlay's own
//! selection-rectangle treatment (accent outline, tinted fill) for what is active.
//!
//! Nothing here uses a shadow: iced's software renderer paints a shadow without honouring the
//! clip, which darkens the contents of the box on partial redraws.

use iced::overlay::menu;
use iced::theme::Palette;
use iced::widget::{button, container, pick_list, slider, text, text_input};
use iced::{color, Background, Border, Color, Shadow, Theme};

/// The colours the windows are drawn with, for the light or the dark mode.
#[derive(Clone, Copy)]
pub struct Tokens {
    /// The window background, and the field the grid is drawn on.
    pub page: Color,
    /// The navigation panel.
    pub nav: Color,
    /// Cards.
    pub card: Color,
    /// The heading cell of a card.
    pub header: Color,
    pub text: Color,
    pub accent: Color,
    pub dark: bool,
}

impl Tokens {
    /// The border of a cell.
    pub fn line(&self) -> Color {
        with_alpha(self.text, 0.16)
    }

    /// A rule inside a cell.
    pub fn rule(&self) -> Color {
        with_alpha(self.text, 0.10)
    }

    /// The grid drawn behind the page.
    pub fn grid(&self) -> Color {
        with_alpha(self.text, 0.045)
    }

    /// Every fourth line of the grid.
    pub fn grid_major(&self) -> Color {
        with_alpha(self.text, 0.085)
    }

    /// Text on an accent-coloured fill.
    pub fn on_accent(&self) -> Color {
        if self.dark {
            color!(0x00131c)
        } else {
            Color::WHITE
        }
    }
}

const DARK: Tokens = Tokens {
    page: color!(0x1e1e1e),
    nav: color!(0x171717),
    card: color!(0x262626),
    header: color!(0x2d2d2d),
    text: color!(0xffffff),
    accent: color!(0x4cc2ff),
    dark: true,
};

const LIGHT: Tokens = Tokens {
    page: color!(0xf1f1f1),
    nav: color!(0xe7e7e7),
    card: color!(0xffffff),
    header: color!(0xf6f6f6),
    text: color!(0x1b1b1b),
    accent: color!(0x0067c0),
    dark: false,
};

pub fn theme(dark: bool) -> Theme {
    let t = if dark { DARK } else { LIGHT };
    let name = if dark { "WindowGridRust Dark" } else { "WindowGridRust Light" };
    Theme::custom(
        name.to_string(),
        Palette {
            background: t.page,
            text: t.text,
            primary: t.accent,
            success: color!(0x2e8b57),
            warning: color!(0xe6a700),
            danger: color!(0xc42b2b),
        },
    )
}

/// The theme of the splash window: the normal colours on a clear background, so the logo floats on
/// the desktop. (The other windows use the page colour as their window background, see
/// [`crate::app`]'s `style`: iced fills each damaged rectangle with it by a fast unmasked fill,
/// where a full-window container background is a huge quad redrawn through a mask for every
/// damaged rectangle.)
pub fn clear_theme(dark: bool) -> Theme {
    let t = if dark { DARK } else { LIGHT };
    Theme::custom(
        "WindowGridRust Clear".to_string(),
        Palette {
            background: Color::TRANSPARENT,
            text: t.text,
            primary: t.accent,
            success: color!(0x2e8b57),
            warning: color!(0xe6a700),
            danger: color!(0xc42b2b),
        },
    )
}

pub fn tokens(theme: &Theme) -> Tokens {
    if theme.extended_palette().is_dark {
        DARK
    } else {
        LIGHT
    }
}

pub fn with_alpha(c: Color, alpha: f32) -> Color {
    Color { a: c.a * alpha, ..c }
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
}

/// Corner radius of every cell and control: nearly square.
const RADIUS: f32 = 2.0;

fn border(color: Color, width: f32) -> Border {
    Border { color, width, radius: RADIUS.into() }
}

// ---- containers ---------------------------------------------------------------------------------

pub fn nav(theme: &Theme) -> container::Style {
    container::Style { background: Some(tokens(theme).nav.into()), ..Default::default() }
}

/// The bar of buttons at the bottom of the settings window.
pub fn bar(theme: &Theme) -> container::Style {
    container::Style { background: Some(tokens(theme).nav.into()), ..Default::default() }
}

pub fn card(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style { background: Some(t.card.into()), border: border(t.line(), 1.0), ..Default::default() }
}

/// The heading cell of a card.
pub fn card_header(theme: &Theme) -> container::Style {
    container::Style { background: Some(tokens(theme).header.into()), ..Default::default() }
}

/// A rule between rows or cells; give it a width or a height of 1.
pub fn rule(theme: &Theme) -> container::Style {
    container::Style { background: Some(tokens(theme).rule().into()), ..Default::default() }
}

pub fn chip(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style { background: Some(t.page.into()), border: border(t.line(), 1.0), ..Default::default() }
}

/// A plain colour swatch.
pub fn swatch(c: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(c.into()),
        border: border(with_alpha(Color::WHITE, 0.3), 1.0),
        ..Default::default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusKind {
    Info,
    Success,
    Warning,
}

pub fn status_bar(kind: StatusKind) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(
            match kind {
                StatusKind::Info => color!(0x1f4e79),
                StatusKind::Success => color!(0x1e5c2f),
                StatusKind::Warning => color!(0x6b4e0b),
            }
            .into(),
        ),
        text_color: Some(Color::WHITE),
        ..Default::default()
    }
}

/// The dimmed layer under a confirmation.
pub fn scrim(_: &Theme) -> container::Style {
    container::Style { background: Some(color!(0x000000, 0.63).into()), ..Default::default() }
}

/// The confirmation box itself.
pub fn dialog(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(t.page.into()),
        border: Border { color: with_alpha(t.text, 0.35), width: 1.0, radius: 4.0.into() },
        ..Default::default()
    }
}

/// The square badge with "!" or "?" in a confirmation.
pub fn badge(danger: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let base = if danger { color!(0xc42b2b) } else { tokens(theme).accent };
        container::Style {
            background: Some(with_alpha(base, if danger { 0.19 } else { 0.2 }).into()),
            border: border(with_alpha(base, 0.6), 1.0),
            ..Default::default()
        }
    }
}

// ---- text -----------------------------------------------------------------------------------------

/// Dims the text of its widget to `alpha` of the normal colour.
pub fn faint(alpha: f32) -> impl Fn(&Theme) -> text::Style {
    move |theme| text::Style { color: Some(with_alpha(tokens(theme).text, alpha)) }
}

pub fn accent_text(theme: &Theme) -> text::Style {
    text::Style { color: Some(tokens(theme).accent) }
}

pub fn warning_text(_: &Theme) -> text::Style {
    text::Style { color: Some(color!(0xe6a700)) }
}

pub fn badge_text(danger: bool) -> impl Fn(&Theme) -> text::Style {
    move |theme| text::Style { color: Some(if danger { color!(0xf0605d) } else { tokens(theme).accent }) }
}

// ---- controls -------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Normal,
    Primary,
    Danger,
}

pub fn button_style(kind: ButtonKind) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let (base, text_color, edge) = match kind {
            ButtonKind::Normal => (t.card, t.text, t.line()),
            ButtonKind::Primary => (t.accent, t.on_accent(), t.accent),
            ButtonKind::Danger => (color!(0xc42b2b), Color::WHITE, color!(0xc42b2b)),
        };
        let lift = |amount: f32| match kind {
            ButtonKind::Normal => mix(base, t.text, amount * 0.6),
            _ => mix(base, Color::WHITE, amount),
        };
        let (background, text_color) = match status {
            button::Status::Active => (base, text_color),
            button::Status::Hovered => (lift(0.14), text_color),
            button::Status::Pressed => (if kind == ButtonKind::Normal { lift(0.26) } else { mix(base, Color::BLACK, 0.18) }, text_color),
            button::Status::Disabled => (with_alpha(base, 0.45), with_alpha(text_color, 0.45)),
        };
        button::Style { background: Some(Background::Color(background)), text_color, border: border(edge, 1.0), shadow: Shadow::default(), snap: false }
    }
}

/// An entry of the navigation: a cell that is outlined and tinted, like the overlay's
/// selection, when it is the current page.
///
/// An idle cell has a transparent background rather than none. iced draws no quad for a button
/// without a background, and compares two frames' quads by position, so a hover that *added*
/// the hover quad shifted every quad after it and made all of them count as changed: one hover
/// repainted the whole navigation (and, on other layouts, the whole window). With the quad
/// always there, only the cell's own colour changes.
pub fn nav_cell(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (background, edge, width) = if selected {
            (with_alpha(t.accent, 0.14), t.accent, 1.0)
        } else if hovered {
            (with_alpha(t.text, 0.05), Color::TRANSPARENT, 0.0)
        } else {
            (Color::TRANSPARENT, Color::TRANSPARENT, 0.0)
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: t.text,
            border: border(edge, width),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

pub fn picker(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let t = tokens(theme);
    let open = matches!(status, pick_list::Status::Hovered | pick_list::Status::Opened { .. });
    pick_list::Style {
        text_color: t.text,
        placeholder_color: with_alpha(t.text, 0.5),
        handle_color: with_alpha(t.text, 0.7),
        background: t.page.into(),
        border: border(if open { t.accent } else { t.line() }, 1.0),
    }
}

/// The drop-down list of a pick list.
pub fn menu(theme: &Theme) -> menu::Style {
    let t = tokens(theme);
    menu::Style {
        background: t.card.into(),
        border: border(t.line(), 1.0),
        text_color: t.text,
        selected_text_color: t.on_accent(),
        selected_background: t.accent.into(),
        shadow: Shadow::default(),
    }
}

/// A flat rail with a rectangular handle: a cell, not a knob.
pub fn slider_style(theme: &Theme, status: slider::Status) -> slider::Style {
    let t = tokens(theme);
    let handle = match status {
        slider::Status::Active => t.accent,
        slider::Status::Hovered => mix(t.accent, Color::WHITE, 0.2),
        slider::Status::Dragged => mix(t.accent, Color::WHITE, 0.35),
    };
    let mut style = slider::default(theme, status);
    style.rail.backgrounds = (t.accent.into(), with_alpha(t.text, 0.2).into());
    style.rail.width = 4.0;
    style.rail.border.radius = 0.0.into();
    style.handle.shape = slider::HandleShape::Rectangle { width: 10, border_radius: RADIUS.into() };
    style.handle.background = handle.into();
    style
}

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let t = tokens(theme);
    let mut style = text_input::default(theme, status);
    style.background = t.page.into();
    style.border = border(if matches!(status, text_input::Status::Focused { .. }) { t.accent } else { t.line() }, 1.0);
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_follow_the_theme_mode() {
        assert!(tokens(&theme(true)).dark);
        assert!(!tokens(&theme(false)).dark);
    }

    #[test]
    fn alpha_is_scaled_not_replaced() {
        let c = with_alpha(Color { a: 0.5, ..Color::WHITE }, 0.5);
        assert!((c.a - 0.25).abs() < 1e-6);
    }

    #[test]
    fn mixing_is_linear() {
        assert_eq!(mix(Color::BLACK, Color::WHITE, 0.0), Color::BLACK);
        assert_eq!(mix(Color::BLACK, Color::WHITE, 1.0), Color::WHITE);
        assert!((mix(Color::BLACK, Color::WHITE, 0.25).r - 0.25).abs() < 1e-6);
    }

    #[test]
    fn the_grid_is_fainter_than_the_borders() {
        for dark in [true, false] {
            let t = tokens(&theme(dark));
            assert!(t.grid().a < t.grid_major().a && t.grid_major().a < t.rule().a && t.rule().a < t.line().a);
        }
    }
}
