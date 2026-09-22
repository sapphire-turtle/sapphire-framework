//! Labelled API keys in a plaintext file, plus bearer-token middleware.
//!
//! This crate holds the two pieces every HTTP entry point of an application
//! shares: the key file itself ([`KeyStore`], [`KeyEntry`]), and the axum
//! middleware ([`protect`], feature `axum`) that checks
//! `Authorization: Bearer <token>` against it before a request reaches any
//! route — the framework's own router and the app's own routes with the *same*
//! key, so `/rpc` is never guarded while `/mcp` is wide open.
//!
//! It is its own crate because sync no longer needs HTTP, but those HTTP
//! endpoints do — and a caller that only wants [`KeyStore`] must not link a
//! web framework (see `extraction_tests::the_crate_does_not_pull_in_axum_by_default`).
//! Splitting it out of the remote server is framework issue **#103**.
//!
//! ```no_run
//! # use std::path::Path;
//! # use sapphire_framework_keys::KeyStore;
//! let keys = KeyStore::load(Path::new("/etc/sapphire/keys.toml")).unwrap();
//! if let Some(entry) = keys.authenticate("token from the request") {
//!     println!("{}", entry.id);
//! }
//! ```

#[cfg(feature = "axum")]
mod auth;
mod config;
mod error;
mod keys;

#[cfg(feature = "axum")]
pub use auth::{Authenticated, protect};
pub use config::AuthConfig;
pub use error::{Error, Result};
pub use keys::{KeyEntry, KeyStore};

// `Authenticated::key_id` と `KeyEntry::id` の型。アプリが uuid を自前で
// 依存に足さなくても名指しできるように出しておく。
pub use uuid::Uuid;
// `Authenticated::device_id` と `KeyEntry::device_id` の型。アプリが grain-id
// を自前で依存に足さなくても名指しできるように出しておく。
pub use grain_id::GrainId;
