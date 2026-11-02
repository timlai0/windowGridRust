//! Building blocks shared by the windows, in the grid look: cards made of a heading cell and a
//! body, tables of label and control cells, coordinate-tagged navigation cells, and the
//! confirmation drawn over a window.

use crate::style;
pub use crate::style::ButtonKind;
use iced::widget::{button, column, container, mouse_area, opaque, row, slider, space, stack, text, Button, Column};
use iced::{font, Center, Element, Fill, Font, Length, Padding};

/// Space between cards, and between the cells of a row of cards.
pub const GUTTER: f32 = 16.0;

/// Width of the control cell of a table row that has its label beside it: for sliders and
/// drop-downs, and for a switch.
pub const CONTROL_W: f32 = 208.0;
pub const SWITCH_W: f32 = 84.0;

/// The columns of a table: whether the control cell sits beside the label or under it, and how
/// wide it is when beside. Every row of one table uses the same, so the rule between the two
/// cells runs straight down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cols {
    pub inline: bool,
    pub cell: f32,
}

impl Cols {
    /// For tables with sliders or drop-downs.
    pub fn wide(inline: bool) -> Cols {
        Cols { inline, cell: CONTROL_W }
    }

    /// For tables that hold only switches.
    pub fn small(inline: bool) -> Cols {
        Cols { inline, cell: SWITCH_W }
    }

    /// Width for a control that is the main thing in its cell: a fixed width when the cell
    /// sits beside the label, the whole cell when it sits under it.
    pub fn control_width(self) -> Length {
        if self.inline {
            Length::Fixed(self.cell - 24.0)
        } else {
            Fill
        }
    }
}

pub fn bold() -> Font {
    Font { weight: font::Weight::Bold, ..Font::DEFAULT }
}

pub fn semibold() -> Font {
    Font { weight: font::Weight::Semibold, ..Font::DEFAULT }
}

/// For coordinates and other small technical labels.
pub fn mono() -> Font {
    Font::with_name("Consolas")
}

pub fn push_button<'a, M: Clone + 'a>(label: &'a str, on_press: Option<M>, kind: ButtonKind) -> Button<'a, M> {
    button(text(label).size(14)).padding([6, 16]).on_press_maybe(on_press).style(style::button_style(kind))
}

/// A hairline across the width of its container.
pub fn rule<'a, M: 'a>() -> Element<'a, M> {
    container(space()).width(Fill).height(1).style(style::rule).into()
}

/// A hairline down the height of its container.
pub fn vrule<'a, M: 'a>() -> Element<'a, M> {
    container(space()).width(1).height(Fill).style(style::rule).into()
}

/// A cell: an optional heading cell, a rule, then the body.
fn cell<'a, M: 'a>(title: &'a str, body: Element<'a, M>) -> Element<'a, M> {
    let mut parts = Column::new().width(Fill);
    if !title.is_empty() {
        parts = parts.push(
            container(text(title.to_uppercase()).size(11).font(mono()).style(style::faint(0.75)))
                .padding([8, 16])
                .width(Fill)
                .style(style::card_header),
        );
        parts = parts.push(rule());
    }
    container(parts.push(body)).width(Fill).clip(true).style(style::card).into()
}

/// A card with a padded body.
pub fn card<'a, M: 'a>(title: &'a str, body: impl Into<Element<'a, M>>) -> Element<'a, M> {
    cell(title, container(body).padding(16).width(Fill).into())
}

/// A card whose rows run edge to edge, separated by rules.
pub fn table<'a, M: 'a>(title: &'a str, rows: Vec<Element<'a, M>>) -> Element<'a, M> {
    let mut body = Column::new().width(Fill);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            body = body.push(rule());
        }
        body = body.push(r);
    }
    cell(title, body.into())
}

/// A row of a table: the label in one cell, the control in the next. With the columns not
/// inline (a narrow card) the control cell goes under the label instead.
pub fn setting_row<'a, M: 'a>(label: &'a str, hint: &'a str, dim: bool, control: impl Into<Element<'a, M>>, cols: Cols) -> Element<'a, M> {
    let alpha = if dim { 0.45 } else { 1.0 };
    let mut texts = Column::new().spacing(2).push(text(label).size(14).style(style::faint(alpha)));
    if !hint.is_empty() {
        texts = texts.push(text(hint).size(12).style(style::faint(0.6)));
    }
    if cols.inline {
        row![
            container(texts).padding([12, 16]).width(Fill),
            vrule(),
            container(control.into()).padding([8, 12]).width(cols.cell).height(Fill).align_y(Center),
        ]
        .into()
    } else {
        column![
            container(texts).padding(Padding::ZERO.top(12).left(16).right(16).bottom(6)).width(Fill),
            container(control.into()).padding(Padding::ZERO.left(16).right(16).bottom(12)).width(Fill),
        ]
        .into()
    }
}

/// A small boxed label.
pub fn chip<'a, M: 'a>(label: String) -> Element<'a, M> {
    container(text(label).size(12).font(mono())).padding([2, 8]).style(style::chip).into()
}

/// A whole-number setting: label, [-] value [+] on one line and a slider underneath. The
/// buttons make single steps easy; the slider covers big jumps.
pub fn stepper<'a, M: Clone + 'a>(
    label: &'a str,
    hint: &'a str,
    value: i32,
    range: (i32, i32),
    on_change: fn(i32) -> M,
) -> Element<'a, M> {
    let (min, max) = range;
    let step = |glyph: &'a str, to: i32, enabled: bool| {
        button(text(glyph).size(16).center())
            .width(36)
            .on_press_maybe(enabled.then(|| on_change(to)))
            .style(style::button_style(ButtonKind::Normal))
    };
    let head = row![
        column![text(label).size(14), text(hint).size(12).style(style::faint(0.6))].width(Fill),
        step("\u{2212}", value - 1, value > min),
        text(value.to_string()).size(18).font(bold()).width(34).center(),
        step("+", value + 1, value < max),
    ]
    .spacing(6)
    .align_y(Center);
    column![head, slider(min..=max, value, on_change).step(1).style(style::slider_style)].spacing(8).into()
}

/// An entry of the navigation: a cell tagged with its coordinate. In the rail (narrow
/// windows) only the coordinate is shown.
pub fn nav_cell<'a, M: Clone + 'a>(tag: &'a str, label: &'a str, selected: bool, rail: bool, on_press: M) -> Element<'a, M> {
    let tag = text(tag).size(12).font(mono());
    let tag = if selected { tag.style(style::accent_text) } else { tag.style(style::faint(0.55)) };
    // A button puts its content at the top, so the content fills the cell and centres itself.
    let content: Element<'a, M> = if rail {
        container(tag).center(Fill).into()
    } else {
        let font = if selected { semibold() } else { Font::DEFAULT };
        container(row![tag.width(24), text(label).size(14).font(font)].spacing(8).align_y(Center)).width(Fill).center_y(Fill).into()
    };
    // The cell is inset in the panel, so its outline stays inside it.
    let cell = button(content).width(Fill).height(40).padding([0, 10]).on_press(on_press).style(style::nav_cell(selected));
    container(cell).padding([2, 6]).into()
}

/// A short heading with a paragraph under it.
pub fn fact<'a, M: 'a>(title: &'a str, body: &'a str) -> Element<'a, M> {
    column![text(title).size(13).font(semibold()), text(body).size(12).style(style::faint(0.72))].spacing(2).into()
}

/// The question drawn over a window by [`modal`].
pub struct Question<'a, M> {
    pub heading: &'a str,
    pub message: &'a str,
    pub confirm_text: &'a str,
    pub dismiss_text: &'a str,
    pub danger: bool,
    pub on_confirm: M,
    pub on_dismiss: M,
}

/// Draws `base`, dimmed, with a question over it. Clicking the dimmed area dismisses.
pub fn modal<'a, M: Clone + 'a>(base: impl Into<Element<'a, M>>, q: Question<'a, M>) -> Element<'a, M> {
    let badge = container(text(if q.danger { "!" } else { "?" }).size(22).font(bold()).style(style::badge_text(q.danger)).center())
        .width(40)
        .height(40)
        .center(40)
        .style(style::badge(q.danger));
    let confirm_kind = if q.danger { ButtonKind::Danger } else { ButtonKind::Primary };
    let buttons = row![
        space().width(Fill),
        push_button(q.dismiss_text, Some(q.on_dismiss.clone()), ButtonKind::Normal),
        push_button(q.confirm_text, Some(q.on_confirm), confirm_kind),
    ]
    .spacing(8);
    let body = column![
        row![
            badge,
            column![text(q.heading).size(18).font(bold()), text(q.message).size(14).style(style::faint(0.75))].spacing(6).width(Fill),
        ]
        .spacing(16),
        buttons,
    ]
    .spacing(20);
    let dialog = container(body).padding(24).max_width(440).style(style::dialog);
    let scrim = mouse_area(container(opaque(dialog)).center(Fill).padding(24).style(style::scrim)).on_press(q.on_dismiss);
    stack![base.into(), Element::from(scrim)].into()
}
