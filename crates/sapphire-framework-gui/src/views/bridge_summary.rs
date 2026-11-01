//! An app's Settings › Sync › Sapphire Bridge, when the bridge's pages are in its own
//! window ([`BridgePages::Linked`](crate::client::BridgePages)): a summary, and buttons
//! that open that window on a page.

use std::path::{Path, PathBuf};

use crate::bridge_window::BridgePage;
use crate::client::{Conn, Snapshot};
use crate::tr;

/// Where "Get Sapphire Bridge" leads.
pub const RELEASES: &str = "https://github.com/sapphire-turtle/sapphire-framework/releases/latest";

/// What runs `exe --page <page>`.
pub type Launcher = Box<dyn Fn(&Path, BridgePage) -> std::io::Result<()>>;

/// What the summary says of the bridge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// The workgroup's name, if the host belongs to one.
    pub workgroup: Option<String>,
    /// Devices connected now, of the workgroup's.
    pub connected: usize,
    pub total: usize,
    /// The bridge's version, when it answers.
    pub version: Option<String>,
    /// Running, not running, or the message saying what to update.
    pub state: BridgeShown,
}

/// The bridge's state, as the summary shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeShown {
    Running,
    NotRunning,
    Incompatible(String),
}

impl Summary {
    /// The summary of `snapshot`'s bridge.
    pub fn of(snapshot: &Snapshot) -> Summary {
        let state = match &snapshot.bridge {
            Conn::Up(_) => BridgeShown::Running,
            Conn::Incompatible(message) => BridgeShown::Incompatible(message.clone()),
            Conn::Absent | Conn::Error(_) => BridgeShown::NotRunning,
        };
        let bridge = snapshot.bridge.up();
        Summary {
            workgroup: bridge
                .and_then(|b| b.status.workgroup.as_ref())
                .map(|w| w.name.clone()),
            connected: bridge.map_or(0, |b| b.peers.iter().filter(|p| p.connected).count()),
            total: bridge.map_or(0, |b| b.peers.len()),
            version: bridge.map(|b| b.status.version.clone()),
            state,
        }
    }
}

/// The summary row and its buttons.
pub struct BridgeSummary {
    launcher: Launcher,
    exe: Option<PathBuf>,
    error: Option<String>,
}

impl BridgeSummary {
    /// The row, opening `exe` (none: offer to get it) with `launcher`.
    pub fn new(exe: Option<PathBuf>, launcher: Launcher) -> BridgeSummary {
        BridgeSummary {
            launcher,
            exe,
            error: None,
        }
    }

    /// Run the window on `page`, keeping the error to show.
    pub fn open(&mut self, page: BridgePage) {
        let Some(exe) = &self.exe else {
            return;
        };
        self.error = (self.launcher)(exe, page).err().map(|e| e.to_string());
    }

    /// Whether the row offers to get Sapphire Bridge, for want of its executable.
    pub fn shows_get(&self) -> bool {
        self.exe.is_none()
    }

    /// The last launch's error.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Draw the row.
    pub fn ui(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let summary = Summary::of(snapshot);
        ui.heading(tr!(ui, "sapphire-bridge-summary-title"));
        match &summary.state {
            BridgeShown::Running => {
                let version = summary.version.clone().unwrap_or_default();
                ui.label(tr!(
                    ui,
                    "sapphire-bridge-summary-running",
                    version = version.as_str()
                ));
            }
            BridgeShown::NotRunning => {
                ui.label(tr!(ui, "sapphire-bridge-status-not-running"));
            }
            BridgeShown::Incompatible(message) => {
                ui.colored_label(ui.visuals().warn_fg_color, message);
            }
        }
        match &summary.workgroup {
            Some(name) => {
                ui.label(name.as_str());
                ui.label(tr!(
                    ui,
                    "sapphire-bridge-summary-connected",
                    connected = summary.connected as i64,
                    total = summary.total as i64
                ));
            }
            None => {
                ui.label(tr!(ui, "sapphire-bridge-summary-no-workgroup"));
            }
        }
        ui.add_space(8.0);
        if self.shows_get() {
            ui.hyperlink_to(tr!(ui, "sapphire-bridge-summary-get"), RELEASES);
            ui.weak(tr!(ui, "sapphire-bridge-summary-cli-hint"));
            return;
        }
        let mut pressed = None;
        ui.horizontal_wrapped(|ui| {
            for (page, id) in [
                (BridgePage::Status, "sapphire-bridge-summary-open"),
                (BridgePage::Workgroup, "sapphire-bridge-summary-workgroup"),
                (BridgePage::Devices, "sapphire-bridge-summary-devices"),
                (BridgePage::Embedding, "sapphire-bridge-summary-embedding"),
            ] {
                if ui.button(tr!(ui, id)).clicked() {
                    pressed = Some(page);
                }
            }
        });
        if let Some(page) = pressed {
            self.open(page);
        }
        if let Some(error) = &self.error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                tr!(
                    ui,
                    "sapphire-bridge-summary-launch-failed",
                    error = error.as_str()
                ),
            );
        }
    }
}

/// Run `exe --page <page>` on its own, so that closing the app leaves the window open.
pub fn spawn_detached(exe: &Path, page: BridgePage) -> std::io::Result<()> {
    std::process::Command::new(exe)
        .args(["--page", page.as_str()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn summary_of_an_absent_bridge() {
        let s = Summary::of(&Snapshot::default());
        assert_eq!(s.state, BridgeShown::NotRunning);
        assert_eq!(s.workgroup, None);
        let incompatible = Snapshot {
            bridge: Conn::Incompatible("update sapphire-bridge".into()),
            ..Snapshot::default()
        };
        assert_eq!(
            Summary::of(&incompatible).state,
            BridgeShown::Incompatible("update sapphire-bridge".into())
        );
    }

    #[test]
    fn buttons_launch_with_the_page() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let record = seen.clone();
        let mut row = BridgeSummary::new(
            Some(PathBuf::from("/b/sapphire-bridge-desktop")),
            Box::new(move |exe, page| {
                record.borrow_mut().push((exe.to_owned(), page));
                Ok(())
            }),
        );
        row.open(BridgePage::Workgroup);
        assert_eq!(
            *seen.borrow(),
            [(
                PathBuf::from("/b/sapphire-bridge-desktop"),
                BridgePage::Workgroup
            )]
        );
        assert!(!row.shows_get());
    }

    #[test]
    fn no_exe_offers_get() {
        let mut row = BridgeSummary::new(None, Box::new(|_, _| Ok(())));
        assert!(row.shows_get());
        row.open(BridgePage::Devices);
        assert!(row.error().is_none(), "nothing to launch, nothing failed");
    }

    #[test]
    fn launch_error_is_shown() {
        let mut row = BridgeSummary::new(
            Some(PathBuf::from("/b/x")),
            Box::new(|_, _| Err(std::io::Error::other("denied"))),
        );
        row.open(BridgePage::Status);
        assert!(row.error().unwrap().contains("denied"));
    }
}
