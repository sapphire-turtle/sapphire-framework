//! [`Settings`]: the app's own sections, the framework's sync screens, the pages the
//! settings schema makes, and About, as a list and a page, wired to a [`FrameworkClient`].

use std::collections::HashMap;
use std::path::PathBuf;

use crate::Health;
use crate::client::{
    Command, CommandError, CommandId, CommandOutput, ConfigState, FrameworkClient, GuiValues,
    Snapshot,
};
use corundum_framework_gui::COMPACT_WIDTH;
use corundum_framework_gui::i18n::{self, Message};
use corundum_framework_gui::settings::{ListItem, SettingsLayout};
use corundum_framework_gui::theme::{self, ThemeSettings};
use corundum_framework_gui::tr;
use sapphire_bridge_api::Value;

use crate::views::appearance::{
    AppearanceCtx, AppearancePage, bridge_down, language_settings, old_bridge, theme_settings,
};
use crate::views::config::model::{Drafts, PageSpec, Placement, pages, pending_restarts};
use crate::views::config::{ConfigPage, RestartBanner};
use crate::views::{
    DeviceList, EmbeddingView, ExternalDeviceList, ServiceStatusBanner, ViewCtx, WorkgroupView,
    WorkspacePicker, error_text_line, unavailable,
};

/// A page the app adds to [`Settings`], under its own heading: corundum-framework-gui's.
pub use corundum_framework_gui::settings::SettingsSection;

/// The framework's pages.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Screen {
    Workspaces,
    Devices,
    Workgroup,
    ExternalDevices,
    Embedding,
}

/// The sync pages and the ids of the messages naming them in the list.
const SYNC_SCREENS: [(Screen, &str); 5] = [
    (Screen::Workspaces, "sapphire-settings-workspace"),
    (Screen::Devices, "sapphire-settings-devices"),
    (Screen::Workgroup, "sapphire-settings-workgroup"),
    (
        Screen::ExternalDevices,
        "sapphire-settings-external-devices",
    ),
    (Screen::Embedding, "sapphire-settings-embedding"),
];

/// The screen a settings section of the same name joins, below its own content.
const JOINED_SCREEN: (Screen, &str) = (Screen::Embedding, "Embedding");

/// The settings section the Appearance page draws itself, with the theme widget.
const APPEARANCE: &str = "Appearance";

/// Which page is open.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Page {
    App(usize),
    Appearance,
    Sync(Screen),
    /// The settings page of this schema section.
    Config(String),
    About,
}

/// Which view sent a command, so its outcome goes back there.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Origin {
    Banner,
    Screen(Screen),
    /// The settings page of this section: its Apply or its "↻".
    Page(String),
    /// A load the settings sent themselves.
    Load,
    /// The restart banner.
    Restart,
    /// The Appearance page's writes.
    Appearance,
}

/// The settings, on corundum-framework-gui's [`SettingsLayout`]: the app's sections, then General (Appearance), then Sync (workspace,
/// devices, workgroup, external devices, embedding), then the settings pages under
/// "Configuration", then About (the services and the version). Hold one; call
/// [`ui`](Self::ui) every frame it is shown.
///
/// The settings pages come from the settings schema: a section that is all the app's is
/// listed under the app's heading, one named like the Embedding screen is shown inside it,
/// and the rest are under "Configuration". They are loaded when first shown, and again
/// after a write, a restart, or a change of the app server's workspace.
///
/// Inside a [`Shell`](crate::Shell), the shell calls [`poll`](Self::poll) and shows the
/// status banner itself. On its own, Settings shows the banner above its pages whenever
/// something is wrong.
pub struct Settings {
    client: FrameworkClient,
    app_heading: String,
    app_sections: Vec<Box<dyn SettingsSection>>,
    /// The list beside the open page, or one at a time when compact. The pages come and
    /// go with the schema, so the list is given to it every frame.
    layout: SettingsLayout<Page>,
    /// Whether [`ui`](Self::ui) draws the status banner (off inside a shell, which does).
    own_banner: bool,
    banner: ServiceStatusBanner,
    workspaces: WorkspacePicker,
    devices: DeviceList,
    workgroup: WorkgroupView,
    embedding: EmbeddingView,
    external: ExternalDeviceList,
    /// The settings as last loaded; `None` until a load answers.
    config: Option<ConfigState>,
    /// The pages the loaded schema makes, in schema order.
    pages: Vec<PageSpec>,
    /// One page per section, kept across reloads with its open tab and its errors.
    config_pages: HashMap<String, ConfigPage>,
    /// What has been typed into the pages and not applied yet.
    drafts: Drafts,
    restart: RestartBanner,
    /// Why the last load failed.
    load_error: Option<String>,
    /// A load was sent since the app server last came up: showing a page again does not
    /// retry a failed one.
    load_tried: bool,
    /// The app server's workspace as last seen (`Some(None)`: it has none).
    workspace: Option<Option<PathBuf>>,
    pending: HashMap<CommandId, Origin>,
    /// The theme's settings as last installed from the snapshot.
    seen_theme: Option<GuiValues>,
    /// The language (`gui.locale`) as last installed from the snapshot.
    seen_locale: Option<Value>,
    appearance: AppearancePage,
    /// The page the last [`ui`](Self::ui) drew, for the tests.
    #[cfg(test)]
    drawn: Option<Page>,
    /// Every command sent, for the tests.
    #[cfg(test)]
    sent: Vec<Command>,
    /// The snapshot the tests show instead of the client's.
    #[cfg(test)]
    test_snapshot: Option<Snapshot>,
}

impl Settings {
    /// Settings over `client`, with only the framework's pages.
    pub fn new(client: FrameworkClient) -> Settings {
        Settings {
            client,
            app_heading: String::new(),
            app_sections: Vec::new(),
            layout: SettingsLayout::new(),
            own_banner: true,
            banner: ServiceStatusBanner::default(),
            workspaces: WorkspacePicker::default(),
            devices: DeviceList::default(),
            workgroup: WorkgroupView::default(),
            embedding: EmbeddingView::default(),
            external: ExternalDeviceList::default(),
            config: None,
            pages: Vec::new(),
            config_pages: HashMap::new(),
            drafts: Drafts::default(),
            restart: RestartBanner::default(),
            load_error: None,
            load_tried: false,
            workspace: None,
            pending: HashMap::new(),
            seen_theme: None,
            seen_locale: None,
            appearance: AppearancePage::default(),
            #[cfg(test)]
            drawn: None,
            #[cfg(test)]
            sent: Vec::new(),
            #[cfg(test)]
            test_snapshot: None,
        }
    }

    /// Add the app's own pages, listed first under `heading` (the app's name, say).
    pub fn with_app_sections(
        mut self,
        heading: impl Into<String>,
        sections: Vec<Box<dyn SettingsSection>>,
    ) -> Settings {
        self.app_heading = heading.into();
        self.app_sections = sections;
        self
    }

    /// Does nothing: the framework keeps the theme in the settings registry, synced
    /// across the workgroup, and installs it itself (#327).
    #[deprecated(note = "the framework saves the theme in the settings registry (#327)")]
    pub fn with_theme(self, _theme: ThemeSettings) -> Settings {
        self
    }

    /// Always `None`: the framework saves the theme itself (#327).
    #[deprecated(note = "the framework saves the theme in the settings registry (#327)")]
    pub fn take_theme_changed(&mut self) -> Option<ThemeSettings> {
        None
    }

    /// The client the framework's pages use.
    pub fn client(&self) -> &FrameworkClient {
        &self.client
    }

    /// The services' state, as of the last refresh.
    pub fn health(&self) -> Health {
        Health::of(&self.client.snapshot())
    }

    /// Whether About is the page open.
    #[cfg(test)]
    pub(crate) fn about_open(&self) -> bool {
        self.layout.current() == Some(&Page::About)
    }

    /// Whether the last [`ui`](Self::ui) drew About.
    #[cfg(test)]
    pub(crate) fn about_drawn(&self) -> bool {
        self.drawn == Some(Page::About)
    }

    /// Open About, where the services' state is.
    pub fn open_about(&mut self) {
        self.layout.open(Some(Page::About));
    }

    /// Hand each finished command's outcome to the view that sent it, and reload the
    /// settings when the app server's workspace changed. [`ui`](Self::ui) does this; a
    /// host that shows Settings only sometimes calls it every frame, so the banner hears
    /// how an install went while another screen is open.
    pub fn poll(&mut self) {
        let snapshot = self.snapshot();
        self.track_workspace(&snapshot);
        for outcome in self.client.drain_outcomes() {
            if let Some(origin) = self.pending.remove(&outcome.id) {
                self.on_outcome(origin, &outcome.result);
            }
        }
    }

    /// Install the theme and the language the settings hold, whenever the bridge reports
    /// other values than last time: at start, after a write, after a sync from another
    /// device. Only a change of the values installs, so a slider being dragged on the
    /// Appearance page keeps its preview; the theme and the language are compared each on
    /// its own. Without the values (no bridge, or an older one) the theme stays; without
    /// `gui.locale` (a bridge that knows the theme but not the language) the language
    /// stays. A [`Shell`](crate::Shell) calls this every frame; on its own,
    /// [`ui`](Self::ui) does.
    pub fn follow_theme(&mut self, ctx: &egui::Context) {
        let snapshot = self.snapshot();
        let Some(gui) = snapshot.bridge.up().and_then(|b| b.gui.as_ref()) else {
            return;
        };
        let theme: GuiValues = gui
            .iter()
            .filter(|(k, _)| sapphire_bridge_api::settings::gui::THEME_KEYS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if self.seen_theme.as_ref() != Some(&theme) {
            theme::install(ctx, &theme_settings(&theme));
            self.seen_theme = Some(theme);
        }
        if let Some(locale) = gui.get(sapphire_bridge_api::settings::gui::LOCALE)
            && self.seen_locale.as_ref() != Some(locale)
        {
            i18n::install(ctx, &language_settings(gui));
            self.seen_locale = Some(locale.clone());
        }
    }

    /// Install the stored theme and language again on the next
    /// [`follow_theme`](Self::follow_theme), even if their values have not changed: the
    /// look has moved off them.
    pub(crate) fn forget_seen_theme(&mut self) {
        self.seen_theme = None;
        self.seen_locale = None;
    }

    /// Render the status banner: the services and their install buttons, a newer release,
    /// an install's failure.
    pub fn status_ui(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot();
        let cx = ViewCtx {
            snapshot: &snapshot,
            app_name: self.client.app().app_name,
            busy: self.busy(&Origin::Banner),
        };
        if let Some(command) = self.banner.ui(ui, &cx) {
            self.send(command, Origin::Banner);
        }
    }

    /// Render into `ui`: the list beside the open page, or — narrower than
    /// [`COMPACT_WIDTH`] — the list, or the open page under a way back to it.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        crate::i18n::register(ui.ctx(), crate::RESOURCES);
        self.poll();
        if self.own_banner {
            self.follow_theme(ui.ctx());
        }
        if self.own_banner && self.health() == Health::Problem {
            egui::Panel::top("settings_status").show(ui, |ui| self.status_ui(ui));
        }
        // Wide, a page opens at once: the app's first, else the workspace, rather than the
        // first in the list (the app's schema pages, loaded later, come before them).
        if self.layout.current().is_none() && ui.available_width() >= COMPACT_WIDTH {
            self.layout.open(Some(self.first_page()));
        }
        #[cfg(test)]
        {
            self.drawn = self.layout.current().cloned();
        }
        let items = self.list(ui.ctx());
        // The pages draw through `self`, which holds the layout.
        let mut layout = std::mem::take(&mut self.layout);
        layout.ui(ui, &items, |ui, page| self.page_ui(ui, page));
        self.layout = layout;
    }

    /// Hosted by a shell, which polls and shows the banner itself.
    pub(crate) fn hosted(&mut self) {
        self.own_banner = false;
    }

    fn first_page(&self) -> Page {
        if self.app_sections.is_empty() {
            Page::Sync(Screen::Workspaces)
        } else {
            Page::App(0)
        }
    }

    /// The list: headings, entries and gaps, in order, in the language installed in `ctx`.
    /// A page with a draft is marked.
    fn list(&self, ctx: &egui::Context) -> Vec<ListItem<Page>> {
        let dirty = self.drafts.dirty_sections();
        let config = |placement: Placement| -> Vec<ListItem<Page>> {
            self.pages
                .iter()
                .filter(|p| p.placement == placement)
                .map(|p| ListItem::Entry {
                    page: Page::Config(p.section.clone()),
                    label: p.section.clone(),
                    dirty: dirty.contains(&p.section),
                })
                .collect()
        };
        let mut out = Vec::new();
        let app_pages = config(Placement::App);
        if !app_pages.is_empty() || !self.app_sections.is_empty() {
            let heading = if self.app_heading.is_empty() {
                self.client.app().app_name.to_owned()
            } else {
                self.app_heading.clone()
            };
            out.push(ListItem::Heading(heading));
            out.extend(app_pages);
            for (i, section) in self.app_sections.iter().enumerate() {
                out.push(ListItem::Entry {
                    page: Page::App(i),
                    label: section.label(ctx),
                    dirty: section.dirty(),
                });
            }
        }
        out.push(ListItem::Heading(tr!(ctx, "sapphire-settings-general")));
        out.push(ListItem::entry(
            Page::Appearance,
            tr!(ctx, "sapphire-settings-appearance"),
        ));
        out.push(ListItem::Heading(tr!(ctx, "sapphire-settings-sync")));
        for (screen, id) in SYNC_SCREENS {
            let joined = screen == JOINED_SCREEN.0
                && self.pages.iter().any(|p| {
                    p.placement == Placement::Screen(JOINED_SCREEN.1.to_owned())
                        && dirty.contains(&p.section)
                });
            out.push(ListItem::Entry {
                page: Page::Sync(screen),
                label: tr!(ctx, id),
                dirty: joined,
            });
        }
        let rest = config(Placement::Configuration);
        if !rest.is_empty() {
            out.push(ListItem::Heading(tr!(
                ctx,
                "sapphire-settings-configuration"
            )));
            out.extend(rest);
        }
        out.push(ListItem::Gap);
        out.push(ListItem::entry(
            Page::About,
            tr!(ctx, "sapphire-settings-about"),
        ));
        out
    }

    fn page_ui(&mut self, ui: &mut egui::Ui, page: Page) {
        match page {
            Page::App(i) => {
                if let Some(section) = self.app_sections.get_mut(i) {
                    section.ui(ui);
                }
            }
            Page::Appearance => self.appearance_ui(ui),
            Page::Sync(screen) => self.screen_ui(ui, screen),
            Page::Config(section) => {
                let snapshot = self.snapshot();
                self.ensure_loaded(&snapshot);
                self.restart_ui(ui, &snapshot);
                if self.config.is_none() {
                    self.unloaded_ui(ui, &snapshot);
                } else {
                    // A page gone from the schema is gone from the list too, which closes
                    // it.
                    self.config_page_ui(ui, &snapshot, &section);
                }
            }
            Page::About => self.about_ui(ui),
        }
    }

    fn appearance_ui(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot();
        if snapshot.server.up().is_none() {
            unavailable(
                ui,
                Message::new("sapphire-unavailable-server").arg("app", self.client.app().app_name),
            );
            return;
        }
        self.ensure_loaded(&snapshot);
        let bridge = snapshot.bridge.up();
        let why = match bridge {
            None => Some(bridge_down()),
            Some(b) if b.gui.is_none() => Some(old_bridge()),
            Some(_) => None,
        };
        if self.config.is_none() && why.is_none() {
            self.unloaded_ui(ui, &snapshot);
            return;
        }
        let cx = AppearanceCtx {
            state: self.config.as_ref(),
            unavailable: why,
            busy: self.busy(&Origin::Appearance),
            workgroup: bridge.is_some_and(|b| b.status.workgroup.is_some()),
            language: bridge
                .and_then(|b| b.gui.as_ref())
                .is_some_and(|g| g.contains_key(sapphire_bridge_api::settings::gui::LOCALE)),
        };
        if let Some(command) = self.appearance.ui(ui, &cx) {
            self.send(command, Origin::Appearance);
        }
    }

    fn screen_ui(&mut self, ui: &mut egui::Ui, screen: Screen) {
        let snapshot = self.snapshot();
        let origin = Origin::Screen(screen);
        let cx = ViewCtx {
            snapshot: &snapshot,
            app_name: self.client.app().app_name,
            busy: self.busy(&origin),
        };
        let command = match screen {
            Screen::Workspaces => self.workspaces.ui(ui, &cx),
            Screen::Devices => self.devices.ui(ui, &cx),
            Screen::Workgroup => self.workgroup.ui(ui, &cx),
            Screen::ExternalDevices => self.external.ui(ui, &cx),
            Screen::Embedding => {
                self.ensure_loaded(&snapshot);
                self.restart_ui(ui, &snapshot);
                self.embedding.ui(ui, &cx);
                ui.add_space(12.0);
                if self.config.is_none() {
                    self.unloaded_ui(ui, &snapshot);
                }
                let joined: Vec<String> = self
                    .pages
                    .iter()
                    .filter(|p| p.placement == Placement::Screen(JOINED_SCREEN.1.into()))
                    .map(|p| p.section.clone())
                    .collect();
                for section in joined {
                    self.config_page_ui(ui, &snapshot, &section);
                }
                None
            }
        };
        if let Some(command) = command {
            self.send(command, origin);
        }
    }

    /// Render the page of `section`; `false` when the loaded schema has no such section.
    fn config_page_ui(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot, section: &str) -> bool {
        let (Some(state), Some(spec)) = (
            &self.config,
            self.pages.iter().find(|p| p.section == section),
        ) else {
            return false;
        };
        let Some(page) = self.config_pages.get_mut(section) else {
            return false;
        };
        let cx = ViewCtx {
            snapshot,
            app_name: self.client.app().app_name,
            busy: self
                .pending
                .values()
                .any(|o| matches!(o, Origin::Page(_) | Origin::Load | Origin::Restart)),
        };
        error_text_line(ui, &mut self.load_error);
        let command = page.ui(ui, &cx, spec, state, &mut self.drafts);
        if let Some(command) = command {
            self.send(command, Origin::Page(section.to_owned()));
        }
        true
    }

    /// Nothing is loaded yet: a spinner while a load runs, else why it failed and a way to
    /// try again. Nothing without an app server, whose absence the page already shows.
    fn unloaded_ui(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        if snapshot.server.up().is_none() {
            return;
        }
        if self.loading() {
            ui.spinner();
            return;
        }
        error_text_line(ui, &mut self.load_error);
        if ui.button(tr!(ui, "sapphire-settings-load")).clicked() {
            self.load();
        }
    }

    /// The restart banner, above the settings pages and the Embedding screen.
    fn restart_ui(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let Some(state) = &self.config else { return };
        let pending = pending_restarts(state);
        let cx = ViewCtx {
            snapshot,
            app_name: self.client.app().app_name,
            busy: self.loading(),
        };
        if let Some(command) = self.restart.ui(ui, &cx, &pending) {
            self.send(command, Origin::Restart);
        }
        if !pending.is_empty() {
            ui.separator();
        }
    }

    fn about_ui(&mut self, ui: &mut egui::Ui) {
        let app = self.client.app();
        ui.heading(app.app_name);
        ui.label(tr!(ui, "sapphire-settings-version", version = app.version));
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tr!(ui, "sapphire-settings-services")).strong());
        self.status_ui(ui);
    }

    /// Take a finished command's outcome, for the view that sent it.
    fn on_outcome(&mut self, origin: Origin, result: &Result<CommandOutput, CommandError>) {
        match origin {
            Origin::Banner => self.banner.on_outcome(result),
            Origin::Screen(Screen::Workspaces) => self.workspaces.on_outcome(result),
            Origin::Screen(Screen::Devices) => self.devices.on_outcome(result),
            Origin::Screen(Screen::Workgroup) => self.workgroup.on_outcome(result),
            Origin::Screen(Screen::ExternalDevices) => self.external.on_outcome(result),
            // It only shows; its settings page sends.
            Origin::Screen(Screen::Embedding) => {}
            Origin::Page(section) => {
                if let Some(page) = self.config_pages.get_mut(&section) {
                    page.on_outcome(result, &mut self.drafts);
                }
                match result {
                    Ok(CommandOutput::Config(state)) => self.loaded(state.clone()),
                    Ok(CommandOutput::Written(_)) => self.load(),
                    _ => {}
                }
            }
            Origin::Load => match result {
                Ok(CommandOutput::Config(state)) => self.loaded(state.clone()),
                Ok(_) => {}
                Err(e) => {
                    self.load_error = Some(e.message.clone());
                    self.appearance.reloaded();
                }
            },
            Origin::Restart => {
                self.restart.on_outcome(result);
                if result.is_ok() {
                    self.load();
                }
            }
            Origin::Appearance => {
                if self.appearance.on_outcome(result) {
                    self.forget_seen_theme();
                }
                self.load();
            }
        }
    }

    /// Keep a load's settings, and the pages their schema makes. Drafts are untouched.
    fn loaded(&mut self, state: ConfigState) {
        let app_name = self.client.app().app_name;
        self.pages = pages(&state.schema, app_name, &[JOINED_SCREEN.1, APPEARANCE]);
        for spec in &self.pages {
            let page = self
                .config_pages
                .entry(spec.section.clone())
                .or_insert_with(|| ConfigPage::new(spec.section.clone()));
            // The screen it joins titles it.
            page.titled = !matches!(spec.placement, Placement::Screen(_));
        }
        self.config = Some(state);
        self.load_error = None;
        self.appearance.reloaded();
    }

    /// Reload once the app server's workspace changes, the settings having been loaded.
    /// The app server going away lets showing a page load again.
    fn track_workspace(&mut self, snapshot: &Snapshot) {
        let Some(server) = snapshot.server.up() else {
            self.load_tried = false;
            return;
        };
        let seen = server.current.as_ref().map(|c| c.root.clone());
        if self.workspace.as_ref() == Some(&seen) {
            return;
        }
        self.workspace = Some(seen);
        if self.config.is_some() {
            self.load();
        }
    }

    /// Load when a page that needs the settings is shown and nothing is loaded.
    fn ensure_loaded(&mut self, snapshot: &Snapshot) {
        if self.config.is_none()
            && !self.load_tried
            && !self.loading()
            && snapshot.server.up().is_some()
        {
            self.load();
        }
    }

    fn load(&mut self) {
        self.load_tried = true;
        self.send(Command::ConfigLoad, Origin::Load);
    }

    /// Whether a load is running, the settings' own or a page's "↻".
    fn loading(&self) -> bool {
        self.pending
            .values()
            .any(|o| matches!(o, Origin::Load | Origin::Page(_)))
    }

    fn busy(&self, origin: &Origin) -> bool {
        self.pending.values().any(|p| p == origin)
    }

    fn send(&mut self, command: Command, origin: Origin) {
        #[cfg(test)]
        self.sent.push(command.clone());
        let id = self.client.send(command);
        self.pending.insert(id, origin);
    }

    /// The latest snapshot (in the tests, the one they set).
    fn snapshot(&self) -> Snapshot {
        #[cfg(test)]
        if let Some(snapshot) = &self.test_snapshot {
            return snapshot.clone();
        }
        self.client.snapshot().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{
        BridgeState, Command, CommandOutput, ConfigState, Conn, ServerState, Snapshot,
    };
    use crate::test_support::{client, render, render_in};
    use crate::views::config::widgets::tests::drawn_text;
    use sapphire_backend::protocol::{CurrentWorkspace, StatusReport, SyncStatusResult};
    use sapphire_bridge_api::settings::{Apply, Process};
    use sapphire_bridge_api::{ConfigWriteResult, Entry, EntryValue, Kind, Scope, SettingDef};
    use sapphire_bridge_api::{
        EmbedInfoResult, EmbedNote, EmbedSettingsResult, ExternalDeviceInfo, GrainId, LocalModel,
        ModelSettings, ModelSource, PeerInfo, RemoteModel, Slot, StatusResult, WorkgroupStatus,
        WorkgroupWorkspaceInfo,
    };

    fn rich() -> Snapshot {
        let wg = GrainId::random();
        Snapshot {
            fetched: true,
            bridge: Conn::Up(BridgeState {
                gui: None,
                status: StatusResult {
                    version: "1".into(),
                    node_id: "aaaa".into(),
                    workgroup: Some(WorkgroupStatus {
                        workgroup_id: wg,
                        name: "home".into(),
                        devices: 2,
                    }),
                    routes: vec![],
                    embedding: None,
                },
                peers: vec![
                    PeerInfo {
                        device_id: GrainId::random(),
                        name: "desk".into(),
                        node_id: "aaaa".into(),
                        connected: true,
                        priority: 1,
                        availability: None,
                    },
                    PeerInfo {
                        device_id: GrainId::random(),
                        name: "laptop".into(),
                        node_id: "bbbb".into(),
                        connected: false,
                        priority: 1,
                        availability: None,
                    },
                ],
                peer_roles: vec![],
                ledger: vec![WorkgroupWorkspaceInfo {
                    workspace_id: GrainId::random(),
                    app_name: "app".into(),
                    name: "remote".into(),
                }],
                external_devices: Some(vec![ExternalDeviceInfo {
                    id: GrainId::random(),
                    name: "pendant".into(),
                    description: None,
                    apps: vec!["app".into()],
                    created_at: "2026-10-11T00:00:00Z".into(),
                    rotated_at: None,
                    retired_at: None,
                }]),
                embedding: Some(EmbedSettingsResult {
                    models: ModelSettings {
                        local: Some(LocalModel::default()),
                        remote: Some(RemoteModel {
                            endpoint: "https://api.example.com".into(),
                            model: "m".into(),
                            dimension: 8,
                        }),
                    },
                    source: Some(ModelSource::Workgroup),
                    remote_enabled: true,
                    active: Some(Slot::Remote),
                    info: EmbedInfoResult {
                        enabled: true,
                        note: Some(EmbedNote::KeyMissing),
                        ..EmbedInfoResult::default()
                    },
                    ..EmbedSettingsResult::default()
                }),
            }),
            server: Conn::Up(ServerState {
                info: StatusReport {
                    running: true,
                    version: Some("1".into()),
                    pid: Some(1),
                    managed_by: None,
                    app: vec![],
                },
                current: Some(CurrentWorkspace {
                    root: "/x/notes".into(),
                    reachable: true,
                    workspace_id: None,
                    sync: SyncStatusResult::not_synced(),
                }),
                update: None,
            }),
        }
    }

    fn render_views(snapshot: &Snapshot) {
        let mut banner = ServiceStatusBanner::default();
        let mut wg = WorkgroupView::default();
        let mut devices = DeviceList::default();
        let mut workspaces = WorkspacePicker::default();
        let mut embedding = EmbeddingView::default();
        let mut external = ExternalDeviceList::default();
        render([800.0, 600.0], |ui| {
            let cx = ViewCtx {
                snapshot,
                app_name: "app",
                busy: false,
            };
            assert!(banner.ui(ui, &cx).is_none());
            assert!(wg.ui(ui, &cx).is_none());
            assert!(devices.ui(ui, &cx).is_none());
            assert!(workspaces.ui(ui, &cx).is_none());
            embedding.ui(ui, &cx);
            assert!(external.ui(ui, &cx).is_none());
        });
    }

    #[test]
    fn every_view_renders_a_full_snapshot_without_emitting_commands() {
        render_views(&rich());
    }

    #[test]
    fn every_view_renders_absent_and_incompatible_processes() {
        render_views(&Snapshot::default());
        render_views(&Snapshot {
            fetched: true,
            bridge: Conn::Incompatible("update sapphire-bridge".into()),
            server: Conn::Error("closed".into()),
        });
    }

    struct Named(&'static str, usize);

    impl SettingsSection for Named {
        fn label(&self, _ctx: &egui::Context) -> String {
            self.0.to_owned()
        }

        fn ui(&mut self, ui: &mut egui::Ui) {
            self.1 += 1;
            ui.label(self.0);
        }
    }

    fn settings() -> (Settings, tokio::runtime::Runtime) {
        let (client, runtime) = client();
        let settings =
            Settings::new(client).with_app_sections("Example", vec![Box::new(Named("General", 0))]);
        (settings, runtime)
    }

    #[test]
    fn settings_list_follows_the_language() {
        let ctx = crate::test_support::english_ctx();
        let (mut settings, _rt) = settings();
        let set = |tag: &str| {
            crate::i18n::install(
                &ctx,
                &crate::i18n::LanguageSettings {
                    language: crate::i18n::Language::Explicit(tag.into()),
                },
            )
        };
        let texts = render_in(&ctx, [900.0, 600.0], |ui| settings.ui(ui));
        assert!(texts.iter().any(|t| t == "Workspace"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "General"), "{texts:?}");
        set("ja");
        let texts = render_in(&ctx, [900.0, 600.0], |ui| settings.ui(ui));
        assert!(texts.iter().any(|t| t == "ワークスペース"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "Workspace"), "{texts:?}");
    }

    #[test]
    fn wide_it_opens_the_first_app_section_beside_the_list() {
        let (mut settings, _rt) = settings();
        render([900.0, 600.0], |ui| settings.ui(ui));
        assert_eq!(settings.layout.current().cloned(), Some(Page::App(0)));
    }

    #[test]
    fn compact_it_shows_the_list_then_one_page() {
        let (mut settings, _rt) = settings();
        render([400.0, 700.0], |ui| settings.ui(ui));
        assert_eq!(
            settings.layout.current().cloned(),
            None,
            "the list alone, no page chosen"
        );
        settings.open_about();
        render([400.0, 700.0], |ui| settings.ui(ui));
        assert_eq!(settings.layout.current().cloned(), Some(Page::About));
    }

    #[test]
    fn without_app_sections_it_opens_on_the_workspace() {
        let (client, _rt) = client();
        let mut settings = Settings::new(client);
        render([900.0, 600.0], |ui| settings.ui(ui));
        assert_eq!(
            settings.layout.current().cloned(),
            Some(Page::Sync(Screen::Workspaces))
        );
    }

    // ── the schema's pages ──────────────────────────────────────────────────

    fn def(key: &str, section: &str, process: Process) -> SettingDef {
        SettingDef {
            key: key.into(),
            kind: Kind::String { non_empty: false },
            scopes: vec![Scope::Device],
            overridable: false,
            apply: Apply::Restart,
            default: None,
            section: section.into(),
            doc: String::new(),
            process,
        }
    }

    /// An app section, two others, and Embedding, whose cache dir awaits a restart.
    fn config_state() -> ConfigState {
        ConfigState {
            schema: vec![
                def("sapphire-gui-test.color", "Look", Process::App),
                def("net.discovery", "Network", Process::Bridge),
                def("embed.cache_dir", "Embedding", Process::Bridge),
                def("server.log", "Server", Process::App),
            ],
            effective: vec![Entry {
                key: "embed.cache_dir".into(),
                value: EntryValue::Unset,
                origin: sapphire_bridge_api::Origin::Default,
                pending_restart: true,
                note: None,
            }],
            by_scope: Default::default(),
        }
    }

    fn loads(settings: &Settings) -> usize {
        settings
            .sent
            .iter()
            .filter(|c| matches!(c, Command::ConfigLoad))
            .count()
    }

    fn entry_labels(settings: &Settings) -> Vec<String> {
        settings
            .list(&crate::test_support::english_ctx())
            .into_iter()
            .map(|item| match item {
                ListItem::Heading(h) => format!("# {h}"),
                ListItem::Entry { label, dirty, .. } if dirty => format!("{label} •"),
                ListItem::Entry { label, .. } => label,
                ListItem::Gap => "-".into(),
            })
            .collect()
    }

    #[test]
    fn config_pages_are_listed_by_placement() {
        let (mut settings, _rt) = settings();
        let before = entry_labels(&settings);
        assert!(
            !before.contains(&"# Configuration".to_owned()),
            "{before:?}"
        );
        settings.loaded(config_state());
        assert_eq!(
            entry_labels(&settings),
            [
                "# Example",
                "Look",
                "General",
                "# General",
                "Appearance",
                "# Sync",
                "Workspace",
                "Devices",
                "Workgroup",
                "External devices",
                "Embedding",
                "# Configuration",
                "Network",
                "Server",
                "-",
                "About",
            ]
        );
        assert!(
            settings
                .list(&crate::test_support::english_ctx())
                .contains(&ListItem::entry(Page::Config("Look".into()), "Look"))
        );
    }

    #[test]
    fn a_dirty_page_is_marked_in_the_list() {
        let (mut settings, _rt) = settings();
        settings.loaded(config_state());
        settings
            .drafts
            .get_mut("Network", Scope::Device)
            .unset("net.discovery");
        let labels = entry_labels(&settings);
        assert!(labels.contains(&"Network •".to_owned()), "{labels:?}");
        assert!(labels.contains(&"Server".to_owned()));
    }

    #[test]
    fn showing_a_config_page_loads_once() {
        // A schema page is listed only once the schema is loaded; the Embedding screen,
        // whose section page joins it, is there before.
        let (mut shown, _rt) = settings();
        shown.test_snapshot = Some(rich());
        shown.layout.open(Some(Page::Sync(Screen::Embedding)));
        render([900.0, 600.0], |ui| shown.ui(ui));
        render([900.0, 600.0], |ui| shown.ui(ui));
        assert_eq!(loads(&shown), 1);
        // Loaded: nothing more to fetch, on a schema page either.
        shown.loaded(config_state());
        render([900.0, 600.0], |ui| shown.ui(ui));
        shown.layout.open(Some(Page::Config("Network".into())));
        render([900.0, 600.0], |ui| shown.ui(ui));
        assert_eq!(shown.drawn, Some(Page::Config("Network".into())));
        assert_eq!(loads(&shown), 1);
        // Another page needs nothing.
        let (mut other, _rt2) = settings();
        other.test_snapshot = Some(rich());
        render([900.0, 600.0], |ui| other.ui(ui));
        assert_eq!(loads(&other), 0);
    }

    #[test]
    fn a_workspace_switch_reloads() {
        let (mut settings, _rt) = settings();
        settings.loaded(config_state());
        settings
            .drafts
            .get_mut("Network", Scope::Device)
            .unset("net.discovery");
        let mut snapshot = rich();
        settings.track_workspace(&snapshot);
        settings.sent.clear();
        settings.track_workspace(&snapshot);
        assert_eq!(loads(&settings), 0, "the same workspace");
        let Conn::Up(server) = &mut snapshot.server else {
            unreachable!()
        };
        server.current.as_mut().unwrap().root = "/x/other".into();
        settings.track_workspace(&snapshot);
        assert_eq!(loads(&settings), 1);
        settings.loaded(config_state());
        assert_eq!(settings.drafts.dirty_sections().len(), 1, "drafts survive");
    }

    #[test]
    fn a_successful_write_reloads() {
        let (mut settings, _rt) = settings();
        settings.loaded(config_state());
        let page = Origin::Page("Network".into());
        settings.on_outcome(page.clone(), &Err("rejected".into()));
        assert_eq!(loads(&settings), 0, "a failed write does not reload");
        settings.on_outcome(
            page,
            &Ok(CommandOutput::Written(ConfigWriteResult::default())),
        );
        assert_eq!(loads(&settings), 1);
        settings.on_outcome(Origin::Restart, &Ok(CommandOutput::Done));
        assert_eq!(loads(&settings), 2, "a restart reloads too");
        settings.on_outcome(Origin::Restart, &Err("no".into()));
        assert_eq!(loads(&settings), 2);
        // A page's own reload lands in the settings.
        let mut fresh = config_state();
        fresh.schema.pop();
        settings.on_outcome(
            Origin::Page("Network".into()),
            &Ok(CommandOutput::Config(fresh.clone())),
        );
        assert_eq!(settings.config.as_ref(), Some(&fresh));
    }

    #[test]
    fn embedding_shows_status_and_its_section_page() {
        let (mut settings, _rt) = settings();
        settings.test_snapshot = Some(rich());
        // The workspace is the one the load read.
        settings.track_workspace(&rich());
        settings.loaded(config_state());
        settings.layout.open(Some(Page::Sync(Screen::Embedding)));
        let drawn = drawn_text(|ui| settings.ui(ui));
        for text in [
            "sapphire-bridge needs a restart to apply: embed.cache_dir",
            "The remote model is configured",
            "API key\nnot set\n",
            "https://api.example.com",
            "embed.cache_dir",
        ] {
            assert!(drawn.contains(text), "{text} missing from {drawn}");
        }
        assert!(!drawn.contains("Save"), "the old forms are gone: {drawn}");
        assert_eq!(
            drawn.lines().filter(|l| *l == "Embedding").count(),
            2,
            "the list entry and one heading: {drawn}"
        );
        let banner = drawn.find("needs a restart").unwrap();
        let status = drawn.find("https://api.example.com").unwrap();
        let row = drawn.rfind("embed.cache_dir").unwrap();
        assert!(banner < status && status < row, "{drawn}");
        assert_eq!(loads(&settings), 0, "already loaded");
    }

    #[test]
    fn appearance_is_listed_and_drawn() {
        for size in [[900.0, 600.0], [400.0, 700.0]] {
            let (mut settings, _rt) = settings();
            settings.layout.open(Some(Page::Appearance));
            render(size, |ui| settings.ui(ui));
            assert_eq!(settings.drawn, Some(Page::Appearance), "{size:?}");
        }
    }

    #[test]
    #[allow(deprecated)]
    fn take_theme_changed_answers_none_after_a_change() {
        let (settings, _rt) = settings();
        let mut settings = settings.with_theme(ThemeSettings {
            contrast: corundum_framework_gui::theme::Contrast::Low,
            ..Default::default()
        });
        settings.on_outcome(Origin::Appearance, &Ok(CommandOutput::Done));
        // The framework stores the theme now: nothing for the app to take.
        assert_eq!(settings.take_theme_changed(), None);
    }

    // ── following the theme ─────────────────────────────────────────────────

    /// `rich()` with the bridge reporting `gui`.
    fn with_gui(gui: Option<crate::client::GuiValues>) -> Snapshot {
        let mut snapshot = rich();
        if let Conn::Up(bridge) = &mut snapshot.bridge {
            bridge.gui = gui;
        }
        snapshot
    }

    fn contrast(value: &str) -> Option<crate::client::GuiValues> {
        Some(crate::client::GuiValues::from([(
            "gui.theme.contrast".to_owned(),
            sapphire_bridge_api::Value::String(value.into()),
        )]))
    }

    fn is_drawn_with(
        ctx: &egui::Context,
        contrast: corundum_framework_gui::theme::Contrast,
    ) -> bool {
        let dark = ctx.theme() == egui::Theme::Dark;
        *theme::current(ctx)
            == corundum_framework_gui::theme::Theme::new(
                dark,
                contrast,
                ThemeSettings::default().accent(),
            )
    }

    #[test]
    fn a_new_theme_in_the_snapshot_is_installed() {
        let (mut settings, _rt) = settings();
        let ctx = egui::Context::default();
        settings.test_snapshot = Some(with_gui(contrast("high")));
        settings.follow_theme(&ctx);
        assert!(is_drawn_with(
            &ctx,
            corundum_framework_gui::theme::Contrast::High
        ));
        settings.test_snapshot = Some(with_gui(contrast("low")));
        settings.follow_theme(&ctx);
        assert!(is_drawn_with(
            &ctx,
            corundum_framework_gui::theme::Contrast::Low
        ));
    }

    #[test]
    fn an_unchanged_snapshot_installs_nothing() {
        let (mut settings, _rt) = settings();
        let ctx = egui::Context::default();
        settings.test_snapshot = Some(with_gui(contrast("high")));
        settings.follow_theme(&ctx);
        // A drag's preview, say.
        let preview = ThemeSettings {
            contrast: corundum_framework_gui::theme::Contrast::Low,
            ..Default::default()
        };
        theme::install(&ctx, &preview);
        settings.follow_theme(&ctx);
        assert!(is_drawn_with(
            &ctx,
            corundum_framework_gui::theme::Contrast::Low
        ));
    }

    #[test]
    fn no_gui_leaves_the_theme() {
        let (mut settings, _rt) = settings();
        let ctx = egui::Context::default();
        settings.test_snapshot = Some(with_gui(None));
        settings.follow_theme(&ctx);
        assert!(!theme::is_installed(&ctx));
    }

    #[test]
    fn a_forgotten_theme_is_installed_again() {
        let (mut settings, _rt) = settings();
        let ctx = egui::Context::default();
        settings.test_snapshot = Some(with_gui(contrast("high")));
        settings.follow_theme(&ctx);
        theme::install(&ctx, &ThemeSettings::default());
        settings.forget_seen_theme();
        settings.follow_theme(&ctx);
        assert!(is_drawn_with(
            &ctx,
            corundum_framework_gui::theme::Contrast::High
        ));
    }

    #[test]
    fn the_appearance_section_is_not_listed_under_configuration() {
        let (mut settings, _rt) = settings();
        let mut state = config_state();
        state
            .schema
            .push(def("gui.theme.contrast", "Appearance", Process::Bridge));
        settings.loaded(state);
        let items = settings.list(&crate::test_support::english_ctx());
        assert!(
            !items.iter().any(
                |i| matches!(i, ListItem::Entry { page: Page::Config(s), .. } if s == "Appearance")
            ),
            "{items:?}"
        );
        assert!(
            items.iter().any(|i| matches!(
                i,
                ListItem::Entry {
                    page: Page::Appearance,
                    ..
                }
            )),
            "{items:?}"
        );
    }

    #[test]
    fn a_failed_appearance_write_forgets_the_seen_theme() {
        let (mut settings, _rt) = settings();
        let ctx = egui::Context::default();
        settings.test_snapshot = Some(with_gui(contrast("high")));
        settings.follow_theme(&ctx);
        theme::install(&ctx, &ThemeSettings::default());
        settings.on_outcome(Origin::Appearance, &Err("rejected".into()));
        settings.follow_theme(&ctx);
        assert!(is_drawn_with(
            &ctx,
            corundum_framework_gui::theme::Contrast::High
        ));
    }

    fn with_locale(gui: Option<crate::client::GuiValues>, locale: &str) -> Snapshot {
        let mut gui = gui.unwrap_or_default();
        gui.insert(
            sapphire_bridge_api::settings::gui::LOCALE.to_owned(),
            sapphire_bridge_api::Value::String(locale.into()),
        );
        with_gui(Some(gui))
    }

    fn registered_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        i18n::register(&ctx, crate::RESOURCES);
        ctx
    }

    #[test]
    fn a_new_locale_in_the_snapshot_is_installed() {
        let (mut settings, _rt) = settings();
        let ctx = registered_ctx();
        settings.test_snapshot = Some(with_locale(contrast("high"), "ja"));
        settings.follow_theme(&ctx);
        assert_eq!(i18n::current(&ctx).to_string(), "ja");
        assert!(is_drawn_with(
            &ctx,
            corundum_framework_gui::theme::Contrast::High
        ));
        settings.test_snapshot = Some(with_locale(contrast("high"), "en"));
        settings.follow_theme(&ctx);
        assert_eq!(i18n::current(&ctx).to_string(), "en");
    }

    #[test]
    fn an_unchanged_locale_installs_nothing() {
        let (mut settings, _rt) = settings();
        let ctx = registered_ctx();
        settings.test_snapshot = Some(with_locale(contrast("high"), "ja"));
        settings.follow_theme(&ctx);
        // The language installed some other way since.
        i18n::install(
            &ctx,
            &i18n::LanguageSettings {
                language: i18n::Language::Explicit("en".into()),
            },
        );
        settings.follow_theme(&ctx);
        assert_eq!(i18n::current(&ctx).to_string(), "en");
        // A change of the theme alone does not install the language either.
        settings.test_snapshot = Some(with_locale(contrast("low"), "ja"));
        settings.follow_theme(&ctx);
        assert_eq!(i18n::current(&ctx).to_string(), "en");
        assert!(is_drawn_with(
            &ctx,
            corundum_framework_gui::theme::Contrast::Low
        ));
    }

    #[test]
    fn a_snapshot_without_the_locale_leaves_the_language() {
        let (mut settings, _rt) = settings();
        let ctx = registered_ctx();
        settings.test_snapshot = Some(with_gui(contrast("high")));
        settings.follow_theme(&ctx);
        assert!(!i18n::is_installed(&ctx));
        // The theme is still followed.
        assert!(is_drawn_with(
            &ctx,
            corundum_framework_gui::theme::Contrast::High
        ));
    }

    #[test]
    fn a_failed_appearance_write_forgets_the_seen_language() {
        let (mut settings, _rt) = settings();
        let ctx = registered_ctx();
        settings.test_snapshot = Some(with_locale(None, "ja"));
        settings.follow_theme(&ctx);
        i18n::install(&ctx, &i18n::LanguageSettings::default());
        settings.on_outcome(Origin::Appearance, &Err("rejected".into()));
        settings.follow_theme(&ctx);
        assert_eq!(i18n::current(&ctx).to_string(), "ja");
    }
}
