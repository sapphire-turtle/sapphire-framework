//! [`Settings`]: the app's own sections, the framework's sync screens and About, as a
//! list and a page, wired to a [`FrameworkClient`].

use std::collections::HashMap;

use crate::Health;
use crate::client::{CommandId, FrameworkClient};
use corundum_gui::COMPACT_WIDTH;
use corundum_gui::theme::{self, ThemeSettings};
use corundum_gui::tr;

use crate::views::{
    DeviceList, EmbeddingView, ExternalDeviceList, ServiceStatusBanner, ViewCtx, WorkgroupView,
    WorkspacePicker,
};

/// A page the app adds to [`Settings`], under its own heading.
pub trait SettingsSection {
    /// The page's name in the list, in the language installed in `ctx`. It is asked every
    /// frame, so a language switch reaches the list at once.
    fn label(&self, ctx: &egui::Context) -> String;

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

/// Which page is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    App(usize),
    Appearance,
    Sync(Screen),
    About,
}

/// Which view sent a command, so its outcome goes back there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Origin {
    Banner,
    Screen(Screen),
}

/// The settings: the app's sections, then General (Appearance), then Sync (workspace, devices, workgroup, external
/// devices, embedding), then About (the services and the version). Hold one; call
/// [`ui`](Self::ui) every frame it is shown.
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
    pending: HashMap<CommandId, Origin>,
    /// What the Appearance page edits.
    theme: ThemeSettings,
    /// Whether the user changed `theme` since the app last took it.
    theme_changed: bool,
    /// The page the last [`ui`](Self::ui) drew, for the tests.
    #[cfg(test)]
    drawn: Option<Page>,
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
            pending: HashMap::new(),
            theme: ThemeSettings::default(),
            theme_changed: false,
            #[cfg(test)]
            drawn: None,
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

    /// Hand each finished command's outcome to the view that sent it. [`ui`](Self::ui)
    /// does this; a host that shows Settings only sometimes calls it every frame, so the
    /// banner hears how an install went while another screen is open.
    pub fn poll(&mut self) {
        for outcome in self.client.drain_outcomes() {
            match self.pending.remove(&outcome.id) {
                Some(Origin::Banner) => self.banner.on_outcome(&outcome.result),
                Some(Origin::Screen(Screen::Workspaces)) => {
                    self.workspaces.on_outcome(&outcome.result)
                }
                Some(Origin::Screen(Screen::Devices)) => self.devices.on_outcome(&outcome.result),
                Some(Origin::Screen(Screen::Workgroup)) => {
                    self.workgroup.on_outcome(&outcome.result)
                }
                Some(Origin::Screen(Screen::ExternalDevices)) => {
                    self.external.on_outcome(&outcome.result)
                }
                Some(Origin::Screen(Screen::Embedding)) => {
                    self.embedding.on_outcome(&outcome.result)
                }
                None => {}
            }
        }
    }

    /// Render the status banner: the services and their install buttons, a newer release,
    /// an install's failure.
    pub fn status_ui(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.client.snapshot().clone();
        let cx = ViewCtx {
            snapshot: &snapshot,
            app_name: self.client.app().app_name,
            busy: self.busy(Origin::Banner),
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
        #[cfg(test)]
        {
            self.drawn = self.page;
        }
        if self.own_banner && self.health() == Health::Problem {
            egui::Panel::top("settings_status").show(ui, |ui| self.status_ui(ui));
        }
        if ui.available_width() < COMPACT_WIDTH {
            egui::CentralPanel::default().show(ui, |ui| match self.page {
                None => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.list_ui(ui));
                }
                Some(page) => {
                    let back = crate::icons::image(ui, crate::icons::arrow_left(), 14.0);
                    if ui
                        .add(egui::Button::image_and_text(
                            back,
                            tr!(ui, "sapphire-settings"),
                        ))
                        .clicked()
                    {
                        self.page = None;
                    }
                    ui.separator();
                    self.page_ui(ui, page);
                }
            });
        } else {
            let page = *self.page.get_or_insert(self.first_page());
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

    fn list_ui(&mut self, ui: &mut egui::Ui) {
        // Full-width rows, their labels at the left.
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            let ctx = ui.ctx().clone();
            let mut entry = |ui: &mut egui::Ui, page: Page, label: &str| {
                let open = self.page == Some(page);
                if ui.selectable_label(open, label).clicked() {
                    self.page = Some(page);
                }
            };
            let heading = |ui: &mut egui::Ui, text: &str| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(text).small().weak());
            };
            if !self.app_sections.is_empty() {
                heading(ui, &self.app_heading);
                let labels: Vec<String> = self.app_sections.iter().map(|s| s.label(&ctx)).collect();
                for (i, label) in labels.iter().enumerate() {
                    entry(ui, Page::App(i), label);
                }
            }
            heading(ui, &tr!(&ctx, "sapphire-settings-general"));
            entry(
                ui,
                Page::Appearance,
                &tr!(&ctx, "sapphire-settings-appearance"),
            );
            heading(ui, &tr!(&ctx, "sapphire-settings-sync"));
            for (screen, id) in SYNC_SCREENS {
                entry(ui, Page::Sync(screen), &tr!(&ctx, id));
            }
            ui.add_space(12.0);
            entry(ui, Page::About, &tr!(&ctx, "sapphire-settings-about"));
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
            Page::About => self.about_ui(ui),
        }
    }

    fn screen_ui(&mut self, ui: &mut egui::Ui, screen: Screen) {
        let snapshot = self.client.snapshot().clone();
        let origin = Origin::Screen(screen);
        let cx = ViewCtx {
            snapshot: &snapshot,
            app_name: self.client.app().app_name,
            busy: self.busy(origin),
        };
        let command = match screen {
            Screen::Workspaces => self.workspaces.ui(ui, &cx),
            Screen::Devices => self.devices.ui(ui, &cx),
            Screen::Workgroup => self.workgroup.ui(ui, &cx),
            Screen::ExternalDevices => self.external.ui(ui, &cx),
            Screen::Embedding => self.embedding.ui(ui, &cx),
        };
        if let Some(command) = command {
            self.send(command, origin);
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

    /// The user changed the theme on the Appearance page.
    fn theme_edited(&mut self) {
        self.theme_changed = true;
    }

    fn busy(&self, origin: Origin) -> bool {
        self.pending.values().any(|p| *p == origin)
    }

    fn send(&mut self, command: crate::client::Command, origin: Origin) {
        let id = self.client.send(command);
        self.pending.insert(id, origin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{BridgeState, Conn, ServerState, Snapshot};
    use crate::test_support::{client, render, render_in};
    use sapphire_backend::protocol::{CurrentWorkspace, StatusReport, SyncStatusResult};
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
            assert!(embedding.ui(ui, &cx).is_none());
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
