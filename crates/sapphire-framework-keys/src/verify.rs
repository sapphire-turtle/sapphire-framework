//! Who a presented token belongs to.

pub use corundum_framework_keys::KeyId;

/// The API key a request came from; put into the request's extensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authenticated {
    /// Its id in the workspace's keys.
    pub id: KeyId,
    /// Its name.
    pub name: String,
}

/// The answer to a presented token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The token belongs to this key, which may use this application.
    Accepted(Authenticated),
    /// It does not.
    Refused,
}

/// Checks presented tokens. An application may wrap the framework's own (the workspace's
/// keys) in one of its own, to map a key to something of its own.
#[async_trait::async_trait]
pub trait Verifier: Send + Sync {
    /// The verdict on `token`.
    async fn verify(&self, token: &str) -> Verdict;
}
