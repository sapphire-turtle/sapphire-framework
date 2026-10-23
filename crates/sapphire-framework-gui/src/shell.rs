//! [`Shell`]: every app's window frame — [`corundum_gui::Shell`] with sapphire's
//! [`Settings`] at the rail's foot and the services' state beside it.
//!
//! The frame itself (the rail, the bottom bar when narrow, the actions) is corundum-gui's,
//! shared with asteria; this adds what is sapphire's: Settings as the footer, the health
//! dot in the foot slot, and the status panel when a service needs attention.

use std::cell::Cell;

use corundum_gui::{Action, Destination};

use crate::{Health, Settings};

/// What the frame has open: one of the app's screens, or Settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Open<D> {
    Screen(D),
    Settings,
}

/// The window frame. Hold one as the app's root; call [`ui`](Self::ui) every frame.
///
/// ```no_run
/// # use sapphire_framework_gui::{Destination, Settings, Shell, icons};
/// # fn example(settings: Settings, ui: &mut egui::Ui) {
/// #[derive(Clone, Copy, PartialEq)]
/// enum Screen { Entries, Summary }
///
/// let mut shell = Shell::new(
///     vec![
///         // The app's own Lucide SVGs: `egui::include_image!("../assets/icons/notebook.svg")`.
///         Destination::new(Screen::Entries, icons::settings(), "entries"),
///         Destination::new(Screen::Summary, icons::settings(), "summary"),
///     ],
///     settings,
/// );
/// shell.ui(ui, |ui, screen| match screen {
///     Screen::Entries => { egui::CentralPanel::default().show(ui, |ui| ui.label("…")); }
///     Screen::Summary => { egui::CentralPanel::default().show(ui, |ui| ui.label("…")); }
/// });
/// # }
/// ```
pub struct Shell<D> {
    frame: corundum_gui::Shell<Open<D>>,
    settings: Settings,
    /// Where the health dot was last drawn, for the tests to click.
    #[cfg(test)]
    dot_rect: egui::Rect,
}

impl<D: Copy + PartialEq> Shell<D> {
    /// A frame with `destinations` on the rail, the first one open. With none, Settings is
    /// all there is.
    pub fn new(destinations: Vec<Destination<D>>, mut settings: Settings) -> Shell<D> {
        settings.hosted();
        let destinations = destinations
            .into_iter()
            .map(|d| Destination::new(Open::Screen(d.id), d.icon, d.label))
            .collect();
        let frame = corundum_gui::Shell::new(destinations).with_footer(vec![Destination::new(
            Open::Settings,
            crate::icons::settings(),
            "sapphire-settings",
        )]);
        Shell {
            frame,
            settings,
            #[cfg(test)]
            dot_rect: egui::Rect::NOTHING,
        }
    }

    /// Put `actions` on the rail, under the screens (on the bottom bar, after them).
    pub fn with_actions(mut self, actions: Vec<Action>) -> Shell<D> {
        self.frame = self.frame.with_actions(actions);
        self
    }

    /// The [`Action`] clicked in the last [`ui`](Self::ui), once: call it after `ui`
    /// every frame and do what it names.
    pub fn take_action(&mut self) -> Option<&'static str> {
        self.frame.take_action()
    }

    /// The app's screen that is open; `None` while Settings is.
    pub fn current(&self) -> Option<D> {
        match self.frame.current() {
            Some(Open::Screen(id)) => Some(id),
            _ => None,
        }
    }

    /// Open the app's screen `id`.
    pub fn open(&mut self, id: D) {
        self.frame.open(Open::Screen(id));
    }

    /// Open Settings.
    pub fn open_settings(&mut self) {
        self.frame.open(Open::Settings);
    }

    /// The Settings the rail's foot opens.
    pub fn settings(&mut self) -> &mut Settings {
        &mut self.settings
    }

    /// The theme settings, once, after the user changed them in Settings: the app saves
    /// these. See [`Settings::take_theme_changed`].
    pub fn take_theme_changed(&mut self) -> Option<corundum_gui::theme::ThemeSettings> {
        self.settings.take_theme_changed()
    }

    /// Render into the window's root `ui`. `content` draws the open screen of the app's
    /// into what the frame leaves of `ui`, as it would at the root: its own panels, then a
    /// `CentralPanel`.
    pub fn ui(&mut self, ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui, D)) {
        crate::i18n::register(ui.ctx(), crate::RESOURCES);
        self.settings.poll();
        let health = self.settings.health();
        // Set by the dot, read by Settings in the same frame: the two closures cannot both
        // hold `settings`.
        let about = Cell::new(false);
        #[cfg(test)]
        let dot_rect = &mut self.dot_rect;
        let settings = &mut self.settings;
        self.frame.ui_with_foot(
            ui,
            |ui| {
                let response = status_dot(ui, health);
                #[cfg(test)]
                {
                    *dot_rect = response.rect;
                }
                response.clicked().then(|| {
                    about.set(true);
                    Open::Settings
                })
            },
            |ui, open| {
                if health == Health::Problem {
                    egui::Panel::top("shell_status").show(ui, |ui| settings.status_ui(ui));
                }
                match open {
                    Open::Screen(id) => content(ui, id),
                    Open::Settings => {
                        if about.get() {
                            settings.open_about();
                        }
                        settings.ui(ui);
                    }
                }
            },
        );
    }
}

/// The services' state as a dot; clicking it opens About.
fn status_dot(ui: &mut egui::Ui, health: Health) -> egui::Response {
    let visuals = ui.visuals();
    let (color, id) = match health {
        Health::Connecting => (visuals.weak_text_color(), "sapphire-shell-connecting"),
        Health::Healthy => (
            crate::theme::current(ui.ctx()).text(crate::theme::Swatch::SUCCESS),
            "sapphire-shell-healthy",
        ),
        Health::UpdateAvailable => (visuals.hyperlink_color, "sapphire-shell-update"),
        Health::Problem => (visuals.error_fg_color, "sapphire-shell-problem"),
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::click());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    response.on_hover_text(corundum_gui::tr!(ui, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{client, render};

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Screen {
        Entries,
        Summary,
    }

    fn shell() -> (Shell<Screen>, tokio::runtime::Runtime) {
        let (client, runtime) = client();
        let shell = Shell::new(
            vec![
                Destination::new(Screen::Entries, crate::icons::settings(), "entries"),
                Destination::new(Screen::Summary, crate::icons::arrow_left(), "summary"),
            ],
            Settings::new(client),
        );
        (shell, runtime)
    }

    #[test]
    fn it_opens_on_the_first_screen_and_draws_it() {
        let (mut shell, _rt) = shell();
        assert_eq!(shell.current(), Some(Screen::Entries));
        for size in [[900.0, 600.0], [400.0, 700.0]] {
            let mut drawn = Vec::new();
            render(size, |ui| {
                shell.ui(ui, |ui, screen| {
                    drawn.push(screen);
                    egui::CentralPanel::default().show(ui, |ui| ui.label("entries"));
                })
            });
            assert_eq!(drawn, [Screen::Entries, Screen::Entries]);
        }
    }

    #[test]
    fn settings_is_drawn_in_place_of_the_screen() {
        let (mut shell, _rt) = shell();
        shell.open_settings();
        assert_eq!(shell.current(), None);
        for size in [[900.0, 600.0], [400.0, 700.0]] {
            render(size, |ui| {
                shell.ui(ui, |_, screen| panic!("{screen:?} drawn under Settings"))
            });
        }
        shell.open(Screen::Summary);
        assert_eq!(shell.current(), Some(Screen::Summary));
    }

    #[test]
    fn clicking_the_dot_opens_about() {
        let (mut shell, _rt) = shell();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0));
        let frame = |events: Vec<egui::Event>, shell: &mut Shell<Screen>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                shell.ui(ui, |ui, _| {
                    egui::CentralPanel::default().show(ui, |ui| ui.label("entries"));
                })
            });
            output.textures_delta.clear();
        };
        frame(vec![], &mut shell);
        frame(vec![], &mut shell);
        let pos = shell.dot_rect.center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(pos)], &mut shell);
        frame(vec![button(true)], &mut shell);
        frame(vec![button(false)], &mut shell);
        assert_eq!(shell.current(), None);
        assert!(shell.settings.about_open());
        // Drawn in the frame of the click, not the one after.
        assert!(shell.settings.about_drawn());
    }

    #[test]
    fn without_screens_settings_is_all_there_is() {
        let (client, _rt) = client();
        let mut shell: Shell<Screen> = Shell::new(vec![], Settings::new(client));
        assert_eq!(shell.current(), None);
        render([900.0, 600.0], |ui| shell.ui(ui, |_, _| unreachable!()));
    }
}
