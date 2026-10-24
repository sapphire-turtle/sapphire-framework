//! The decisions the views make, as plain functions, so they can be tested without egui.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use corundum_framework_gui::i18n::Message;
use grain_id::GrainId;
use sapphire_backend::protocol::{CurrentWorkspace, Topology};
use sapphire_bridge_api::{
    EmbedSettingsResult, PeerInfo, Slot, WorkgroupWorkspaceInfo, WorkspaceRoles,
};

/// The workspace's state, as one badge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Badge {
    /// Synced, with this many other devices in the workgroup.
    Syncing {
        /// Other devices.
        peers: usize,
        /// Whether the workspace syncs through a primary device.
        star: bool,
        /// Files this host has yet to give a vector (#188); 0 when none or not embedding.
        embedding_pending: u64,
    },
    /// Synced but paused, for this reason.
    Paused(String),
    /// Synced, and the last attempt failed.
    Error(String),
    /// Not synced.
    NotSynced,
    /// The folder or its marker is gone.
    Unreachable,
}

impl Badge {
    /// The text shown beside the workspace.
    pub fn label(&self) -> Message {
        match self {
            Badge::Syncing {
                peers,
                star,
                embedding_pending,
            } => Message::new("sapphire-badge-syncing")
                .arg("peers", *peers)
                .arg("star", if *star { "yes" } else { "no" })
                .arg("pending", *embedding_pending),
            Badge::Paused(why) => Message::new("sapphire-badge-paused").arg("reason", why.as_str()),
            Badge::Error(e) => Message::new("sapphire-badge-error").arg("reason", e.as_str()),
            Badge::NotSynced => Message::new("sapphire-badge-not-synced"),
            Badge::Unreachable => Message::new("sapphire-badge-unreachable"),
        }
    }
}

/// The badge for `entry`: unreachable beats everything, then not-synced, error, paused.
pub fn badge(entry: &CurrentWorkspace) -> Badge {
    if !entry.reachable {
        Badge::Unreachable
    } else if !entry.sync.enabled {
        Badge::NotSynced
    } else if let Some(e) = &entry.sync.last_error {
        Badge::Error(e.clone())
    } else if let Some(p) = &entry.sync.paused {
        Badge::Paused(p.clone())
    } else {
        Badge::Syncing {
            peers: entry.sync.peers,
            star: matches!(entry.sync.topology, Topology::Star { .. }),
            embedding_pending: entry.sync.embedding.as_ref().map_or(0, |e| e.pending),
        }
    }
}

/// The workgroup's workspaces of `app_name` other than the current one, by sync id.
pub fn others<'a>(
    ledger: &'a [WorkgroupWorkspaceInfo],
    current: Option<&CurrentWorkspace>,
    app_name: &str,
) -> Vec<&'a WorkgroupWorkspaceInfo> {
    let mine = current.and_then(|c| c.workspace_id);
    ledger
        .iter()
        .filter(|w| w.app_name == app_name)
        .filter(|w| mine != Some(w.workspace_id))
        .collect()
}

/// Keep `value`, the priority shown in `peer`'s row, as an edit only while it differs from
/// the bridge's priority. An unchanged row then follows changes made elsewhere.
pub fn record_edit(edits: &mut HashMap<GrainId, u8>, peer: &PeerInfo, value: u8) {
    if value == peer.priority {
        edits.remove(&peer.device_id);
    } else {
        edits.insert(peer.device_id, value);
    }
}

/// Drop the edits the bridge now agrees with, and those for devices no longer listed.
pub fn prune_edits(edits: &mut HashMap<GrainId, u8>, peers: &[PeerInfo]) {
    edits.retain(|id, p| peers.iter().any(|x| x.device_id == *id && x.priority != *p));
}

/// Whether `peer` is this host. An empty node id never matches.
pub fn is_this_device(peer: &PeerInfo, this_node_id: &str) -> bool {
    !this_node_id.is_empty() && peer.node_id == this_node_id
}

/// `input` trimmed, or `None` when nothing is left.
pub fn valid_name(input: &str) -> Option<String> {
    let t = input.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// Invite lifetimes offered: the id of the message naming each, and its seconds.
pub const TTL_CHOICES: [(&str, u64); 3] = [
    ("sapphire-ttl-hour", 3_600),
    ("sapphire-ttl-day", 86_400),
    ("sapphire-ttl-week", 604_800),
];

/// This host's name: the device-name default when founding a workgroup.
///
/// Not for joining — a join must repeat the name the invite was issued for.
pub fn default_device_name() -> String {
    ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "device".to_owned())
}

/// Whether `dir` is already a workspace of `app_name` (holds its `.{app_name}` marker).
fn has_marker(dir: &Path, app_name: &str) -> bool {
    dir.join(format!(".{app_name}")).is_dir()
}

/// `name` made safe as one folder name on every platform: path separators and the
/// characters Windows forbids become `-`, leading and trailing dots and spaces go, and
/// nothing left becomes `workspace`.
fn folder_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let trimmed = replaced.trim_matches(|c| c == '.' || c == ' ');
    if trimmed.is_empty() {
        "workspace".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Where "Bring to this host…" puts `workspace_name` when the user picked `picked`.
///
/// A picked folder that already is a workspace of `app_name` is used as it is; any other
/// folder gets a new one inside it, named after the workspace — picking `~/Documents` must
/// not turn all of `~/Documents` into a synced workspace.
pub fn bring_target(picked: &Path, workspace_name: &str, app_name: &str) -> PathBuf {
    if has_marker(picked, app_name) {
        picked.to_owned()
    } else {
        picked.join(folder_name(workspace_name))
    }
}

/// Where "Move…" puts the workspace at `root` when the user picked `picked`: a folder of
/// the same name inside it, which must not exist yet. The error is the message to show.
pub fn move_target(picked: &Path, root: &Path) -> Result<PathBuf, Message> {
    let name = root.file_name().ok_or_else(|| {
        Message::new("sapphire-move-unmovable")
            .arg("path", display_path(root).display().to_string())
    })?;
    let target = picked.join(name);
    if target.exists() {
        return Err(Message::new("sapphire-move-exists")
            .arg("folder", display_path(picked).display().to_string())
            .arg("name", name.to_string_lossy().into_owned()));
    }
    Ok(target)
}

/// Whether "New…" may create a workspace in `picked`: it must already be a workspace of
/// `app_name`, or be empty. The error is the message to show.
pub fn new_target_ok(picked: &Path, app_name: &str) -> Result<(), Message> {
    if has_marker(picked, app_name) {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(picked).map_err(|e| {
        Message::new("sapphire-new-unreadable")
            .arg("path", picked.display().to_string())
            .arg("error", e.to_string())
    })?;
    if entries.next().is_none() {
        Ok(())
    } else {
        Err(Message::new("sapphire-new-not-empty")
            .arg("path", display_path(picked).display().to_string()))
    }
}

/// `p` as a person writes it: without the Windows verbatim prefix `\\?\` that
/// canonicalisation adds (`\\?\UNC\server\share` becomes `\\server\share`).
///
/// For showing a path and handing it to the file manager only; the registry keeps the
/// canonical form. Plain string logic, the same on every platform; a path that is not
/// UTF-8 is returned unchanged.
pub fn display_path(p: &Path) -> PathBuf {
    let Some(s) = p.to_str() else {
        return p.to_owned();
    };
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        p.to_owned()
    }
}

/// The message ids of the roles `peer` holds in any workspace, each named once: what its
/// device row shows.
pub fn role_badges(peer: &PeerInfo, roles: &[WorkspaceRoles]) -> Vec<&'static str> {
    let mut out = Vec::new();
    if roles.iter().any(|r| r.primary == Some(peer.device_id)) {
        out.push("sapphire-role-primary");
    }
    if roles.iter().any(|r| r.secondary == Some(peer.device_id)) {
        out.push("sapphire-role-secondary");
    }
    out
}

/// The first eight characters of an id, for a compact column.
pub fn short_id(id: &GrainId) -> String {
    id.to_string().chars().take(8).collect()
}

/// The workspace's title: its folder's name.
pub fn display_name(entry: &CurrentWorkspace) -> String {
    entry
        .root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| entry.root.display().to_string())
}

// ── embedding ───────────────────────────────────────────────────────────────

/// The embedding screen's status line, and whether it is a warning. The bridge's note
/// reaches the message as it wrote it.
pub fn embedding_status(s: &EmbedSettingsResult) -> (Message, bool) {
    let note = s.info.note.as_ref();
    match (s.active, &s.info.model) {
        (Some(slot), Some(model)) => {
            let line = Message::new(if note.is_some() {
                "sapphire-embedding-active-note"
            } else {
                "sapphire-embedding-active"
            })
            .arg("slot", slot.to_string())
            .arg("model", model.model.as_str())
            .arg("dimension", model.dimension)
            .arg("loaded", if s.info.loaded { "yes" } else { "no" });
            match note {
                Some(n) => (line.arg("note", n.to_string()), true),
                None => (line, false),
            }
        }
        (Some(slot), None) => (
            Message::new("sapphire-embedding-unrunnable").arg("slot", slot.to_string()),
            true,
        ),
        (None, _) => {
            let off = match note {
                Some(n) => Message::new("sapphire-embedding-off-note").arg("note", n.to_string()),
                None => Message::new("sapphire-embedding-off"),
            };
            (
                off,
                matches!(note, Some(sapphire_bridge_api::EmbedNote::Invalid(_))),
            )
        }
    }
}

/// A slot's three-way switch on this device, as message ids: `None` is Auto.
pub const SWITCH_CHOICES: [(&str, Option<bool>); 3] = [
    ("sapphire-switch-auto", None),
    ("sapphire-switch-on", Some(true)),
    ("sapphire-switch-off", Some(false)),
];

/// The message id of what Auto resolves to for `slot`, and why, for the label beside the
/// switch.
pub fn auto_resolution(s: &EmbedSettingsResult, slot: Slot) -> &'static str {
    match slot {
        Slot::Local if !s.avx2 => "sapphire-auto-off-no-avx2",
        Slot::Local => "sapphire-auto-on",
        Slot::Remote => "sapphire-auto-on",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sapphire_backend::protocol::{SyncStatusResult, Topology};
    use sapphire_bridge_api::WorkspaceRoles;
    use std::path::{Path, PathBuf};

    fn entry(
        id: &str,
        ws: Option<GrainId>,
        reachable: bool,
        sync: SyncStatusResult,
    ) -> CurrentWorkspace {
        CurrentWorkspace {
            root: PathBuf::from(format!("/x/{id}")),
            reachable,
            workspace_id: ws,
            sync,
        }
    }

    fn synced(peers: usize) -> SyncStatusResult {
        SyncStatusResult {
            enabled: true,
            peers,
            ..SyncStatusResult::not_synced()
        }
    }

    #[test]
    fn badge_precedence() {
        assert_eq!(
            badge(&entry("a", None, false, synced(2))),
            Badge::Unreachable
        );
        assert_eq!(
            badge(&entry("a", None, true, SyncStatusResult::not_synced())),
            Badge::NotSynced
        );
        let mut s = synced(1);
        s.paused = Some("root missing".into());
        s.last_error = Some("boom".into());
        assert_eq!(
            badge(&entry("a", None, true, s.clone())),
            Badge::Error("boom".into())
        );
        s.last_error = None;
        assert_eq!(
            badge(&entry("a", None, true, s)),
            Badge::Paused("root missing".into())
        );
        assert_eq!(
            badge(&entry("a", None, true, synced(3))),
            Badge::Syncing {
                peers: 3,
                star: false,
                embedding_pending: 0,
            }
        );
        let mut s = synced(1);
        s.embedding = Some(sapphire_backend::protocol::EmbeddingProgress {
            vectors: 10,
            pending: 4,
            running: true,
        });
        assert_eq!(
            badge(&entry("a", None, true, s)).label(),
            Message::new("sapphire-badge-syncing")
                .arg("peers", 1)
                .arg("star", "no")
                .arg("pending", 4)
        );
    }

    #[test]
    fn others_excludes_the_current_workspace_and_other_apps() {
        let mine = GrainId::random();
        let other = GrainId::random();
        let foreign = GrainId::random();
        let ledger = vec![
            WorkgroupWorkspaceInfo {
                workspace_id: mine,
                app_name: "app".into(),
                name: "mine".into(),
            },
            WorkgroupWorkspaceInfo {
                workspace_id: other,
                app_name: "app".into(),
                name: "other".into(),
            },
            WorkgroupWorkspaceInfo {
                workspace_id: foreign,
                app_name: "journal".into(),
                name: "j".into(),
            },
        ];
        let current = entry("mine", Some(mine), true, SyncStatusResult::not_synced());
        let left = others(&ledger, Some(&current), "app");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].workspace_id, other);
        assert_eq!(others(&ledger, None, "app").len(), 2);
    }

    fn peer(id: GrainId, priority: u8) -> PeerInfo {
        PeerInfo {
            device_id: id,
            name: "desk".into(),
            node_id: "aaaa".into(),
            connected: true,
            priority,
            availability: None,
        }
    }

    /// One frame of the device row, as the view runs it: prune, show, record. `typed` is what
    /// the user dials in this frame, if anything. Returns the value shown.
    fn frame(edits: &mut HashMap<GrainId, u8>, p: &PeerInfo, typed: Option<u8>) -> u8 {
        prune_edits(edits, std::slice::from_ref(p));
        let shown = typed.unwrap_or(edits.get(&p.device_id).copied().unwrap_or(p.priority));
        record_edit(edits, p, shown);
        shown
    }

    #[test]
    fn a_change_made_elsewhere_shows_in_an_untouched_row() {
        let id = GrainId::random();
        let mut edits = HashMap::new();
        assert_eq!(frame(&mut edits, &peer(id, 1), None), 1);
        assert_eq!(frame(&mut edits, &peer(id, 3), None), 3, "the row follows");
        assert!(edits.is_empty(), "nothing to Set");
    }

    #[test]
    fn a_pending_edit_survives_until_set() {
        let id = GrainId::random();
        let mut edits = HashMap::new();
        frame(&mut edits, &peer(id, 1), Some(5));
        assert_eq!(frame(&mut edits, &peer(id, 1), None), 5);
        assert_eq!(
            frame(&mut edits, &peer(id, 3), None),
            5,
            "outlives other changes"
        );
        assert_eq!(edits.get(&id), Some(&5));
        // Once the bridge agrees, the edit is done.
        assert_eq!(frame(&mut edits, &peer(id, 5), None), 5);
        assert!(edits.is_empty());
        // Dialling back to the bridge's value is no edit either.
        frame(&mut edits, &peer(id, 5), Some(7));
        frame(&mut edits, &peer(id, 5), Some(5));
        assert!(edits.is_empty());
    }

    #[test]
    fn an_edit_for_a_vanished_device_is_dropped() {
        let (gone, kept) = (GrainId::random(), GrainId::random());
        let mut edits = HashMap::from([(gone, 4), (kept, 4)]);
        prune_edits(&mut edits, &[peer(kept, 1)]);
        assert_eq!(edits, HashMap::from([(kept, 4)]));
    }

    #[test]
    fn this_device_is_matched_by_node_id() {
        let p = PeerInfo {
            device_id: GrainId::random(),
            name: "desk".into(),
            node_id: "aaaa".into(),
            connected: true,
            priority: 1,
            availability: None,
        };
        assert!(is_this_device(&p, "aaaa"));
        assert!(!is_this_device(&p, "bbbb"));
        assert!(!is_this_device(
            &PeerInfo {
                node_id: String::new(),
                ..p
            },
            ""
        ));
    }

    #[test]
    fn names_are_trimmed_and_must_not_be_empty() {
        assert_eq!(valid_name("  home "), Some("home".into()));
        assert_eq!(valid_name("   "), None);
    }

    #[test]
    fn move_target_keeps_the_name_and_refuses_one_already_there() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Path::new("/somewhere/Notes");
        assert_eq!(move_target(tmp.path(), root), Ok(tmp.path().join("Notes")));
        std::fs::create_dir(tmp.path().join("Notes")).unwrap();
        let err = move_target(tmp.path(), root).unwrap_err();
        assert_eq!(err.id, "sapphire-move-exists");
        assert_eq!(
            err.args,
            vec![
                ("folder", tmp.path().display().to_string().into()),
                ("name", "Notes".into()),
            ]
        );
        let err = move_target(tmp.path(), Path::new("/")).unwrap_err();
        assert_eq!(err.id, "sapphire-move-unmovable");
    }

    #[test]
    fn bring_target_nests_a_folder_named_after_the_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            bring_target(tmp.path(), "Notes", "app"),
            tmp.path().join("Notes")
        );
        assert_eq!(
            bring_target(tmp.path(), "a/b\\c:d*e?f\"g<h>i|j", "app"),
            tmp.path().join("a-b-c-d-e-f-g-h-i-j")
        );
        assert_eq!(
            bring_target(tmp.path(), " ..x.. ", "app"),
            tmp.path().join("x")
        );
        assert_eq!(
            bring_target(tmp.path(), " . ", "app"),
            tmp.path().join("workspace")
        );
        assert_eq!(
            bring_target(tmp.path(), "", "app"),
            tmp.path().join("workspace")
        );
    }

    #[test]
    fn bring_target_uses_a_picked_workspace_folder_as_is() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".app")).unwrap();
        assert_eq!(bring_target(tmp.path(), "Notes", "app"), tmp.path());
    }

    #[test]
    fn a_new_workspace_needs_an_empty_folder_or_a_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(new_target_ok(tmp.path(), "app"), Ok(()));
        std::fs::write(tmp.path().join("file.txt"), "x").unwrap();
        let err = new_target_ok(tmp.path(), "app").unwrap_err();
        assert_eq!(err.id, "sapphire-new-not-empty");
        assert_eq!(
            err.args,
            vec![("path", tmp.path().display().to_string().into())]
        );
        let err = new_target_ok(&tmp.path().join("missing"), "app").unwrap_err();
        assert_eq!(err.id, "sapphire-new-unreadable");
        std::fs::create_dir(tmp.path().join(".app")).unwrap();
        assert_eq!(new_target_ok(tmp.path(), "app"), Ok(()));
    }

    #[test]
    fn display_path_drops_the_verbatim_prefix() {
        assert_eq!(
            display_path(Path::new(r"\\?\C:\Users\me\notes")),
            PathBuf::from(r"C:\Users\me\notes")
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share\notes")),
            PathBuf::from(r"\\server\share\notes")
        );
        assert_eq!(
            display_path(Path::new("/home/me/notes")),
            PathBuf::from("/home/me/notes")
        );
        assert_eq!(
            display_path(Path::new(r"C:\plain")),
            PathBuf::from(r"C:\plain")
        );
    }

    #[test]
    fn short_id_is_a_prefix() {
        let id = GrainId::random();
        let s = short_id(&id);
        assert!(id.to_string().starts_with(&s));
        assert!(s.len() <= 8);
    }

    #[test]
    fn display_name_is_the_folder_name() {
        let e = entry("notes-2", None, true, SyncStatusResult::not_synced());
        assert_eq!(display_name(&e), "notes-2");
    }

    #[test]
    fn role_badges_name_each_role_once() {
        let me = GrainId::random();
        let p = PeerInfo {
            device_id: me,
            name: "a".into(),
            node_id: String::new(),
            connected: true,
            priority: 1,
            availability: None,
        };
        let roles = vec![
            WorkspaceRoles {
                workspace_id: GrainId::random(),
                primary: Some(me),
                secondary: None,
            },
            WorkspaceRoles {
                workspace_id: GrainId::random(),
                primary: Some(me),
                secondary: None,
            },
            WorkspaceRoles {
                workspace_id: GrainId::random(),
                primary: None,
                secondary: Some(me),
            },
        ];
        assert_eq!(
            role_badges(&p, &roles),
            vec!["sapphire-role-primary", "sapphire-role-secondary"]
        );
    }

    #[test]
    fn a_star_workspace_says_so() {
        let mut s = synced(2);
        s.topology = Topology::Star {
            primary: GrainId::random(),
            secondary: None,
        };
        assert_eq!(
            badge(&entry("a", None, true, s)).label(),
            Message::new("sapphire-badge-syncing")
                .arg("peers", 2)
                .arg("star", "yes")
                .arg("pending", 0)
        );
    }

    /// The English the old `format!` produced, from the message the badge now returns.
    #[test]
    fn badge_text_in_english() {
        let ctx = egui::Context::default();
        corundum_framework_gui::i18n::register(&ctx, crate::RESOURCES);
        corundum_framework_gui::i18n::install(
            &ctx,
            &corundum_framework_gui::i18n::LanguageSettings {
                language: corundum_framework_gui::i18n::Language::Explicit("en".into()),
            },
        );
        let one = Badge::Syncing {
            peers: 1,
            star: false,
            embedding_pending: 0,
        };
        assert_eq!(one.label().text(&ctx), "syncing · 1 peer");
        let two = Badge::Syncing {
            peers: 2,
            star: true,
            embedding_pending: 3,
        };
        assert_eq!(
            two.label().text(&ctx),
            "syncing · 2 peers · star · embedding 3 pending"
        );
        assert_eq!(
            Badge::Paused("why".into()).label().text(&ctx),
            "paused: why"
        );
        assert_eq!(Badge::Error("e".into()).label().text(&ctx), "error: e");
        assert_eq!(Badge::NotSynced.label().text(&ctx), "not synced");
        assert_eq!(Badge::Unreachable.label().text(&ctx), "unreachable");
    }

    // ── embedding ───────────────────────────────────────────────────────────

    fn embed_settings() -> sapphire_bridge_api::EmbedSettingsResult {
        sapphire_bridge_api::EmbedSettingsResult {
            avx2: true,
            ..Default::default()
        }
    }

    #[test]
    fn embedding_status_names_the_model_or_why_it_is_off() {
        use sapphire_bridge_api::{EmbedInfoResult, EmbedModelInfo, EmbedNote};

        let mut s = embed_settings();
        s.info.note = Some(EmbedNote::NotConfigured);
        assert_eq!(
            embedding_status(&s),
            (
                Message::new("sapphire-embedding-off-note").arg("note", "no model is configured"),
                false
            )
        );
        s.info.note = None;
        assert_eq!(
            embedding_status(&s),
            (Message::new("sapphire-embedding-off"), false)
        );
        s.info.note = Some(EmbedNote::NotConfigured);

        s.active = Some(Slot::Remote);
        s.info = EmbedInfoResult {
            enabled: true,
            model: Some(EmbedModelInfo {
                model: "m".into(),
                dimension: 8,
                template_version: 0,
                revision: None,
                max_tokens: None,
                images: false,
            }),
            loaded: true,
            note: Some(EmbedNote::KeyMissing),
        };
        let (line, warn) = embedding_status(&s);
        assert!(warn);
        assert_eq!(
            line,
            Message::new("sapphire-embedding-active-note")
                .arg("slot", "remote")
                .arg("model", "m")
                .arg("dimension", 8)
                .arg("loaded", "yes")
                .arg("note", "no API key is set for the remote model")
        );
        s.active = Some(Slot::Local);
        s.info.model = None;
        assert_eq!(
            embedding_status(&s),
            (
                Message::new("sapphire-embedding-unrunnable").arg("slot", "local"),
                true
            )
        );
    }

    #[test]
    fn auto_says_why_the_local_slot_is_off() {
        let mut s = embed_settings();
        assert_eq!(auto_resolution(&s, Slot::Local), "sapphire-auto-on");
        s.avx2 = false;
        assert_eq!(
            auto_resolution(&s, Slot::Local),
            "sapphire-auto-off-no-avx2"
        );
        assert_eq!(auto_resolution(&s, Slot::Remote), "sapphire-auto-on");
    }
}
