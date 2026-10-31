//! Where a workspace's API keys are.

use std::path::{Path, PathBuf};

use corundum_framework_keys::KeyStore;

/// What a new token starts with, so a leaked one says what it is.
pub const TOKEN_PREFIX: &str = "sapphire-ak-";

/// The directory of `app_name`'s API keys in the workspace at `root`:
/// `<root>/.<app>/api_keys`. `.<app>/` syncs with the workspace.
pub fn store_dir(app_name: &str, root: &Path) -> PathBuf {
    root.join(format!(".{app_name}")).join("api_keys")
}

/// The store of `app_name`'s API keys in the workspace at `root`, created if absent, its new
/// tokens starting with [`TOKEN_PREFIX`].
pub fn open_store(app_name: &str, root: &Path) -> corundum_framework_keys::Result<KeyStore> {
    Ok(KeyStore::open(&store_dir(app_name, root))?.token_prefix(TOKEN_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_is_in_the_app_directory_of_the_workspace() {
        assert_eq!(
            store_dir("sapphire-agent", Path::new("/w")),
            Path::new("/w/.sapphire-agent/api_keys")
        );
    }

    #[test]
    fn new_tokens_have_the_prefix() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open_store("sapphire-test", root.path()).unwrap();
        let (_, token) = store.add("p", None, None).unwrap();
        assert!(token.expose().starts_with(TOKEN_PREFIX));
        assert!(store_dir("sapphire-test", root.path()).is_dir());
    }
}
