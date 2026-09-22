//! The privilege-separation types an application's configuration is written in.
//!
//! These types used to live in `sapphire-framework-server::privilege`; they moved here so a
//! service installation can describe a unit's privilege separation without pulling in the
//! server's runtime stack (tokio, redb and the rest), and so `-server` — whose CLI embeds
//! this crate's service commands — can re-export them without a dependency cycle. See the
//! process-architecture spec, §3.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which OS user to become, by name or by numeric id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserSpec {
    /// A login name, looked up in the password database.
    Name(String),
    /// A numeric user id.
    Uid(u32),
}

impl std::str::FromStr for UserSpec {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.is_empty() {
            return Err("a user must be named".to_owned());
        }
        // All digits is a uid; anything else is a name. A login name made entirely of digits
        // is legal on some systems and unreachable here — say so rather than guessing.
        match s.parse::<u32>() {
            Ok(uid) => Ok(UserSpec::Uid(uid)),
            Err(_) => Ok(UserSpec::Name(s.to_owned())),
        }
    }
}

impl std::fmt::Display for UserSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserSpec::Name(name) => f.write_str(name),
            UserSpec::Uid(uid) => write!(f, "{uid}"),
        }
    }
}

impl Serialize for UserSpec {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for UserSpec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The lower-privileged helper an application wants forked before the drop.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HelperSpec {
    /// The user the helper runs as. Must not be root, and should not be `run_as` either —
    /// a helper with the same identity separates nothing.
    pub user: UserSpec,
    /// The program to run.
    pub program: PathBuf,
    /// Its arguments.
    #[serde(default)]
    pub args: Vec<String>,
}

/// What an application wants privilege separation to do.
///
/// Deserialised from the application's configuration file. Its presence also tells the
/// application's CLI not to try starting the server itself (spec §2.6): a process running as
/// the human user cannot spawn a root one.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrivilegeConfig {
    /// The user that owns the workspace, the cache and the sockets.
    pub run_as: UserSpec,
    /// An optional helper forked before the drop, under a different user.
    #[serde(default)]
    pub helper: Option<HelperSpec>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_spec_parses_a_name_or_a_number() {
        assert_eq!(
            "alice".parse::<UserSpec>().unwrap(),
            UserSpec::Name("alice".into())
        );
        assert_eq!("1000".parse::<UserSpec>().unwrap(), UserSpec::Uid(1000));
    }

    #[test]
    fn an_empty_user_spec_is_refused() {
        assert!("".parse::<UserSpec>().is_err());
    }

    #[test]
    fn a_user_spec_round_trips_through_its_text_form() {
        let spec = UserSpec::Name("alice".into());
        assert_eq!(spec.to_string().parse::<UserSpec>().unwrap(), spec);
        let spec = UserSpec::Uid(1000);
        assert_eq!(spec.to_string().parse::<UserSpec>().unwrap(), spec);
    }

    #[test]
    fn a_user_spec_deserialises_from_a_plain_string() {
        assert_eq!(
            serde_json::from_str::<UserSpec>(r#""alice""#).unwrap(),
            UserSpec::Name("alice".into())
        );
    }

    #[test]
    fn a_configuration_round_trips_through_toml() {
        let text = r#"
run_as = "alice"

[helper]
user = "sapphire-agent-tools"
program = "/usr/lib/sapphire-agent/tool-broker"
args = ["--quiet"]
"#;
        let config: PrivilegeConfig = toml::from_str(text).unwrap();
        assert_eq!(config.run_as, UserSpec::Name("alice".into()));
        let helper = config.helper.unwrap();
        assert_eq!(helper.user, UserSpec::Name("sapphire-agent-tools".into()));
        assert_eq!(helper.args, vec!["--quiet".to_owned()]);
    }
}
