//! [`Shell`]: every app's window frame — its screens on a rail, Settings at the rail's
//! foot, and the services' state beside it.
//!
//! Wide, the rail runs down the left (as Obsidian's does); narrower than
//! [`COMPACT_WIDTH`], it becomes a bar along the bottom, as on a phone. The width decides,
//! not the platform, so a desktop window made narrow shows the phone layout.

use crate::Health;
use crate::settings::{COMPACT_WIDTH, Settings};

/// The rail's width, wide.
const RAIL_WIDTH: f32 = 48.0;

/// One of the app's screens on the rail.
pub struct Destination<D> {
    /// What the app matches on to draw it.
    pub id: D,
    /// A Lucide icon, as the apps' toolbars use (`egui::include_image!` of its SVG, stroked
    /// in white so it takes the text colour): the rail shows only this.
    pub icon: egui::ImageSource<'static>,
    /// The screen's name, shown when the icon is hovered.
    pub label: String,
}

impl<D> Destination<D> {
    /// A screen called `label`, drawn as `icon`.
    pub fn new(
        id: D,
        icon: egui::ImageSource<'static>,
        label: impl Into<String>,
    ) -> Destination<D> {
        Destination {
            id,
            icon,
            label: label.into(),
        }
    }
}

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
///         Destination::new(Screen::Entries, icons::settings(), "Entries"),
///         Destination::new(Screen::Summary, icons::settings(), "Summary"),
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
    destinations: Vec<Destination<D>>,
    open: Open<D>,
    settings: Settings,
    /// Whether the SVG loader has been put in place.
    loaders: bool,
}

impl<D: Copy + PartialEq> Shell<D> {
    /// A frame with `destinations` on the rail, the first one open. With none, Settings is
    /// all there is.
    pub fn new(destinations: Vec<Destination<D>>, mut settings: Settings) -> Shell<D> {
        settings.hosted();
        let open = destinations
            .first()
            .map_or(Open::Settings, |d| Open::Screen(d.id));
        Shell {
            destinations,
            open,
            settings,
            loaders: false,
        }
    }

    /// The app's screen that is open; `None` while Settings is.
    pub fn current(&self) -> Option<D> {
        match self.open {
            Open::Screen(id) => Some(id),
            Open::Settings => None,
        }
    }

    /// Open the app's screen `id`.
    pub fn open(&mut self, id: D) {
        self.open = Open::Screen(id);
    }

    /// Open Settings.
    pub fn open_settings(&mut self) {
        self.open = Open::Settings;
    }

    /// The Settings the rail's foot opens.
    pub fn settings(&mut self) -> &mut Settings {
        &mut self.settings
    }

    /// Render into the window's root `ui`. `content` draws the open screen of the app's
    /// into what the frame leaves of `ui`, as it would at the root: its own panels, then a
    /// `CentralPanel`.
    pub fn ui(&mut self, ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui, D)) {
        if !std::mem::replace(&mut self.loaders, true) {
            crate::icons::install_loaders(ui.ctx());
        }
        self.settings.poll();
        let health = self.settings.health();
        if ui.available_width() < COMPACT_WIDTH {
            egui::Panel::bottom("shell_bar").show(ui, |ui| self.bar_ui(ui, health));
        } else {
            egui::Panel::left("shell_rail")
                .resizable(false)
                .exact_size(RAIL_WIDTH)
                .show(ui, |ui| self.rail_ui(ui, health));
        }
        if health == Health::Problem {
            egui::Panel::top("shell_status").show(ui, |ui| self.settings.status_ui(ui));
        }
        match self.open {
            Open::Screen(id) => content(ui, id),
            Open::Settings => self.settings.ui(ui),
        }
    }

    fn rail_ui(&mut self, ui: &mut egui::Ui, health: Health) {
        ui.vertical_centered(|ui| {
            ui.add_space(4.0);
            for i in 0..self.destinations.len() {
                self.destination_ui(ui, i);
            }
        });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            self.settings_button_ui(ui);
            self.status_dot_ui(ui, health);
        });
    }

    /// The phone layout: one cell per screen, Settings last.
    ///
    /// Every screen gets a cell for now. When an app has more than a bar can hold, the ones
    /// that do not fit move to a separate list of all of them (with a choice of which stay
    /// on the bar) — not into Settings (#275).
    fn bar_ui(&mut self, ui: &mut egui::Ui, health: Health) {
        let cells = self.destinations.len() + 1;
        ui.columns(cells, |columns| {
            for (i, column) in columns.iter_mut().enumerate() {
                column.vertical_centered(|ui| {
                    if i < self.destinations.len() {
                        self.destination_ui(ui, i);
                    } else {
                        ui.horizontal(|ui| {
                            self.settings_button_ui(ui);
                            self.status_dot_ui(ui, health);
                        });
                    }
                });
            }
        });
    }

    fn destination_ui(&mut self, ui: &mut egui::Ui, i: usize) {
        let d = &self.destinations[i];
        let open = self.open == Open::Screen(d.id);
        if icon_button(ui, open, d.icon.clone())
            .on_hover_text(&d.label)
            .clicked()
        {
            self.open = Open::Screen(d.id);
        }
    }

    fn settings_button_ui(&mut self, ui: &mut egui::Ui) {
        let open = self.open == Open::Settings;
        if icon_button(ui, open, crate::icons::settings())
            .on_hover_text("Settings")
            .clicked()
        {
            self.open = Open::Settings;
        }
    }

    /// The services' state as a dot; clicking it opens About.
    fn status_dot_ui(&mut self, ui: &mut egui::Ui, health: Health) {
        let visuals = ui.visuals();
        let (color, text) = match health {
            Health::Connecting => (visuals.weak_text_color(), "Connecting…"),
            Health::Healthy => (egui::Color32::from_rgb(80, 180, 100), "Services running"),
            Health::UpdateAvailable => (visuals.hyperlink_color, "An update is available"),
            Health::Problem => (visuals.error_fg_color, "A service needs attention"),
        };
        let (rect, response) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::click());
        ui.painter().circle_filled(rect.center(), 4.0, color);
        if response.on_hover_text(text).clicked() {
            self.settings.open_about();
            self.open = Open::Settings;
        }
    }
}

fn icon_button(ui: &mut egui::Ui, open: bool, icon: egui::ImageSource<'static>) -> egui::Response {
    let image = crate::icons::image(ui, icon, 20.0);
    ui.add(
        egui::Button::image(image)
            .selected(open)
            .frame_when_inactive(open)
            .min_size(egui::vec2(36.0, 36.0)),
    )
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
                Destination::new(Screen::Entries, crate::icons::settings(), "Entries"),
                Destination::new(Screen::Summary, crate::icons::arrow_left(), "Summary"),
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
    fn without_screens_settings_is_all_there_is() {
        let (client, _rt) = client();
        let mut shell: Shell<Screen> = Shell::new(vec![], Settings::new(client));
        assert_eq!(shell.current(), None);
        render([900.0, 600.0], |ui| shell.ui(ui, |_, _| unreachable!()));
    }
}
