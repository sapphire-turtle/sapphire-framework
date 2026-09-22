//! Registering a sapphire application with the OS service manager.
//!
//! An application describes itself with a [`ServiceSpec`]; this crate turns that into a unit
//! file and hands it to the OS service manager — one real implementation per platform, plus
//! a recording one for tests, so no test ever touches the host's service manager. See the
//! process-architecture spec, §3.2 and §9 step 10.
//!
//! The decision table this crate implements:
//!
//! | Invoked as | Unit | Activation |
//! |---|---|---|
//! | a regular user | `~/.config/systemd/user/<app>.service` | `systemctl --user enable --now` |
//! | root, including `sudo` | `/etc/systemd/system/<app>.service`, `After=network-online.target` | `systemctl enable --now` |
//!
//! And for the user a system unit runs as: [`RunAs::Root`] carries no `User=` (the app drops
//! privileges itself); [`RunAs::InvokingUser`] takes `$SUDO_USER`, and refuses to run with an
//! explanation when neither it nor `--run-as` is available — a root `sapphire-bridge` would
//! put the bridge directory under `/root` and create synced files owned by root.
//!
//! This crate also owns the privilege-separation configuration types ([`UserSpec`],
//! [`HelperSpec`], [`PrivilegeConfig`]), which `-server` re-exports: the server's CLI embeds
//! this crate's service commands, so the dependency runs `-server` → `-service`, never the
//! other way. The scope decision lands first; unit rendering and the install flow build on
//! it.

#![warn(missing_docs)]

pub mod error;
pub mod privilege;
pub mod scope;
pub mod systemd;

pub use error::{Error, Result};
pub use privilege::{HelperSpec, PrivilegeConfig, UserSpec};
pub use scope::{
    Environment, InstallContext, Os, PostInstall, RunAs, Scope, ServiceSpec, resolve_scope,
    resolve_target_user,
};
pub use systemd::{activation, linger_hint, render_unit, unit_path};
