//! The served workspace's API keys (#364): the HTTP layer's verifier.
//!
//! The keys live in the workspace (`<root>/.<app>/api_keys/`, see
//! [`sapphire_framework_keys::store_dir`]), so they sync with it and no bridge is asked.

mod import;

pub use import::{IMPORTED_MARKER, ImportReport, import_external_devices};

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sapphire_framework_keys::{
    Authenticated, KeyProfiles, KeyVerifier, Verdict, Verifier, fill_missing_profiles, store_dir,
};
use sapphire_workspace::AppContext;

use crate::selection::SelectionFile;

/// Checks tokens against the keys of the workspace this application serves.
///
/// It finds the served root in the application's selection file (`workspace.toml`, which
/// the server rewrites on a switch), parsing it again only when its text changed — not its
/// modification time, which two writes within one tick share — and keeps one
/// [`KeyVerifier`] — and so one cache — per root. With no workspace
/// served, every token is refused. A key's profile gives its [`Access`]
/// ([`KeyProfiles::standard`] unless [`profiles`](Self::profiles) says otherwise); a key
/// of a profile the application does not offer is [`Verdict::Forbidden`].
///
/// [`Access`]: sapphire_framework_keys::Access
pub struct WorkspaceKeyVerifier {
    app: String,
    file: SelectionFile,
    ttl: Option<Duration>,
    profiles: KeyProfiles,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The selection file's text when last parsed (`None` inside: no file); `None` before
    /// the first.
    text: Option<Option<String>>,
    root: Option<PathBuf>,
    verifier: Option<Arc<KeyVerifier>>,
}

impl WorkspaceKeyVerifier {
    /// The verifier for `ctx`'s application, on its own selection file.
    pub fn for_app(ctx: &'static AppContext) -> WorkspaceKeyVerifier {
        WorkspaceKeyVerifier::at(ctx.app_name, SelectionFile::for_app(ctx))
    }

    /// The verifier for `app_name`, on the selection file `file`.
    pub fn at(app_name: &str, file: SelectionFile) -> WorkspaceKeyVerifier {
        WorkspaceKeyVerifier {
            app: app_name.to_owned(),
            file,
            ttl: None,
            profiles: KeyProfiles::standard(),
            state: Mutex::new(State::default()),
        }
    }

    /// The application's own profiles, instead of the framework's three.
    pub fn profiles(mut self, profiles: KeyProfiles) -> WorkspaceKeyVerifier {
        self.profiles = profiles;
        self
    }

    /// Trust a success for `ttl` instead of corundum's default.
    pub fn cache_ttl(mut self, ttl: Duration) -> WorkspaceKeyVerifier {
        self.ttl = Some(ttl);
        self
    }

    /// The verifier for the root served now, if one is.
    fn current(&self) -> Option<Arc<KeyVerifier>> {
        // The file is a few lines; reading it is cheaper than being wrong about a switch.
        let text = match std::fs::read_to_string(self.file.path()) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                tracing::warn!("could not read the served workspace: {err}");
                return None;
            }
        };
        let mut state = self.state.lock().expect("key verifier state");
        if state.text.as_ref() != Some(&text) {
            state.text = Some(text);
            // The same text is not parsed (nor warned about) again.
            let root = match self.file.load(&self.app) {
                Ok(selection) => selection.map(|s| s.root),
                Err(err) => {
                    tracing::warn!("could not read the served workspace: {err}");
                    None
                }
            };
            if root != state.root {
                state.verifier = root.as_deref().map(|root| {
                    let dir = store_dir(&self.app, root);
                    // Keys from before profiles, which corundum would skip.
                    if let Err(err) = fill_missing_profiles(&dir) {
                        tracing::warn!("could not give the API keys a profile: {err}");
                    }
                    let verifier = KeyVerifier::new(&dir);
                    Arc::new(match self.ttl {
                        Some(ttl) => verifier.cache_ttl(ttl),
                        None => verifier,
                    })
                });
                state.root = root;
            }
        }
        state.verifier.clone()
    }
}

/// Import the workgroup's external devices into the workspace at `root`, once per host
/// (see [`import_external_devices`]). Failures are logged and retried at the next start.
pub(crate) fn import_once(ctx: &AppContext, root: &std::path::Path) {
    let bridge_dir = match std::env::var_os("SAPPHIRE_BRIDGE_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => match sapphire_workspace::dirs::data_dir() {
            Some(dir) => dir.join(sapphire_ipc::BRIDGE_DIR_NAME),
            None => return,
        },
    };
    match import_external_devices(ctx.app_name, ctx.data_dir(), &bridge_dir, root) {
        Ok(ImportReport { imported: 0, .. }) => {}
        Ok(report) => tracing::info!(
            imported = report.imported,
            "the workgroup's external devices are now this workspace's API keys; the workgroup's ledger is no longer read: manage them with `api-key`"
        ),
        Err(err) => tracing::warn!("could not import the external devices: {err}"),
    }
}

#[async_trait::async_trait]
impl Verifier for WorkspaceKeyVerifier {
    async fn verify(&self, token: &str) -> Verdict {
        let Some(verifier) = self.current() else {
            return Verdict::Refused;
        };
        let token = token.to_owned();
        // A miss reads the store's files.
        match tokio::task::spawn_blocking(move || verifier.verify(&token)).await {
            Ok(corundum_framework_keys::Verdict::Accepted(key)) => {
                match self.profiles.access(&key.profile) {
                    Some(access) => Verdict::Accepted(Authenticated {
                        id: key.id,
                        name: key.name,
                        profile: key.profile,
                        access,
                    }),
                    None => Verdict::Forbidden,
                }
            }
            Ok(corundum_framework_keys::Verdict::Refused) => Verdict::Refused,
            Err(err) => {
                tracing::error!("the key check did not finish: {err}");
                Verdict::Refused
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::Selection;
    use sapphire_framework_keys::open_store;

    const APP: &str = "sapphire-keytest";

    fn rw() -> sapphire_framework_keys::ProfileId {
        sapphire_framework_keys::ProfileId::new(sapphire_framework_keys::READ_WRITE).unwrap()
    }

    struct Host {
        _tmp: tempfile::TempDir,
        file: SelectionFile,
        a: PathBuf,
        b: PathBuf,
    }

    fn host() -> Host {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let file = SelectionFile::at(tmp.path().join("config").join("workspace.toml"));
        Host {
            _tmp: tmp,
            file,
            a,
            b,
        }
    }

    fn select(file: &SelectionFile, root: &std::path::Path) {
        // A switch lands in a later instant than the read before it, as a real one does.
        std::thread::sleep(Duration::from_millis(20));
        file.save(&Selection {
            root: root.to_owned(),
            sync: false,
        })
        .unwrap();
    }

    fn accepted(v: Verdict) -> Option<String> {
        match v {
            Verdict::Accepted(who) => Some(who.name),
            Verdict::Forbidden | Verdict::Refused => None,
        }
    }

    #[tokio::test]
    async fn the_access_is_the_profile_s() {
        let h = host();
        select(&h.file, &h.a);
        let read_only = sapphire_framework_keys::ProfileId::new("read-only").unwrap();
        let (_, token) = open_store(APP, &h.a)
            .unwrap()
            .add("p", None, read_only)
            .unwrap();
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone());
        let Verdict::Accepted(who) = v.verify(token.expose()).await else {
            panic!("refused");
        };
        assert_eq!(who.profile.as_str(), "read-only");
        assert_eq!(who.access, sapphire_framework_keys::Access::Read);
    }

    #[tokio::test]
    async fn a_key_of_a_profile_the_app_does_not_offer_is_forbidden() {
        let h = host();
        select(&h.file, &h.a);
        let gone = sapphire_framework_keys::ProfileId::new("room-gone").unwrap();
        let (_, token) = open_store(APP, &h.a).unwrap().add("p", None, gone).unwrap();
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone());
        assert_eq!(v.verify(token.expose()).await, Verdict::Forbidden);
    }

    #[tokio::test]
    async fn an_app_s_own_profiles_decide() {
        let h = host();
        select(&h.file, &h.a);
        let room = sapphire_framework_keys::ProfileId::new("room-1").unwrap();
        let (_, token) = open_store(APP, &h.a).unwrap().add("p", None, room).unwrap();
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone()).profiles(
            sapphire_framework_keys::KeyProfiles::new(Vec::new, |id| {
                (id.as_str() == "room-1").then_some(sapphire_framework_keys::Access::Write)
            }),
        );
        let Verdict::Accepted(who) = v.verify(token.expose()).await else {
            panic!("refused");
        };
        assert_eq!(who.access, sapphire_framework_keys::Access::Write);
    }

    // A key file from before profiles, synced into the served workspace: read-only.
    #[tokio::test]
    async fn a_key_without_a_profile_is_read_only() {
        let h = host();
        let (key, token) = open_store(APP, &h.a).unwrap().add("p", None, rw()).unwrap();
        let file = store_dir(APP, &h.a).join(format!("{}.toml", key.id));
        let text = std::fs::read_to_string(&file).unwrap();
        let old: String = text
            .lines()
            .filter(|l| !l.starts_with("profile"))
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(&file, old).unwrap();
        select(&h.file, &h.a);
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone());
        let Verdict::Accepted(who) = v.verify(token.expose()).await else {
            panic!("refused");
        };
        assert_eq!(who.access, sapphire_framework_keys::Access::Read);
    }

    #[tokio::test]
    async fn key_in_workspace_is_accepted() {
        let h = host();
        select(&h.file, &h.a);
        let (_, token) = open_store(APP, &h.a).unwrap().add("p", None, rw()).unwrap();
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone());
        assert_eq!(
            accepted(v.verify(token.expose()).await).as_deref(),
            Some("p")
        );
        assert_eq!(accepted(v.verify("sapphire-ak-wrong").await), None);
    }

    #[tokio::test]
    async fn retire_takes_effect_after_ttl() {
        let h = host();
        select(&h.file, &h.a);
        let mut store = open_store(APP, &h.a).unwrap();
        let (_, token) = store.add("p", None, rw()).unwrap();
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone()).cache_ttl(Duration::from_millis(300));
        assert!(accepted(v.verify(token.expose()).await).is_some());
        store.retire("p").unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(accepted(v.verify(token.expose()).await), None);
    }

    #[tokio::test]
    async fn copied_key_file_is_accepted() {
        let h = host();
        select(&h.file, &h.a);
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone());
        // Made on another host's copy of the workspace, then synced here.
        let elsewhere = tempfile::tempdir().unwrap();
        let (key, token) = open_store(APP, elsewhere.path())
            .unwrap()
            .add("p", None, rw())
            .unwrap();
        let name = format!("{}.toml", key.id);
        std::fs::create_dir_all(store_dir(APP, &h.a)).unwrap();
        std::fs::copy(
            store_dir(APP, elsewhere.path()).join(&name),
            store_dir(APP, &h.a).join(&name),
        )
        .unwrap();
        assert!(accepted(v.verify(token.expose()).await).is_some());
    }

    #[tokio::test]
    async fn switch_changes_the_keys() {
        let h = host();
        let (_, in_a) = open_store(APP, &h.a).unwrap().add("a", None, rw()).unwrap();
        let (_, in_b) = open_store(APP, &h.b).unwrap().add("b", None, rw()).unwrap();
        select(&h.file, &h.a);
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone());
        assert!(accepted(v.verify(in_a.expose()).await).is_some());
        select(&h.file, &h.b);
        assert_eq!(
            accepted(v.verify(in_a.expose()).await),
            None,
            "the old workspace's key"
        );
        assert!(accepted(v.verify(in_b.expose()).await).is_some());
    }

    #[tokio::test]
    async fn missing_root_refuses() {
        let h = host();
        let (_, token) = open_store(APP, &h.a).unwrap().add("p", None, rw()).unwrap();
        select(&h.file, &h.a);
        std::fs::remove_dir_all(&h.a).unwrap();
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone());
        assert_eq!(accepted(v.verify(token.expose()).await), None);
        assert!(
            !h.a.exists(),
            "the verifier must not recreate the workspace"
        );
    }

    #[tokio::test]
    async fn no_selection_refuses() {
        let h = host();
        let (_, token) = open_store(APP, &h.a).unwrap().add("p", None, rw()).unwrap();
        h.file.clear().unwrap();
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone());
        assert_eq!(accepted(v.verify(token.expose()).await), None);
    }

    #[tokio::test]
    async fn a_switch_in_the_same_tick_is_seen() {
        let h = host();
        let (_, in_a) = open_store(APP, &h.a).unwrap().add("a", None, rw()).unwrap();
        let (_, in_b) = open_store(APP, &h.b).unwrap().add("b", None, rw()).unwrap();
        let save = |root: &std::path::Path| {
            h.file
                .save(&Selection {
                    root: root.to_owned(),
                    sync: false,
                })
                .unwrap()
        };
        save(&h.a);
        let v = WorkspaceKeyVerifier::at(APP, h.file.clone());
        assert!(accepted(v.verify(in_a.expose()).await).is_some());
        let tick = std::fs::metadata(h.file.path())
            .unwrap()
            .modified()
            .unwrap();
        // Rewritten within the file system's timestamp tick (1 s on some): same mtime.
        save(&h.b);
        std::fs::File::options()
            .write(true)
            .open(h.file.path())
            .unwrap()
            .set_modified(tick)
            .unwrap();
        assert_eq!(
            accepted(v.verify(in_a.expose()).await),
            None,
            "the old workspace's key"
        );
        assert!(accepted(v.verify(in_b.expose()).await).is_some());
    }
}
