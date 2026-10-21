//! The settings registry: which settings exist, who owns them, and what they resolve to.
//!
//! Each process that has settings (the bridge, an app server) is a [`SettingsOwner`]. A
//! [`Registry`] merges the owners' definitions into one namespace keyed by TOML path;
//! [`list`] and [`get`] read the scope files and resolve each key's effective value;
//! [`apply`] writes a batch of changes, validated as a whole, keeping comments.

mod error;
mod owner;
mod read;
mod registry;
mod store;

pub use error::{Error, Result};
pub use owner::{FieldError, SettingsOwner, Staged};
pub use read::{StartState, get, list};
pub use registry::Registry;
pub use sapphire_bridge_api::settings::*;
pub use store::{Applied, Op, apply, write_atomic, write_private};
