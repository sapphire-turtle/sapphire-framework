//! Newer releases of a sapphire binary: finding them, and installing one.
//!
//! This is [corundum-framework-update](corundum_framework_update), re-exported, with
//! sapphire-turtle's minisign public key, [`UPDATE_PUBKEY`]. A [`ReleaseSource`] given that
//! key ([`ReleaseSource::pubkey`]) installs only a release file signed with it: the `<file>.sig`
//! beside it in the release, which sapphire-turtle/.github's `release-binary.yml` makes with
//! `sign: true` (#402). `AppServer::releases` gives an application's source the key unless it
//! names its own.

pub use corundum_framework_update::*;

/// sapphire-turtle's minisign public key (key ID F4A73F39AA17C8EB): the base64 of its `.pub`
/// file. The organization's `PACKAGER_SIGN_PRIVATE_KEY` signs, with it, the desktop packages
/// and their updates (sapphire-turtle/.github's `release-packager.yml`) and the bare binaries
/// (`release-binary.yml` with `sign: true`). A file signed otherwise is not installed.
pub const UPDATE_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEY0QTczRjM5QUExN0M4RUIKUldUcnlCZXFPVCtuOUJ6Zld0MkIzaW5wUXlsdlBBSll2U3k1dEJtT0VBYXV6ZHFtYlVNcWF5aHEK";
