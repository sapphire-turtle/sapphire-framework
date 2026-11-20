//! [`Delivery`]: whether this device's edits have reached a peer, for the shell's status bar.

use std::path::PathBuf;

use crate::client::{Conn, Snapshot};

/// A `SyncDelivered` the client heard: every edit this device had made to the workspace at
/// `root` reached `device_name` at `at`.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivered {
    /// The workspace the event came from.
    pub root: PathBuf,
    /// The peer that confirmed, as the workgroup names it.
    pub device_name: String,
    /// When the client heard it.
    pub at: jiff::Timestamp,
}

/// Whether this device's edits to the current workspace have left it.
#[derive(Clone, Debug, PartialEq)]
pub enum Delivery {
    /// An edit is pending and a peer that confirms is connected.
    Syncing,
    /// An edit is pending and nothing can take it: no confirming peer, or sync paused.
    Stuck,
    /// Nothing pending; the last delivery for this workspace, if one was heard.
    Synced(Option<Delivered>),
}

impl Delivery {
    /// Read `snapshot`, with `last` the latest delivery heard. `None` when the server is
    /// not up, serves no workspace, or does not sync it.
    pub fn of(snapshot: &Snapshot, last: Option<&Delivered>) -> Option<Delivery> {
        let Conn::Up(server) = &snapshot.server else {
            return None;
        };
        let current = server.current.as_ref()?;
        let sync = &current.sync;
        if !sync.enabled {
            return None;
        }
        Some(if sync.pending_delivery {
            if sync.acknowledging_peers == 0 || sync.paused.is_some() {
                Delivery::Stuck
            } else {
                Delivery::Syncing
            }
        } else {
            // A delivery heard for another workspace — before a switch — says nothing of
            // this one.
            Delivery::Synced(last.filter(|d| d.root == current.root).cloned())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ServerState;
    use sapphire_backend::protocol::{CurrentWorkspace, StatusReport, SyncStatusResult};

    fn snapshot(current: Option<SyncStatusResult>) -> Snapshot {
        Snapshot {
            fetched: true,
            bridge: Conn::Absent,
            server: Conn::Up(ServerState {
                info: StatusReport {
                    running: true,
                    version: Some("1".into()),
                    pid: Some(1),
                    managed_by: None,
                    app: vec![],
                },
                current: current.map(|sync| CurrentWorkspace {
                    root: "/ws".into(),
                    reachable: true,
                    workspace_id: None,
                    sync,
                }),
                update: None,
            }),
        }
    }

    fn sync(pending_delivery: bool, acknowledging_peers: usize) -> SyncStatusResult {
        SyncStatusResult {
            enabled: true,
            pending_delivery,
            acknowledging_peers,
            ..SyncStatusResult::not_synced()
        }
    }

    fn delivered(root: &str) -> Delivered {
        Delivered {
            root: root.into(),
            device_name: "laptop".into(),
            at: jiff::Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn nothing_without_sync() {
        assert_eq!(
            Delivery::of(&snapshot(Some(SyncStatusResult::not_synced())), None),
            None
        );
        assert_eq!(Delivery::of(&snapshot(None), None), None);
        let absent = Snapshot {
            fetched: true,
            ..Snapshot::default()
        };
        assert_eq!(Delivery::of(&absent, None), None);
    }

    #[test]
    fn pending_with_a_peer_is_syncing() {
        assert_eq!(
            Delivery::of(&snapshot(Some(sync(true, 1))), None),
            Some(Delivery::Syncing)
        );
    }

    #[test]
    fn pending_without_a_peer_is_stuck() {
        assert_eq!(
            Delivery::of(&snapshot(Some(sync(true, 0))), None),
            Some(Delivery::Stuck)
        );
    }

    #[test]
    fn paused_and_pending_is_stuck() {
        let paused = SyncStatusResult {
            paused: Some("Offline".into()),
            ..sync(true, 1)
        };
        assert_eq!(
            Delivery::of(&snapshot(Some(paused)), None),
            Some(Delivery::Stuck)
        );
    }

    #[test]
    fn pending_overrides_last_delivery() {
        assert_eq!(
            Delivery::of(&snapshot(Some(sync(true, 1))), Some(&delivered("/ws"))),
            Some(Delivery::Syncing)
        );
    }

    #[test]
    fn synced_carries_the_last_delivery() {
        let last = delivered("/ws");
        assert_eq!(
            Delivery::of(&snapshot(Some(sync(false, 1))), Some(&last)),
            Some(Delivery::Synced(Some(last)))
        );
        assert_eq!(
            Delivery::of(&snapshot(Some(sync(false, 1))), None),
            Some(Delivery::Synced(None))
        );
    }

    #[test]
    fn delivery_of_another_root_is_ignored() {
        assert_eq!(
            Delivery::of(&snapshot(Some(sync(false, 1))), Some(&delivered("/old"))),
            Some(Delivery::Synced(None))
        );
    }
}
