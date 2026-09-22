//! The auth configuration the [`protect`](crate::protect) layer reads.
//!
//! Lives in this crate so that the middleware pinned to
//! `protect(Arc<ServerState>, …)` in the remote server can move here and keep
//! its pinned behaviours: the fail-closed branch and the test-only bypass are
//! part of what the layer is *for*, not wiring a caller happens to own, and a
//! signature taking a bare `KeyStore` can carry neither. An application hosting
//! its own routes builds one the same way the remote server does.

use std::sync::Arc;

use crate::KeyStore;

/// Auth configuration for the [`protect`](crate::protect) layer: the key store
/// the layer checks tokens against, and whether the test-only bypass is
/// engaged.
#[derive(Clone, Default)]
pub struct AuthConfig {
    keys: Option<Arc<KeyStore>>,
    insecure: bool,
}

impl AuthConfig {
    /// Check every request against `keys`.
    pub fn new(keys: Arc<KeyStore>) -> Self {
        Self {
            keys: Some(keys),
            insecure: false,
        }
    }

    /// No key store configured: the layer fails **closed** and refuses every
    /// request with 503 (see [`protect`](crate::protect)). This is the state a
    /// server whose keys have not been provisioned yet is in — it is the
    /// [`Default`], and it lets nothing through.
    pub fn unconfigured() -> Self {
        Self::default()
    }

    /// Same config, with the test-only bypass switched on: a missing key
    /// store lets requests through instead of refusing them.
    ///
    /// The remote server calls this from its `insecure_for_tests`
    /// constructor; it must never be reachable from production wiring.
    pub fn insecure_for_tests(mut self) -> Self {
        self.insecure = true;
        self
    }

    /// The configured key store, if any.
    pub fn keys(&self) -> Option<&Arc<KeyStore>> {
        self.keys.as_ref()
    }

    /// Whether the test-only bypass is engaged.
    pub fn is_insecure(&self) -> bool {
        self.insecure
    }
}
