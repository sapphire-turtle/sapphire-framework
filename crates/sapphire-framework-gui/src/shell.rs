//! [`Shell`]: every app's window frame — [`corundum_framework_gui::Shell`] with sapphire's
//! [`Settings`] at the rail's foot and the services' state beside it.
//!
//! The frame itself (the rail, the bottom bar when narrow, the actions, the side pane and
//! the status bar) is corundum-framework-gui's, shared with asteria; this adds what is
//! sapphire's: Settings as the footer, the health dot in the foot slot, and the status
//! panel when a service needs attention.

use std::cell::Cell;

use corundum_framework_gui::{Action, Destination, Side};

use crate::{Health, Settings};

/// What the frame has open: one of the app's screens, or Settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Open<D> {
    Screen(D),
    Settings,
}

/// An app as the [`Shell`] draws it, each frame, with [`Shell::show`]: its screens, its
/// side pane, its status and what its actions do. The shell calls these in turn, each with
/// the app's `&mut self`, so a screen and the side pane share the app's state freely.
///
/// As corundum-framework-gui's [`ShellApp`](corundum_framework_gui::ShellApp), less the
/// foot, which is sapphire's health dot; and the side pane is told `None` while Settings is
/// open, for Settings is the framework's and not one of the app's screens.
///
/// ```no_run
/// # use sapphire_framework_gui::{Action, Destination, Settings, Shell, ShellApp, Side, icons};
/// # fn example(settings: Settings, ui: &mut egui::Ui) {
/// #[derive(Clone, Copy, PartialEq)]
/// enum Screen { Entries }
///
/// struct Journal { open: Option<usize> }
///
/// impl ShellApp<Screen> for Journal {
///     fn screen_ui(&mut self, ui: &mut egui::Ui, screen: Screen, side: &mut Side) {
///         egui::CentralPanel::default().show(ui, |ui| {
///             if ui.button("entry 1").clicked() {
///                 self.open = Some(1);
///                 // On a card, or on a phone in the screen's place; the column shows it
///                 // anyway.
///                 side.open();
///             }
///         });
///     }
///
///     fn side_ui(&mut self, ui: &mut egui::Ui, _: Option<Screen>, _: &mut Side) {
///         match self.open {
///             Some(entry) => { ui.label(format!("entry {entry}")); }
///             None => { ui.weak("Nothing selected"); }
///         }
///     }
/// }
///
/// let mut shell = Shell::new(
///     vec![Destination::new(Screen::Entries, icons::file_text(), "entries")],
///     settings,
/// );
/// let mut journal = Journal { open: None };
/// // Every frame:
/// shell.show(ui, &mut journal);
/// # }
/// ```
pub trait ShellApp<D> {
    /// Draw `screen` into what the frame leaves of `ui`, as it would at the root: its own
    /// panels, then a `CentralPanel`. Through `side` it opens the side pane — on a card, or
    /// on a phone in the screen's place — when the person picks an item.
    fn screen_ui(&mut self, ui: &mut egui::Ui, screen: D, side: &mut Side);

    /// Fill the side pane (no panels of its own): the item picked on `screen` — an entry, a transaction, the chat of a session — else
    /// a word that there is none. `screen` is `None` while Settings is open. Through `side`
    /// it closes the pane once the item is saved or dropped.
    fn side_ui(&mut self, ui: &mut egui::Ui, screen: Option<D>, side: &mut Side) {
        let _ = (ui, screen, side);
    }

    /// Fill the status bar from the left, when wide — a log's last line, a timer running.
    /// Its right end holds the health dot and the side column's toggle.
    fn status_ui(&mut self, ui: &mut egui::Ui) {
        let _ = ui;
    }

    /// Do what the [`Action`] `id` names: it was clicked on the rail or in the More menu. A
    /// "new entry" begins the entry and opens it in the side pane through `side`.
    fn action(&mut self, id: &'static str, side: &mut Side) {
        let _ = (id, side);
    }
}

/// The window frame. Hold one as the app's root; call [`show`](Self::show) every frame with
/// the app, a [`ShellApp`] (see there for an example).
///
/// The side pane is beside every screen, Settings included: a right column when wide,
/// which a button at the status bar's right end hides; a card over the screen when it is
/// hidden or there is no room; on a phone, the screen's place, under a way back. Whether
/// the column is on is kept on the device, with the look.
pub struct Shell<D> {
    frame: corundum_framework_gui::Shell<Open<D>>,
    settings: Settings,
    /// The health dot was clicked and About is still to open: with a status bar, the dot is
    /// drawn after the screen, so Settings opens on About a frame later.
    about: bool,
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
        let frame =
            corundum_framework_gui::Shell::new(destinations).with_footer(vec![Destination::new(
                Open::Settings,
                crate::icons::settings(),
                "sapphire-settings",
            )]);
        Shell {
            frame,
            settings,
            about: false,
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
    #[deprecated(note = "draw with `show`, which hands the action to `ShellApp::action`")]
    pub fn take_action(&mut self) -> Option<&'static str> {
        #[allow(deprecated)]
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

    /// Always `None`: the framework saves the theme itself (#327).
    #[deprecated(note = "the framework saves the theme in the settings registry (#327)")]
    pub fn take_theme_changed(&mut self) -> Option<corundum_framework_gui::theme::ThemeSettings> {
        None
    }

    /// Render into the window's root `ui`. `content` draws the open screen of the app's
    /// into what the frame leaves of `ui`, as it would at the root: its own panels, then a
    /// `CentralPanel`. An action clicked waits for [`take_action`](Self::take_action). There
    /// is no side pane or status bar here.
    #[deprecated(note = "implement `ShellApp` and draw with `show`")]
    pub fn ui(&mut self, ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui, D)) {
        crate::i18n::register(ui.ctx(), crate::RESOURCES);
        self.settings.poll();
        self.settings.follow_theme(ui.ctx());
        let health = self.settings.health();
        // Set by the dot, read by Settings in the same frame: the two closures cannot both
        // hold `settings`.
        let about = Cell::new(false);
        #[cfg(test)]
        let dot_rect = &mut self.dot_rect;
        let settings = &mut self.settings;
        #[allow(deprecated)]
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

    /// Render `app` into the window's root `ui`: the rail, the open screen
    /// ([`ShellApp::screen_ui`]) or Settings, the side pane beside it
    /// ([`ShellApp::side_ui`]) and, wide, the status bar ([`ShellApp::status_ui`], then the
    /// health dot and the side column's toggle); then hand an action clicked to
    /// [`ShellApp::action`].
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut impl ShellApp<D>) {
        crate::i18n::register(ui.ctx(), crate::RESOURCES);
        self.settings.poll();
        self.settings.follow_theme(ui.ctx());
        let mut frame = Frame {
            health: self.settings.health(),
            app,
            settings: &mut self.settings,
            about: &mut self.about,
            #[cfg(test)]
            dot_rect: &mut self.dot_rect,
        };
        self.frame.show(ui, &mut frame);
    }
}

/// The app with what is sapphire's around it, as corundum's shell draws it.
struct Frame<'a, A> {
    app: &'a mut A,
    settings: &'a mut Settings,
    health: Health,
    about: &'a mut bool,
    #[cfg(test)]
    dot_rect: &'a mut egui::Rect,
}

impl<D, A: ShellApp<D>> corundum_framework_gui::ShellApp<Open<D>> for Frame<'_, A> {
    fn screen_ui(&mut self, ui: &mut egui::Ui, open: Open<D>, side: &mut Side) {
        if self.health == Health::Problem {
            egui::Panel::top("shell_status").show(ui, |ui| self.settings.status_ui(ui));
        }
        match open {
            Open::Screen(id) => self.app.screen_ui(ui, id, side),
            Open::Settings => {
                if std::mem::take(self.about) {
                    self.settings.open_about();
                }
                self.settings.ui(ui);
            }
        }
    }

    fn side_ui(&mut self, ui: &mut egui::Ui, open: Open<D>, side: &mut Side) {
        let screen = match open {
            Open::Screen(id) => Some(id),
            Open::Settings => None,
        };
        self.app.side_ui(ui, screen, side);
    }

    fn foot_ui(&mut self, ui: &mut egui::Ui) -> Option<Open<D>> {
        let response = status_dot(ui, self.health);
        #[cfg(test)]
        {
            *self.dot_rect = response.rect;
        }
        response.clicked().then(|| {
            *self.about = true;
            Open::Settings
        })
    }

    fn status_ui(&mut self, ui: &mut egui::Ui) {
        self.app.status_ui(ui);
    }

    fn action(&mut self, id: &'static str, side: &mut Side) {
        self.app.action(id, side);
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
    response.on_hover_text(corundum_framework_gui::tr!(ui, id))
}

// The deprecated `ui` is what the apps call until they move to `show`.
#[cfg(test)]
#[allow(deprecated)]
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
            // Once a frame, or twice when egui discards a frame's first pass.
            assert!(drawn.len() >= 2, "{drawn:?}");
            assert!(drawn.iter().all(|s| *s == Screen::Entries), "{drawn:?}");
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

    /// Frames of `shell` at a wide size — drawn with `show` when `app` is given, else with
    /// `ui` — then a click on the health dot, then `after` frames more.
    fn click_dot(shell: &mut Shell<Screen>, mut app: Option<&mut App>, after: usize) {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0));
        let mut frame = |events: Vec<egui::Event>, shell: &mut Shell<Screen>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| match app.as_deref_mut() {
                Some(app) => shell.show(ui, app),
                None => shell.ui(ui, |ui, _| {
                    egui::CentralPanel::default().show(ui, |ui| ui.label("entries"));
                }),
            });
            output.textures_delta.clear();
        };
        frame(vec![], shell);
        frame(vec![], shell);
        let pos = shell.dot_rect.center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(pos)], shell);
        frame(vec![button(true)], shell);
        frame(vec![button(false)], shell);
        for _ in 0..after {
            frame(vec![], shell);
        }
    }

    #[test]
    fn clicking_the_dot_opens_about() {
        let (mut shell, _rt) = shell();
        click_dot(&mut shell, None, 0);
        assert_eq!(shell.current(), None);
        assert!(shell.settings.about_open());
        // Drawn in the frame of the click, not the one after.
        assert!(shell.settings.about_drawn());
    }

    #[test]
    fn clicking_the_dot_on_the_status_bar_opens_about() {
        let (mut shell, _rt) = shell();
        click_dot(&mut shell, Some(&mut App::default()), 1);
        assert_eq!(shell.current(), None);
        // The dot is drawn after the screen: About opens with Settings, a frame later.
        assert!(shell.settings.about_open());
        assert!(shell.settings.about_drawn());
        assert!(!shell.about);
    }

    /// An app recording what the shell asked it to draw.
    #[derive(Default)]
    struct App {
        screens: Vec<Screen>,
        sides: Vec<Option<Screen>>,
        /// Open the side pane from the screen.
        pick: bool,
    }

    impl ShellApp<Screen> for App {
        fn screen_ui(&mut self, ui: &mut egui::Ui, screen: Screen, side: &mut Side) {
            self.screens.push(screen);
            if std::mem::take(&mut self.pick) {
                side.open();
            }
            egui::CentralPanel::default().show(ui, |ui| ui.label("entries"));
        }

        fn side_ui(&mut self, ui: &mut egui::Ui, screen: Option<Screen>, _: &mut Side) {
            self.sides.push(screen);
            ui.label("detail");
        }
    }

    #[test]
    fn wide_the_side_pane_is_beside_the_screen_and_settings() {
        let (mut shell, _rt) = shell();
        let mut app = App::default();
        render([1200.0, 700.0], |ui| shell.show(ui, &mut app));
        assert!(
            app.screens.iter().all(|s| *s == Screen::Entries),
            "{:?}",
            app.screens
        );
        assert!(!app.sides.is_empty());
        assert!(
            app.sides.iter().all(|s| *s == Some(Screen::Entries)),
            "{:?}",
            app.sides
        );

        shell.open_settings();
        let mut app = App::default();
        render([1200.0, 700.0], |ui| shell.show(ui, &mut app));
        assert!(app.screens.is_empty(), "{:?}", app.screens);
        assert!(!app.sides.is_empty());
        assert!(app.sides.iter().all(|s| s.is_none()), "{:?}", app.sides);
    }

    #[test]
    fn on_a_phone_a_picked_item_takes_the_screens_place() {
        let (mut shell, _rt) = shell();
        let mut app = App::default();
        render([390.0, 844.0], |ui| shell.show(ui, &mut app));
        assert!(app.sides.is_empty(), "{:?}", app.sides);

        app.pick = true;
        render([390.0, 844.0], |ui| shell.show(ui, &mut app));
        app.screens.clear();
        app.sides.clear();
        render([390.0, 844.0], |ui| shell.show(ui, &mut app));
        assert!(app.screens.is_empty(), "{:?}", app.screens);
        assert!(
            app.sides.iter().all(|s| *s == Some(Screen::Entries)),
            "{:?}",
            app.sides
        );
        assert!(!app.sides.is_empty());
    }

    #[test]
    fn without_screens_settings_is_all_there_is() {
        let (client, _rt) = client();
        let mut shell: Shell<Screen> = Shell::new(vec![], Settings::new(client));
        assert_eq!(shell.current(), None);
        render([900.0, 600.0], |ui| shell.ui(ui, |_, _| unreachable!()));
    }
}
