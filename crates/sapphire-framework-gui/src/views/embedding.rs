use crate::theme::Swatch;
use sapphire_bridge_api::{EmbedSettingsResult, LOCAL_MODEL, ModelSource, Slot};

use super::model::{SWITCH_CHOICES, auto_resolution, embedding_status};
use super::{ViewCtx, unavailable};

/// The embedding status: the workgroup's local and remote model, this device's switch for
/// each, and whether its API key is set. Read-only; the Embedding section's settings page,
/// shown below it, edits them.
#[derive(Default)]
pub struct EmbeddingView {}

impl EmbeddingView {
    /// Render.
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &ViewCtx) {
        ui.horizontal(|ui| {
            ui.heading("Embedding");
            if cx.busy {
                ui.spinner();
            }
        });
        ui.separator();
        let Some(bridge) = cx.snapshot.bridge.up() else {
            unavailable(ui, "The bridge");
            return;
        };
        let Some(s) = &bridge.embedding else {
            ui.label("This bridge is too old to report embedding; update sapphire-bridge.");
            return;
        };

        let (line, warn) = embedding_status(s);
        if warn {
            ui.colored_label(crate::theme::current(ui.ctx()).text(Swatch::WARNING), line);
        } else {
            ui.label(line);
        }
        let whose = match s.source {
            Some(ModelSource::Device) => "this device",
            _ if bridge.status.workgroup.is_some() => "the workgroup",
            _ => "this device",
        };
        if s.shadowed_device_models {
            ui.small("This device's own model settings are ignored: the workgroup's apply.");
        }
        ui.add_space(8.0);

        ui.label(egui::RichText::new(format!("Remote model ({whose})")).strong());
        egui::Grid::new("embed_remote")
            .num_columns(2)
            .show(ui, |ui| {
                match &s.models.remote {
                    Some(m) => {
                        row(ui, "Endpoint", &m.endpoint);
                        row(ui, "Model", &m.model);
                        row(ui, "Dimension", &m.dimension.to_string());
                    }
                    None => row(ui, "Model", "not configured"),
                }
                switch_row(ui, s, Slot::Remote);
                row(ui, "API key", if s.key_set { "set" } else { "not set" });
            });
        ui.small("The key stays on this device; it is never synced.");
        ui.add_space(8.0);

        ui.label(egui::RichText::new(format!("Local model ({whose})")).strong());
        egui::Grid::new("embed_local")
            .num_columns(2)
            .show(ui, |ui| {
                match &s.models.local {
                    Some(m) => {
                        row(ui, "Model", LOCAL_MODEL);
                        row(ui, "Dimension", &m.dimension.to_string());
                        row(ui, "Max tokens", &m.max_tokens.to_string());
                    }
                    None => row(ui, "Model", "not configured"),
                }
                switch_row(ui, s, Slot::Local);
            });
        ui.small("Computed on each device's CPU; needs AVX2 to be usable.");
    }
}

/// One label and value of the status grid.
fn row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(label);
    ui.monospace(value);
    ui.end_row();
}

/// This device's Auto / On / Off for one slot, and what Auto resolves to.
fn switch_row(ui: &mut egui::Ui, s: &EmbedSettingsResult, slot: Slot) {
    let current = s.device.enabled(slot);
    let label = SWITCH_CHOICES
        .iter()
        .find(|(_, value)| *value == current)
        .map_or("Auto", |(label, _)| label);
    ui.label("On this device");
    if current.is_none() {
        ui.monospace(auto_resolution(s, slot));
    } else {
        ui.monospace(label);
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
