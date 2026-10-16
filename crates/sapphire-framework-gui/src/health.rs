//! [`Health`]: the services' state in one word, for the shell's status dot and banner.

use crate::client::{Conn, Snapshot};

/// How the bridge and the app server are doing, as the shell shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    /// The first refresh has not finished.
    Connecting,
    /// Everything the app needs answers.
    Healthy,
    /// Everything answers, and the app server found a newer release.
    UpdateAvailable,
    /// A process the app needs is missing or failing: the shell shows the status banner.
    Problem,
}

impl Health {
    /// Read `snapshot`.
    pub fn of(snapshot: &Snapshot) -> Health {
        if !snapshot.fetched {
            return Health::Connecting;
        }
        let Conn::Up(server) = &snapshot.server else {
            return Health::Problem;
        };
        if bridge_needed(snapshot) && !matches!(snapshot.bridge, Conn::Up(_)) {
            return Health::Problem;
        }
        match &server.update {
            Some(update) if update.enabled && update.update_available => Health::UpdateAvailable,
            _ => Health::Healthy,
        }
    }
}

/// Whether the app needs the bridge right now.
///
/// The bridge carries sync. A workspace that is selected and not synced does not need it,
/// so its absence is not a problem then — only what sync, the workgroup and embedding
/// screens offer waits for it. Until the server says which workspace it serves (or while
/// it serves none), the answer is yes: choosing a workspace may mean joining one.
pub fn bridge_needed(snapshot: &Snapshot) -> bool {
    match snapshot.server.up().and_then(|s| s.current.as_ref()) {
        Some(current) => current.sync.enabled,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ServerState;
    use sapphire_backend::protocol::{
        CurrentWorkspace, StatusReport, SyncStatusResult, UpdateStatus,
    };

    fn server(sync: Option<bool>, update: Option<UpdateStatus>) -> Conn<ServerState> {
        Conn::Up(ServerState {
            info: StatusReport {
                running: true,
                version: Some("1".into()),
                pid: Some(1),
                managed_by: None,
                app: vec![],
            },
            current: sync.map(|enabled| CurrentWorkspace {
                root: "/x/notes".into(),
                reachable: true,
                workspace_id: None,
                sync: SyncStatusResult {
                    enabled,
                    ..SyncStatusResult::not_synced()
                },
            }),
            update,
        })
    }

    fn snapshot(server: Conn<ServerState>) -> Snapshot {
        Snapshot {
            fetched: true,
            bridge: Conn::Absent,
            server,
        }
    }

    #[test]
    fn nothing_is_known_before_the_first_refresh() {
        assert_eq!(Health::of(&Snapshot::default()), Health::Connecting);
    }

    #[test]
    fn a_missing_app_server_is_a_problem() {
        assert_eq!(Health::of(&snapshot(Conn::Absent)), Health::Problem);
        assert_eq!(
            Health::of(&snapshot(Conn::Error("closed".into()))),
            Health::Problem
        );
    }

    #[test]
    fn a_missing_bridge_is_fine_while_the_workspace_is_not_synced() {
        let s = snapshot(server(Some(false), None));
        assert!(!bridge_needed(&s));
        assert_eq!(Health::of(&s), Health::Healthy);
    }

    #[test]
    fn a_missing_bridge_is_a_problem_while_syncing_or_without_a_workspace() {
        assert_eq!(
            Health::of(&snapshot(server(Some(true), None))),
            Health::Problem
        );
        assert_eq!(Health::of(&snapshot(server(None, None))), Health::Problem);
        let incompatible = Snapshot {
            bridge: Conn::Incompatible("update sapphire-bridge".into()),
            ..snapshot(server(Some(true), None))
        };
        assert_eq!(Health::of(&incompatible), Health::Problem);
    }

    #[test]
    fn a_found_release_is_reported_when_all_is_well() {
        let update = UpdateStatus {
            enabled: true,
            latest: None,
            update_available: true,
            last_error: None,
        };
        assert_eq!(
            Health::of(&snapshot(server(Some(false), Some(update.clone())))),
            Health::UpdateAvailable
        );
        // A problem outranks it.
        assert_eq!(
            Health::of(&snapshot(server(Some(true), Some(update)))),
            Health::Problem
        );
    }
}
