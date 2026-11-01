//! The Status page: the bridge's version and state, this device, the workgroup and its
//! devices, the synced workspaces, and Restart.

use crate::client::Command;
use crate::tr;
use crate::views::ViewCtx;
use sapphire_bridge_api::settings::Process;

/// The Status page.
#[derive(Default)]
pub struct StatusView {}

impl StatusView {
    /// Draw the page; Restart sends [`Command::Restart`] for the bridge.
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &ViewCtx<'_>) -> Option<Command> {
        ui.heading(tr!(ui, "sapphire-bridge-page-status"));
        let Some(bridge) = cx.snapshot.bridge.up() else {
            ui.label(tr!(ui, "sapphire-bridge-status-not-running"));
            return None;
        };
        let status = &bridge.status;
        egui::Grid::new("sapphire_bridge_status")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label(tr!(ui, "sapphire-bridge-status-version"));
                ui.label(status.version.as_str());
                ui.end_row();
                ui.label(tr!(ui, "sapphire-bridge-status-workgroup"));
                match &status.workgroup {
                    Some(w) => ui.label(w.name.as_str()),
                    None => ui.label(tr!(ui, "sapphire-bridge-summary-no-workgroup")),
                };
                ui.end_row();
            });
        if !bridge.peers.is_empty() {
            ui.add_space(8.0);
            ui.strong(tr!(ui, "sapphire-settings-devices"));
            for peer in &bridge.peers {
                let mark = if peer.connected { "●" } else { "○" };
                ui.label(format!("{mark} {}", peer.name));
            }
        }
        if !bridge.ledger.is_empty() {
            ui.add_space(8.0);
            ui.strong(tr!(ui, "sapphire-bridge-status-workspaces"));
            for workspace in &bridge.ledger {
                ui.label(format!("{} ({})", workspace.name, workspace.app_name));
            }
        }
        ui.add_space(8.0);
        let restart = ui.add_enabled(
            !cx.busy,
            egui::Button::new(tr!(ui, "sapphire-bridge-status-restart")),
        );
        restart
            .clicked()
            .then_some(Command::Restart(Process::Bridge))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Snapshot;
    use crate::test_support::render;

    #[test]
    fn status_renders_without_a_bridge() {
        let snapshot = Snapshot::default();
        let mut view = StatusView::default();
        render([600.0, 400.0], |ui| {
            let cx = ViewCtx {
                snapshot: &snapshot,
                app_name: "sapphire-bridge-desktop",
                busy: false,
            };
            assert!(view.ui(ui, &cx).is_none());
        });
    }
}
