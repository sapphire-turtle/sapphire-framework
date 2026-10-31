//! Bearer-token authentication for an application's HTTP routes, against the API keys of
//! the workspace the application serves (#364).
//!
//! An API key belongs to a client that reaches the application over HTTP instead of
//! syncing: a recording pendant, a remote ACP client, a webhook. The keys live in the
//! workspace (`<root>/.<app>/api_keys/`), so they sync with it, and every host serving the
//! workspace accepts them, with no bridge involved.
//!
//! - [`Verifier`]: who a presented token belongs to — [`Verdict::Accepted`] or refused.
//! - [`protect`] (feature `axum`): the layer that checks `Authorization: Bearer <token>`
//!   before a request reaches any route, the framework's and the app's own alike.
//! - [`store_dir`] and [`open_store`]: where a workspace's keys are, and the store on them.
//!
//! The application server's verifier, `WorkspaceKeyVerifier`, is in
//! sapphire-framework-server.

#[cfg(feature = "axum")]
mod auth;
mod config;
mod verify;

#[cfg(feature = "axum")]
pub use auth::protect;
pub use config::AuthConfig;
pub use verify::{Authenticated, KeyId, Verdict, Verifier};
