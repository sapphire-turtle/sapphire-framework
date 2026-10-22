//! [`Shell`]: an app's window frame — its screens on a rail, its actions under them, and
//! at the rail's foot the screens that belong there (Settings) and a slot of the app's own
//! (a status dot).
//!
//! Wide, the rail runs down the left (as Obsidian's does); narrower than
//! [`COMPACT_WIDTH`], it becomes a bar along the bottom, as on a phone. The width decides,
//! not the platform, so a desktop window made narrow shows the phone layout.

/// Below this width the layout is a phone's: the rail becomes a bottom bar, and screens
/// with a list and a page show one at a time.
pub const COMPACT_WIDTH: f32 = 600.0;

/// The rail's width, wide.
const RAIL_WIDTH: f32 = 48.0;

/// One of the app's screens on the rail.
pub struct Destination<D> {
    /// What the app matches on to draw it.
    pub id: D,
    /// A Lucide icon (see [`icons`](crate::icons)), stroked in white so it takes the text
    /// colour: the rail shows only this.
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

/// A button of the app's on the rail that does something — "New entry" — rather than
/// open a screen. [`Shell::take_action`] says which one was clicked.
pub struct Action {
    /// What [`Shell::take_action`] returns when it is clicked.
    pub id: &'static str,
    /// A Lucide icon, as for a [`Destination`].
    pub icon: egui::ImageSource<'static>,
    /// What it does, shown when the icon is hovered.
    pub label: String,
}

impl Action {
    /// A button called `label`, drawn as `icon`, reported as `id`.
    pub fn new(
        id: &'static str,
        icon: egui::ImageSource<'static>,
        label: impl Into<String>,
    ) -> Action {
        Action {
            id,
            icon,
            label: label.into(),
        }
    }
}

/// The window frame. Hold one as the app's root; call [`ui`](Self::ui) every frame.
///
/// ```no_run
/// # use corundum_gui::{Destination, Shell, icons};
/// # fn example(ui: &mut egui::Ui) {
/// #[derive(Clone, Copy, PartialEq)]
/// enum Screen { Gallery, Albums, Settings }
///
/// let mut shell = Shell::new(vec![
///     // The app's own Lucide SVGs: `egui::include_image!("../assets/icons/images.svg")`.
///     Destination::new(Screen::Gallery, icons::folder(), "Gallery"),
///     Destination::new(Screen::Albums, icons::folder(), "Albums"),
/// ])
/// .with_footer(vec![Destination::new(Screen::Settings, icons::settings(), "Settings")]);
/// shell.ui(ui, |ui, screen| match screen {
///     Screen::Gallery => { egui::CentralPanel::default().show(ui, |ui| ui.label("…")); }
///     Screen::Albums => { egui::CentralPanel::default().show(ui, |ui| ui.label("…")); }
///     Screen::Settings => { egui::CentralPanel::default().show(ui, |ui| ui.label("…")); }
/// });
/// # }
/// ```
pub struct Shell<D> {
    destinations: Vec<Destination<D>>,
    actions: Vec<Action>,
    footer: Vec<Destination<D>>,
    /// The action clicked and not yet taken.
    clicked: Option<&'static str>,
    /// Where each action and footer destination was last drawn, for the tests to click.
    #[cfg(test)]
    action_rects: Vec<egui::Rect>,
    #[cfg(test)]
    footer_rects: Vec<egui::Rect>,
    open: Option<D>,
    /// Whether the SVG loader has been put in place.
    loaders: bool,
}

impl<D: Copy + PartialEq> Shell<D> {
    /// A frame with `destinations` on the rail, the first one open.
    pub fn new(destinations: Vec<Destination<D>>) -> Shell<D> {
        let open = destinations.first().map(|d| d.id);
        Shell {
            destinations,
            actions: Vec::new(),
            footer: Vec::new(),
            clicked: None,
            #[cfg(test)]
            action_rects: Vec::new(),
            #[cfg(test)]
            footer_rects: Vec::new(),
            open,
            loaders: false,
        }
    }

    /// Put `actions` on the rail, under the screens (on the bottom bar, after them).
    pub fn with_actions(mut self, actions: Vec<Action>) -> Shell<D> {
        self.actions = actions;
        self
    }

    /// Put `footer` at the rail's foot (at the bottom bar's end): where an app puts
    /// Settings. With no other destinations, the first of these is open.
    pub fn with_footer(mut self, footer: Vec<Destination<D>>) -> Shell<D> {
        self.footer = footer;
        if self.open.is_none() {
            self.open = self.footer.first().map(|d| d.id);
        }
        self
    }

    /// The [`Action`] clicked in the last [`ui`](Self::ui), once: call it after `ui`
    /// every frame and do what it names.
    pub fn take_action(&mut self) -> Option<&'static str> {
        self.clicked.take()
    }

    /// The destination that is open; `None` only when there are none.
    pub fn current(&self) -> Option<D> {
        self.open
    }

    /// Open the destination `id`.
    pub fn open(&mut self, id: D) {
        self.open = Some(id);
    }

    /// Render into the window's root `ui`. `content` draws the open destination into what
    /// the frame leaves of `ui`, as it would at the root: its own panels, then a
    /// `CentralPanel`.
    pub fn ui(&mut self, ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui, D)) {
        self.ui_with_foot(ui, |_| None, content);
    }

    /// As [`ui`](Self::ui), with `foot` drawn beside the footer — a status dot, say. A
    /// destination `foot` returns is opened, and drawn this frame.
    pub fn ui_with_foot(
        &mut self,
        ui: &mut egui::Ui,
        foot: impl FnOnce(&mut egui::Ui) -> Option<D>,
        content: impl FnOnce(&mut egui::Ui, D),
    ) {
        if !std::mem::replace(&mut self.loaders, true) {
            crate::icons::install_loaders(ui.ctx());
        }
        let opened = if ui.available_width() < COMPACT_WIDTH {
            egui::Panel::bottom("shell_bar")
                .show(ui, |ui| self.bar_ui(ui, foot))
                .inner
        } else {
            egui::Panel::left("shell_rail")
                .resizable(false)
                .exact_size(RAIL_WIDTH)
                .show(ui, |ui| self.rail_ui(ui, foot))
                .inner
        };
        if let Some(id) = opened {
            self.open = Some(id);
        }
        if let Some(id) = self.open {
            content(ui, id);
        }
    }

    fn rail_ui(
        &mut self,
        ui: &mut egui::Ui,
        foot: impl FnOnce(&mut egui::Ui) -> Option<D>,
    ) -> Option<D> {
        ui.vertical_centered(|ui| {
            ui.add_space(4.0);
            for i in 0..self.destinations.len() {
                self.destination_ui(ui, i);
            }
            if !self.actions.is_empty() {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                for i in 0..self.actions.len() {
                    self.action_ui(ui, i);
                }
            }
        });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            for i in 0..self.footer.len() {
                self.footer_ui(ui, i);
            }
            foot(ui)
        })
        .inner
    }

    /// The phone layout: one cell per screen, then one per action, the footer and the
    /// foot last.
    ///
    /// Every screen gets a cell for now. When an app has more than a bar can hold, the ones
    /// that do not fit move to a separate list of all of them (with a choice of which stay
    /// on the bar) — not into Settings (#275).
    fn bar_ui(
        &mut self,
        ui: &mut egui::Ui,
        foot: impl FnOnce(&mut egui::Ui) -> Option<D>,
    ) -> Option<D> {
        let screens = self.destinations.len();
        let cells = screens + self.actions.len() + 1;
        let mut foot = Some(foot);
        let mut opened = None;
        ui.columns(cells, |columns| {
            for (i, column) in columns.iter_mut().enumerate() {
                column.vertical_centered(|ui| {
                    if i < screens {
                        self.destination_ui(ui, i);
                    } else if i < cells - 1 {
                        self.action_ui(ui, i - screens);
                    } else {
                        ui.horizontal(|ui| {
                            for i in 0..self.footer.len() {
                                self.footer_ui(ui, i);
                            }
                            if let Some(foot) = foot.take() {
                                opened = foot(ui);
                            }
                        });
                    }
                });
            }
        });
        opened
    }

    fn destination_ui(&mut self, ui: &mut egui::Ui, i: usize) {
        let d = &self.destinations[i];
        let open = self.open == Some(d.id);
        if icon_button(ui, open, d.icon.clone())
            .on_hover_text(&d.label)
            .clicked()
        {
            self.open = Some(d.id);
        }
    }

    fn footer_ui(&mut self, ui: &mut egui::Ui, i: usize) {
        let d = &self.footer[i];
        let open = self.open == Some(d.id);
        let response = icon_button(ui, open, d.icon.clone()).on_hover_text(&d.label);
        #[cfg(test)]
        {
            self.footer_rects
                .resize(self.footer.len(), egui::Rect::NOTHING);
            self.footer_rects[i] = response.rect;
        }
        if response.clicked() {
            self.open = Some(self.footer[i].id);
        }
    }

    fn action_ui(&mut self, ui: &mut egui::Ui, i: usize) {
        let a = &self.actions[i];
        let response = icon_button(ui, false, a.icon.clone()).on_hover_text(&a.label);
        #[cfg(test)]
        {
            self.action_rects
                .resize(self.actions.len(), egui::Rect::NOTHING);
            self.action_rects[i] = response.rect;
        }
        if response.clicked() {
            self.clicked = Some(a.id);
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
    use crate::test_support::render;

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Screen {
        Entries,
        Summary,
        Prefs,
    }

    const SIZES: [[f32; 2]; 2] = [[900.0, 600.0], [400.0, 700.0]];

    fn shell() -> Shell<Screen> {
        Shell::new(vec![
            Destination::new(Screen::Entries, crate::icons::settings(), "Entries"),
            Destination::new(Screen::Summary, crate::icons::arrow_left(), "Summary"),
        ])
    }

    fn prefs() -> Vec<Destination<Screen>> {
        vec![Destination::new(
            Screen::Prefs,
            crate::icons::settings(),
            "Preferences",
        )]
    }

    fn screen(ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| ui.label("screen"));
    }

    /// Lay `shell` out over two frames of a `size` screen, then click where `at` says.
    fn click(shell: &mut Shell<Screen>, size: [f32; 2], at: impl Fn(&Shell<Screen>) -> egui::Pos2) {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0], size[1]));
        let frame = |events: Vec<egui::Event>, shell: &mut Shell<Screen>| {
            let input = egui::RawInput {
                screen_rect: Some(rect),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| shell.ui(ui, |ui, _| screen(ui)));
            output.textures_delta.clear();
        };
        frame(vec![], shell);
        frame(vec![], shell);
        let pos = at(shell);
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(pos)], shell);
        frame(vec![button(true)], shell);
        frame(vec![button(false)], shell);
    }

    #[test]
    fn it_opens_on_the_first_screen_and_draws_it() {
        let mut shell = shell();
        assert_eq!(shell.current(), Some(Screen::Entries));
        for size in SIZES {
            let mut drawn = Vec::new();
            render(size, |ui| {
                shell.ui(ui, |ui, s| {
                    drawn.push(s);
                    screen(ui);
                })
            });
            assert_eq!(drawn, [Screen::Entries, Screen::Entries]);
        }
    }

    #[test]
    fn actions_are_drawn_in_both_layouts_and_report_nothing_unclicked() {
        let mut shell =
            shell().with_actions(vec![Action::new("new", crate::icons::plus(), "New entry")]);
        for size in SIZES {
            shell.action_rects.clear();
            render(size, |ui| shell.ui(ui, |ui, _| screen(ui)));
            assert!(shell.action_rects[0].is_positive(), "{size:?}");
            assert_eq!(shell.take_action(), None);
        }
        assert_eq!(shell.current(), Some(Screen::Entries));
    }

    #[test]
    fn a_clicked_action_is_taken_once_and_opens_nothing() {
        let mut shell =
            shell().with_actions(vec![Action::new("new", crate::icons::plus(), "New entry")]);
        click(&mut shell, SIZES[0], |s| s.action_rects[0].center());
        assert_eq!(shell.take_action(), Some("new"));
        assert_eq!(shell.take_action(), None);
        assert_eq!(shell.current(), Some(Screen::Entries));
    }

    #[test]
    fn footer_and_foot_are_in_both_layouts() {
        let mut shell = shell().with_footer(prefs());
        for size in SIZES {
            shell.footer_rects.clear();
            let mut foot = egui::Rect::NOTHING;
            render(size, |ui| {
                shell.ui_with_foot(
                    ui,
                    |ui| {
                        foot = ui
                            .allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::click())
                            .0;
                        None
                    },
                    |ui, _| screen(ui),
                )
            });
            assert!(shell.footer_rects[0].is_positive(), "{size:?}");
            assert!(foot.is_positive(), "{size:?}");
        }
    }

    #[test]
    fn clicking_a_footer_destination_opens_it() {
        for size in SIZES {
            let mut shell = shell().with_footer(prefs());
            click(&mut shell, size, |s| s.footer_rects[0].center());
            assert_eq!(shell.current(), Some(Screen::Prefs), "{size:?}");
        }
        let mut shell = shell().with_footer(prefs());
        shell.open(Screen::Prefs);
        let mut drawn = Vec::new();
        render([900.0, 600.0], |ui| {
            shell.ui(ui, |ui, s| {
                drawn.push(s);
                screen(ui);
            })
        });
        assert_eq!(drawn, [Screen::Prefs, Screen::Prefs]);
    }

    #[test]
    fn a_destination_the_foot_returns_is_opened() {
        let mut shell = shell();
        let mut drawn = Vec::new();
        render([900.0, 600.0], |ui| {
            shell.ui_with_foot(
                ui,
                |_| Some(Screen::Summary),
                |ui, s| {
                    drawn.push(s);
                    screen(ui);
                },
            )
        });
        assert_eq!(shell.current(), Some(Screen::Summary));
        assert_eq!(drawn, [Screen::Summary, Screen::Summary]);
    }

    #[test]
    fn with_only_a_footer_it_opens_on_the_footer() {
        let shell = Shell::new(vec![]).with_footer(prefs());
        assert_eq!(shell.current(), Some(Screen::Prefs));
    }

    #[test]
    fn with_nothing_there_is_nothing_to_draw() {
        let mut shell: Shell<Screen> = Shell::new(vec![]);
        assert_eq!(shell.current(), None);
        for size in SIZES {
            render(size, |ui| shell.ui(ui, |_, s| panic!("{s:?} drawn")));
        }
    }
}
