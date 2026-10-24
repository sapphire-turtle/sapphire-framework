use egui::{Align, Layout};

use crate::client::{Command, CommandError, CommandOutput};

use super::model::{default_device_name, is_this_device, valid_name};
use super::{ViewCtx, backend_error, bridge_unavailable, error_line, register};
use crate::i18n::Message;
use corundum_gui::tr;

/// Join with a ticket: the ticket and the name this device was invited as.
///
/// The name starts empty on purpose: the inviting device fixed it when it issued the ticket,
/// and the bridge refuses any other. The invite dialog shows the inviter which name to pass on.
#[derive(Default)]
pub struct JoinDialog {
    ticket: String,
    device_name: String,
    error: Option<Message>,
}

impl JoinDialog {
    /// Reset the form.
    pub fn open(&mut self) {
        *self = JoinDialog::default();
    }

    /// The join to send, once both the ticket and the name are filled in.
    fn submission(&self) -> Option<Command> {
        let ticket = valid_name(&self.ticket)?;
        let device_name = valid_name(&self.device_name)?;
        Some(Command::WorkgroupJoin {
            ticket,
            device_name: Some(device_name),
        })
    }

    /// Render the form inline. Returns [`Command::WorkgroupJoin`] on submit.
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &ViewCtx) -> Option<Command> {
        register(ui);
        let mut out = None;
        ui.horizontal(|ui| {
            ui.label(tr!(ui, "sapphire-workgroup-ticket"));
            // Where egui has no clipboard of its own (Android), a ticket is pasted from here.
            if crate::platform::needs_paste_button()
                && ui
                    .small_button(tr!(ui, "sapphire-workgroup-paste"))
                    .clicked()
                && let Some(text) = crate::platform::clipboard_text()
            {
                self.ticket = text;
            }
        });
        ui.add(
            egui::TextEdit::multiline(&mut self.ticket)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        ui.label(tr!(ui, "sapphire-workgroup-invited-name"));
        ui.text_edit_singleline(&mut self.device_name);
        let submission = self.submission();
        if ui
            .add_enabled(
                !cx.busy && submission.is_some(),
                egui::Button::new(tr!(ui, "sapphire-workgroup-join")),
            )
            .clicked()
        {
            out = submission;
        }
        if cx.busy {
            ui.spinner();
        }
        error_line(ui, &mut self.error);
        out
    }

    /// Clear on success, show the error otherwise.
    pub fn on_outcome(&mut self, result: &Result<CommandOutput, CommandError>) {
        match result {
            Ok(_) => *self = JoinDialog::default(),
            Err(e) => self.error = Some(backend_error(&e.message)),
        }
    }
}

/// The workgroup screen: create or join when there is none, details when there is.
#[derive(Default)]
pub struct WorkgroupView {
    name: String,
    device_name: Option<String>,
    join: JoinDialog,
    error: Option<Message>,
    last: Option<Last>,
}

#[derive(Clone, Copy)]
enum Last {
    Create,
    Join,
}

impl WorkgroupView {
    /// Render.
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &ViewCtx) -> Option<Command> {
        register(ui);
        ui.horizontal(|ui| {
            ui.heading(tr!(ui, "sapphire-workgroup-title"));
            if cx.busy {
                ui.spinner();
            }
        });
        ui.separator();
        let Some(bridge) = cx.snapshot.bridge.up() else {
            bridge_unavailable(ui);
            return None;
        };
        if let Some(wg) = &bridge.status.workgroup {
            egui::Grid::new("workgroup").num_columns(2).show(ui, |ui| {
                ui.label(tr!(ui, "sapphire-workgroup-name"));
                ui.strong(&wg.name);
                ui.end_row();
                ui.label(tr!(ui, "sapphire-workgroup-id"));
                ui.horizontal(|ui| {
                    ui.monospace(wg.workgroup_id.to_string());
                    if ui.small_button(tr!(ui, "sapphire-action-copy")).clicked() {
                        ui.ctx().copy_text(wg.workgroup_id.to_string());
                    }
                });
                ui.end_row();
                ui.label(tr!(ui, "sapphire-workgroup-devices"));
                ui.label(wg.devices.to_string());
                ui.end_row();
                if let Some(me) = bridge
                    .peers
                    .iter()
                    .find(|p| is_this_device(p, &bridge.status.node_id))
                {
                    ui.label(tr!(ui, "sapphire-workgroup-this-device"));
                    ui.label(&me.name);
                    ui.end_row();
                }
            });
            return None;
        }

        let mut out = None;
        let device_name = self.device_name.get_or_insert_with(default_device_name);
        ui.columns(2, |cols| {
            egui::Frame::group(cols[0].style()).show(&mut cols[0], |ui| {
                ui.strong(tr!(ui, "sapphire-workgroup-create-heading"));
                ui.label(tr!(ui, "sapphire-workgroup-name"));
                ui.text_edit_singleline(&mut self.name);
                ui.label(tr!(ui, "sapphire-workgroup-device-name"));
                ui.text_edit_singleline(device_name);
                let ready = valid_name(&self.name).zip(valid_name(device_name));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !cx.busy && ready.is_some(),
                            egui::Button::new(tr!(ui, "sapphire-action-create")),
                        )
                        .clicked()
                        && let Some((name, device_name)) = ready
                    {
                        self.last = Some(Last::Create);
                        out = Some(Command::WorkgroupCreate { name, device_name });
                    }
                });
            });
            egui::Frame::group(cols[1].style()).show(&mut cols[1], |ui| {
                ui.strong(tr!(ui, "sapphire-workgroup-join-heading"));
                if let Some(c) = self.join.ui(ui, cx) {
                    self.last = Some(Last::Join);
                    out = Some(c);
                }
            });
        });
        error_line(ui, &mut self.error);
        out
    }

    /// Route the result to the form that sent it.
    pub fn on_outcome(&mut self, result: &Result<CommandOutput, CommandError>) {
        match self.last.take() {
            Some(Last::Join) => self.join.on_outcome(result),
            _ => {
                if let Err(e) = result {
                    self.error = Some(backend_error(&e.message));
                } else {
                    self.name.clear();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_starts_with_an_empty_name_and_needs_both_fields() {
        let mut d = JoinDialog::default();
        assert!(d.device_name.is_empty(), "the name must not be pre-filled");
        d.ticket = "  sapphire:abc \n".into();
        assert!(d.submission().is_none(), "no name yet");
        d.device_name = "   ".into();
        assert!(d.submission().is_none(), "a blank name is no name");
        d.device_name = " laptop ".into();
        match d.submission() {
            Some(Command::WorkgroupJoin {
                ticket,
                device_name,
            }) => {
                assert_eq!(ticket, "sapphire:abc");
                assert_eq!(device_name.as_deref(), Some("laptop"));
            }
            other => panic!("expected a join, got {other:?}"),
        }
        d.ticket.clear();
        assert!(d.submission().is_none(), "no ticket");
    }

    #[test]
    fn a_join_failure_goes_to_the_join_dialog() {
        let mut view = WorkgroupView {
            last: Some(Last::Join),
            ..WorkgroupView::default()
        };
        view.on_outcome(&Err("bad ticket".into()));
        assert_eq!(view.join.error, Some(backend_error("bad ticket")));
        assert!(view.error.is_none());
    }

    #[test]
    fn a_create_failure_goes_to_the_view_error_line() {
        let mut view = WorkgroupView {
            last: Some(Last::Create),
            ..WorkgroupView::default()
        };
        view.on_outcome(&Err("exists".into()));
        assert_eq!(view.error, Some(backend_error("exists")));
        assert!(view.join.error.is_none());
    }
}
