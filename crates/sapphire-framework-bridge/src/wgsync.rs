//! The workgroup's own workspace, replicated like any other workspace.
//!
//! The device ledger and the workspace list live inside the workgroup root, so a pairing on
//! one device reaches the rest by the same replication every other workspace uses. The
//! bridge is the app server that owns this workspace: it scans it, and it serves a
//! replication session to any peer the data plane hands it.
//!
//! See `docs/superpowers/specs/2026-09-16-process-architecture-design.md` §1: the bridge is
//! the app server of one app, and that app is the workgroup.

use std::sync::Arc;
use std::time::{Duration, Instant};

use grain_id::GrainId;
use sapphire_framework_session::run_session;
use sapphire_sync::{Replica, ScanOutcome};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;

use crate::dir::BridgeDir;
use crate::error::{Error, Result};
use crate::workgroup::Workgroup;

/// The application name the workgroup workspace is owned under.
///
/// This is the `app_name` of the routing-table row [`Bridge::run`] registers for the
/// workgroup's own workspace, and the `app_name` of its replica — which is also what puts
/// the `.bridge` marker directory in the workgroup root.
///
/// [`Bridge::run`]: crate::Bridge::run
pub const WORKSPACE_APP_NAME: &str = "bridge";

/// How long a scan waits, at most, for a running session to release the replica.
const SCAN_WAIT: Duration = Duration::from_secs(5);

/// How often a waiting scan re-checks the replica's lock.
const SCAN_POLL: Duration = Duration::from_millis(10);

/// The workgroup's own workspace as a replica.
///
/// The replica's root is the workgroup root (`<bridge dir>/workgroups/<id>/root/`), its
/// store `<bridge dir>/workgroups/<id>/replica/`. One host holds at most one workgroup and
/// therefore at most one of these, opened once by [`Bridge::run`] and kept open for the
/// process's life: the store is a redb database, so a second open while this one lives
/// fails.
///
/// [`Bridge::run`]: crate::Bridge::run
pub struct WorkgroupReplica {
    workspace_id: GrainId,
    /// The replication core. Locked across a whole session — which waits on a peer — so a
    /// `scan` arriving meanwhile takes the lock only briefly or not at all.
    replica: Mutex<Replica>,
}

impl WorkgroupReplica {
    /// Open the workgroup's own replica.
    ///
    /// `device_id` is this host's device record id in the workgroup's ledger — the author
    /// stamped on this replica's local writes. It is taken explicitly because *which* record
    /// is ours is a question a `Workgroup` cannot answer on its own until it knows this
    /// host's node id.
    pub fn open(
        _dir: &BridgeDir,
        workgroup: &Workgroup,
        device_id: GrainId,
    ) -> Result<WorkgroupReplica> {
        // The sync core pauses a workspace whose marker directory is gone, so a root whose
        // files have vanished cannot be read as "everything deleted". The marker belongs to
        // this type: the replica is the thing that syncs the root.
        let root = workgroup.dir.join("root");
        std::fs::create_dir_all(root.join(format!(".{WORKSPACE_APP_NAME}")))?;

        let config = sapphire_sync::ReplicaConfig::new(
            WORKSPACE_APP_NAME,
            root,
            device_id,
            &workgroup.dir.join("replica"),
        );
        let replica = Replica::open(config, Arc::new(sapphire_sync::SystemClock))
            .map_err(|e| Error::Config(format!("the workgroup's replica store: {e}")))?;
        Ok(WorkgroupReplica {
            workspace_id: workgroup.id,
            replica: Mutex::new(replica),
        })
    }

    /// The workgroup id, which doubles as the workspace id of its own workspace.
    pub fn workspace_id(&self) -> GrainId {
        self.workspace_id
    }

    /// Record every unrecorded edit under the workgroup root.
    ///
    /// The replication core is held exclusively across a session — which waits on a peer —
    /// so a scan arriving meanwhile waits up to [`SCAN_WAIT`] for it and then gives up with
    /// an error rather than blocking its caller's thread for the length of a stalled
    /// session. A scan that loses the race is retried by whatever wanted it; nothing is
    /// lost, because a scan only records what is still there.
    pub fn scan(&self) -> Result<()> {
        let outcome = {
            let mut replica = self.lock_for_scan()?;
            replica
                .scan()
                .map_err(|e| Error::Peer(format!("scanning the workgroup workspace: {e}")))?
        };
        match outcome {
            ScanOutcome::Scanned(report) => {
                tracing::debug!(
                    recorded = report.recorded.len(),
                    "scanned the workgroup root"
                );
                Ok(())
            }
            // A paused workgroup root means a user removed the root or its marker while
            // files were materialized. The bridge carries on; the next scan retries.
            ScanOutcome::Paused(reason) => {
                tracing::warn!("the workgroup workspace is paused: {reason:?}");
                Ok(())
            }
        }
    }

    /// Run one replication session over `stream`.
    ///
    /// The session is symmetric: this side sends what the peer lacks and applies what it is
    /// sent, so the same method serves a stream dialed to a peer and one a peer dialed to
    /// the bridge. The replica is locked for the session's whole life.
    pub async fn session<S>(&self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let mut replica = self.replica.lock().await;
        let outcome = run_session(stream, &mut replica, self.workspace_id)
            .await
            .map_err(|e| Error::Peer(format!("a workgroup replication session failed: {e}")))?;
        tracing::debug!(?outcome, "a workgroup replication session ended");
        Ok(())
    }

    /// Take the replica's lock without waiting longer than [`SCAN_WAIT`].
    fn lock_for_scan(&self) -> Result<tokio::sync::MutexGuard<'_, Replica>> {
        let deadline = Instant::now() + SCAN_WAIT;
        loop {
            match self.replica.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(_) if Instant::now() >= deadline => {
                    return Err(Error::Peer(
                        "a replication session holds the workgroup's replica".to_owned(),
                    ));
                }
                // A scan is called from sync code; sleeping here is what makes waiting for
                // a session possible without making `scan` async.
                Err(_) => std::thread::sleep(SCAN_POLL),
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::BridgeDir;

    const NODE_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    const NODE_B: &str = "b1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

    fn host(name: &str, node: &str) -> (tempfile::TempDir, BridgeDir, Workgroup) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let wg = Workgroup::create(&dir, "test", name, node).unwrap();
        (tmp, dir, wg)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_device_record_replicates_to_the_other_side() {
        let (_ta, dir_a, wg_a) = host("host-a", NODE_A);
        let (_tb, dir_b, _wg_b) = host("host-b", NODE_B);

        // Force both sides onto the same workgroup id, as a real join would.
        let wg_b = crate::testing::adopt_workgroup(&dir_b, &wg_a).unwrap();

        let a = WorkgroupReplica::open(&dir_a, &wg_a, wg_a.this_device().unwrap().id).unwrap();
        let b = WorkgroupReplica::open(&dir_b, &wg_b, wg_b.this_device().unwrap().id).unwrap();

        // A learns about B.
        wg_a.devices()
            .unwrap()
            .add("host-b", Some(NODE_B.to_owned()), None)
            .unwrap();
        a.scan().unwrap();

        let (left, right) = tokio::io::duplex(64 * 1024);
        let (x, y) = tokio::join!(a.session(left), b.session(right));
        x.unwrap();
        y.unwrap();

        assert!(
            wg_b.devices().unwrap().by_node_id(NODE_B).is_some(),
            "B must learn its own record from A"
        );
    }

    #[tokio::test]
    async fn a_retirement_replicates() {
        let (_ta, dir_a, wg_a) = host("host-a", NODE_A);
        let (_tb, dir_b, _) = host("host-b", NODE_B);
        let wg_b = crate::testing::adopt_workgroup(&dir_b, &wg_a).unwrap();

        wg_a.devices()
            .unwrap()
            .add("phone", Some("c1".repeat(32)), None)
            .unwrap();
        let a = WorkgroupReplica::open(&dir_a, &wg_a, wg_a.this_device().unwrap().id).unwrap();
        let b = WorkgroupReplica::open(&dir_b, &wg_b, wg_b.this_device().unwrap().id).unwrap();
        a.scan().unwrap();
        let (l, r) = tokio::io::duplex(64 * 1024);
        let _ = tokio::join!(a.session(l), b.session(r));
        assert!(
            wg_b.devices()
                .unwrap()
                .by_node_id(&"c1".repeat(32))
                .is_some()
        );

        // A retires the phone.
        wg_a.devices().unwrap().retire("phone").unwrap();
        a.scan().unwrap();
        let (l, r) = tokio::io::duplex(64 * 1024);
        let _ = tokio::join!(a.session(l), b.session(r));

        let devices = wg_b.devices().unwrap();
        let phone = devices
            .by_node_id(&"c1".repeat(32))
            .expect("the record stays");
        assert!(phone.is_retired(), "revocation must reach every device");
    }

    #[test]
    fn the_workgroup_id_is_its_own_workspace_id() {
        let (_t, dir, wg) = host("host-a", NODE_A);
        let replica = WorkgroupReplica::open(&dir, &wg, wg.this_device().unwrap().id).unwrap();
        assert_eq!(replica.workspace_id(), wg.id);
    }

    #[test]
    fn opening_twice_against_one_directory_fails() {
        let (_t, dir, wg) = host("host-a", NODE_A);
        let device = wg.this_device().unwrap().id;
        let _first = WorkgroupReplica::open(&dir, &wg, device).unwrap();
        assert!(
            WorkgroupReplica::open(&dir, &wg, device).is_err(),
            "the replica store is a redb database; a second open must fail loudly"
        );
    }
}
