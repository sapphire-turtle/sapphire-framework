use crate::theme::Swatch;
use sapphire_bridge_api::{EmbedSettingsResult, LOCAL_MODEL, ModelSource, Slot};

use super::model::{SWITCH_CHOICES, auto_resolution, embedding_status};
use super::{ViewCtx, bridge_unavailable, register};
use corundum_gui::tr;

/// The embedding status: the workgroup's local and remote model, this device's switch for
/// each, and whether its API key is set. Read-only; the Embedding section's settings page,
/// shown below it, edits them.
#[derive(Default)]
pub struct EmbeddingView {}

impl EmbeddingView {
    /// Render.
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &ViewCtx) {
        register(ui);
        ui.horizontal(|ui| {
            ui.heading(tr!(ui, "sapphire-embedding-title"));
            if cx.busy {
                ui.spinner();
            }
        });
        ui.separator();
        let Some(bridge) = cx.snapshot.bridge.up() else {
            bridge_unavailable(ui);
            return;
        };
        let Some(s) = &bridge.embedding else {
            ui.label(tr!(ui, "sapphire-embedding-too-old"));
            return;
        };

        let (line, warn) = embedding_status(s);
        let line = line.text(ui.ctx());
        if warn {
            ui.colored_label(crate::theme::current(ui.ctx()).text(Swatch::WARNING), line);
        } else {
            ui.label(line);
        }
        let whose = match s.source {
            Some(ModelSource::Device) => "device",
            _ if bridge.status.workgroup.is_some() => "workgroup",
            _ => "device",
        };
        if s.shadowed_device_models {
            ui.small(tr!(ui, "sapphire-embedding-shadowed"));
        }
        ui.add_space(8.0);

        ui.label(
            egui::RichText::new(tr!(ui, "sapphire-embedding-remote-heading", whose = whose))
                .strong(),
        );
        egui::Grid::new("embed_remote")
            .num_columns(2)
            .show(ui, |ui| {
                match &s.models.remote {
                    Some(m) => {
                        row(ui, "sapphire-embedding-endpoint", &m.endpoint);
                        row(ui, "sapphire-embedding-model", &m.model);
                        row(ui, "sapphire-embedding-dimension", &m.dimension.to_string());
                    }
                    None => row(
                        ui,
                        "sapphire-embedding-model",
                        &tr!(ui, "sapphire-embedding-not-configured"),
                    ),
                }
                switch_row(ui, s, Slot::Remote);
                let key = if s.key_set {
                    tr!(ui, "sapphire-embedding-key-is-set")
                } else {
                    tr!(ui, "sapphire-embedding-key-not-set")
                };
                row(ui, "sapphire-embedding-api-key", &key);
            });
        ui.small(tr!(ui, "sapphire-embedding-key-note"));
        ui.add_space(8.0);

        ui.label(
            egui::RichText::new(tr!(ui, "sapphire-embedding-local-heading", whose = whose))
                .strong(),
        );
        egui::Grid::new("embed_local")
            .num_columns(2)
            .show(ui, |ui| {
                match &s.models.local {
                    Some(m) => {
                        row(ui, "sapphire-embedding-model", LOCAL_MODEL);
                        row(ui, "sapphire-embedding-dimension", &m.dimension.to_string());
                        row(
                            ui,
                            "sapphire-embedding-max-tokens",
                            &m.max_tokens.to_string(),
                        );
                    }
                    None => row(
                        ui,
                        "sapphire-embedding-model",
                        &tr!(ui, "sapphire-embedding-not-configured"),
                    ),
                }
                switch_row(ui, s, Slot::Local);
            });
        ui.small(tr!(ui, "sapphire-embedding-local-hint"));
    }
}

/// One label (the id of its message) and value of the status grid.
fn row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(crate::i18n::tr(ui.ctx(), label));
    ui.monospace(value);
    ui.end_row();
}

/// This device's Auto / On / Off for one slot, and what Auto resolves to.
fn switch_row(ui: &mut egui::Ui, s: &EmbedSettingsResult, slot: Slot) {
    let current = s.device.enabled(slot);
    let label = SWITCH_CHOICES
        .iter()
        .find(|(_, value)| *value == current)
        .map_or("sapphire-switch-auto", |(label, _)| label);
    ui.label(tr!(ui, "sapphire-embedding-on-this-device"));
    if current.is_none() {
        ui.monospace(crate::i18n::tr(ui.ctx(), auto_resolution(s, slot)));
    } else {
        ui.monospace(crate::i18n::tr(ui.ctx(), label));
    }
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{BridgeState, Conn, Snapshot};
    use crate::views::config::widgets::tests::drawn_text;
    use sapphire_bridge_api::{LocalModel, ModelSettings, RemoteModel, StatusResult};

    #[test]
    fn it_shows_the_models_switches_and_key_read_only() {
        let snapshot = Snapshot {
            fetched: true,
            bridge: Conn::Up(BridgeState {
                gui: None,
                status: StatusResult {
                    version: "1".into(),
                    node_id: "aaaa".into(),
                    workgroup: None,
                    routes: vec![],
                    embedding: None,
                },
                peers: vec![],
                peer_roles: vec![],
                ledger: vec![],
                external_devices: None,
                embedding: Some(EmbedSettingsResult {
                    models: ModelSettings {
                        local: Some(LocalModel::default()),
                        remote: Some(RemoteModel {
                            endpoint: "https://api.example.com".into(),
                            model: "remote-m".into(),
                            dimension: 8,
                        }),
                    },
                    key_set: true,
                    avx2: true,
                    ..EmbedSettingsResult::default()
                }),
            }),
            server: Conn::Absent,
        };
        let cx = ViewCtx {
            snapshot: &snapshot,
            app_name: "app",
            busy: false,
        };
        let mut view = EmbeddingView::default();
        let drawn = drawn_text(|ui| view.ui(ui, &cx));
        for text in [
            "Remote model (this device)",
            "https://api.example.com",
            "remote-m",
            LOCAL_MODEL,
            "API key\nset\n",
            "Auto: on",
        ] {
            assert!(drawn.contains(text), "{text} missing from {drawn}");
        }
        for gone in ["Save", "Clear"] {
            assert!(!drawn.contains(gone), "{gone} is still drawn: {drawn}");
        }
    }
}
