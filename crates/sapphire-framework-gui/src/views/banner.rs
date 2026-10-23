use sapphire_backend::protocol::ReleaseInfo;

use crate::client::{Command, CommandError, CommandOutput, Conn, ServerState, ServiceTarget};

use super::{ViewCtx, error_line};

/// The status line: is the bridge up, is the app server up. The shell shows it when
/// something is wrong ([`Health::Problem`](crate::Health::Problem)), and Settings shows it
/// under About.
#[derive(Default)]
pub struct ServiceStatusBanner {
    error: Option<String>,
}

impl ServiceStatusBanner {
    /// Render. Returns a [`Command::ServiceInstall`] when a start button is clicked.
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &ViewCtx) -> Option<Command> {
        let mut out = None;
        if !cx.snapshot.fetched {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Connecting…");
            });
            return None;
        }
        let mut bridge = describe(&cx.snapshot.bridge, |b| format!("v{}", b.status.version));
        if !crate::health::bridge_needed(cx.snapshot) && !matches!(cx.snapshot.bridge, Conn::Up(_))
        {
            // Not a fault: the workspace is not synced (see `Health`).
            bridge.push_str(" — needed only to sync");
        }
        let server = describe(&cx.snapshot.server, |s| {
            format!("v{}", s.info.version.clone().unwrap_or_default())
        });
        ui.horizontal_wrapped(|ui| {
            for (label, state, absent, target) in [
                (
                    "sapphire-bridge",
                    &bridge,
                    matches!(cx.snapshot.bridge, Conn::Absent),
                    ServiceTarget::Bridge,
                ),
                (
                    cx.app_name,
                    &server,
                    matches!(cx.snapshot.server, Conn::Absent),
                    ServiceTarget::App,
                ),
            ] {
                ui.strong(label);
                ui.label(state);
                if absent
                    && ui
                        .add_enabled(!cx.busy, egui::Button::new("Install & start service"))
                        .clicked()
                {
                    out = Some(Command::ServiceInstall(target));
                }
                ui.separator();
            }
        });
        if let Some(release) = available_release(&cx.snapshot.server) {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "{} {} is available. Install it, then restart the service.",
                    cx.app_name, release.version
                ));
                ui.hyperlink_to("Release notes", &release.url);
            });
        }
        if cx.busy {
            ui.spinner();
        }
        error_line(ui, &mut self.error);
        if let Some(command) = self.error.as_deref().and_then(manual_command) {
            let command = command.to_owned();
            ui.horizontal(|ui| {
                ui.small("To install by hand:");
                ui.monospace(&command);
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(command.clone());
                }
            });
        }
        out
    }

    /// Show an install failure (it carries the command to run by hand).
    pub fn on_outcome(&mut self, result: &Result<CommandOutput, CommandError>) {
        self.error = result.as_ref().err().map(|e| e.message.clone());
    }
}

/// The newer release the app server found, if it found one.
fn available_release(server: &Conn<ServerState>) -> Option<&ReleaseInfo> {
    let Conn::Up(state) = server else { return None };
    let update = state.update.as_ref()?;
    if update.enabled && update.update_available {
        update.latest.as_ref()
    } else {
        None
    }
}

fn describe<T>(conn: &Conn<T>, up: impl Fn(&T) -> String) -> String {
    match conn {
        Conn::Absent => "not running".to_owned(),
        Conn::Incompatible(msg) => format!("incompatible: {msg}"),
        Conn::Error(msg) => format!("error: {msg}"),
        Conn::Up(t) => format!("running {}", up(t)),
    }
}

/// The command an install failure says to run by hand: whatever follows its last `run: `.
fn manual_command(message: &str) -> Option<&str> {
    let (_, command) = message.rsplit_once("run: ")?;
    let command = command.trim();
    (!command.is_empty()).then_some(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manual_command_is_what_follows_run() {
        assert_eq!(
            manual_command(r"access denied; run: C:\x\sapphire-bridge.exe service install"),
            Some(r"C:\x\sapphire-bridge.exe service install")
        );
        assert_eq!(
            manual_command("/a/b was not found; run: /a/b service install\n"),
            Some("/a/b service install")
        );
        assert_eq!(manual_command("closed"), None);
        assert_eq!(manual_command("run: "), None);
    }

    fn server(update: Option<sapphire_backend::protocol::UpdateStatus>) -> Conn<ServerState> {
        Conn::Up(ServerState {
            info: sapphire_backend::protocol::StatusReport {
                running: true,
                version: Some("0.19.0".into()),
                pid: Some(1),
                managed_by: None,
                app: vec![],
            },
            current: None,
            update,
        })
    }

    #[test]
    fn only_a_found_newer_release_is_shown() {
        use sapphire_backend::protocol::UpdateStatus;
        let release = ReleaseInfo {
            version: "0.20.0".into(),
            url: "https://example.invalid/v0.20.0".into(),
        };
        let found = UpdateStatus {
            enabled: true,
            latest: Some(release.clone()),
            update_available: true,
            last_error: None,
        };
        assert_eq!(
            available_release(&server(Some(found.clone()))),
            Some(&release)
        );
        let current = UpdateStatus {
            update_available: false,
            ..found
        };
        assert_eq!(available_release(&server(Some(current))), None);
        // An older server, or one not checking, says nothing.
        assert_eq!(available_release(&server(None)), None);
        assert_eq!(available_release(&Conn::Absent), None);
    }

    #[test]
    fn an_install_failure_renders_with_its_copy_row() {
        let mut banner = ServiceStatusBanner::default();
        banner.on_outcome(&Err("exit status 1; run: /a/b service install".into()));
        let snapshot = crate::client::Snapshot {
            fetched: true,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let cx = ViewCtx {
                snapshot: &snapshot,
                app_name: "app",
                busy: false,
            };
            assert!(banner.ui(ui, &cx).is_none());
        });
        output.textures_delta.clear();
        assert!(banner.error.is_some());
    }
}
