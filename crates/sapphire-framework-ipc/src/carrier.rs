//! Which carrier this process listens on, and the listener over either.
//!
//! [`Carrier::Os`] — a Unix domain socket or a Windows named pipe — is the default and what
//! every desktop process uses: the bridge, the app server and their clients are separate
//! processes. [`Carrier::InProcess`] is for a host that runs them all in one process (a
//! phone): no socket, no file, no path-length limit. It is chosen once, at start-up, before
//! anything binds.
//!
//! Connecting needs no choice: an endpoint served in this process is reached in this
//! process whatever the setting, and one that is not is looked for on the OS carrier only
//! while that is the setting.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::conn::Connection;
use crate::endpoint::Endpoint;
use crate::error::Result;
use crate::inproc::{self, InProcessListener};
use crate::raw::RawStream;

/// How this process's servers listen and its clients look for servers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carrier {
    /// A Unix domain socket or a Windows named pipe.
    Os,
    /// A table in this process's memory: every server and client is in this process.
    InProcess,
}

static CARRIER: AtomicU8 = AtomicU8::new(0);

/// Choose the carrier for this process. Call once at start-up, before anything binds.
pub fn set_carrier(carrier: Carrier) {
    CARRIER.store(
        match carrier {
            Carrier::Os => 0,
            Carrier::InProcess => 1,
        },
        Ordering::SeqCst,
    );
}

/// The carrier this process uses.
pub fn carrier() -> Carrier {
    match CARRIER.load(Ordering::SeqCst) {
        0 => Carrier::Os,
        _ => Carrier::InProcess,
    }
}

#[cfg(unix)]
type OsListener = crate::unix::UnixListenerHandle;
#[cfg(windows)]
type OsListener = crate::windows::PipeListener;

/// A bound endpoint, on whichever carrier it was bound.
#[derive(Debug)]
pub struct Listener(Inner);

#[derive(Debug)]
enum Inner {
    Os(OsListener),
    InProcess(InProcessListener),
}

#[cfg(unix)]
impl Listener {
    /// Accept one connection (on a socket, from the same OS user only).
    pub async fn accept(&self) -> Result<Connection> {
        match &self.0 {
            Inner::Os(l) => l.accept().await,
            Inner::InProcess(l) => l.accept().await,
        }
    }

    /// Accept one connection and hand back the byte stream, unframed.
    pub async fn accept_raw(&self) -> Result<RawStream> {
        match &self.0 {
            Inner::Os(l) => l.accept_raw().await,
            Inner::InProcess(l) => l.accept_raw().await,
        }
    }
}

#[cfg(windows)]
impl Listener {
    /// Accept one connection (on a pipe, from the same OS user only).
    pub async fn accept(&mut self) -> Result<Connection> {
        match &mut self.0 {
            Inner::Os(l) => l.accept().await,
            Inner::InProcess(l) => l.accept().await,
        }
    }

    /// Accept one connection and hand back the byte stream, unframed.
    pub async fn accept_raw(&mut self) -> Result<RawStream> {
        match &mut self.0 {
            Inner::Os(l) => l.accept_raw().await,
            Inner::InProcess(l) => l.accept_raw().await,
        }
    }
}

/// Listen on `endpoint` on this process's [`carrier`].
#[cfg(unix)]
pub async fn bind(endpoint: &Endpoint) -> Result<Listener> {
    bind_with(carrier(), endpoint).await
}

/// Listen on `endpoint` on this process's [`carrier`].
#[cfg(windows)]
pub fn bind(endpoint: &Endpoint) -> Result<Listener> {
    match carrier() {
        Carrier::Os => Ok(Listener(Inner::Os(crate::windows::bind(endpoint)?))),
        Carrier::InProcess => Ok(Listener(Inner::InProcess(inproc::bind(endpoint)?))),
    }
}

/// Listen on `endpoint` on `carrier`, whatever this process's choice — for tests that run
/// both carriers side by side.
pub async fn bind_with(carrier: Carrier, endpoint: &Endpoint) -> Result<Listener> {
    Ok(Listener(match carrier {
        #[cfg(unix)]
        Carrier::Os => Inner::Os(crate::unix::bind(endpoint).await?),
        #[cfg(windows)]
        Carrier::Os => Inner::Os(crate::windows::bind(endpoint)?),
        Carrier::InProcess => Inner::InProcess(inproc::bind(endpoint)?),
    }))
}

/// Where a client should look for `endpoint`.
pub(crate) fn served_in_process(endpoint: &Endpoint) -> bool {
    carrier() == Carrier::InProcess || inproc::is_bound(endpoint)
}
