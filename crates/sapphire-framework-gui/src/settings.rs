//! [`Settings`]: the app's own sections, the framework's sync screens, the pages the
//! settings schema makes, and About, as a list and a page, wired to a [`FrameworkClient`].

use std::collections::HashMap;
use std::path::PathBuf;

use crate::Health;
use crate::client::{
    Command, CommandError, CommandId, CommandOutput, ConfigState, FrameworkClient, Snapshot,
};
use corundum_gui::COMPACT_WIDTH;
use corundum_gui::theme::{self, ThemeSettings};

use crate::views::config::model::{Drafts, PageSpec, Placement, pages, pending_restarts};
use crate::views::config::{ConfigPage, RestartBanner};
use crate::views::{
    DeviceList, EmbeddingView, ExternalDeviceList, ServiceStatusBanner, ViewCtx, WorkgroupView,
    WorkspacePicker, error_line,
};

/// A page the app adds to [`Settings`], under its own heading.
pub trait SettingsSection {
    /// The page's name in the list.
    fn label(&self) -> String;

    /// Render the page.
    fn ui(&mut self, ui: &mut egui::Ui);
}

/// The framework's pages.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Screen {
    Workspaces,
    Devices,
    Workgroup,
    ExternalDevices,
    Embedding,
}

const SYNC_SCREENS: [(Screen, &str); 5] = [
    (Screen::Workspaces, "Workspace"),
    (Screen::Devices, "Devices"),
    (Screen::Workgroup, "Workgroup"),
    (Screen::ExternalDevices, "External devices"),
    (Screen::Embedding, "Embedding"),
];

/// The screen a settings section of the same name joins, below its own content.
const JOINED_SCREEN: (Screen, &str) = (Screen::Embedding, "Embedding");

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
}

/// One line of the list.
#[derive(Clone, PartialEq, Eq, Debug)]
enum ListItem {
    Heading(String),
    Entry(Page, String),
    Gap,
}

/// The settings: the app's sections, then General (Appearance), then Sync (workspace,
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
    /// `None` shows the list alone when compact; when wide, the first page opens.
    page: Option<Page>,
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
    /// What the Appearance page edits.
    theme: ThemeSettings,
    /// Whether the user changed `theme` since the app last took it.
    theme_changed: bool,
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
            page: None,
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
            theme: ThemeSettings::default(),
            theme_changed: false,
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

    /// Start the Appearance page from the theme settings the app saved. This only records
    /// them: the app installs them ([`theme::install`]), or the shell installs the
    /// defaults.
    pub fn with_theme(mut self, theme: ThemeSettings) -> Settings {
        self.theme = theme;
        self
    }

    /// The theme settings, once, after the user changed them on the Appearance page (which
    /// has installed them already): the app saves these.
    pub fn take_theme_changed(&mut self) -> Option<ThemeSettings> {
        std::mem::take(&mut self.theme_changed).then(|| self.theme.clone())
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
        self.page == Some(Page::About)
    }

    /// Whether the last [`ui`](Self::ui) drew About.
    #[cfg(test)]
    pub(crate) fn about_drawn(&self) -> bool {
        self.drawn == Some(Page::About)
    }

    /// Open About, where the services' state is.
    pub fn open_about(&mut self) {
        self.page = Some(Page::About);
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
        self.poll();
        #[cfg(test)]
        {
            self.drawn = self.page.clone();
        }
        if self.own_banner && self.health() == Health::Problem {
            egui::Panel::top("settings_status").show(ui, |ui| self.status_ui(ui));
        }
        if ui.available_width() < COMPACT_WIDTH {
            egui::CentralPanel::default().show(ui, |ui| match self.page.clone() {
                None => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.list_ui(ui));
                }
                Some(page) => {
                    let back = crate::icons::image(ui, crate::icons::arrow_left(), 14.0);
                    if ui
                        .add(egui::Button::image_and_text(back, "Settings"))
                        .clicked()
                    {
                        self.page = None;
                    }
                    ui.separator();
                    self.page_ui(ui, page);
                }
            });
        } else {
            let first = self.first_page();
            let page = self.page.get_or_insert(first).clone();
            egui::Panel::left("settings_nav")
                .resizable(false)
                .exact_size(170.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.list_ui(ui));
                });
            egui::CentralPanel::default().show(ui, |ui| self.page_ui(ui, page));
        }
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

    /// The list: headings, entries and gaps, in order. A page with a draft is marked.
    fn list(&self) -> Vec<ListItem> {
        let dirty = self.drafts.dirty_sections();
        let mark = |label: &str, dirty: bool| {
            if dirty {
                format!("{label} •")
            } else {
                label.to_owned()
            }
        };
        let config = |placement: Placement| -> Vec<ListItem> {
            self.pages
                .iter()
                .filter(|p| p.placement == placement)
                .map(|p| {
                    let label = mark(&p.section, dirty.contains(&p.section));
                    ListItem::Entry(Page::Config(p.section.clone()), label)
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
                out.push(ListItem::Entry(Page::App(i), section.label()));
            }
        }
        out.push(ListItem::Heading("General".into()));
        out.push(ListItem::Entry(Page::Appearance, "Appearance".into()));
        out.push(ListItem::Heading("Sync".into()));
        for (screen, label) in SYNC_SCREENS {
            let joined = self.pages.iter().any(|p| {
                p.placement == Placement::Screen(label.to_owned()) && dirty.contains(&p.section)
            });
            out.push(ListItem::Entry(Page::Sync(screen), mark(label, joined)));
        }
        let rest = config(Placement::Configuration);
        if !rest.is_empty() {
            out.push(ListItem::Heading("Configuration".into()));
            out.extend(rest);
        }
        out.push(ListItem::Gap);
        out.push(ListItem::Entry(Page::About, "About".into()));
        out
    }

    fn list_ui(&mut self, ui: &mut egui::Ui) {
        let items = self.list();
        // Full-width rows, their labels at the left.
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            for item in items {
                match item {
                    ListItem::Heading(text) => {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(text).small().weak());
                    }
                    ListItem::Entry(page, label) => {
                        let open = self.page.as_ref() == Some(&page);
                        if ui.selectable_label(open, label).clicked() {
                            self.page = Some(page);
                        }
                    }
                    ListItem::Gap => {
                        ui.add_space(12.0);
                    }
                }
            }
        });
    }

    fn page_ui(&mut self, ui: &mut egui::Ui, page: Page) {
        match page {
            Page::App(i) => match self.app_sections.get_mut(i) {
                Some(section) => section.ui(ui),
                None => self.page = None,
            },
            Page::Appearance => {
                if theme::settings_ui(ui, &mut self.theme) {
                    self.theme_edited();
                }
            }
            Page::Sync(screen) => self.screen_ui(ui, screen),
            Page::Config(section) => {
                let snapshot = self.snapshot();
                self.ensure_loaded(&snapshot);
                egui::ScrollArea::vertical()
                    .id_salt("settings_page")
                    .show(ui, |ui| {
                        self.restart_ui(ui, &snapshot);
                        if self.config.is_none() {
                            self.unloaded_ui(ui, &snapshot);
                        } else if !self.config_page_ui(ui, &snapshot, &section) {
                            // Gone from the schema.
                            self.page = None;
                        }
                    });
            }
            Page::About => self.about_ui(ui),
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
                egui::ScrollArea::vertical()
                    .id_salt("settings_page")
                    .show(ui, |ui| {
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
                    });
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
        error_line(ui, &mut self.load_error);
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
        error_line(ui, &mut self.load_error);
        if ui.button("Load settings").clicked() {
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
        ui.label(format!("Version {}", app.version));
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Services").strong());
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
                Err(e) => self.load_error = Some(e.message.clone()),
            },
            Origin::Restart => {
                self.restart.on_outcome(result);
                if result.is_ok() {
                    self.load();
                }
            }
        }
    }

    /// The user changed the theme on the Appearance page.
    fn theme_edited(&mut self) {
        self.theme_changed = true;
    }

    /// Keep a load's settings, and the pages their schema makes. Drafts are untouched.
    fn loaded(&mut self, state: ConfigState) {
        let app_name = self.client.app().app_name;
        self.pages = pages(&state.schema, app_name, &[JOINED_SCREEN.1]);
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
    use crate::test_support::{client, render};
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
        fn label(&self) -> String {
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
    fn wide_it_opens_the_first_app_section_beside_the_list() {
        let (mut settings, _rt) = settings();
        render([900.0, 600.0], |ui| settings.ui(ui));
        assert_eq!(settings.page, Some(Page::App(0)));
    }

    #[test]
    fn compact_it_shows_the_list_then_one_page() {
        let (mut settings, _rt) = settings();
        render([400.0, 700.0], |ui| settings.ui(ui));
        assert_eq!(settings.page, None, "the list alone, no page chosen");
        settings.open_about();
        render([400.0, 700.0], |ui| settings.ui(ui));
        assert_eq!(settings.page, Some(Page::About));
    }

    #[test]
    fn without_app_sections_it_opens_on_the_workspace() {
        let (client, _rt) = client();
        let mut settings = Settings::new(client);
        render([900.0, 600.0], |ui| settings.ui(ui));
        assert_eq!(settings.page, Some(Page::Sync(Screen::Workspaces)));
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
            .list()
            .into_iter()
            .map(|item| match item {
                ListItem::Heading(h) => format!("# {h}"),
                ListItem::Entry(_, label) => label,
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
                .list()
                .contains(&ListItem::Entry(Page::Config("Look".into()), "Look".into()))
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
        for page in [
            Page::Config("Network".into()),
            Page::Sync(Screen::Embedding),
        ] {
            let (mut settings, _rt) = settings();
            settings.test_snapshot = Some(rich());
            settings.page = Some(page.clone());
            render([900.0, 600.0], |ui| settings.ui(ui));
            render([900.0, 600.0], |ui| settings.ui(ui));
            assert_eq!(loads(&settings), 1, "{page:?}");
            // Loaded: nothing more to fetch.
            settings.loaded(config_state());
            render([900.0, 600.0], |ui| settings.ui(ui));
            assert_eq!(loads(&settings), 1, "{page:?}");
        }
        // Another page needs nothing.
        let (mut settings, _rt) = settings();
        settings.test_snapshot = Some(rich());
        render([900.0, 600.0], |ui| settings.ui(ui));
        assert_eq!(loads(&settings), 0);
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
        settings.page = Some(Page::Sync(Screen::Embedding));
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
            settings.page = Some(Page::Appearance);
            render(size, |ui| settings.ui(ui));
            assert_eq!(settings.drawn, Some(Page::Appearance), "{size:?}");
        }
    }

    #[test]
    fn a_theme_change_is_taken_once() {
        let (mut settings, _rt) = settings();
        settings.theme.contrast = corundum_gui::theme::Contrast::High;
        settings.theme_edited();
        let taken = settings.take_theme_changed().expect("a change");
        assert_eq!(taken.contrast, corundum_gui::theme::Contrast::High);
        assert_eq!(settings.take_theme_changed(), None);
    }

    #[test]
    fn with_theme_is_what_the_page_edits() {
        let (settings, _rt) = settings();
        let saved = ThemeSettings {
            contrast: corundum_gui::theme::Contrast::Low,
            ..Default::default()
        };
        let mut settings = settings.with_theme(saved.clone());
        assert_eq!(settings.theme, saved);
        assert_eq!(settings.take_theme_changed(), None);
    }
}
