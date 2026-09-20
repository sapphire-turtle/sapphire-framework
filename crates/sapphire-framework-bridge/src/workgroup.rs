//! The workgroup this host belongs to, and who is allowed to connect.

use std::path::PathBuf;

use grain_id::GrainId;
use sapphire_registry::{Device, Devices};
use serde::{Deserialize, Serialize};

use crate::dir::BridgeDir;
use crate::error::{Error, Result};
use crate::invite::Ticket;
use crate::pairing::{self, JoinRequest, JoinResponse};
use crate::peer::PeerTransport;

/// The `workgroup.toml` inside a workgroup's root: what the workgroup says about itself.
///
/// It lives in the workgroup's synced root, so every device of the workgroup sees the same
/// name once the workgroup is shared.
#[derive(Debug, Deserialize, Serialize)]
struct WorkgroupFile {
    name: String,
}

/// A set of devices that share workspaces.
#[derive(Clone, Debug)]
pub struct Workgroup {
    /// Its id.
    pub id: GrainId,
    /// Its name.
    pub name: String,
    /// `<bridge dir>/workgroups/<id>/`.
    pub dir: PathBuf,
    /// `<bridge dir>/workgroups/<id>/root/devices/`, the ledger directory.
    devices_dir: PathBuf,
}

impl Workgroup {
    /// Create a workgroup and write this device's own record into it.
    ///
    /// The founding device needs a record before its first sync, because `Entry.author` is
    /// its device id.
    ///
    /// The first release allows one workgroup per host; the layout and the wire format
    /// support several, so lifting the limit is a CLI change.
    pub fn create(
        dir: &BridgeDir,
        name: &str,
        this_device: &str,
        node_id: &str,
    ) -> Result<Workgroup> {
        if Workgroup::open(dir)?.is_some() {
            return Err(Error::Config(
                "this host already belongs to a workgroup".to_owned(),
            ));
        }
        let id = GrainId::random();
        let wg_dir = dir.workgroup_dir(id);
        std::fs::create_dir_all(wg_dir.join("root"))?;
        std::fs::create_dir_all(dir.devices_dir(id))?;
        let file = wg_dir.join("root").join("workgroup.toml");
        std::fs::write(
            &file,
            toml::to_string_pretty(&WorkgroupFile {
                name: name.to_owned(),
            })
            .map_err(|e| Error::Config(format!("{}: {e}", file.display())))?,
        )?;

        let workgroup = Workgroup {
            id,
            name: name.to_owned(),
            dir: wg_dir,
            devices_dir: dir.devices_dir(id),
        };
        let mut devices = workgroup.devices()?;
        devices.add(this_device, Some(node_id.to_owned()), None)?;
        Ok(workgroup)
    }

    /// The workgroup this host belongs to, if any.
    pub fn open(dir: &BridgeDir) -> Result<Option<Workgroup>> {
        let entries = match std::fs::read_dir(dir.workgroups_dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::Io(e)),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            // A directory that is not named after a workgroup id is not one: a leftover
            // temporary file, or something a user put here.
            let Some(id) = name.to_str().and_then(|s| s.parse::<GrainId>().ok()) else {
                continue;
            };
            let file = entry.path().join("root").join("workgroup.toml");
            let text = std::fs::read_to_string(&file).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error::Config(format!(
                        "{}: there is no workgroup here ({} is missing)",
                        entry.path().display(),
                        file.display()
                    ))
                } else {
                    Error::Io(e)
                }
            })?;
            let parsed: WorkgroupFile = toml::from_str(&text)
                .map_err(|e| Error::Config(format!("{}: {e}", file.display())))?;
            return Ok(Some(Workgroup {
                id,
                name: parsed.name,
                dir: entry.path(),
                devices_dir: dir.devices_dir(id),
            }));
        }
        Ok(None)
    }

    /// The device ledger, read fresh.
    ///
    /// Always reads from disk: the ledger is synced, so a pairing or a retirement that
    /// arrived from another device must take effect without restarting the bridge.
    pub fn devices(&self) -> Result<Devices> {
        Ok(Devices::open(&self.devices_dir)?)
    }

    /// This host's own record: the one whose `node_id` is this host's.
    ///
    /// A `join`ed host did not found its workgroup, so its record may sort anywhere in the
    /// ledger; which record is ours is a question only this host's node id can answer.
    pub fn this_device(&self, node_id: &str) -> Result<Device> {
        self.devices()?.by_node_id(node_id).cloned().ok_or_else(|| {
            Error::Config(format!(
                "no device of this workgroup has the node id {node_id}"
            ))
        })
    }

    /// Join the workgroup a ticket names, and write it into this bridge directory.
    ///
    /// Dials the ticket's address over the pairing protocol
    /// ([`PAIR_ALPN`](crate::pairing::PAIR_ALPN)), and on an
    /// [`Admitted`](crate::pairing::JoinResponse::Admitted) answer creates the workgroup
    /// directory — the id and the
    /// name come from the answer, so both sides agree on them without a second exchange —
    /// and writes this host's own device record under the id the inviter assigned: the
    /// record already exists in the inviter's ledger, and the ledger is replicated, so a
    /// second id for one device would fork it. A refused join removes everything it
    /// created, so the host may simply try again.
    pub async fn join(
        dir: &BridgeDir,
        ticket: &Ticket,
        device_name: &str,
        transport: &dyn PeerTransport,
    ) -> Result<Workgroup> {
        if Workgroup::open(dir)?.is_some() {
            return Err(Error::Config(
                "this host already belongs to a workgroup".to_owned(),
            ));
        }

        let node_id = transport.node_id();
        let stream = transport.open_pairing(&ticket.node_addr).await?;
        let request = JoinRequest {
            secret: ticket.secret,
            device_name: device_name.to_owned(),
            node_id: node_id.clone(),
        };
        let response = pairing::join(stream, request).await?;
        let (workgroup_id, workgroup_name) = match response {
            JoinResponse::Admitted {
                workgroup_id,
                workgroup_name,
                ..
            } => (workgroup_id, workgroup_name),
            JoinResponse::Rejected(why) => {
                return Err(Error::Unauthorized(format!(
                    "the invite was refused: {why}"
                )));
            }
        };

        match Workgroup::materialize(dir, workgroup_id, &workgroup_name, device_name, &node_id) {
            Ok(workgroup) => Ok(workgroup),
            Err(err) => {
                // Whatever half of it landed goes away again: a workgroup that was not
                // joined must not be one this host "already belongs to".
                std::fs::remove_dir_all(dir.workgroup_dir(workgroup_id)).ok();
                Err(err)
            }
        }
    }

    /// Write a workgroup directory with `id` and `name`, and this host's record in it.
    ///
    /// The record keeps the inviter-assigned id, so it is the same record the inviter's
    /// ledger holds — not a fresh one, which would make two hosts disagree about a device.
    fn materialize(
        dir: &BridgeDir,
        id: GrainId,
        name: &str,
        device_name: &str,
        node_id: &str,
    ) -> Result<Workgroup> {
        let wg_dir = dir.workgroup_dir(id);
        // `join` only gets here when `open` reports no workgroup, so anything already
        // sitting under this id is a leftover of a crashed attempt — possibly with a
        // record of a device this host no longer is. Start clean; a workgroup that was
        // successfully joined would have been refused above.
        std::fs::remove_dir_all(&wg_dir).ok();
        std::fs::create_dir_all(wg_dir.join("root"))?;
        std::fs::create_dir_all(dir.devices_dir(id))?;
        let file = wg_dir.join("root").join("workgroup.toml");
        std::fs::write(
            &file,
            toml::to_string_pretty(&WorkgroupFile {
                name: name.to_owned(),
            })
            .map_err(|e| Error::Config(format!("{}: {e}", file.display())))?,
        )?;

        let workgroup = Workgroup {
            id,
            name: name.to_owned(),
            dir: wg_dir,
            devices_dir: dir.devices_dir(id),
        };
        let own = Device {
            id,
            name: device_name.to_owned(),
            node_id: Some(node_id.to_owned()),
            description: None,
            created_at: chrono::Utc::now(),
            retired_at: None,
        };
        Devices::write_record(&workgroup.devices_dir, &own)?;
        Ok(workgroup)
    }

    /// The device behind `node_id`, if it may connect.
    ///
    /// Reads the ledger on every call: authorization is a live question, and a retirement
    /// that arrived from another device must close the door without a restart.
    pub fn authorize(&self, node_id: &str) -> Result<Device> {
        let devices = self.devices()?;
        let Some(device) = devices.by_node_id(node_id) else {
            return Err(Error::Unauthorized(format!(
                "{node_id} is not a device of this workgroup"
            )));
        };
        if device.is_retired() {
            return Err(Error::Unauthorized(format!(
                "the device {} ({node_id}) is retired",
                device.name
            )));
        }
        Ok(device.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::BridgeDir;
    use crate::net::NetConfig;

    const NODE_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    const NODE_B: &str = "b1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

    fn bridge_dir() -> (tempfile::TempDir, BridgeDir) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        (tmp, dir)
    }

    #[test]
    fn creating_a_workgroup_writes_this_devices_record() {
        let (_tmp, dir) = bridge_dir();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();

        let me = wg.this_device(NODE_A).unwrap();
        assert_eq!(me.name, "laptop");
        assert_eq!(me.node_id.as_deref(), Some(NODE_A));
    }

    #[test]
    fn a_created_workgroup_reopens() {
        let (_tmp, dir) = bridge_dir();
        let created = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        let reopened = Workgroup::open(&dir).unwrap().expect("a workgroup");
        assert_eq!(reopened.id, created.id);
        assert_eq!(reopened.name, "home");
    }

    #[test]
    fn a_host_without_a_workgroup_reports_none() {
        let (_tmp, dir) = bridge_dir();
        assert!(Workgroup::open(&dir).unwrap().is_none());
    }

    #[test]
    fn a_second_workgroup_is_refused_for_now() {
        let (_tmp, dir) = bridge_dir();
        Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        let err = Workgroup::create(&dir, "work", "laptop", NODE_A).unwrap_err();
        assert!(err.to_string().contains("already"), "{err}");
    }

    #[test]
    fn a_known_node_is_authorized() {
        let (_tmp, dir) = bridge_dir();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        let mut devices = wg.devices().unwrap();
        devices.add("phone", Some(NODE_B.to_owned()), None).unwrap();

        let device = wg.authorize(NODE_B).unwrap();
        assert_eq!(device.name, "phone");
    }

    #[test]
    fn an_unknown_node_is_refused() {
        let (_tmp, dir) = bridge_dir();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        let err = wg.authorize(NODE_B).unwrap_err();
        assert!(
            err.to_string().contains("not a device of this workgroup"),
            "{err}"
        );
    }

    #[test]
    fn a_retired_node_is_refused_and_says_so() {
        let (_tmp, dir) = bridge_dir();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        let mut devices = wg.devices().unwrap();
        devices.add("phone", Some(NODE_B.to_owned()), None).unwrap();
        devices.retire("phone").unwrap();

        let err = wg.authorize(NODE_B).unwrap_err();
        assert!(err.to_string().contains("retired"), "{err}");
    }

    #[test]
    fn authorization_rereads_the_ledger_each_time() {
        let (_tmp, dir) = bridge_dir();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        let mut devices = wg.devices().unwrap();
        devices.add("phone", Some(NODE_B.to_owned()), None).unwrap();
        assert!(wg.authorize(NODE_B).is_ok());

        // Retirement arrives from another device while the bridge is running.
        let mut fresh = wg.devices().unwrap();
        fresh.retire("phone").unwrap();

        assert!(
            wg.authorize(NODE_B).is_err(),
            "a revocation must take effect without restarting the bridge"
        );
    }

    #[test]
    fn net_configuration_defaults_to_waking_owners() {
        let (_tmp, dir) = bridge_dir();
        let net = NetConfig::load(&dir.net_toml()).unwrap();
        assert!(net.wake_on_sync);
        assert!(net.discovery);
        assert!(net.relays.is_empty());
    }

    #[test]
    fn this_device_is_found_by_node_id_not_by_position() {
        let (_tmp, dir) = bridge_dir();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        // Another device is added and happens to sort first.
        wg.devices()
            .unwrap()
            .add("aaa-phone", Some(NODE_B.to_owned()), None)
            .unwrap();

        assert_eq!(wg.this_device(NODE_A).unwrap().name, "laptop");
        assert_eq!(wg.this_device(NODE_B).unwrap().name, "aaa-phone");
    }

    #[test]
    fn a_node_id_that_is_not_a_member_has_no_record() {
        let (_tmp, dir) = bridge_dir();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        assert!(wg.this_device(&"c1".repeat(32)).is_err());
    }

    /// An inviter on a loopback network, plus a ticket for it.
    ///
    /// `LoopbackTransport` carries `node_addr` as the node id's bytes, so a ticket made here
    /// reaches it the same way a real one reaches an iroh address.
    async fn invited(
        net: &crate::peer::LoopbackNetwork,
        device_name: &str,
    ) -> (
        tempfile::TempDir,
        BridgeDir,
        Workgroup,
        crate::invite::Ticket,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();

        let (_invite, secret) = crate::invite::Invites::load(&dir.root.join("invites.toml"))
            .unwrap()
            .create(device_name, crate::invite::DEFAULT_TTL)
            .unwrap();
        let ticket = crate::invite::Ticket {
            workgroup_id: wg.id,
            node_addr: NODE_A.as_bytes().to_vec(),
            secret,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
        };

        // Answer one pairing connection in the background, as `Bridge::run` would.
        let transport = net.transport(NODE_A);
        let answering_dir = dir.clone();
        let answering_wg = wg.clone();
        tokio::spawn(async move {
            if let Ok((_from, stream)) = transport.accept_pairing().await {
                let mut invites =
                    crate::invite::Invites::load(&answering_dir.root.join("invites.toml")).unwrap();
                let _ = crate::pairing::admit(stream, &mut invites, &answering_wg).await;
            }
        });

        (tmp, dir, wg, ticket)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn joining_writes_the_workgroup_locally() {
        let net = crate::peer::LoopbackNetwork::new();
        let (_inviter_tmp, _inviter_dir, inviter_wg, ticket) = invited(&net, "phone").await;

        let joiner_tmp = tempfile::tempdir().unwrap();
        let joiner_dir = BridgeDir::at(joiner_tmp.path().join("bridge")).unwrap();
        let joiner_transport = net.transport(NODE_B);

        let joined = Workgroup::join(&joiner_dir, &ticket, "phone", &joiner_transport)
            .await
            .unwrap();

        assert_eq!(
            joined.id, inviter_wg.id,
            "both sides must agree on the workgroup"
        );
        assert_eq!(joined.name, "home");
        assert_eq!(
            joined.this_device(NODE_B).unwrap().name,
            "phone",
            "the joiner must have its own record locally"
        );
        assert!(
            Workgroup::open(&joiner_dir).unwrap().is_some(),
            "the workgroup must survive a reopen"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn joining_a_second_workgroup_is_refused_for_now() {
        let net = crate::peer::LoopbackNetwork::new();
        let (_inviter_tmp, _inviter_dir, _wg, ticket) = invited(&net, "phone").await;

        let joiner_tmp = tempfile::tempdir().unwrap();
        let joiner_dir = BridgeDir::at(joiner_tmp.path().join("bridge")).unwrap();
        // The joiner already belongs to one.
        Workgroup::create(&joiner_dir, "work", "phone", NODE_B).unwrap();

        let err = Workgroup::join(&joiner_dir, &ticket, "phone", &net.transport(NODE_B))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rejected_join_leaves_no_workgroup_directory_behind() {
        // A half-created workgroup would make every later attempt fail with "already belongs
        // to a workgroup", which is the worst possible message for someone who has joined
        // nothing.
        let net = crate::peer::LoopbackNetwork::new();
        let (_inviter_tmp, _inviter_dir, wg, mut ticket) = invited(&net, "phone").await;
        ticket.secret = [0u8; 32]; // not the invited secret

        let joiner_tmp = tempfile::tempdir().unwrap();
        let joiner_dir = BridgeDir::at(joiner_tmp.path().join("bridge")).unwrap();

        assert!(
            Workgroup::join(&joiner_dir, &ticket, "phone", &net.transport(NODE_B))
                .await
                .is_err()
        );
        assert!(
            Workgroup::open(&joiner_dir).unwrap().is_none(),
            "a failed join must leave the host able to try again"
        );
        assert!(
            !joiner_dir.workgroup_dir(wg.id).exists(),
            "no partial directory may remain"
        );
    }
}
