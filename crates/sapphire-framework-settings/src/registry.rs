//! The merged namespace of every owner's settings.

use std::collections::HashMap;
use std::sync::Arc;

use sapphire_bridge_api::settings::{Kind, Scope, SettingDef};

use crate::owner::SettingsOwner;
use crate::{Error, Result};

/// Every owner's definitions in one namespace, checked for clashes.
pub struct Registry {
    owners: Vec<Arc<dyn SettingsOwner>>,
    /// Definitions with the index of their owner, in owner then definition order.
    defs: Vec<(SettingDef, usize)>,
    by_key: HashMap<String, usize>,
}

impl Registry {
    /// Merge the owners' definitions.
    ///
    /// `reserved_for` maps a top-level name (a key's first segment) to the one owner, by
    /// index, allowed to use it; names not listed are free. A key defined twice is
    /// [`Error::Duplicate`], a reserved name used by another owner is [`Error::Reserved`],
    /// and a definition without scopes or a secret outside the device scope is
    /// [`Error::InvalidDef`].
    pub fn new(
        owners: Vec<Arc<dyn SettingsOwner>>,
        reserved_for: &[(&str, usize)],
    ) -> Result<Registry> {
        let mut defs = Vec::new();
        let mut by_key = HashMap::new();
        for (idx, owner) in owners.iter().enumerate() {
            for def in owner.defs() {
                check(&def)?;
                let top = def.key.split('.').next().unwrap_or_default();
                if reserved_for
                    .iter()
                    .any(|(name, allowed)| *name == top && *allowed != idx)
                {
                    return Err(Error::Reserved {
                        name: top.to_string(),
                        key: def.key,
                    });
                }
                if by_key.insert(def.key.clone(), defs.len()).is_some() {
                    return Err(Error::Duplicate(def.key));
                }
                defs.push((def, idx));
            }
        }
        Ok(Registry {
            owners,
            defs,
            by_key,
        })
    }

    /// The definition of `key` and the owner that holds its files.
    pub fn def(&self, key: &str) -> Result<(&SettingDef, &dyn SettingsOwner)> {
        let i = *self
            .by_key
            .get(key)
            .ok_or_else(|| Error::UnknownKey(key.to_string()))?;
        let (def, owner) = &self.defs[i];
        Ok((def, self.owners[*owner].as_ref()))
    }

    /// The definitions whose key is `prefix` or starts with `prefix.`.
    pub fn under(&self, prefix: &str) -> Vec<&SettingDef> {
        self.defs
            .iter()
            .map(|(def, _)| def)
            .filter(|def| {
                def.key == prefix
                    || def
                        .key
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('.'))
            })
            .collect()
    }

    /// Every definition.
    pub fn schema(&self) -> Vec<SettingDef> {
        self.defs.iter().map(|(def, _)| def.clone()).collect()
    }

    /// Every definition with its owner.
    pub(crate) fn entries(&self) -> impl Iterator<Item = (&SettingDef, &dyn SettingsOwner)> {
        self.defs
            .iter()
            .map(|(def, owner)| (def, self.owners[*owner].as_ref()))
    }
}

/// The rules every definition keeps: it names its home (the first scope), and a secret
/// lives on the device only — its file is this host's, never synced.
fn check(def: &SettingDef) -> Result<()> {
    let invalid = |message: &str| Error::InvalidDef {
        key: def.key.clone(),
        message: message.to_string(),
    };
    if def.scopes.is_empty() {
        return Err(invalid("it names no scope"));
    }
    if def.kind == Kind::Secret && def.scopes != [Scope::Device] {
        return Err(invalid("a secret's only scope must be the device"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::owner::test_support::{TestOwner, def};
    use std::sync::Arc;

    fn owner(keys: &[&str]) -> Arc<dyn SettingsOwner> {
        Arc::new(TestOwner::new(
            keys.iter().map(|k| def(k, Kind::Bool)).collect(),
        ))
    }

    #[test]
    fn a_def_without_scopes_is_rejected() {
        let mut d = def("a.b", Kind::Bool);
        d.scopes = vec![];
        let err = Registry::new(vec![Arc::new(TestOwner::new(vec![d]))], &[])
            .err()
            .unwrap();
        assert!(matches!(err, Error::InvalidDef { key, .. } if key == "a.b"));
    }

    #[test]
    fn a_secret_outside_the_device_scope_is_rejected() {
        for scopes in [
            vec![Scope::Workgroup],
            vec![Scope::Device, Scope::Workgroup],
        ] {
            let mut d = def("a.key", Kind::Secret);
            d.scopes = scopes;
            let err = Registry::new(vec![Arc::new(TestOwner::new(vec![d]))], &[])
                .err()
                .unwrap();
            assert!(matches!(err, Error::InvalidDef { key, .. } if key == "a.key"));
        }
        let mut d = def("a.key", Kind::Secret);
        d.scopes = vec![Scope::Device];
        assert!(Registry::new(vec![Arc::new(TestOwner::new(vec![d]))], &[]).is_ok());
    }

    #[test]
    fn duplicate_keys_across_owners_are_rejected() {
        let err = Registry::new(vec![owner(&["net.relays"]), owner(&["net.relays"])], &[])
            .err()
            .unwrap();
        assert!(matches!(err, Error::Duplicate(k) if k == "net.relays"));
    }

    #[test]
    fn a_reserved_name_is_rejected_for_another_owner() {
        let err = Registry::new(
            vec![owner(&["net.a"]), owner(&["embed.x"])],
            &[("embed", 0)],
        )
        .err()
        .unwrap();
        assert!(
            matches!(err, Error::Reserved { key, name } if key == "embed.x" && name == "embed")
        );
    }

    #[test]
    fn the_reserved_owner_and_free_names_are_accepted() {
        let reg = Registry::new(
            vec![owner(&["embed.x", "embed.y.z"]), owner(&["mine.a"])],
            &[("embed", 0)],
        )
        .unwrap();
        assert_eq!(reg.under("embed").len(), 2);
        assert_eq!(reg.under("embed.y").len(), 1);
        assert_eq!(reg.under("emb").len(), 0);
        assert_eq!(reg.schema().len(), 3);
        assert!(reg.def("mine.a").is_ok());
        assert!(matches!(
            reg.def("nope").err().unwrap(),
            Error::UnknownKey(_)
        ));
    }
}
