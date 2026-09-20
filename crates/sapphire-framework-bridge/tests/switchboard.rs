//! Two bridges on a loopback network, with a stub app server on each.

mod common;

use common::*;
use sapphire_bridge_api::{ManagedBy, RegisterParams, WorkspaceRegistration};
use sapphire_framework_bridge::{BridgeDir, LoopbackNetwork, Workgroup};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// `Result::unwrap_err` wants the success type to be `Debug`, and a raw stream is not: it is
/// an arbitrary reader and writer. Unwrapping by hand keeps the data plane's error the thing
/// under test.
#[track_caller]
fn expect_err<T>(result: sapphire_ipc::Result<T>) -> sapphire_ipc::Error {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(err) => err,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_reaches_the_owning_app_server_on_the_other_host() {
    let net = LoopbackNetwork::new();
    let a = start(&net, NODE_A, "host-a").await;
    let b = start(&net, NODE_B, "host-b").await;
    let (client_a, client_b, ws, device_a, device_b) = pair(&a, &b).await;

    // B waits for an announcement; A opens a stream to B.
    let mut incoming = client_b.incoming();
    let mut from_a = client_a.open_stream(ws, device_b).await.unwrap();

    let announced = tokio::time::timeout(std::time::Duration::from_secs(10), incoming.recv())
        .await
        .expect("an announcement")
        .unwrap();
    assert_eq!(announced.workspace_id, ws);
    // B names A by A's device id, because both hosts share one record for A: the id is the
    // record's filename and the `Entry.author` in synced content, so a sync carries it along.
    assert_eq!(announced.peer_device_id, device_a);

    let mut on_b = client_b.accept_stream(announced.ticket).await.unwrap();

    from_a.write_all(b"sync me").await.unwrap();
    let mut buf = [0u8; 7];
    on_b.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"sync me");

    on_b.write_all(b"ok").await.unwrap();
    let mut back = [0u8; 2];
    from_a.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, b"ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ticket_works_once() {
    let net = LoopbackNetwork::new();
    let a = start(&net, NODE_A, "host-a").await;
    let b = start(&net, NODE_B, "host-b").await;
    let (client_a, client_b, ws, _device_a, device_b) = pair(&a, &b).await;

    let mut incoming = client_b.incoming();
    let _from_a = client_a.open_stream(ws, device_b).await.unwrap();
    let announced = incoming.recv().await.unwrap();

    let _first = client_b
        .accept_stream(announced.ticket.clone())
        .await
        .unwrap();
    let err = expect_err(client_b.accept_stream(announced.ticket).await);
    assert!(err.to_string().contains("ticket"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_for_an_unowned_workspace_is_refused() {
    let net = LoopbackNetwork::new();
    let a = start(&net, NODE_A, "host-a").await;
    let b = start(&net, NODE_B, "host-b").await;
    let (client_a, _client_b, _ws, _device_a, device_b) = pair(&a, &b).await;

    let err = expect_err(
        client_a
            .open_stream(grain_id::GrainId::random(), device_b)
            .await,
    );
    assert!(err.to_string().contains("workspace"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_outside_the_workgroup_is_refused_before_any_app_server_hears_of_it() {
    let net = LoopbackNetwork::new();
    let a = start(&net, NODE_A, "host-a").await;
    let b = start(&net, NODE_B, "host-b").await;
    let (client_a, client_b, ws, _device_a, device_b) = pair(&a, &b).await;

    // B retires A.
    let mut devices = sapphire_registry::Devices::open(&b.dir.devices_dir(b.workgroup_id)).unwrap();
    devices.retire("host-a").unwrap();

    let mut incoming = client_b.incoming();
    let _ = client_a.open_stream(ws, device_b).await;

    let announced = tokio::time::timeout(std::time::Duration::from_secs(2), incoming.recv()).await;
    assert!(
        announced.is_err(),
        "a retired device's stream must not reach an app server"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_server_that_registers_twice_replaces_its_own_routes() {
    // Not in the brief, but it is the rule `replace_app` exists for, and the switchboard is
    // where an app server reaches it.
    let net = LoopbackNetwork::new();
    let a = start(&net, NODE_A, "host-a").await;
    let client = connect(&a).await;

    let first = grain_id::GrainId::random();
    let second = grain_id::GrainId::random();
    let register = |ws| RegisterParams {
        app_name: "test-app".into(),
        exe_path: "/bin/true".into(),
        managed_by: ManagedBy::Service,
        workspaces: vec![WorkspaceRegistration {
            workspace_id: ws,
            root: "/a".into(),
        }],
    };

    client.register(register(first)).await.unwrap();
    client.register(register(second)).await.unwrap();

    let status = client.status().await.unwrap();
    // The bridge's own row for the workgroup workspace is in the table too; the app's
    // registration only governs the rows named after the app.
    let owned: Vec<_> = status
        .routes
        .iter()
        .filter(|route| route.app_name == "test-app")
        .collect();
    assert_eq!(
        owned.len(),
        1,
        "a registration is the app's complete current list"
    );
    assert_eq!(owned[0].workspace_id, second);
}

#[tokio::test(flavor = "multi_thread")]
async fn registering_a_workspace_publishes_it_to_the_workgroup() {
    let net = LoopbackNetwork::new();
    let a = common::start(&net, common::NODE_A, "host-a").await;
    let client = common::connect(&a).await;

    let ws = grain_id::GrainId::random();
    client
        .register(RegisterParams {
            app_name: "sapphire-journal".into(),
            exe_path: "/bin/true".into(),
            managed_by: ManagedBy::Service,
            workspaces: vec![WorkspaceRegistration {
                workspace_id: ws,
                root: "/a/notes".into(),
            }],
        })
        .await
        .unwrap();

    let wg = Workgroup::open(&BridgeDir::at(a.tmp.path().join("bridge")).unwrap())
        .unwrap()
        .expect("a workgroup");
    let listed = wg.workspaces().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].workspace_id, ws);
    assert_eq!(listed[0].app_name, "sapphire-journal");
    assert_eq!(
        listed[0].name, "notes",
        "the published name defaults to the root directory's name"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_the_other_host_published_appears_in_workspace_list() {
    let net = LoopbackNetwork::new();
    let a = common::start(&net, common::NODE_A, "host-a").await;
    let b = common::start(&net, common::NODE_B, "host-b").await;
    common::introduce_both(&a, &b);

    // A publishes; the workgroup workspace replicates to B.
    let client_a = common::connect(&a).await;
    let ws = grain_id::GrainId::random();
    client_a
        .register(RegisterParams {
            app_name: "sapphire-journal".into(),
            exe_path: "/bin/true".into(),
            managed_by: ManagedBy::Service,
            workspaces: vec![WorkspaceRegistration {
                workspace_id: ws,
                root: "/a/notes".into(),
            }],
        })
        .await
        .unwrap();

    // A publishes; the workgroup workspace replicates to B — one session, the way the
    // running bridges drive it.
    common::sync_workgroup_between(&a, &b).await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let wg_b = Workgroup::open(&BridgeDir::at(b.tmp.path().join("bridge")).unwrap())
            .unwrap()
            .expect("a workgroup");
        if wg_b
            .workspaces()
            .unwrap()
            .iter()
            .any(|w| w.workspace_id == ws)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "B never learned about A's workspace"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}
