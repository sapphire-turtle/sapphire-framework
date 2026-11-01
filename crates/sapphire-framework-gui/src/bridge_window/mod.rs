//! Sapphire Bridge's own window (#363): the bridge's pages, as [`Settings`](crate::Settings)
//! shows them for a [`ClientMode::BridgeOnly`](crate::client::ClientMode) client, and the
//! page a start or an activation asks for.

#[cfg(feature = "package")]
mod about;
#[cfg(feature = "package")]
mod first_run;
mod logs;
mod status;
#[cfg(feature = "eframe")]
mod window;

#[cfg(feature = "package")]
pub use about::PackageAbout;
#[cfg(feature = "package")]
pub use first_run::{WelcomePage, first_run_pages};
pub use logs::LogsView;
pub use status::StatusView;
#[cfg(feature = "package")]
pub use window::PackageWindow;
#[cfg(feature = "eframe")]
pub use window::{BridgeWindowOptions, install_logging, run_bridge};

/// The pages activations ask for, handed from the listener's thread to the window's.
pub struct PageInbox {
    rx: std::sync::mpsc::Receiver<String>,
}

impl PageInbox {
    /// An inbox, and what the activation callback sends into it.
    pub fn new() -> (std::sync::mpsc::Sender<String>, PageInbox) {
        let (tx, rx) = std::sync::mpsc::channel();
        (tx, PageInbox { rx })
    }

    /// The page the latest activation that named one asks for, if any arrived since the
    /// last call; messages that name no page are dropped.
    pub fn take(&self) -> Option<BridgePage> {
        self.rx
            .try_iter()
            .filter_map(|m| BridgePage::parse(&m))
            .last()
    }
}

/// A page of the bridge's window: what `--page` and an activation's message name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BridgePage {
    Status,
    Workgroup,
    Devices,
    Embedding,
    Appearance,
    Logs,
    About,
}

impl BridgePage {
    /// Every page, in the window's order.
    pub const ALL: [BridgePage; 7] = [
        BridgePage::Status,
        BridgePage::Workgroup,
        BridgePage::Devices,
        BridgePage::Embedding,
        BridgePage::Appearance,
        BridgePage::Logs,
        BridgePage::About,
    ];

    /// The page `s` names (`status`, `workgroup`, …), if any.
    pub fn parse(s: &str) -> Option<BridgePage> {
        BridgePage::ALL.into_iter().find(|p| p.as_str() == s)
    }

    /// Its name, as [`parse`](Self::parse) reads it.
    pub fn as_str(self) -> &'static str {
        match self {
            BridgePage::Status => "status",
            BridgePage::Workgroup => "workgroup",
            BridgePage::Devices => "devices",
            BridgePage::Embedding => "embedding",
            BridgePage::Appearance => "appearance",
            BridgePage::Logs => "logs",
            BridgePage::About => "about",
        }
    }
}

/// The bridge's log directory: `$SAPPHIRE_BRIDGE_DIR/logs`, else `logs` in the bridge's
/// directory under the user's data directory (as `BridgeDir::log_dir`).
pub fn bridge_log_dir() -> Option<std::path::PathBuf> {
    let root = match std::env::var_os("SAPPHIRE_BRIDGE_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => std::path::PathBuf::from(dir),
        None => corundum_framework_platform::dirs::data_dir()?.join(sapphire_ipc::BRIDGE_DIR_NAME),
    };
    Some(root.join("logs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inbox_gives_the_latest_page() {
        let (tx, inbox) = PageInbox::new();
        assert_eq!(inbox.take(), None);
        tx.send("devices".into()).unwrap();
        tx.send("nope".into()).unwrap();
        tx.send("logs".into()).unwrap();
        tx.send("also nope".into()).unwrap();
        assert_eq!(inbox.take(), Some(BridgePage::Logs));
        assert_eq!(inbox.take(), None);
    }

    #[test]
    fn bridge_page_round_trips() {
        for page in BridgePage::ALL {
            assert_eq!(BridgePage::parse(page.as_str()), Some(page));
        }
        assert_eq!(BridgePage::parse("devices"), Some(BridgePage::Devices));
        assert_eq!(BridgePage::parse("nope"), None);
        assert_eq!(BridgePage::parse(""), None);
    }
}
