use egui::{Align, Color32, Layout};

use crate::client::{Command, CommandError, CommandOutput};

use super::model::{
    Badge, badge, bring_target, display_name, display_path, move_target, new_target_ok, others,
};
use super::{ViewCtx, error_line, unavailable};
use crate::platform;

/// The remove confirmation: whether to delete the folder too, the name to type when it
/// does, and the last failure.
#[derive(Default)]
struct RemoveConfirm {
    name: String,
    delete: bool,
    typed: String,
    error: Option<String>,
    /// A remove was sent from this dialog and has not come back yet.
    sent: bool,
}

/// The workspace screen: the one workspace the server serves (#215), the ways to switch
/// it — open another folder, create one, or bring one from the workgroup — and to move or
/// remove it.
#[derive(Default)]
pub struct WorkspacePicker {
    confirm: Option<RemoveConfirm>,
    error: Option<String>,
    /// The name typed for a new workspace, where there is no folder dialog.
    new_name: String,
}

impl WorkspacePicker {
    /// Render.
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &ViewCtx) -> Option<Command> {
        let mut out = None;
        ui.horizontal(|ui| {
            ui.heading("Workspace");
            if cx.busy {
                ui.spinner();
            }
        });
        ui.separator();
        error_line(ui, &mut self.error);
        let Some(server) = cx.snapshot.server.up() else {
            unavailable(ui, &format!("The {} server", cx.app_name));
            return out;
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            match &server.current {
                None => {
                    ui.label(
                        "No workspace yet. Open an existing folder, create a new one, or bring one from the workgroup.",
                    );
                }
                Some(current) => {
                    let b = badge(current);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.strong(display_name(current));
                                    let colour = match b {
                                        Badge::Syncing { .. } => Color32::GREEN,
                                        Badge::NotSynced => Color32::GRAY,
                                        _ => Color32::YELLOW,
                                    };
                                    ui.colored_label(colour, b.label());
                                });
                                ui.small(display_path(&current.root).display().to_string());
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add_enabled(!cx.busy, egui::Button::new("Remove…"))
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
                                            egui::Button::new("Move…"),
                                        )
                                        .clicked()
                                    && let Some(dir) = platform::pick_folder(&format!(
                                        "Choose where to move {} (it keeps its folder name)",
                                        display_name(current)
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
                                            egui::Button::new("Open folder"),
                                        )
                                        .clicked()
                                {
                                    platform::open_in_file_manager(&display_path(&current.root));
                                }
                                let (label, cmd) = if current.sync.enabled {
                                    ("Stop sync", Command::SyncDisable)
                                } else {
                                    ("Start sync", Command::SyncEnable)
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
            ui.strong("Switch to another workspace");
            ui.small("Only the selected workspace syncs on this device.");
            if let Some(storage) = platform::app_storage().filter(|_| !platform::folder_picker_available()) {
                if let Some(c) = self.new_in_storage_ui(ui, cx, storage) {
                    out = Some(c);
                }
            } else {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!cx.busy, egui::Button::new("Open folder…"))
                    .clicked()
                    && let Some(dir) = platform::pick_folder("Open a workspace folder")
                {
                    out = Some(if dir.join(format!(".{}", cx.app_name)).is_dir() {
                        Command::WorkspaceSelect { dir }
                    } else {
                        // A folder that is not a workspace yet becomes one.
                        Command::WorkspaceInit { dir, sync: false }
                    });
                }
                if ui
                    .add_enabled(!cx.busy, egui::Button::new("New…"))
                    .clicked()
                    && let Some(dir) =
                        platform::pick_folder("Choose an empty folder for the new workspace")
                {
                    match new_target_ok(&dir, cx.app_name) {
                        Ok(()) => out = Some(Command::WorkspaceInit { dir, sync: true }),
                        Err(e) => self.error = Some(e),
                    }
                }
            });
            }

            ui.add_space(12.0);
            ui.strong("From the workgroup");
            match cx.snapshot.bridge.up() {
                None => {
                    ui.small("The bridge is not available.");
                }
                Some(bridge) => {
                    let elsewhere = others(&bridge.ledger, server.current.as_ref(), cx.app_name);
                    if elsewhere.is_empty() {
                        ui.small("No other workspaces in the workgroup.");
                    }
                    for w in elsewhere {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(&w.name);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .add_enabled(
                                            !cx.busy,
                                            egui::Button::new("Bring to this host…"),
                                        )
                                        .clicked()
                                        && let Some(dir) = bring_into(&w.name)
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
                    .hint_text("New workspace name")
                    .desired_width(180.0),
            );
            let name = self.new_name.trim();
            let valid = platform::valid_workspace_name(name);
            if ui
                .add_enabled(valid && !cx.busy, egui::Button::new("Create"))
                .clicked()
            {
                let dir = storage.join(name);
                if dir.exists() && !dir.join(format!(".{}", cx.app_name)).is_dir() {
                    self.error = Some(format!("{name} already exists"));
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
        egui::Window::new("Remove workspace")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "{} stops syncing on this device. Other devices keep their copies, and \
                     it stays in the workgroup.",
                    confirm.name
                ));
                ui.checkbox(
                    &mut confirm.delete,
                    "Also delete the folder and every file in it",
                );
                if confirm.delete {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Type");
                        ui.strong(confirm.name.as_str());
                        ui.label("to confirm:");
                    });
                    ui.text_edit_singleline(&mut confirm.typed);
                }
                error_line(ui, &mut confirm.error);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ready = !confirm.delete || confirm.typed.trim() == confirm.name;
                    let label = if confirm.delete {
                        "Remove and delete"
                    } else {
                        "Remove"
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
                    if ui.button("Cancel").clicked() {
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
    pub fn on_outcome(&mut self, result: &Result<CommandOutput, CommandError>) {
        match (result, self.confirm.as_mut()) {
            (Ok(_), Some(c)) if c.sent => self.confirm = None,
            (Err(e), Some(c)) if c.sent => {
                c.sent = false;
                c.error = Some(e.message.clone());
            }
            (Err(e), _) => self.error = Some(e.message.clone()),
            (Ok(_), _) => {}
        }
    }
}

/// Where to bring workspace `name`: a folder the user picks, or — without a folder dialog —
/// the app's storage, where it gets a folder of its own.
fn bring_into(name: &str) -> Option<std::path::PathBuf> {
    if platform::folder_picker_available() {
        platform::pick_folder(&format!(
            "Choose where to put {name} (a folder named {name} is created inside)"
        ))
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
        assert_eq!(c.error.as_deref(), Some("busy"));
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
        assert_eq!(picker.error.as_deref(), Some("move failed"));
        assert!(picker.confirm.as_ref().unwrap().error.is_none());
    }
}
