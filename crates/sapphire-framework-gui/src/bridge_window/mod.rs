//! Sapphire Bridge's own window (#363): the bridge's pages, as [`Settings`](crate::Settings)
//! shows them for a [`ClientMode::BridgeOnly`](crate::client::ClientMode) client, and the
//! page a start or an activation asks for.

mod logs;
mod status;

pub use logs::LogsView;
pub use status::StatusView;

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
    fn bridge_page_round_trips() {
        for page in BridgePage::ALL {
            assert_eq!(BridgePage::parse(page.as_str()), Some(page));
        }
        assert_eq!(BridgePage::parse("devices"), Some(BridgePage::Devices));
        assert_eq!(BridgePage::parse("nope"), None);
        assert_eq!(BridgePage::parse(""), None);
    }
}
