//! The in-process carrier: endpoints that live in this process's memory, not the file system.
//!
//! On a phone the bridge, the app server and the GUI share one process, and a Unix socket
//! there is a liability rather than a convenience — an iOS container path carries several
//! UUIDs and overflows `sun_path`. This carrier keeps the endpoint's identity (`dir` +
//! `name`) as a key in a process-wide table and hands each connecting client one end of a
//! [`tokio::io::duplex`] pipe. Everything above it — framing, the handshake, the data
//! plane's raw bytes — runs exactly as it does over a socket.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use tokio::io::DuplexStream;
use tokio::sync::mpsc;

use crate::conn::Connection;
use crate::endpoint::Endpoint;
use crate::error::{Error, Result};
use crate::raw::RawStream;

/// Bytes each direction of an in-process pipe buffers before a writer waits.
const PIPE_CAPACITY: usize = 64 * 1024;

/// Connections waiting for a listener to accept them, per endpoint.
const BACKLOG: usize = 64;

type Key = (PathBuf, String);

static REGISTRY: LazyLock<Mutex<HashMap<Key, mpsc::Sender<DuplexStream>>>> =
    LazyLock::new(Mutex::default);

fn key(endpoint: &Endpoint) -> Key {
    (endpoint.dir.clone(), endpoint.name.clone())
}

fn registry() -> std::sync::MutexGuard<'static, HashMap<Key, mpsc::Sender<DuplexStream>>> {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

/// A listener on an in-process endpoint. Dropping it frees the endpoint.
#[derive(Debug)]
pub struct InProcessListener {
    key: Key,
    sender: mpsc::Sender<DuplexStream>,
    incoming: tokio::sync::Mutex<mpsc::Receiver<DuplexStream>>,
}

impl InProcessListener {
    /// Accept one connection.
    pub async fn accept(&self) -> Result<Connection> {
        Ok(Connection::from_io(self.accept_stream().await?))
    }

    /// Accept one connection and hand back the byte stream, unframed.
    pub async fn accept_raw(&self) -> Result<RawStream> {
        Ok(Box::new(self.accept_stream().await?))
    }

    async fn accept_stream(&self) -> Result<DuplexStream> {
        // The listener holds a sender itself, so the channel never reports closed while it
        // is alive; `None` cannot happen, but is an error rather than a panic.
        self.incoming.lock().await.recv().await.ok_or(Error::Closed)
    }
}

impl Drop for InProcessListener {
    fn drop(&mut self) {
        let mut table = registry();
        // Only our own entry: a listener bound after this one was dropped from the table
        // (it cannot be, while we are in it) must never be removed by us.
        if table
            .get(&self.key)
            .is_some_and(|s| s.same_channel(&self.sender))
        {
            table.remove(&self.key);
        }
    }
}

/// Listen on `endpoint` in this process. Nothing is created on disk.
pub fn bind(endpoint: &Endpoint) -> Result<InProcessListener> {
    let key = key(endpoint);
    let mut table = registry();
    if table.get(&key).is_some_and(|s| !s.is_closed()) {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            format!("{} is already being served in this process", endpoint.name),
        )));
    }
    let (sender, incoming) = mpsc::channel(BACKLOG);
    table.insert(key.clone(), sender.clone());
    Ok(InProcessListener {
        key,
        sender,
        incoming: tokio::sync::Mutex::new(incoming),
    })
}

/// Is `endpoint` served in this process right now?
pub fn is_bound(endpoint: &Endpoint) -> bool {
    registry()
        .get(&key(endpoint))
        .is_some_and(|s| !s.is_closed())
}

/// Connect to `endpoint` in this process.
pub async fn connect(endpoint: &Endpoint) -> Result<Connection> {
    Ok(Connection::from_io(connect_stream(endpoint).await?))
}

/// Connect to `endpoint` in this process and hand back the byte stream, unframed.
pub async fn connect_raw(endpoint: &Endpoint) -> Result<RawStream> {
    Ok(Box::new(connect_stream(endpoint).await?))
}

async fn connect_stream(endpoint: &Endpoint) -> Result<DuplexStream> {
    let sender = registry().get(&key(endpoint)).cloned();
    let not_running =
        || Error::NotRunning(format!("nothing serves {} in this process", endpoint.name));
    let sender = sender.ok_or_else(not_running)?;
    let (ours, theirs) = tokio::io::duplex(PIPE_CAPACITY);
    sender.send(theirs).await.map_err(|_| not_running())?;
    Ok(ours)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, Notification};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A fresh endpoint per test: the table is process-wide and tests run in parallel.
    fn endpoint(tmp: &tempfile::TempDir, name: &str) -> Endpoint {
        Endpoint::in_dir(name, tmp.path().to_path_buf())
    }

    fn hello() -> Message {
        Message::Notification(Notification {
            method: "hello".into(),
            params: serde_json::Value::Null,
        })
    }

    #[tokio::test]
    async fn a_client_reaches_the_in_process_listener() {
        let tmp = tempfile::tempdir().unwrap();
        let ep = endpoint(&tmp, "reach");
        let listener = bind(&ep).unwrap();
        let server = tokio::spawn(async move {
            let mut conn = listener.accept().await.unwrap();
            conn.recv().await.unwrap().unwrap()
        });
        let client = crate::connect(&ep).await.unwrap();
        client.send(hello()).await.unwrap();
        assert_eq!(server.await.unwrap(), hello());
    }

    #[tokio::test]
    async fn raw_bytes_pass_unframed() {
        let tmp = tempfile::tempdir().unwrap();
        let ep = endpoint(&tmp, "raw");
        let listener = bind(&ep).unwrap();
        let server = tokio::spawn(async move {
            let mut stream = listener.accept_raw().await.unwrap();
            let mut buf = [0u8; 5];
            stream.read_exact(&mut buf).await.unwrap();
            buf
        });
        let mut client = crate::connect_raw(&ep).await.unwrap();
        client.write_all(b"hello").await.unwrap();
        client.flush().await.unwrap();
        assert_eq!(&server.await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn a_second_bind_is_address_in_use() {
        let tmp = tempfile::tempdir().unwrap();
        let ep = endpoint(&tmp, "twice");
        let _first = bind(&ep).unwrap();
        match bind(&ep) {
            Err(Error::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::AddrInUse),
            other => panic!("expected AddrInUse, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn probe_is_false_after_the_listener_drops() {
        let tmp = tempfile::tempdir().unwrap();
        let ep = endpoint(&tmp, "probe");
        let listener = bind(&ep).unwrap();
        assert!(crate::probe(&ep).await.unwrap());
        drop(listener);
        assert!(!is_bound(&ep));
    }

    #[tokio::test]
    async fn connect_before_bind_is_not_running() {
        let tmp = tempfile::tempdir().unwrap();
        let ep = endpoint(&tmp, "early");
        assert!(matches!(connect(&ep).await, Err(Error::NotRunning(_))));
    }

    #[tokio::test]
    async fn rebind_after_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let ep = endpoint(&tmp, "again");
        let first = bind(&ep).unwrap();
        let held = crate::connect(&ep).await.unwrap();
        drop(first);
        let second = bind(&ep).unwrap();
        let server = tokio::spawn(async move {
            let mut conn = second.accept().await.unwrap();
            conn.recv().await.unwrap().unwrap()
        });
        let client = crate::connect(&ep).await.unwrap();
        client.send(hello()).await.unwrap();
        assert_eq!(server.await.unwrap(), hello());
        drop(held);
    }

    #[tokio::test]
    async fn binding_in_process_creates_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let ep = endpoint(&tmp, "nofile");
        let _listener = bind(&ep).unwrap();
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn the_carrier_switch_routes_bind() {
        // `crate::bind_with` takes the carrier explicitly, so this test does not race the
        // others over the process-wide choice.
        let tmp = tempfile::tempdir().unwrap();
        let ep = endpoint(&tmp, "switch");
        let _listener = crate::bind_with(crate::Carrier::InProcess, &ep)
            .await
            .unwrap();
        assert!(is_bound(&ep));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }
}
