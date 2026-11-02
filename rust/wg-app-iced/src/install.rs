//! The setup dialog: install into a folder, or run portable. The counterpart of
//! `InstallWindow` in `wg-app`'s `ui/app.slint`.

use crate::backdrop::Backdrop;
use crate::style;
use crate::widgets::{bold, modal, mono, push_button, rule, setting_row, table, ButtonKind, Cols, Question};
use crate::assets;
use iced::widget::{column, container, row, scrollable, space, stack, text, text_input, toggler};
use iced::{window, Center, Element, Fill, Size};
use std::path::{Path, PathBuf};
use wg_app::{dialogs, installer};

/// What the user chose in the setup dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallChoice {
    pub dir: PathBuf,
    pub portable: bool,
    pub shortcut: bool,
    pub autostart: bool,
    pub splash_screen: bool,
}

/// Where the dialog was opened from, which decides what happens when it is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Before the tray app runs. With `then_run` the app carries on as the tray app unless the
    /// installed copy took over (first run); without it the process ends (`/install`).
    Startup { then_run: bool },
    /// From the tray menu of the running app.
    Tray,
}

#[derive(Clone, Debug)]
pub enum Msg {
    /// `true` for portable, `false` for install. Both reset the folder to its default.
    SetPortable(bool),
    PathEdited(String),
    Browse,
    UseDefault,
    Shortcut(bool),
    Autostart(bool),
    Splash(bool),
    Accept,
    Cancel,
    KeepSettingUp,
    ConfirmCancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    /// The dialog is finished: `Some` for OK, `None` if cancelled.
    Done(Option<InstallChoice>),
}

pub fn window_settings() -> window::Settings {
    let size = Size::new(520.0, 500.0);
    window::Settings {
        size,
        min_size: Some(size),
        max_size: Some(size),
        resizable: false,
        position: window::Position::Centered,
        icon: assets::window_icon(),
        // Closing asks "Cancel installation?" first, so the window is closed by the app.
        exit_on_close_request: false,
        ..Default::default()
    }
}

pub struct InstallWindow {
    pub id: window::Id,
    pub origin: Origin,
    version: String,
    /// The folder the program runs from, offered for a portable setup.
    current_dir: PathBuf,
    path: String,
    portable: bool,
    shortcut: bool,
    autostart: bool,
    splash_screen: bool,
    needs_admin: bool,
    confirm_cancel: bool,
}

impl InstallWindow {
    pub fn new(
        id: window::Id,
        origin: Origin,
        version: &str,
        initial_dir: PathBuf,
        current_dir: PathBuf,
        splash_screen: bool,
    ) -> InstallWindow {
        InstallWindow {
            id,
            origin,
            version: version.to_string(),
            current_dir,
            needs_admin: !installer::can_write(&initial_dir),
            path: initial_dir.display().to_string(),
            portable: false,
            shortcut: true,
            autostart: true,
            splash_screen,
            confirm_cancel: false,
        }
    }

    fn set_dir(&mut self, dir: &Path) {
        self.needs_admin = !installer::can_write(dir);
        self.path = dir.display().to_string();
    }

    fn use_default(&mut self) {
        let dir = if self.portable { self.current_dir.clone() } else { installer::default_dir() };
        self.set_dir(&dir);
    }

    /// The window's close button: ask first.
    pub fn request_cancel(&mut self) {
        self.confirm_cancel = true;
    }

    pub fn has_question(&self) -> bool {
        self.confirm_cancel
    }

    pub fn update(&mut self, msg: Msg) -> Outcome {
        match msg {
            Msg::SetPortable(portable) => {
                self.portable = portable;
                self.use_default();
            }
            Msg::PathEdited(text) => {
                self.needs_admin = !installer::can_write(Path::new(&text));
                self.path = text;
            }
            Msg::Browse => {
                if let Some(dir) = dialogs::pick_folder(Path::new(&self.path)) {
                    self.set_dir(&dir);
                }
            }
            Msg::UseDefault => self.use_default(),
            Msg::Shortcut(on) => self.shortcut = on,
            Msg::Autostart(on) => self.autostart = on,
            Msg::Splash(on) => self.splash_screen = on,
            Msg::Accept => {
                return Outcome::Done(Some(InstallChoice {
                    dir: PathBuf::from(&self.path),
                    portable: self.portable,
                    // Neither applies to a portable copy.
                    shortcut: self.shortcut && !self.portable,
                    autostart: self.autostart && !self.portable,
                    splash_screen: self.splash_screen,
                }));
            }
            Msg::Cancel => self.confirm_cancel = true,
            Msg::KeepSettingUp => self.confirm_cancel = false,
            Msg::ConfirmCancel => return Outcome::Done(None),
        }
        Outcome::None
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let portable = self.portable;
        let kind = |selected: bool| if selected { ButtonKind::Primary } else { ButtonKind::Normal };
        let folder = column![
            text(if portable { "RUN FROM THIS FOLDER" } else { "INSTALL INTO" }).size(11).font(mono()).style(style::faint(0.6)),
            row![
                text_input("", &self.path).on_input(Msg::PathEdited).padding([7, 10]).width(Fill).style(style::input),
                push_button("Browse\u{2026}", Some(Msg::Browse), ButtonKind::Normal),
                push_button("Default", Some(Msg::UseDefault), ButtonKind::Normal),
            ]
            .spacing(8)
            .align_y(Center),
        ]
        .spacing(6)
        .extend(
            (self.needs_admin && !portable)
                .then(|| Element::from(text("\u{26A0} Administrator permission is needed for this folder.").size(12).style(style::warning_text))),
        );
        // Neither applies to a portable copy.
        let installed_only = |msg: fn(bool) -> Msg| (!portable).then_some(msg);
        let options = table(
            "Options",
            vec![
                setting_row("Create a Start Menu shortcut", "", portable, toggler(self.shortcut).on_toggle_maybe(installed_only(Msg::Shortcut)), Cols::small(true)),
                setting_row("Start WindowGridRust when I sign in", "", portable, toggler(self.autostart).on_toggle_maybe(installed_only(Msg::Autostart)), Cols::small(true)),
                setting_row("Show a splash screen at start", "", false, toggler(self.splash_screen).on_toggle(Msg::Splash), Cols::small(true)),
            ],
        );
        // The mode switch is one cell split in two.
        let mode = row![
            push_button("Install", Some(Msg::SetPortable(false)), kind(!portable)),
            push_button("Portable", Some(Msg::SetPortable(true)), kind(portable)),
        ];
        let content = column![
            column![
                text("SETUP").size(11).font(mono()).style(style::accent_text),
                text(format!("Set up WindowGridRust {}", self.version)).size(20).font(bold()),
            ]
            .spacing(4),
            mode,
            folder,
            options,
        ]
        .spacing(16)
        .padding(24);
        let buttons = row![
            space().width(Fill),
            push_button("Cancel", Some(Msg::Cancel), ButtonKind::Normal),
            push_button(if portable { "Start" } else { "Install" }, Some(Msg::Accept), ButtonKind::Primary),
        ]
        .spacing(8);
        let body = column![
            stack![Backdrop { step: 32.0 }.view(), scrollable(content).width(Fill).height(Fill)].width(Fill).height(Fill),
            rule(),
            container(buttons).padding([12, 24]).width(Fill).style(style::bar),
        ];
        let root = container(body).width(Fill).height(Fill);
        if self.confirm_cancel {
            modal(
                root,
                Question {
                    heading: "Cancel installation?",
                    message: "The setup will close without installing anything.",
                    confirm_text: "Cancel installation",
                    dismiss_text: "Keep setting up",
                    danger: false,
                    on_confirm: Msg::ConfirmCancel,
                    on_dismiss: Msg::KeepSettingUp,
                },
            )
        } else {
            root.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> InstallWindow {
        let current = std::env::temp_dir();
        InstallWindow::new(window::Id::unique(), Origin::Tray, "9.9.9", current.join("somewhere"), current, false)
    }

    #[test]
    fn install_and_portable_reset_the_folder() {
        let mut d = dialog();
        d.update(Msg::SetPortable(true));
        assert_eq!(d.path, std::env::temp_dir().display().to_string());
        assert!(!d.needs_admin, "the temp folder is writable");
        d.update(Msg::SetPortable(false));
        assert_eq!(d.path, installer::default_dir().display().to_string());
        d.update(Msg::PathEdited("changed".into()));
        d.update(Msg::UseDefault);
        assert_eq!(d.path, installer::default_dir().display().to_string());
    }

    #[test]
    fn editing_the_path_checks_whether_it_is_writable() {
        let mut d = dialog();
        d.needs_admin = true;
        d.update(Msg::PathEdited(std::env::temp_dir().join("not-created-yet").display().to_string()));
        assert!(!d.needs_admin, "a new folder under a writable one is fine");
    }

    #[test]
    fn accepting_returns_the_choice_and_portable_drops_shortcut_and_autostart() {
        let mut d = dialog();
        d.update(Msg::Splash(true));
        d.update(Msg::PathEdited("D:\\wg".into()));
        let Outcome::Done(Some(c)) = d.update(Msg::Accept) else { panic!("accepted") };
        assert_eq!(
            c,
            InstallChoice { dir: PathBuf::from("D:\\wg"), portable: false, shortcut: true, autostart: true, splash_screen: true }
        );
        d.update(Msg::SetPortable(true));
        let Outcome::Done(Some(c)) = d.update(Msg::Accept) else { panic!("accepted") };
        assert!(c.portable && !c.shortcut && !c.autostart);
    }

    #[test]
    fn cancelling_asks_first() {
        let mut d = dialog();
        assert_eq!(d.update(Msg::Cancel), Outcome::None);
        assert!(d.has_question());
        assert_eq!(d.update(Msg::KeepSettingUp), Outcome::None);
        assert!(!d.has_question());
        d.request_cancel();
        assert!(d.has_question(), "the close button asks too");
        assert_eq!(d.update(Msg::ConfirmCancel), Outcome::Done(None));
    }

    #[test]
    fn both_views_build() {
        let mut d = dialog();
        let _ = d.view();
        d.update(Msg::SetPortable(true));
        d.update(Msg::Cancel);
        let _ = d.view();
    }
}
