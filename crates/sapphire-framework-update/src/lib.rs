//! Newer releases of a sapphire binary: finding them, and installing one.
//!
//! A binary's releases are a GitHub repository's, tagged `<prefix><version>`
//! ([`ReleaseSource`]). [`latest_release`] finds the newest; the app server asks it once a
//! day and shows the answer in `status`. The `update` command ([`UpdateArgs`]) goes one step
//! further and replaces the running executable with the release's, but only:
//!
//! - with a newer version: never a downgrade;
//! - with the asset for this platform, `<asset>-<os>-<arch>[.exe]`, downloaded over HTTPS
//!   from the repository's own release: GitHub is the authority on what was released;
//! - when the executable was placed by hand, not by `cargo install` or a package manager,
//!   whose own bookkeeping a replaced file would contradict ([`InstallMethod`]).
//!
//! Restarting the running server or service afterwards stays with the user.

#![warn(missing_docs)]

mod install;
mod release;

pub use install::{InstallMethod, UpdateArgs, asset_name, install, install_method, platform, run};
pub use release::{Asset, Release, ReleaseSource, client, latest_release};

/// Errors from finding, fetching or installing a release.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Talking to GitHub failed.
    #[error("{0}")]
    Http(String),
    /// The repository has no release this source matches.
    #[error("{0}")]
    NoRelease(String),
    /// The release has no file for this platform.
    #[error("{0}")]
    NoAsset(String),
    /// The binary does not say where its releases are, or its own version is unusable.
    #[error("{0}")]
    NotConfigured(String),
    /// Reading the executable or writing its replacement failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// This crate's result.
pub type Result<T> = std::result::Result<T, Error>;
