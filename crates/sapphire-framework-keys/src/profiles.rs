//! What a key may do: its profile, and the access the application gives that profile.
//!
//! corundum stores a profile id with every key and leaves its meaning to the application.
//! The framework offers three, [`READ_ONLY`], [`READ_WRITE`] and [`ADMIN`]
//! ([`KeyProfiles::standard`]); an application with profiles of its own (sapphire-agent's
//! room profiles) gives its list and says what [`Access`] each has
//! ([`KeyProfiles::new`]). The framework enforces the access on its own routes: `/mcp`'s
//! tools and the application's HTTP methods.

use std::sync::Arc;

pub use corundum_framework_keys::{Profile, ProfileId};

/// May read: `/mcp`'s read-only tools, and `GET` / `HEAD` on the application's routes.
pub const READ_ONLY: &str = "read-only";
/// May read and write.
pub const READ_WRITE: &str = "read-write";
/// May read and write; kept for operations only an administrator may do.
pub const ADMIN: &str = "admin";

/// The profile a key stored without one is given (#364's keys, the imported external
/// devices): the least.
pub const MIGRATED: &str = READ_ONLY;

/// What a profile lets a key do, least first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Access {
    /// Read only.
    Read,
    /// Read and write.
    Write,
    /// Read, write and administer.
    Admin,
}

impl Access {
    /// Whether it may change anything.
    pub fn may_write(self) -> bool {
        self >= Access::Write
    }
}

type Resolve = dyn Fn(&ProfileId) -> Option<Access> + Send + Sync;

/// The profiles an application offers, and what each may do.
#[derive(Clone)]
pub struct KeyProfiles {
    list: Arc<dyn Fn() -> Vec<Profile> + Send + Sync>,
    resolve: Arc<Resolve>,
}

impl std::fmt::Debug for KeyProfiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyProfiles")
            .field("list", &(self.list)())
            .finish_non_exhaustive()
    }
}

impl Default for KeyProfiles {
    fn default() -> Self {
        KeyProfiles::standard()
    }
}

impl KeyProfiles {
    /// The framework's three: [`READ_ONLY`], [`READ_WRITE`] and [`ADMIN`].
    pub fn standard() -> KeyProfiles {
        KeyProfiles::new(
            || {
                [
                    (READ_ONLY, "Read only"),
                    (READ_WRITE, "Read and write"),
                    (ADMIN, "Admin"),
                ]
                .into_iter()
                .map(|(id, label)| Profile {
                    id: ProfileId::new(id).expect("a profile id is not empty"),
                    label: label.to_owned(),
                })
                .collect()
            },
            |id| match id.as_str() {
                READ_ONLY => Some(Access::Read),
                READ_WRITE => Some(Access::Write),
                ADMIN => Some(Access::Admin),
                _ => None,
            },
        )
    }

    /// An application's own: `list` gives the profiles a new key may take (asked each time,
    /// so it may read the application's settings), `resolve` what a stored profile may do,
    /// `None` for one the application no longer has (its keys are refused with 403).
    pub fn new(
        list: impl Fn() -> Vec<Profile> + Send + Sync + 'static,
        resolve: impl Fn(&ProfileId) -> Option<Access> + Send + Sync + 'static,
    ) -> KeyProfiles {
        KeyProfiles {
            list: Arc::new(list),
            resolve: Arc::new(resolve),
        }
    }

    /// The profiles a new key may take.
    pub fn list(&self) -> Vec<Profile> {
        (self.list)()
    }

    /// What a key with `profile` may do; `None` if the application has no such profile.
    pub fn access(&self, profile: &ProfileId) -> Option<Access> {
        (self.resolve)(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> ProfileId {
        ProfileId::new(s).unwrap()
    }

    #[test]
    fn the_standard_profiles_and_their_access() {
        let p = KeyProfiles::standard();
        let ids: Vec<String> = p.list().into_iter().map(|p| p.id.to_string()).collect();
        assert_eq!(ids, [READ_ONLY, READ_WRITE, ADMIN]);
        assert_eq!(p.access(&id(READ_ONLY)), Some(Access::Read));
        assert_eq!(p.access(&id(READ_WRITE)), Some(Access::Write));
        assert_eq!(p.access(&id(ADMIN)), Some(Access::Admin));
        assert_eq!(p.access(&id("room-1")), None);
    }

    #[test]
    fn only_read_may_not_write() {
        assert!(!Access::Read.may_write());
        assert!(Access::Write.may_write());
        assert!(Access::Admin.may_write());
    }

    #[test]
    fn an_app_gives_its_own() {
        let p = KeyProfiles::new(
            || {
                vec![Profile {
                    id: ProfileId::new("room-1").unwrap(),
                    label: "Room 1".into(),
                }]
            },
            |id| (id.as_str() == "room-1").then_some(Access::Write),
        );
        assert_eq!(p.list().len(), 1);
        assert_eq!(p.access(&id("room-1")), Some(Access::Write));
        assert_eq!(p.access(&id(READ_ONLY)), None);
    }
}
