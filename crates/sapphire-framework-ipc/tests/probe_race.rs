//! Probes made while a server starts, and while it is busy, must never strand it (#235).
//!
//! Binding a Unix socket creates its file before the socket listens, and a connect in
//! between is refused; on macOS a live socket whose listen queue is full refuses too.
//! `probe` used to read "the file is there, the connection is refused" as a dead server's
//! leftover and unlink the file — and the server went on listening on a path nobody could
//! reach. A client polling `probe` while the server starts is exactly what the CLI, the GUI
//! and the tests do.
#![cfg(unix)]

use std::time::Duration;

use sapphire_framework_ipc::Endpoint;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn probing_while_a_server_binds_never_strands_it() {
    // The window between bind and listen is a few microseconds: several probers that
    // never yield make landing in it likely within a few rounds (the old bind stranded its socket within a dozen).
    const PROBERS: usize = 6;
    for round in 0..300 {
        let tmp = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::in_dir("race", tmp.path().to_path_buf());

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probers: Vec<_> = (0..PROBERS)
            .map(|_| {
                let endpoint = endpoint.clone();
                let stop = std::sync::Arc::clone(&stop);
                tokio::spawn(async move {
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = sapphire_framework_ipc::probe(&endpoint).await;
                    }
                })
            })
            .collect();
        tokio::time::sleep(Duration::from_micros(100)).await;

        let listener = std::sync::Arc::new(sapphire_framework_ipc::bind(&endpoint).await.unwrap());
        // Accept and drop, as a server does: a backlog nobody drains would stall the probes.
        let acceptor = {
            let listener = std::sync::Arc::clone(&listener);
            tokio::spawn(async move { while listener.accept_raw().await.is_ok() {} })
        };
        // Let the probers keep going a moment after the bind: a probe already past its
        // `exists` check when the file appeared is the one that unlinks it.
        tokio::time::sleep(Duration::from_micros(200)).await;
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for prober in probers {
            prober.await.unwrap();
        }

        // On macOS the probers can leave the listen queue full for a moment, and a full
        // queue refuses a connect like a dead socket does. A stranded socket never becomes
        // reachable again, so retry until a deadline rather than trusting one connect.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let reachable = loop {
            let reachable = tokio::time::timeout(
                Duration::from_secs(5),
                sapphire_framework_ipc::probe(&endpoint),
            )
            .await
            .expect("the probe answered")
            .unwrap();
            if reachable || std::time::Instant::now() >= deadline {
                break reachable;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        acceptor.abort();
        assert!(
            reachable,
            "round {round}: the server is listening, but its socket cannot be reached"
        );
    }
}

#[tokio::test]
async fn a_second_server_on_a_served_endpoint_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::in_dir("taken", tmp.path().to_path_buf());
    let first = sapphire_framework_ipc::bind(&endpoint).await.unwrap();
    let second = sapphire_framework_ipc::bind(&endpoint).await;
    assert!(second.is_err(), "a second server took a served endpoint");
    assert!(
        sapphire_framework_ipc::probe(&endpoint).await.unwrap(),
        "the first server's socket is gone"
    );
    drop(first);
    // Once the first is gone, the endpoint is free again.
    let third = sapphire_framework_ipc::bind(&endpoint).await.unwrap();
    assert!(sapphire_framework_ipc::probe(&endpoint).await.unwrap());
    drop(third);
    // And no staging names are left behind.
    let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}
