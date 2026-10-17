//! Connecting to a server that is already running (spec §2.6).
//!
//! This crate no longer starts anything: an app server is started by `serve` in a
//! terminal or by the OS service manager, and the CLI merely finds it. A socket with
//! nobody behind it is a corpse, not a task — [`probe`] reports nothing listening, and the
//! next server's `bind` clears it.

use crate::client::Client;
use crate::conn::Connection;
use crate::endpoint::Endpoint;
use crate::error::{Error, Result};
use crate::handshake::{ClientInfo, ServerInfo};
use crate::raw::RawStream;

/// The method a client calls to retire a server of the wrong version.
///
/// This crate only names it, so that both sides agree; `sapphire-framework-server`
/// implements it.
pub const SHUTDOWN_METHOD: &str = "server.shutdown";

/// Connect to `endpoint`: in this process when it is served here (or the process's
/// [`Carrier`](crate::Carrier) is in-process), otherwise over this platform's socket or pipe.
pub async fn connect(endpoint: &Endpoint) -> Result<Connection> {
    if crate::carrier::served_in_process(endpoint) {
        return crate::inproc::connect(endpoint).await;
    }
    #[cfg(unix)]
    {
        crate::unix::connect(endpoint).await
    }
    #[cfg(windows)]
    {
        crate::windows::connect(endpoint).await
    }
}

/// Connect to `endpoint` and hand back the byte stream, unframed.
///
/// The bridge's data plane speaks one JSON line and then raw bytes, so it cannot use
/// [`Connection`](crate::Connection).
pub async fn connect_raw(endpoint: &Endpoint) -> Result<RawStream> {
    if crate::carrier::served_in_process(endpoint) {
        return crate::inproc::connect_raw(endpoint).await;
    }
    #[cfg(unix)]
    {
        crate::unix::connect_raw(endpoint).await
    }
    #[cfg(windows)]
    {
        crate::windows::connect_raw(endpoint).await
    }
}

/// Is a server listening on `endpoint`? Never removes anything: a dead server's Unix socket
/// file is cleared by the next `bind` (#235).
pub async fn probe(endpoint: &Endpoint) -> Result<bool> {
    if crate::carrier::served_in_process(endpoint) {
        return Ok(crate::inproc::is_bound(endpoint));
    }
    #[cfg(unix)]
    {
        crate::unix::probe(endpoint).await
    }
    #[cfg(windows)]
    {
        crate::windows::probe(endpoint).await
    }
}

/// Handshake with the server on `endpoint`, or report that nothing is there.
///
/// `Ok(None)` covers both "nothing is listening" and "a corpse socket", which the next
/// server's `bind` clears. A server of a different API version is an error, not a
/// replacement — start-on-demand retired spawned servers by re-exec, and this crate no
/// longer starts anything. It is reported as [`Error::ApiVersionMismatch`]: the caller
/// upgrades and restarts the service, and no client fixes it. A different crate version
/// alone is no mismatch.
pub async fn connect_or_absent(
    endpoint: &Endpoint,
    app: &str,
    client: ClientInfo,
) -> Result<Option<(Client, ServerInfo)>> {
    if !probe(endpoint).await? {
        return Ok(None);
    }
    // The API gate lives in the handshake itself: a server of another API version is
    // [`Error::ApiVersionMismatch`], whose advice is to upgrade and restart. Nothing
    // replaces a server any more.
    match Client::handshake(connect(endpoint).await?, app, client).await {
        Ok(pair) => Ok(Some(pair)),
        // A half-open socket, or a server shutting down: nothing to connect to.
        Err(Error::Io(_) | Error::Closed) => Ok(None),
        Err(err) => Err(err),
    }
}
