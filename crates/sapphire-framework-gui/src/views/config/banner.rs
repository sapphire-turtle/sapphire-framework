//! The line per process that waits for a restart before a change applies.

use std::collections::BTreeMap;

use sapphire_bridge_api::settings::Process;

use crate::client::{Command, CommandError, CommandOutput};

use super::super::{ViewCtx, error_text_line, register};

/// One line per process with keys awaiting a restart, each with its restart button.
#[derive(Debug, Default)]
pub(crate) struct RestartBanner {
    /// The process a sent restart is waiting for.
    pub busy: Option<Process>,
    /// Why the last restart failed.
    pub error: Option<String>,
    /// The process whose restart failed, so its line shows the error.
    failed: Option<Process>,
}

impl RestartBanner {
    /// Render the lines for `pending` (see
    /// [`pending_restarts`](super::model::pending_restarts)). Returns a
    /// [`Command::Restart`] when a button is clicked.
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        cx: &ViewCtx,
        pending: &BTreeMap<Process, Vec<String>>,
    ) -> Option<Command> {
        register(ui);
        let mut out = None;
        for (process, keys) in pending {
            let (line, button) = texts(*process, cx.app_name, keys);
            ui.horizontal_wrapped(|ui| {
                ui.label(line);
                if self.busy == Some(*process) {
                    ui.spinner();
                } else if ui
                    .add_enabled(!cx.busy && self.busy.is_none(), egui::Button::new(button))
                    .clicked()
                {
                    out = Some(self.restart_command(*process));
                }
            });
            if self.failed == Some(*process) {
                error_text_line(ui, &mut self.error);
            }
        }
        // A failed process whose line is gone (its keys applied meanwhile) still says why.
        if !self.failed.is_some_and(|p| pending.contains_key(&p)) {
            error_text_line(ui, &mut self.error);
        }
        out
    }

    /// The restart of `process`, marked as running.
    pub(crate) fn restart_command(&mut self, process: Process) -> Command {
        self.busy = Some(process);
        self.error = None;
        self.failed = None;
        Command::Restart(process)
    }

    /// Take the outcome of the restart this banner sent.
    pub(crate) fn on_outcome(&mut self, result: &Result<CommandOutput, CommandError>) {
        self.failed = self.busy.take().filter(|_| result.is_err());
        self.error = result.as_ref().err().map(|e| e.message.clone());
    }
}

/// A process's line and its button's label.
fn texts(process: Process, app: &str, keys: &[String]) -> (String, String) {
    let keys = keys.join(", ");
    match process {
        Process::Bridge => (
            format!("sapphire-bridge needs a restart to apply: {keys}"),
            "Restart bridge".to_owned(),
        ),
        Process::App => (
            format!("the {app} server needs a restart to apply: {keys}"),
            format!("Restart {app} server"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::super::widgets::tests::drawn_text;
    use super::*;
    use crate::client::Snapshot;

    #[test]
    fn the_banner_lists_each_process_with_its_button() {
        let snapshot = Snapshot::default();
        let cx = ViewCtx {
            snapshot: &snapshot,
            app_name: "notes",
            busy: false,
        };
        let pending = BTreeMap::from([
            (
                Process::App,
                vec!["notes.a".to_owned(), "notes.b".to_owned()],
            ),
            (Process::Bridge, vec!["embed.x".to_owned()]),
        ]);
        let mut banner = RestartBanner::default();
        let drawn = drawn_text(|ui| assert!(banner.ui(ui, &cx, &pending).is_none()));
        for text in [
            "sapphire-bridge needs a restart to apply: embed.x",
            "Restart bridge",
            "the notes server needs a restart to apply: notes.a, notes.b",
            "Restart notes server",
        ] {
            assert!(drawn.contains(text), "{text} missing from {drawn}");
        }
        assert!(
            drawn_text(|ui| {
                banner.ui(ui, &cx, &BTreeMap::new());
            })
            .trim()
            .is_empty()
        );

        assert!(matches!(
            banner.restart_command(Process::App),
            Command::Restart(Process::App)
        ));
        assert_eq!(banner.busy, Some(Process::App));
        banner.on_outcome(&Err("the notes server did not come back within 30 s".into()));
        assert_eq!(banner.busy, None);
        let drawn = drawn_text(|ui| {
            banner.ui(ui, &cx, &pending);
        });
        let app_line = drawn.find("the notes server needs").unwrap();
        let error = drawn.find("did not come back within 30 s").unwrap();
        let bridge_line = drawn.find("sapphire-bridge needs").unwrap();
        assert!(
            bridge_line < app_line && app_line < error,
            "the error is under its own line: {drawn}"
        );
        assert_eq!(banner.failed, Some(Process::App));
        // Under the bridge's line when the bridge failed.
        banner.restart_command(Process::Bridge);
        banner.on_outcome(&Err("bridge failed".into()));
        let drawn = drawn_text(|ui| {
            banner.ui(ui, &cx, &pending);
        });
        let error = drawn.find("bridge failed").unwrap();
        assert!(
            error < drawn.find("the notes server needs").unwrap(),
            "{drawn}"
        );
        banner.restart_command(Process::Bridge);
        banner.on_outcome(&Ok(CommandOutput::Done));
        assert_eq!((banner.busy, banner.error.as_deref()), (None, None));
    }
}
