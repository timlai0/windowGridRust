//! The settings window: state, messages, update and views. The counterpart of
//! `SettingsWindow` in `wg-app`'s `ui/app.slint` and of the handlers in its `ui.rs`.
//!
//! The window can be any size: WindowGridRust itself snaps windows (this one too) into grid
//! cells. [`Layout`] turns the window's width into the arrangement used: the full navigation
//! or a slim rail of coordinate cells, cards side by side or stacked, table rows with the
//! control beside its label or under it.

use crate::backdrop::Backdrop;
use crate::preview::GridPreview;
use crate::style::{self, swatch, StatusKind};
use crate::widgets::{
    bold, card, chip, fact, mono, modal, nav_cell, push_button, rule, setting_row, stepper, table, vrule, ButtonKind, Cols, Question,
    GUTTER,
};
use crate::{assets, AppState, Paths};
use iced::alignment::Horizontal;
use iced::widget::{button, column, container, image, pick_list, responsive, row, scrollable, slider, space, stack, text, toggler, Column, Row};
use iced::{window, Center, Color, Element, Fill, FillPortion, Length, Padding, Size};
use std::path::PathBuf;
use wg_app::viewmodel::{screen_summary, SettingsModel, PRESETS};
use wg_app::{dialogs, osopts};
use wg_core::keycode::{all_key_names, Command, KeyCode, Modifiers};
use wg_core::settings::{Color as SettingsColor, Settings, WindowAnchor};
use wg_win::logger::log;

static ANCHORS: [&str; 3] = ["TopLeft", "Center", "GrabPoint"];

/// Width of the navigation, and of the rail it shrinks to.
const NAV_W: f32 = 224.0;
const RAIL_W: f32 = 56.0;

/// The pages of the navigation, with their coordinates.
const NAV: [(Page, &str, &str); 7] = [
    (Page::Grid, "01", "Grid"),
    (Page::Appearance, "02", "Appearance"),
    (Page::Behavior, "03", "Behavior"),
    (Page::Hud, "04", "Selection HUD"),
    (Page::Shortcuts, "05", "Shortcuts"),
    (Page::Info, "06", "Info & Usage"),
    (Page::Hood, "07", "Under the hood"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Grid,
    Appearance,
    Behavior,
    Hud,
    Shortcuts,
    Info,
    Hood,
}

impl Page {
    /// The coordinate and group shown above the page title.
    fn eyebrow(self) -> String {
        let (index, group) = match NAV.iter().position(|(p, ..)| *p == self) {
            Some(i) if i < 5 => (i, "CONFIGURE"),
            Some(i) => (i, "HELP"),
            None => (0, ""),
        };
        format!("{:02} / {group}", index + 1)
    }
}

/// The question asked before an action that cannot be undone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confirm {
    Exit,
    Reset,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Command,
    Modifier,
    Key,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Page(Page),
    DismissStatus,
    Columns(i32),
    Rows(i32),
    Preset(usize),
    Theme(String),
    Opacity(f32),
    Blur(f32),
    Fill(bool),
    Radial(bool),
    Anchor(&'static str),
    CompensateMargins(bool),
    PreserveSize(bool),
    DeferResize(bool),
    Hud(bool),
    Dimensions(bool),
    CellSpan(bool),
    Coverage(bool),
    Aspect(bool),
    Mode(bool),
    Hints(bool),
    Splash(bool),
    AeroShake(bool),
    DragFullWindows(bool),
    /// The change asked for by `DragFullWindows` has been made (or has failed).
    DragFullWindowsDone(bool),
    AddBinding,
    RemoveBinding(usize),
    EditBinding(usize, Field, String),
    OpenDir,
    OpenLog,
    Ask(Confirm),
    Dismiss,
    Confirmed,
    Save,
    Reload,
    Cancel,
}

/// What the app must do after a message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    None,
    /// Close the window.
    Close,
    /// Quit the whole app.
    Quit,
    /// Set the Windows option "show window contents while dragging", and answer with
    /// `Msg::DragFullWindowsDone`. Windows tells every open program about this change and waits
    /// for them, so it can take long; the app does it off the interface thread.
    SetDragFullWindows(bool),
}

/// How the window is arranged at its current width.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Layout {
    /// The navigation is a slim rail of coordinates instead of the full panel.
    pub rail: bool,
    /// Cards sit one under the other instead of side by side.
    pub stacked: bool,
    /// Margin around the page, which is also the distance between the lines of the grid.
    pub pad: f32,
    /// Width available to the cards.
    pub inner: f32,
}

impl Layout {
    pub fn for_width(width: f32) -> Layout {
        let rail = width < 720.0;
        let content = width - if rail { RAIL_W } else { NAV_W };
        let pad = if content < 440.0 { 16.0 } else { 32.0 };
        let inner = (content - 2.0 * pad).max(0.0);
        Layout { rail, stacked: inner < 560.0, pad, inner }
    }

    /// Width of a card that takes `share` (0 to 1) of a row of cards.
    fn card_width(self, share: f32) -> f32 {
        if self.stacked {
            self.inner
        } else {
            (self.inner - GUTTER) * share
        }
    }

    /// Columns for a table of such a card whose controls need room (sliders, drop-downs):
    /// beside the label when the card is wide enough, under it otherwise.
    fn wide_cols(self, share: f32) -> Cols {
        Cols::wide(self.card_width(share) >= 380.0)
    }

    /// Columns for a table that holds only switches, which need very little room.
    fn small_cols(self, share: f32) -> Cols {
        Cols::small(self.card_width(share) >= 260.0)
    }

    /// Cards in a row, in the given proportions; one under the other when stacked.
    fn split<'a>(self, parts: Vec<(u16, Element<'a, Msg>)>) -> Element<'a, Msg> {
        // The heights are pinned: a table's rows have `Fill`-height dividers, and a row of two
        // of them would otherwise get an unbounded height inside the scrolling page.
        if self.stacked {
            Column::with_children(parts.into_iter().map(|(_, e)| e)).spacing(GUTTER).height(Length::Shrink).into()
        } else {
            Row::with_children(parts.into_iter().map(|(share, e)| container(e).width(FillPortion(share)).height(Length::Shrink).into()))
                .spacing(GUTTER)
                .height(Length::Shrink)
                .into()
        }
    }
}

struct ThemeColors {
    name: String,
    line: Color,
    start: Color,
    end: Color,
}

fn argb(c: SettingsColor) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, f32::from(c.a) / 255.0)
}

fn primary_screen() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    // SAFETY: plain queries.
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    (w.max(1), h.max(1))
}

fn anchor_name(a: WindowAnchor) -> &'static str {
    ANCHORS[(a as i32).clamp(0, ANCHORS.len() as i32 - 1) as usize]
}

pub fn window_settings() -> window::Settings {
    window::Settings {
        size: Size::new(960.0, 680.0),
        // Small on purpose: the window may be snapped into a small grid cell.
        min_size: Some(Size::new(320.0, 380.0)),
        position: window::Position::Centered,
        icon: assets::window_icon(),
        ..Default::default()
    }
}

pub struct SettingsWindow {
    pub id: window::Id,
    page: Page,
    /// The settings being edited; `Save` writes them back, `Cancel` throws them away.
    model: SettingsModel,
    status: Option<(StatusKind, String)>,
    confirm: Option<Confirm>,
    themes: Vec<ThemeColors>,
    theme_names: Vec<String>,
    preset_labels: Vec<String>,
    command_names: Vec<String>,
    modifier_names: Vec<String>,
    key_names: Vec<String>,
    aero_shake_disabled: bool,
    drag_full_windows: bool,
    /// The Windows option is being changed (see `Outcome::SetDragFullWindows`).
    drag_pending: bool,
    screen_summary: String,
    screen_ratio: f32,
    version: String,
    work_dir: PathBuf,
    log_file: PathBuf,
}

impl SettingsWindow {
    pub fn new(id: window::Id, state: &AppState, paths: &Paths) -> SettingsWindow {
        let (sw, sh) = primary_screen();
        let mut w = SettingsWindow {
            id,
            page: Page::Grid,
            model: SettingsModel::from_settings(&state.config.settings),
            status: None,
            confirm: None,
            themes: Vec::new(),
            theme_names: Vec::new(),
            preset_labels: PRESETS.iter().map(|(x, y)| format!("{x} \u{d7} {y}")).collect(),
            command_names: ["Resize", "Move"].map(String::from).to_vec(),
            modifier_names: ["None", "Alt", "Control", "Shift", "Windows"].map(String::from).to_vec(),
            key_names: all_key_names().map(String::from).collect(),
            aero_shake_disabled: false,
            drag_full_windows: true,
            drag_pending: false,
            screen_summary: screen_summary(sw, sh),
            screen_ratio: sw as f32 / sh as f32,
            version: paths.version.clone(),
            work_dir: paths.work_dir.clone(),
            log_file: paths.log_file.clone(),
        };
        w.load(state);
        w
    }

    /// Takes the values to show from the settings (at open, and after a reload or reset).
    fn load(&mut self, state: &AppState) {
        let settings = &state.config.settings;
        self.model = SettingsModel::from_settings(settings);
        self.themes = settings
            .grid
            .themes
            .iter()
            .map(|t| ThemeColors { name: t.name.clone(), line: argb(t.color_line), start: argb(t.color_start), end: argb(t.color_end) })
            .collect();
        self.theme_names = self.themes.iter().map(|t| t.name.clone()).collect();
        // The first theme stands in for one the list does not know, as the old window did.
        if let Some(t) = self.themes.get(self.theme_index()) {
            self.model.theme = t.name.clone();
        }
        self.aero_shake_disabled = osopts::aero_shake_disabled();
        self.drag_full_windows = osopts::drag_full_windows();
    }

    fn theme_index(&self) -> usize {
        self.themes.iter().position(|t| t.name.eq_ignore_ascii_case(&self.model.theme)).unwrap_or(0)
    }

    /// Line, start and end colours of the chosen theme.
    fn colors(&self) -> (Color, Color, Color) {
        self.themes.get(self.theme_index()).map_or(
            (Color::from_rgb8(0x00, 0xbd, 0xd4), Color::from_rgb8(0x00, 0x97, 0xa7), Color::from_rgb8(0xb2, 0xeb, 0xf2)),
            |t| (t.line, t.start, t.end),
        )
    }

    pub fn has_question(&self) -> bool {
        self.confirm.is_some()
    }

    /// Test-only switches, read in isolated builds only: `WG_TEST_PAGE` (0-6) is the page shown
    /// first and `WG_TEST_ASK` (`exit` or `reset`) opens that question.
    pub fn apply_test_switches(&mut self) {
        if let Some(page) = std::env::var("WG_TEST_PAGE").ok().and_then(|v| v.parse::<usize>().ok()).and_then(|i| NAV.get(i)) {
            self.page = page.0;
        }
        self.confirm = match std::env::var("WG_TEST_ASK").as_deref() {
            Ok("exit") => Some(Confirm::Exit),
            Ok("reset") => Some(Confirm::Reset),
            _ => self.confirm,
        };
    }

    fn show(&mut self, kind: StatusKind, message: impl Into<String>) {
        self.status = Some((kind, message.into()));
    }

    fn publish(state: &AppState) {
        if let Some(shared) = &state.shared {
            shared.publish(&state.config.settings);
        }
    }

    pub fn update(&mut self, msg: Msg, state: &mut AppState) -> Outcome {
        let m = &mut self.model;
        match msg {
            Msg::Page(p) => self.page = p,
            Msg::DismissStatus => self.status = None,
            Msg::Columns(v) => m.set_grid(v, m.grid_y),
            Msg::Rows(v) => m.set_grid(m.grid_x, v),
            Msg::Preset(i) => {
                if let Some(&(x, y)) = PRESETS.get(i) {
                    m.set_grid(x, y);
                }
            }
            Msg::Theme(name) => m.theme = name,
            Msg::Opacity(v) => m.set_opacity(v),
            Msg::Blur(v) => m.set_blur(v),
            Msg::Fill(on) => m.fill_window = on,
            Msg::Radial(on) => m.radial_gradient = on,
            Msg::Anchor(name) => m.anchor = WindowAnchor::from_i32(ANCHORS.iter().position(|a| *a == name).unwrap_or(0) as i32),
            Msg::CompensateMargins(on) => m.compensate_window_margins = on,
            Msg::PreserveSize(on) => m.preserve_size_on_move = on,
            Msg::DeferResize(on) => m.defer_resize_until_release = on,
            Msg::Hud(on) => m.show_selection_hud = on,
            Msg::Dimensions(on) => m.show_dimensions = on,
            Msg::CellSpan(on) => m.show_cell_span = on,
            Msg::Coverage(on) => m.show_screen_coverage = on,
            Msg::Aspect(on) => m.show_aspect_ratio = on,
            Msg::Mode(on) => m.show_mode_indicator = on,
            Msg::Hints(on) => m.show_interaction_hints = on,
            Msg::Splash(on) => m.splash_screen = on,
            // The two Windows options apply at once; the window then shows what Windows says.
            Msg::AeroShake(on) => {
                let ok = osopts::set_aero_shake_disabled(on);
                self.aero_shake_disabled = osopts::aero_shake_disabled();
                if !ok {
                    self.show(StatusKind::Warning, "Could not change the Aero Shake setting.");
                }
            }
            // The switch moves at once; Windows is asked in the background and then has the last word.
            Msg::DragFullWindows(on) => {
                if !self.drag_pending {
                    self.drag_pending = true;
                    self.drag_full_windows = on;
                    return Outcome::SetDragFullWindows(on);
                }
            }
            Msg::DragFullWindowsDone(ok) => {
                self.drag_pending = false;
                self.drag_full_windows = osopts::drag_full_windows();
                if !ok {
                    self.show(StatusKind::Warning, "Could not change the window dragging setting.");
                }
            }
            Msg::AddBinding => m.add_binding(),
            Msg::RemoveBinding(i) => m.remove_binding(i),
            Msg::EditBinding(i, field, value) => {
                if let Some(b) = m.bindings.get_mut(i) {
                    match field {
                        Field::Command => b.command = Command::parse(&value).unwrap_or(b.command),
                        Field::Modifier => b.modifier = Modifiers::parse(&value).unwrap_or(b.modifier),
                        Field::Key => b.key = KeyCode::parse(&value).unwrap_or(b.key),
                    }
                }
            }
            Msg::OpenDir => {
                if !dialogs::open_path(&self.work_dir) {
                    dialogs::tell("WindowGridRust", "Could not open the working directory.", true);
                }
            }
            Msg::OpenLog => {
                if !self.log_file.exists() {
                    dialogs::tell("WindowGridRust", "The log file does not exist yet.", false);
                } else if !dialogs::open_path(&self.log_file) {
                    dialogs::tell("WindowGridRust", "Could not open the log file.", true);
                }
            }
            Msg::Ask(c) => self.confirm = Some(c),
            Msg::Dismiss => self.confirm = None,
            Msg::Confirmed => match self.confirm.take() {
                Some(Confirm::Exit) => return Outcome::Quit,
                Some(Confirm::Reset) => self.reset(state),
                None => {}
            },
            Msg::Save => self.save(state),
            Msg::Reload => self.reload(state),
            Msg::Cancel => return Outcome::Close,
        }
        Outcome::None
    }

    fn save(&mut self, state: &mut AppState) {
        self.model.apply_to(&mut state.config.settings);
        match state.config.save() {
            Ok(()) => {
                Self::publish(state);
                log("Settings saved");
                self.show(StatusKind::Success, "Settings saved successfully.");
            }
            Err(e) => {
                log(&format!("Saving settings failed: {e}"));
                self.show(StatusKind::Warning, format!("Could not save settings: {e}"));
            }
        }
    }

    fn reload(&mut self, state: &mut AppState) {
        match state.config.reload() {
            Ok(()) => {
                self.load(state);
                Self::publish(state);
                self.show(StatusKind::Info, "Settings reloaded.");
            }
            Err(e) => self.show(StatusKind::Warning, format!("Could not reload settings: {e}")),
        }
    }

    fn reset(&mut self, state: &mut AppState) {
        state.config.settings = Settings::default();
        let saved = state.config.save();
        self.load(state);
        Self::publish(state);
        match saved {
            Ok(()) => self.show(StatusKind::Warning, "Settings reset to default values."),
            Err(e) => self.show(StatusKind::Warning, format!("Could not save the reset settings: {e}")),
        }
    }

    // ---- views ---------------------------------------------------------------------------

    pub fn view(&self) -> Element<'_, Msg> {
        let arranged = responsive(move |size| {
            let layout = Layout::for_width(size.width);
            row![self.nav_panel(layout), self.content(layout)].into()
        });
        let root = container(arranged).width(Fill).height(Fill);
        match self.confirm {
            None => root.into(),
            Some(Confirm::Exit) => modal(
                root,
                Question {
                    heading: "Exit WindowGridRust?",
                    message: "WindowGridRust will close completely. Windows will no longer snap to the grid until you start it again.",
                    confirm_text: "Exit WindowGridRust",
                    dismiss_text: "Cancel",
                    danger: true,
                    on_confirm: Msg::Confirmed,
                    on_dismiss: Msg::Dismiss,
                },
            ),
            Some(Confirm::Reset) => modal(
                root,
                Question {
                    heading: "Reset all settings?",
                    message: "Every setting goes back to its default value, including your shortcuts. This cannot be undone.",
                    confirm_text: "Reset settings",
                    dismiss_text: "Cancel",
                    danger: true,
                    on_confirm: Msg::Confirmed,
                    on_dismiss: Msg::Dismiss,
                },
            ),
        }
    }

    /// The navigation: a column of cells tagged with their coordinates, or, in a narrow
    /// window, a rail of just the coordinates.
    fn nav_panel(&self, l: Layout) -> Element<'_, Msg> {
        let logo: Element<Msg> = match assets::image_handle() {
            Some(h) => image(h).width(28).height(28).into(),
            None => space().width(28).into(),
        };
        let brand: Element<Msg> = if l.rail {
            container(logo).center_x(Fill).into()
        } else {
            row![logo, text("WindowGridRust").size(15).font(bold())].spacing(10).align_y(Center).into()
        };
        let heading = |label: &'static str| {
            container(text(label).size(11).font(mono()).style(style::faint(0.5))).padding(Padding::ZERO.top(14).left(10).bottom(6)).width(Fill)
        };
        let mut cells = Column::new().width(Fill).push(container(brand).padding([0, 10]).width(Fill).height(56).align_y(Center)).push(rule());
        for (i, (page, tag, label)) in NAV.iter().enumerate() {
            if i == 0 || i == 5 {
                cells = if l.rail { cells.push(space().height(if i == 0 { 0 } else { 12 })) } else { cells.push(heading(if i == 0 { "CONFIGURE" } else { "HELP" })) };
            }
            cells = cells.push(nav_cell(tag, label, self.page == *page, l.rail, Msg::Page(*page))).push(rule());
        }
        let width = if l.rail { RAIL_W } else { NAV_W };
        row![container(cells).width(width).height(Fill).style(style::nav), vrule()].into()
    }

    /// The status line, the page on its grid, and the buttons.
    fn content(&self, l: Layout) -> Element<'_, Msg> {
        let status = self.status.as_ref().map(|(kind, message)| {
            container(
                row![
                    text(message.as_str()).size(13).width(Fill),
                    button(text("\u{d7}").size(18).center())
                        .width(28)
                        .on_press(Msg::DismissStatus)
                        .style(button::text),
                ]
                .spacing(8)
                .align_y(Center),
            )
            .padding(Padding::ZERO.left(l.pad.min(16.0)).right(8))
            .width(Fill)
            .center_y(40)
            .style(style::status_bar(*kind))
        });
        let field = stack![Backdrop { step: l.pad }.view(), self.page_body(l)].width(Fill).height(Fill);
        // Switching pages changes everything, and iced then repaints it as dozens of separate
        // damaged rectangles, each redrawing every large card: ~190 ms. When a layer's bounds differ
        // between two frames iced instead damages the whole layer as one rectangle (one pass,
        // ~14 ms). The stack's top layer, which holds the page, takes its bounds from the
        // container around it, and iced compares bounds exactly, so that container is a hundredth
        // of a pixel shorter for each step down the navigation: invisible, but different for every
        // pair of pages, which makes any page switch a single repaint. Nothing else changes:
        // within a page the bounds stay the same and iced still repaints only what changed.
        let step = NAV.iter().position(|(p, ..)| *p == self.page).unwrap_or(0);
        let field = container(container(field).width(Fill).height(Fill).clip(true)).width(Fill).height(Fill).padding(Padding::ZERO.bottom(0.01 * step as f32));
        Column::new().extend(status.map(Element::from)).push(field).push(self.bottom_bar(l)).width(Fill).height(Fill).into()
    }

    pub fn page_body(&self, l: Layout) -> Element<'_, Msg> {
        match self.page {
            Page::Grid => self.grid_page(l),
            Page::Appearance => self.appearance_page(l),
            Page::Behavior => self.behavior_page(l),
            Page::Hud => self.hud_page(l),
            Page::Shortcuts => self.shortcuts_page(l),
            Page::Info => self.info_page(l),
            Page::Hood => self.hood_page(l),
        }
    }

    fn bottom_bar(&self, l: Layout) -> Element<'_, Msg> {
        let exit = push_button("Exit app", Some(Msg::Ask(Confirm::Exit)), ButtonKind::Normal);
        let reset = push_button("Reset defaults", Some(Msg::Ask(Confirm::Reset)), ButtonKind::Normal);
        let reload = push_button("Reload", Some(Msg::Reload), ButtonKind::Normal);
        let cancel = push_button("Cancel", Some(Msg::Cancel), ButtonKind::Normal);
        let save = push_button("Save changes", Some(Msg::Save), ButtonKind::Primary);
        let buttons: Element<Msg> = if l.inner < 560.0 {
            Row::with_children([exit.into(), reset.into(), reload.into(), cancel.into(), save.into()]).spacing(8).wrap().vertical_spacing(8).into()
        } else {
            row![exit, space().width(Fill), row![reset, reload, cancel, save].spacing(8)].align_y(Center).into()
        };
        column![rule(), container(buttons).padding([12.0, l.pad]).width(Fill).style(style::bar)].into()
    }

    fn preview(&self, opacity: f32) -> GridPreview {
        let (line, start, end) = self.colors();
        GridPreview {
            cols: self.model.grid_x,
            rows: self.model.grid_y,
            ratio: self.screen_ratio,
            line,
            start,
            end,
            radial: self.model.radial_gradient,
            fill: self.model.fill_window,
            opacity,
        }
    }

    fn grid_page(&self, l: Layout) -> Element<'_, Msg> {
        let m = &self.model;
        let side_by_side = l.card_width(1.0) >= 560.0;
        let preview = column![
            self.preview(1.0).view(if side_by_side { 220.0 } else { 170.0 }),
            Row::with_children([
                chip(self.screen_summary.clone()),
                chip(m.grid_summary()),
                chip(m.cell_shape_summary(f64::from(self.screen_ratio))),
            ])
            .spacing(6)
            .wrap()
            .vertical_spacing(6),
        ]
        .spacing(10);
        let steppers = column![
            stepper("Columns", "Cells across the screen", m.grid_x, (1, 32), Msg::Columns),
            stepper("Rows", "Cells down the screen", m.grid_y, (1, 24), Msg::Rows),
            text("Tip: use \u{2212} and + for single steps, the slider for big jumps, or pick a preset below.").size(12).style(style::faint(0.6)),
        ]
        .spacing(16);
        let dimensions: Element<Msg> = if side_by_side {
            row![container(preview).width(FillPortion(3)), container(steppers).width(FillPortion(2))].spacing(24).into()
        } else {
            column![preview, steppers].spacing(16).into()
        };
        let presets = Row::with_children(
            self.preset_labels.iter().enumerate().map(|(i, label)| push_button(label, Some(Msg::Preset(i)), ButtonKind::Normal).width(84).into()),
        )
        .spacing(8)
        .wrap()
        .vertical_spacing(8);
        page(
            l,
            Page::Grid,
            "Grid",
            "How the screen is divided. Windows snap to blocks of these cells.",
            vec![card("Dimensions", dimensions), card("Presets", presets)],
        )
    }

    fn appearance_page(&self, l: Layout) -> Element<'_, Msg> {
        let m = &self.model;
        let cols = l.wide_cols(0.6);
        let width = cols.control_width();
        let (line, start, end) = self.colors();
        let selected = self.themes.get(self.theme_index()).map(|t| t.name.clone());
        let swatches = |c: Color| container(space()).width(16).height(16).style(swatch(c));
        let overlay = table(
            "Overlay",
            vec![
                setting_row(
                    "Color theme",
                    "",
                    false,
                    row![swatches(line), swatches(start), swatches(end), picker(&self.theme_names[..], selected, Fill, Msg::Theme)]
                        .spacing(8)
                        .align_y(Center),
                    cols,
                ),
                setting_row(
                    "Overlay opacity",
                    "",
                    false,
                    row![
                        slider(0.05..=1.0, m.opacity, Msg::Opacity).step(0.01f32).width(Fill).style(style::slider_style),
                        text(format!("{}%", (m.opacity * 100.0).round() as i32)).width(40).align_x(Horizontal::Right),
                    ]
                    .spacing(10)
                    .align_y(Center)
                    .width(width),
                    cols,
                ),
                setting_row(
                    "Shadow blur",
                    "0 turns the shadow off.",
                    false,
                    row![
                        slider(0.0..=50.0, m.blur_radius, Msg::Blur).step(1.0f32).width(Fill).style(style::slider_style),
                        text(format!("{}", m.blur_radius.round() as i32)).width(40).align_x(Horizontal::Right),
                    ]
                    .spacing(10)
                    .align_y(Center)
                    .width(width),
                    cols,
                ),
                setting_row("Fill the selection", "Off draws only an outline.", false, toggler(m.fill_window).on_toggle(Msg::Fill), cols),
                setting_row("Radial gradient", "", false, toggler(m.radial_gradient).on_toggle(Msg::Radial), cols),
            ],
        );
        let preview = card(
            "Preview",
            column![self.preview(0.25 + 0.75 * m.opacity).view(180.0), text(m.grid_summary()).size(12).font(mono()).style(style::faint(0.65))]
                .spacing(10),
        );
        page(
            l,
            Page::Appearance,
            "Appearance",
            "Colors and look of the grid overlay. The preview updates as you change them.",
            vec![l.split(vec![(3, overlay), (2, preview)])],
        )
    }

    fn behavior_page(&self, l: Layout) -> Element<'_, Msg> {
        let m = &self.model;
        let wide = l.wide_cols(1.0);
        let small = l.small_cols(1.0);
        let snapping = table(
            "Snapping",
            vec![
                setting_row(
                    "Window anchor",
                    "Where a window that is larger than the selection extends.",
                    false,
                    picker(&ANCHORS[..], Some(anchor_name(m.anchor)), wide.control_width(), Msg::Anchor),
                    wide,
                ),
                setting_row(
                    "Compensate window margins",
                    "Align the visible window edge, not the invisible resize border.",
                    false,
                    toggler(m.compensate_window_margins).on_toggle(Msg::CompensateMargins),
                    wide,
                ),
                setting_row(
                    "Preserve size on move",
                    "A moved window keeps its size instead of stretching to the cells.",
                    false,
                    toggler(m.preserve_size_on_move).on_toggle(Msg::PreserveSize),
                    wide,
                ),
                setting_row(
                    "Defer resize until release",
                    "Resize once when you let go, instead of live while dragging.",
                    false,
                    toggler(m.defer_resize_until_release).on_toggle(Msg::DeferResize),
                    wide,
                ),
            ],
        );
        let system = table(
            "System & application",
            vec![
                setting_row("Show splash screen at start", "", false, toggler(m.splash_screen).on_toggle(Msg::Splash), small),
                setting_row("Disable Windows Aero Shake", "Applies immediately.", false, toggler(self.aero_shake_disabled).on_toggle(Msg::AeroShake), small),
                setting_row(
                    "Show window contents while dragging",
                    "Applies immediately. Needed for live resizing.",
                    false,
                    toggler(self.drag_full_windows).on_toggle_maybe((!self.drag_pending).then_some(Msg::DragFullWindows as fn(bool) -> Msg)),
                    small,
                ),
            ],
        );
        page(l, Page::Behavior, "Behavior", "How windows are placed, and the Windows options WindowGridRust depends on.", vec![snapping, system])
    }

    fn hud_page(&self, l: Layout) -> Element<'_, Msg> {
        let m = &self.model;
        let cols = l.small_cols(0.5);
        let on = m.show_selection_hud;
        let sub = |label: &'static str, value: bool, msg: fn(bool) -> Msg| {
            setting_row(label, "", !on, toggler(value).on_toggle_maybe(on.then_some(msg)), cols)
        };
        let readout = table(
            "Readout",
            vec![
                setting_row("Enable selection HUD", "", false, toggler(on).on_toggle(Msg::Hud), cols),
                sub("Pixel dimensions", m.show_dimensions, Msg::Dimensions),
                sub("Grid cell span", m.show_cell_span, Msg::CellSpan),
                sub("Screen coverage", m.show_screen_coverage, Msg::Coverage),
                sub("Window aspect ratio", m.show_aspect_ratio, Msg::Aspect),
            ],
        );
        let extras = table(
            "Extras",
            vec![
                setting_row("Mode indicator badge", "Shows whether you are resizing or moving.", false, toggler(m.show_mode_indicator).on_toggle(Msg::Mode), cols),
                setting_row(
                    "Interaction and key hints",
                    "A hint bar with the keys that apply right now.",
                    false,
                    toggler(m.show_interaction_hints).on_toggle(Msg::Hints),
                    cols,
                ),
            ],
        );
        page(
            l,
            Page::Hud,
            "Selection HUD",
            "The small readout shown next to the selection while you drag.",
            vec![l.split(vec![(1, readout), (1, extras)])],
        )
    }

    fn shortcuts_page(&self, l: Layout) -> Element<'_, Msg> {
        let narrow = l.inner < 520.0;
        let faint = |s: &'static str| text(s).size(11).font(mono()).style(style::faint(0.6));
        let mut rows: Vec<Element<Msg>> = Vec::new();
        if !narrow {
            rows.push(
                container(row![faint("COMMAND").width(120), faint("MODIFIER").width(120), faint("KEY").width(Fill), space().width(40)].spacing(8))
                    .padding([8, 16])
                    .width(Fill)
                    .into(),
            );
        }
        for (i, b) in self.model.bindings.iter().enumerate() {
            let command = picker(&self.command_names[..], Some(b.command.to_string()), if narrow { Fill } else { Length::Fixed(120.0) }, move |v| {
                Msg::EditBinding(i, Field::Command, v)
            });
            let modifier = picker(&self.modifier_names[..], Some(b.modifier.to_string()), if narrow { Fill } else { Length::Fixed(120.0) }, move |v| {
                Msg::EditBinding(i, Field::Modifier, v)
            });
            let key = picker(&self.key_names[..], Some(b.key.to_string()), Fill, move |v| Msg::EditBinding(i, Field::Key, v));
            let remove = push_button("\u{d7}", Some(Msg::RemoveBinding(i)), ButtonKind::Normal).width(40);
            let line: Element<Msg> = if narrow {
                column![row![command, modifier].spacing(8), row![key, remove].spacing(8)].spacing(8).into()
            } else {
                row![command, modifier, key, remove].spacing(8).align_y(Center).into()
            };
            rows.push(container(line).padding([10, 16]).width(Fill).into());
        }
        rows.push(container(push_button("Add shortcut", Some(Msg::AddBinding), ButtonKind::Normal)).padding([12, 16]).width(Fill).into());
        page(l, Page::Shortcuts, "Shortcuts", "Each row starts a command while you drag a window: a modifier or a key.", vec![table("Input shortcuts", rows)])
    }

    fn info_page(&self, l: Layout) -> Element<'_, Msg> {
        let steps = |lines: [&'static str; 3]| Column::with_children(lines.map(|line| text(line).into())).spacing(12);
        let resize = card(
            "Resize a window",
            steps([
                "1. Hold the left mouse button and drag a window by its title bar.",
                "2. Press and hold your Resize shortcut (for example Space or the right mouse button).",
                "3. Move the pointer to the first cell, release the shortcut, move to the opposite cell and release the mouse button.",
            ]),
        );
        let moving = card(
            "Move a window",
            steps([
                "1. Hold the left mouse button and drag a window by its title bar.",
                "2. Press your Move shortcut (for example Left Ctrl or the middle mouse button).",
                "3. Release the mouse button: the window snaps onto the cells, keeping its size. Esc cancels.",
            ]),
        );
        let path = |label: &'static str, value: &PathBuf, action: &'static str, msg: Msg| {
            column![
                text(label).size(12).style(style::faint(0.7)),
                text(value.display().to_string()).font(mono()).size(13),
                push_button(action, Some(msg), ButtonKind::Normal),
            ]
            .spacing(4)
        };
        let paths = card(
            "Diagnostic paths",
            column![path("Working directory", &self.work_dir, "Open folder", Msg::OpenDir), path("Log file", &self.log_file, "Open log", Msg::OpenLog)]
                .spacing(14),
        );
        let about = card(
            "About WindowGridRust",
            column![
                row![text("Version").style(style::faint(0.7)).width(70), text(self.version.as_str()).font(mono())].spacing(12),
                text("This is the all-Rust build: its windows use iced, with no .slint files.").size(13).style(style::faint(0.8)),
                text("iced is MIT licensed. iced.rs").size(12).style(style::faint(0.6)),
            ]
            .spacing(12),
        );
        page(
            l,
            Page::Info,
            "Info & Usage",
            "How to use WindowGridRust, and where its files live.",
            vec![l.split(vec![(1, resize), (1, moving)]), l.split(vec![(1, paths), (1, about)])],
        )
    }

    fn hood_page(&self, l: Layout) -> Element<'_, Msg> {
        // (what, C# / .NET, Rust)
        const TABLE: [(&str, &str, &str); 6] = [
            ("", "C# / .NET", "Rust"),
            ("App executable", "141 MB", "3.6 MB (includes overlay and hooks)"),
            ("App memory, idle", "245 MB", "20 MB"),
            ("Overlay executable", "71 MB", "447 KB"),
            ("Overlay memory, idle", "213 MB", "9 MB"),
            ("Hook DLLs", "2016 binaries, no source", "17\u{2013}20 KB each, Rust source"),
        ];
        let rows: Vec<Element<Msg>> = TABLE
            .iter()
            .enumerate()
            .map(|(i, (what, old, new))| {
                let head = i == 0;
                let new_font = if head { mono() } else { bold() };
                container(
                    row![
                        text(*what).size(13).width(FillPortion(2)),
                        text(*old).size(13).style(style::faint(0.6)).width(FillPortion(2)),
                        text(*new).size(13).font(new_font).width(FillPortion(3)),
                    ]
                    .spacing(16),
                )
                .padding([9, 16])
                .width(Fill)
                .into()
            })
            .collect();
        let how = card(
            "How a drag works",
            column![
                fact(
                    "Hook DLLs, one per bitness",
                    "Started through rundll32. A WinEvent hook is loaded into the process whose window you drag, and watches your shortcuts and the mouse for that one drag. No C runtime, so they are tiny.",
                ),
                fact(
                    "Overlay",
                    "A click-through window drawn on the GPU (DirectComposition, Direct2D), visible only while you drag. It also computes the final snap, including the invisible window borders.",
                ),
                fact("Talking to each other", "Shared memory carries the settings; four registered window messages signal target, show, area and stop."),
            ]
            .spacing(12),
        );
        let app = card(
            "This app",
            column![
                fact(
                    "One executable",
                    "Carries the overlay and hooks and unpacks them next to itself. The windows use iced (tiny-skia software renderer): no GPU or web runtime needed.",
                ),
                fact(
                    "Supervisor",
                    "Runs the overlay and hook hosts in a job object: they end with the app, and a crashed one restarts, up to a crash-loop limit. It never touches other programs' rundll32.",
                ),
            ]
            .spacing(12),
        );
        page(
            l,
            Page::Hood,
            "Under the hood",
            "WindowGridRust reimplements the original WindowGrid in Rust. What changed, and how the pieces work together.",
            vec![
                table("Original C# WindowGrid vs. WindowGridRust", rows),
                l.split(vec![(1, how), (1, app)]),
                text("Built for x64, x86 and ARM64. Only the x64 hook is drag-tested; the ARM64 one builds but has never run on real hardware. Settings use the same format as the original WindowGrid.settings. Details: docs/adr 000\u{2013}006.")
                    .size(12)
                    .style(style::faint(0.55))
                    .into(),
            ],
        )
    }
}

/// A drop-down list in the grid look.
fn picker<'a, T>(options: &'a [T], selected: Option<T>, width: Length, on_selected: impl Fn(T) -> Msg + 'a) -> Element<'a, Msg>
where
    T: ToString + PartialEq + Clone + 'a,
{
    pick_list(options, selected, on_selected).width(width).padding([6, 10]).style(style::picker).menu_style(style::menu).into()
}

/// A page of the window: its coordinate, title and one-line description, then the content.
/// It scrolls when the window is too small for it.
fn page<'a>(l: Layout, page: Page, title: &'a str, subtitle: &'a str, body: Vec<Element<'a, Msg>>) -> Element<'a, Msg> {
    let head = column![
        text(page.eyebrow()).size(11).font(mono()).style(style::accent_text),
        text(title).size(24).font(bold()),
        text(subtitle).size(13).style(style::faint(0.65)),
    ]
    .spacing(4);
    scrollable(Column::new().push(head).extend(body).spacing(GUTTER).padding(l.pad).width(Fill)).width(Fill).height(Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use wg_app::config::Config;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wg-iced-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn state(dir: &Path) -> AppState {
        AppState { config: Config { settings: Settings::default(), path: Config::path_in(dir) }, shared: None }
    }

    fn window(state: &AppState, dir: &Path) -> SettingsWindow {
        let paths = Paths { work_dir: dir.to_path_buf(), log_file: dir.join("WindowGridRust.log"), version: "test".into() };
        SettingsWindow::new(window::Id::unique(), state, &paths)
    }

    #[test]
    fn starts_on_the_grid_page_with_the_settings_shown() {
        let dir = temp_dir("open");
        let s = state(&dir);
        let w = window(&s, &dir);
        assert_eq!(w.page, Page::Grid);
        assert_eq!((w.model.grid_x, w.model.grid_y), (12, 6));
        assert!(!w.has_question());
        assert_eq!(w.preset_labels[0], "12 \u{d7} 6");
        assert!(w.key_names.iter().any(|k| k == "KEY_SPACE"));
    }

    #[test]
    fn grid_edits_are_clamped_and_presets_apply() {
        let dir = temp_dir("grid");
        let mut s = state(&dir);
        let mut w = window(&s, &dir);
        w.update(Msg::Columns(500), &mut s);
        assert_eq!(w.model.grid_x, 64);
        w.update(Msg::Rows(0), &mut s);
        assert_eq!(w.model.grid_y, 1);
        w.update(Msg::Preset(1), &mut s);
        assert_eq!((w.model.grid_x, w.model.grid_y), (16, 9));
        w.update(Msg::Preset(99), &mut s);
        assert_eq!((w.model.grid_x, w.model.grid_y), (16, 9), "an unknown preset changes nothing");
        w.update(Msg::Opacity(0.0), &mut s);
        assert_eq!(w.model.opacity, 0.05);
        w.update(Msg::Anchor("GrabPoint"), &mut s);
        assert_eq!(w.model.anchor, WindowAnchor::GrabPoint);
        assert_eq!(anchor_name(WindowAnchor::GrabPoint), "GrabPoint");
    }

    #[test]
    fn the_dragging_option_is_left_to_the_app_to_set_in_the_background() {
        let dir = temp_dir("drag");
        let mut s = state(&dir);
        let mut w = window(&s, &dir);
        // Windows tells every open program about this change and waits for them, so the window
        // never makes the call itself: it asks the app to, and nothing here touches the real option.
        assert_eq!(w.update(Msg::DragFullWindows(false), &mut s), Outcome::SetDragFullWindows(false));
        assert!(!w.drag_full_windows, "the switch moves at once");
        assert_eq!(w.update(Msg::DragFullWindows(true), &mut s), Outcome::None, "one change at a time");
        assert!(!w.drag_full_windows);
        w.update(Msg::DragFullWindowsDone(false), &mut s);
        assert!(!w.drag_pending);
        assert!(matches!(w.status, Some((StatusKind::Warning, _))), "a failure is reported");
        assert_eq!(w.update(Msg::DragFullWindows(true), &mut s), Outcome::SetDragFullWindows(true), "and the next change can follow");
    }

    #[test]
    fn shortcut_rows_can_be_added_edited_and_removed() {
        let dir = temp_dir("bindings");
        let mut s = state(&dir);
        let mut w = window(&s, &dir);
        let before = w.model.bindings.len();
        w.update(Msg::AddBinding, &mut s);
        assert_eq!(w.model.bindings.len(), before + 1);
        let last = before;
        w.update(Msg::EditBinding(last, Field::Command, "Move".into()), &mut s);
        w.update(Msg::EditBinding(last, Field::Modifier, "Shift".into()), &mut s);
        w.update(Msg::EditBinding(last, Field::Key, "KEY_ESCAPE".into()), &mut s);
        let b = w.model.bindings[last];
        assert_eq!((b.command, b.modifier, b.key), (Command::MOVE, Modifiers::SHIFT, KeyCode::ESCAPE));
        w.update(Msg::EditBinding(last, Field::Key, "NoSuchKey".into()), &mut s);
        assert_eq!(w.model.bindings[last].key, KeyCode::ESCAPE, "an unknown name is ignored");
        w.update(Msg::EditBinding(99, Field::Key, "KEY_SPACE".into()), &mut s);
        w.update(Msg::RemoveBinding(last), &mut s);
        assert_eq!(w.model.bindings.len(), before);
    }

    #[test]
    fn save_writes_the_file_and_reload_takes_it_back() {
        let dir = temp_dir("save");
        let mut s = state(&dir);
        let mut w = window(&s, &dir);
        w.update(Msg::Preset(3), &mut s); // 16 x 8
        w.update(Msg::Hud(false), &mut s);
        assert_eq!(w.update(Msg::Save, &mut s), Outcome::None);
        assert_eq!(w.status.as_ref().map(|(k, _)| *k), Some(StatusKind::Success));
        let on_disk = Settings::from_json_str(&std::fs::read_to_string(&s.config.path).unwrap()).unwrap();
        assert_eq!((on_disk.grid.grid_x, on_disk.grid.grid_y), (16, 8));
        assert!(!on_disk.grid.show_selection_hud);

        // An edit that is not saved is dropped by Reload.
        w.update(Msg::Preset(0), &mut s);
        w.update(Msg::Reload, &mut s);
        assert_eq!((w.model.grid_x, w.model.grid_y), (16, 8));
        assert_eq!(w.status.as_ref().map(|(k, _)| *k), Some(StatusKind::Info));
    }

    #[test]
    fn reload_reports_an_unreadable_file_and_keeps_the_edits() {
        let dir = temp_dir("reload-missing");
        let mut s = state(&dir); // nothing was saved, so there is no file
        let mut w = window(&s, &dir);
        w.update(Msg::Columns(20), &mut s);
        w.update(Msg::Reload, &mut s);
        assert_eq!(w.status.as_ref().map(|(k, _)| *k), Some(StatusKind::Warning));
        assert_eq!(w.model.grid_x, 20);
    }

    #[test]
    fn reset_needs_confirmation_then_restores_and_saves_the_defaults() {
        let dir = temp_dir("reset");
        let mut s = state(&dir);
        let mut w = window(&s, &dir);
        w.update(Msg::Columns(30), &mut s);
        w.update(Msg::Save, &mut s);
        assert_eq!(s.config.settings.grid.grid_x, 30);

        w.update(Msg::Ask(Confirm::Reset), &mut s);
        assert!(w.has_question());
        w.update(Msg::Dismiss, &mut s);
        assert!(!w.has_question());
        assert_eq!(s.config.settings.grid.grid_x, 30, "dismissing changes nothing");

        w.update(Msg::Ask(Confirm::Reset), &mut s);
        assert_eq!(w.update(Msg::Confirmed, &mut s), Outcome::None);
        assert_eq!(w.model.grid_x, Settings::default().grid.grid_x);
        let on_disk = Settings::from_json_str(&std::fs::read_to_string(&s.config.path).unwrap()).unwrap();
        assert_eq!(on_disk.grid.grid_x, Settings::default().grid.grid_x);
    }

    #[test]
    fn exit_needs_confirmation_and_cancel_closes_the_window() {
        let dir = temp_dir("exit");
        let mut s = state(&dir);
        let mut w = window(&s, &dir);
        assert_eq!(w.update(Msg::Ask(Confirm::Exit), &mut s), Outcome::None);
        assert_eq!(w.update(Msg::Dismiss, &mut s), Outcome::None);
        assert_eq!(w.update(Msg::Confirmed, &mut s), Outcome::None, "nothing is pending any more");
        w.update(Msg::Ask(Confirm::Exit), &mut s);
        assert_eq!(w.update(Msg::Confirmed, &mut s), Outcome::Quit);
        assert_eq!(w.update(Msg::Cancel, &mut s), Outcome::Close);
    }

    #[test]
    fn an_unknown_theme_name_falls_back_to_the_first_theme() {
        let dir = temp_dir("theme");
        let mut s = state(&dir);
        s.config.settings.grid.theme_name = "no such theme".into();
        let w = window(&s, &dir);
        assert_eq!(w.model.theme, s.config.settings.grid.themes[0].name);
        let mut s2 = state(&dir);
        let second = s2.config.settings.grid.themes[1].name.to_uppercase();
        s2.config.settings.grid.theme_name = second.clone();
        let w2 = window(&s2, &dir);
        assert_eq!(w2.theme_index(), 1, "names match without regard to case");
    }

    #[test]
    fn every_page_builds_in_every_arrangement() {
        let dir = temp_dir("pages");
        let mut s = state(&dir);
        let mut w = window(&s, &dir);
        for width in [2000.0, 960.0, 800.0, 720.0, 600.0, 400.0, 320.0] {
            let layout = Layout::for_width(width);
            for (page, ..) in NAV {
                w.update(Msg::Page(page), &mut s);
                let _ = w.page_body(layout);
                let _ = w.nav_panel(layout);
                let _ = w.content(layout);
            }
        }
        let _ = w.view();
        w.update(Msg::Ask(Confirm::Exit), &mut s);
        let _ = w.view();
    }

    #[test]
    fn the_layout_follows_the_width() {
        // Wide: full navigation, cards side by side, 32 px grid.
        let wide = Layout::for_width(960.0);
        assert!(!wide.rail && !wide.stacked);
        assert_eq!(wide.pad, 32.0);
        assert_eq!(wide.inner, 960.0 - NAV_W - 64.0);
        assert!(wide.wide_cols(0.6).inline && wide.wide_cols(1.0).inline);
        assert!(!wide.wide_cols(0.4).inline, "a small share of the width leaves the control under its label");
        assert!(wide.small_cols(0.5).inline, "a switch needs far less room than a slider");
        assert!(wide.small_cols(0.5).cell < wide.wide_cols(0.5).cell);

        // The default minimum of the old design: the navigation still fits, the cards stack.
        let medium = Layout::for_width(800.0);
        assert!(!medium.rail && medium.stacked);

        // Narrow: the rail, a finer grid, everything stacked.
        let narrow = Layout::for_width(360.0);
        assert!(narrow.rail && narrow.stacked);
        assert_eq!(narrow.pad, 16.0);
        assert_eq!(narrow.inner, 360.0 - RAIL_W - 32.0);
        assert!(!narrow.wide_cols(1.0).inline);

        // The rail starts exactly where the full navigation stops fitting.
        assert!(!Layout::for_width(720.0).rail && Layout::for_width(719.0).rail);
    }

    #[test]
    fn a_degenerate_width_does_not_break_the_layout() {
        for width in [0.0, 1.0, RAIL_W, 100.0] {
            let l = Layout::for_width(width);
            assert!(l.inner >= 0.0 && l.stacked && l.rail);
        }
    }

    #[test]
    fn every_page_has_a_unique_coordinate() {
        let mut seen = std::collections::HashSet::new();
        for (page, tag, _) in NAV {
            assert!(seen.insert(tag), "{tag} is used twice");
            assert!(page.eyebrow().starts_with(tag));
        }
        assert_eq!(Page::Grid.eyebrow(), "01 / CONFIGURE");
        assert_eq!(Page::Hood.eyebrow(), "07 / HELP");
    }
}
