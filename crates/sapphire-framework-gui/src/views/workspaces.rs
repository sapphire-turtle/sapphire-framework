use egui::{Align, Layout};

use crate::theme::{Swatch, Tone};

use crate::client::{Command, CommandOutput};

use super::model::{
    Badge, badge, bring_target, display_name, display_path, move_target, new_target_ok, others,
};
use super::{ViewCtx, backend_error, error_line, register, type_to_confirm, unavailable};
use crate::i18n::Message;
use crate::platform;
use corundum_gui::tr;

/// The remove confirmation: whether to delete the folder too, the name to type when it
/// does, and the last failure.
#[derive(Default)]
struct RemoveConfirm {
    name: String,
    delete: bool,
    typed: String,
    error: Option<Message>,
    /// A remove was sent from this dialog and has not come back yet.
    sent: bool,
}

/// The workspace screen: the one workspace the server serves (#215), the ways to switch
/// it — open another folder, create one, or bring one from the workgroup — and to move or
/// remove it.
#[derive(Default)]
pub struct WorkspacePicker {
    confirm: Option<RemoveConfirm>,
    error: Option<Message>,
    /// The name typed for a new workspace, where there is no folder dialog.
    new_name: String,
}

impl WorkspacePicker {
    /// Render.
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &ViewCtx) -> Option<Command> {
        register(ui);
        let mut out = None;
        ui.horizontal(|ui| {
            ui.heading(tr!(ui, "sapphire-workspaces-title"));
            if cx.busy {
                ui.spinner();
            }
        });
        ui.separator();
        error_line(ui, &mut self.error);
        let Some(server) = cx.snapshot.server.up() else {
            unavailable(
                ui,
                Message::new("sapphire-unavailable-server").arg("app", cx.app_name),
            );
            return out;
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            match &server.current {
                None => {
                    ui.label(tr!(ui, "sapphire-workspaces-none"));
                }
                Some(current) => {
                    let b = badge(current);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.strong(display_name(current));
                                    let theme = crate::theme::current(ui.ctx());
                                    let colour = match b {
                                        Badge::Syncing { .. } => theme.text(Swatch::SUCCESS),
                                        Badge::NotSynced => {
                                            theme.color(Tone::Muted, Swatch::Neutral)
                                        }
                                        _ => theme.text(Swatch::WARNING),
                                    };
                                    ui.colored_label(colour, b.label().text(ui.ctx()));
                                });
                                ui.small(display_path(&current.root).display().to_string());
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add_enabled(
                                        !cx.busy,
                                        egui::Button::new(tr!(ui, "sapphire-workspaces-remove")),
                                    )
                                    .clicked()
                                {
                                    self.confirm = Some(RemoveConfirm {
                                        name: display_name(current),
                                        ..RemoveConfirm::default()
                                    });
                                }
                                if platform::folder_picker_available()
                                    && ui
                                        .add_enabled(
                                            current.reachable && !cx.busy,
                                            egui::Button::new(tr!(ui, "sapphire-workspaces-move")),
                                        )
                                        .clicked()
                                    && let Some(dir) = platform::pick_folder(&tr!(
                                        ui,
                                        "sapphire-workspaces-move-title",
                                        name = display_name(current)
                                    ))
                                {
                                    match move_target(&dir, &current.root) {
                                        Ok(dir) => out = Some(Command::WorkspaceMove { dir }),
                                        Err(e) => self.error = Some(e),
                                    }
                                }
                                if platform::file_manager_available()
                                    && ui
                                        .add_enabled(
                                            current.reachable,
                                            egui::Button::new(tr!(
                                                ui,
                                                "sapphire-workspaces-show-folder"
                                            )),
                                        )
                                        .clicked()
                                {
                                    platform::open_in_file_manager(&display_path(&current.root));
                                }
                                let (label, cmd) = if current.sync.enabled {
                                    (
                                        tr!(ui, "sapphire-workspaces-stop-sync"),
                                        Command::SyncDisable,
                                    )
                                } else {
                                    (
                                        tr!(ui, "sapphire-workspaces-start-sync"),
                                        Command::SyncEnable,
                                    )
                                };
                                if ui
                                    .add_enabled(
                                        current.reachable && !cx.busy,
                                        egui::Button::new(label),
                                    )
                                    .clicked()
                                {
                                    out = Some(cmd);
                                }
                            });
                        });
                    });
                }
            }

            ui.add_space(12.0);
            ui.strong(tr!(ui, "sapphire-workspaces-switch"));
            ui.small(tr!(ui, "sapphire-workspaces-switch-hint"));
            if let Some(storage) =
                platform::app_storage().filter(|_| !platform::folder_picker_available())
            {
                if let Some(c) = self.new_in_storage_ui(ui, cx, storage) {
                    out = Some(c);
                }
            } else {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !cx.busy,
                            egui::Button::new(tr!(ui, "sapphire-workspaces-open-folder")),
                        )
                        .clicked()
                        && let Some(dir) =
                            platform::pick_folder(&tr!(ui, "sapphire-workspaces-open-title"))
                    {
                        out = Some(if dir.join(format!(".{}", cx.app_name)).is_dir() {
                            Command::WorkspaceSelect { dir }
                        } else {
                            // A folder that is not a workspace yet becomes one.
                            Command::WorkspaceInit { dir, sync: false }
                        });
                    }
                    if ui
                        .add_enabled(
                            !cx.busy,
                            egui::Button::new(tr!(ui, "sapphire-workspaces-new")),
                        )
                        .clicked()
                        && let Some(dir) =
                            platform::pick_folder(&tr!(ui, "sapphire-workspaces-new-title"))
                    {
                        match new_target_ok(&dir, cx.app_name) {
                            Ok(()) => out = Some(Command::WorkspaceInit { dir, sync: true }),
                            Err(e) => self.error = Some(e),
                        }
                    }
                });
            }

            ui.add_space(12.0);
            ui.strong(tr!(ui, "sapphire-workspaces-from-workgroup"));
            match cx.snapshot.bridge.up() {
                None => {
                    ui.small(tr!(ui, "sapphire-unavailable-bridge"));
                }
                Some(bridge) => {
                    let elsewhere = others(&bridge.ledger, server.current.as_ref(), cx.app_name);
                    if elsewhere.is_empty() {
                        ui.small(tr!(ui, "sapphire-workspaces-no-others"));
                    }
                    for w in elsewhere {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(&w.name);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .add_enabled(
                                            !cx.busy,
                                            egui::Button::new(tr!(ui, "sapphire-workspaces-bring")),
                                        )
                                        .clicked()
                                        && let Some(dir) = bring_into(ui, &w.name)
                                    {
                                        out = Some(Command::WorkspaceMap {
                                            workspace_id: w.workspace_id,
                                            dir: bring_target(&dir, &w.name, cx.app_name),
                                        });
                                    }
                                });
                            });
                        });
                    }
                }
            }
        });
        if let Some(c) = self.confirm_ui(ui, cx) {
            out = Some(c);
        }
        out
    }

    /// "New workspace" without a folder dialog: a name, and a folder of that name in the
    /// app's storage.
    fn new_in_storage_ui(
        &mut self,
        ui: &mut egui::Ui,
        cx: &ViewCtx,
        storage: &std::path::Path,
    ) -> Option<Command> {
        let mut out = None;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_name)
                    .hint_text(tr!(ui, "sapphire-workspaces-new-name"))
                    .desired_width(180.0),
            );
            let name = self.new_name.trim();
            let valid = platform::valid_workspace_name(name);
            if ui
                .add_enabled(
                    valid && !cx.busy,
                    egui::Button::new(tr!(ui, "sapphire-action-create")),
                )
                .clicked()
            {
                let dir = storage.join(name);
                if dir.exists() && !dir.join(format!(".{}", cx.app_name)).is_dir() {
                    self.error = Some(Message::new("sapphire-workspaces-exists").arg("name", name));
                } else {
                    out = Some(if dir.exists() {
                        Command::WorkspaceSelect { dir }
                    } else {
                        Command::WorkspaceInit { dir, sync: true }
                    });
                    self.new_name.clear();
                }
            }
        });
        out
    }

    fn confirm_ui(&mut self, ui: &mut egui::Ui, cx: &ViewCtx) -> Option<Command> {
        let confirm = self.confirm.as_mut()?;
        let mut out = None;
        let mut open = true;
        let mut cancel = false;
        egui::Window::new(tr!(ui, "sapphire-workspaces-remove-title"))
            .id(egui::Id::new("workspace_remove"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(tr!(
                    ui,
                    "sapphire-workspaces-remove-body",
                    name = confirm.name.as_str()
                ));
                ui.checkbox(
                    &mut confirm.delete,
                    tr!(ui, "sapphire-workspaces-delete-too"),
                );
                if confirm.delete {
                    type_to_confirm(ui, &confirm.name);
                    ui.text_edit_singleline(&mut confirm.typed);
                }
                error_line(ui, &mut confirm.error);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ready = !confirm.delete || confirm.typed.trim() == confirm.name;
                    let label = if confirm.delete {
                        tr!(ui, "sapphire-workspaces-remove-delete")
                    } else {
                        tr!(ui, "sapphire-action-remove")
                    };
                    if ui
                        .add_enabled(ready && !cx.busy, egui::Button::new(label))
                        .clicked()
                    {
                        confirm.error = None;
                        confirm.sent = true;
                        out = Some(Command::WorkspaceRemove {
                            delete: confirm.delete,
                        });
                    }
                    if ui.button(tr!(ui, "sapphire-action-cancel")).clicked() {
                        cancel = true;
                    }
                    if cx.busy {
                        ui.spinner();
                    }
                });
            });
        if cancel || !open {
            self.confirm = None;
        }
        out
    }

    /// Route the result: a remove's back to its dialog, which closes on success; any other
    /// failure to the error line.
    pub fn on_outcome(&mut self, result: &Result<CommandOutput, String>) {
        match (result, self.confirm.as_mut()) {
            (Ok(_), Some(c)) if c.sent => self.confirm = None,
            (Err(e), Some(c)) if c.sent => {
                c.sent = false;
                c.error = Some(backend_error(e));
            }
            (Err(e), _) => self.error = Some(backend_error(e)),
            (Ok(_), _) => {}
        }
    }
}

/// Where to bring workspace `name`: a folder the user picks, or — without a folder dialog —
/// the app's storage, where it gets a folder of its own.
fn bring_into(ui: &egui::Ui, name: &str) -> Option<std::path::PathBuf> {
    if platform::folder_picker_available() {
        platform::pick_folder(&tr!(ui, "sapphire-workspaces-bring-title", name = name))
    } else {
        platform::app_storage().map(std::path::Path::to_path_buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remove_outcome_closes_its_dialog_or_shows_there() {
        let mut picker = WorkspacePicker {
            confirm: Some(RemoveConfirm {
                name: "notes".into(),
                sent: true,
                ..RemoveConfirm::default()
            }),
            ..WorkspacePicker::default()
        };
        picker.on_outcome(&Err("busy".into()));
        let c = picker.confirm.as_ref().expect("stays open on failure");
        assert_eq!(c.error, Some(backend_error("busy")));
        assert!(picker.error.is_none());

        picker.confirm.as_mut().unwrap().sent = true;
        picker.on_outcome(&Ok(CommandOutput::Done));
        assert!(picker.confirm.is_none(), "closes on success");
    }

    #[test]
    fn other_failures_go_to_the_error_line_even_with_the_dialog_open() {
        let mut picker = WorkspacePicker {
            confirm: Some(RemoveConfirm::default()),
            ..WorkspacePicker::default()
        };
        picker.on_outcome(&Err("move failed".into()));
        assert_eq!(picker.error, Some(backend_error("move failed")));
        assert!(picker.confirm.as_ref().unwrap().error.is_none());
    }
}
