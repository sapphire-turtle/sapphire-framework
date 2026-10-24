//! The egui-free half of the file browser: listing a directory under the root, reading a
//! preview, and the breadcrumbs of a path. Every path here is relative to the root and never
//! leaves it.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use corundum_gui::i18n::Message;

/// The most of a file the preview reads.
pub const PREVIEW_LIMIT: usize = 1024 * 1024;

/// One entry of a directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The file name.
    pub name: String,
    /// A directory (never true for a symlink, which is not followed).
    pub is_dir: bool,
    /// A symbolic link: listed, never followed.
    pub is_symlink: bool,
    /// The size in bytes (of the link itself for a symlink).
    pub size: u64,
}

/// What the preview shows for one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    /// UTF-8 text: the whole file, or its first [`PREVIEW_LIMIT`] bytes.
    Text {
        /// The text read.
        text: String,
        /// The file goes on past what was read.
        truncated: bool,
    },
    /// Not UTF-8: only its size is shown.
    Binary {
        /// The file's size in bytes.
        size: u64,
    },
}

/// An io error the system reported, as a message; its text is the system's, not translated.
fn io_error(e: std::io::Error) -> Message {
    Message::new("sapphire-files-io-error").arg("error", e.to_string())
}

/// `rel` under `root`, refused unless every component is a plain name.
///
/// `..`, a root or a prefix would let a path leave the root; `.` is harmless but has no
/// business in a path this module built. Symlinks along the way are refused too, so a link
/// inside the workspace cannot carry the browser outside it.
fn resolve(root: &Path, rel: &Path) -> Result<PathBuf, Message> {
    let mut path = root.to_path_buf();
    for component in rel.components() {
        match component {
            Component::Normal(name) => {
                path.push(name);
                if std::fs::symlink_metadata(&path)
                    .map_err(io_error)?
                    .file_type()
                    .is_symlink()
                {
                    return Err(Message::new("sapphire-files-symlink")
                        .arg("path", path.display().to_string()));
                }
            }
            _ => {
                return Err(
                    Message::new("sapphire-files-outside").arg("path", rel.display().to_string())
                );
            }
        }
    }
    Ok(path)
}

/// The visible entries of the directory `rel` under `root`: directories first, then files,
/// each by name ignoring case. Names starting with `.` are hidden.
pub fn list_dir(root: &Path, rel: &Path) -> Result<Vec<Entry>, Message> {
    let dir = resolve(root, rel)?;
    let mut entries = Vec::new();
    for item in std::fs::read_dir(&dir).map_err(io_error)? {
        let item = item.map_err(io_error)?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let meta = std::fs::symlink_metadata(item.path()).map_err(io_error)?;
        let is_symlink = meta.file_type().is_symlink();
        entries.push(Entry {
            name,
            is_dir: !is_symlink && meta.is_dir(),
            is_symlink,
            size: meta.len(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

/// Read the file `rel` under `root` for the preview.
pub fn read_preview(root: &Path, rel: &Path) -> Result<Preview, Message> {
    let path = resolve(root, rel)?;
    let file = std::fs::File::open(&path).map_err(io_error)?;
    let size = file.metadata().map_err(io_error)?.len();
    let mut bytes = Vec::with_capacity(PREVIEW_LIMIT.min(size as usize) + 1);
    // One byte past the limit tells "exactly the limit" from "more than the limit".
    file.take(PREVIEW_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    let truncated = bytes.len() > PREVIEW_LIMIT;
    bytes.truncate(PREVIEW_LIMIT);
    match String::from_utf8(bytes) {
        Ok(text) => Ok(Preview::Text { text, truncated }),
        Err(err) if truncated => {
            // Cut mid-character by the limit: text, if everything before the cut is.
            let valid = err.utf8_error().valid_up_to();
            let bytes = err.into_bytes();
            if bytes.len() - valid < 4 {
                let text = String::from_utf8(bytes[..valid].to_vec()).expect("valid prefix");
                Ok(Preview::Text {
                    text,
                    truncated: true,
                })
            } else {
                Ok(Preview::Binary { size })
            }
        }
        Err(_) => Ok(Preview::Binary { size }),
    }
}

/// The way back up from `rel`: the root (`"/"`, empty path), then each directory down to
/// `rel` itself.
pub fn breadcrumbs(rel: &Path) -> Vec<(String, PathBuf)> {
    let mut crumbs = vec![("/".to_owned(), PathBuf::new())];
    let mut path = PathBuf::new();
    for component in rel.components() {
        if let Component::Normal(name) = component {
            path.push(name);
            crumbs.push((name.to_string_lossy().into_owned(), path.clone()));
        }
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join("b-dir")).unwrap();
        std::fs::create_dir(root.join("A-dir")).unwrap();
        std::fs::create_dir(root.join(".sapphire-sync")).unwrap();
        std::fs::write(root.join("c.md"), "c").unwrap();
        std::fs::write(root.join("B.txt"), "b").unwrap();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        std::fs::write(root.join(".sapphireignore"), "").unwrap();
        tmp
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn dirs_come_first_then_names_case_insensitively() {
        let tmp = tree();
        let entries = list_dir(tmp.path(), Path::new("")).unwrap();
        assert_eq!(
            names(&entries),
            ["A-dir", "b-dir", "a.txt", "B.txt", "c.md"]
        );
        assert!(entries[0].is_dir && entries[1].is_dir && !entries[2].is_dir);
    }

    #[test]
    fn dot_entries_are_hidden() {
        let tmp = tree();
        let entries = list_dir(tmp.path(), Path::new("")).unwrap();
        assert!(entries.iter().all(|e| !e.name.starts_with('.')));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_listed_not_followed() {
        let tmp = tree();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("link")).unwrap();

        let entries = list_dir(tmp.path(), Path::new("")).unwrap();
        let link = entries.iter().find(|e| e.name == "link").unwrap();
        assert!(link.is_symlink);
        assert!(!link.is_dir);
        let err = list_dir(tmp.path(), Path::new("link")).unwrap_err();
        assert_eq!(err.id, "sapphire-files-symlink");
        let err = read_preview(tmp.path(), Path::new("link/secret.txt")).unwrap_err();
        assert_eq!(err.id, "sapphire-files-symlink");
    }

    #[test]
    fn parent_components_are_refused() {
        let tmp = tree();
        let err = list_dir(tmp.path(), Path::new("../")).unwrap_err();
        assert_eq!(err.id, "sapphire-files-outside");
        let err = read_preview(tmp.path(), Path::new("b-dir/../../x")).unwrap_err();
        assert_eq!(err.id, "sapphire-files-outside");
    }

    #[test]
    fn other_io_errors_carry_the_systems_text() {
        let tmp = tree();
        let err = list_dir(tmp.path(), Path::new("missing")).unwrap_err();
        assert_eq!(err.id, "sapphire-files-io-error");
        assert_eq!(err.args.len(), 1);
        assert_eq!(err.args[0].0, "error");
    }

    #[test]
    fn utf8_text_previews() {
        let tmp = tree();
        std::fs::write(tmp.path().join("j.md"), "日本語のメモ\n").unwrap();
        assert_eq!(
            read_preview(tmp.path(), Path::new("j.md")).unwrap(),
            Preview::Text {
                text: "日本語のメモ\n".into(),
                truncated: false
            }
        );
    }

    #[test]
    fn empty_file_is_empty_text() {
        let tmp = tree();
        std::fs::write(tmp.path().join("empty"), "").unwrap();
        assert_eq!(
            read_preview(tmp.path(), Path::new("empty")).unwrap(),
            Preview::Text {
                text: String::new(),
                truncated: false
            }
        );
    }

    #[test]
    fn invalid_utf8_is_binary_with_size() {
        let tmp = tree();
        std::fs::write(tmp.path().join("bin"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        assert_eq!(
            read_preview(tmp.path(), Path::new("bin")).unwrap(),
            Preview::Binary { size: 4 }
        );
    }

    #[test]
    fn large_text_is_truncated_at_the_limit() {
        let tmp = tree();
        // A three-byte character straddles the limit: the cut must fall before it.
        let mut text = "a".repeat(PREVIEW_LIMIT - 1);
        text.push('あ');
        text.push_str("tail");
        std::fs::write(tmp.path().join("big.txt"), &text).unwrap();
        match read_preview(tmp.path(), Path::new("big.txt")).unwrap() {
            Preview::Text { text, truncated } => {
                assert!(truncated);
                assert!(text.len() <= PREVIEW_LIMIT);
                assert_eq!(text.len(), PREVIEW_LIMIT - 1);
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn exactly_the_limit_is_not_truncated() {
        let tmp = tree();
        std::fs::write(tmp.path().join("full.txt"), "a".repeat(PREVIEW_LIMIT)).unwrap();
        match read_preview(tmp.path(), Path::new("full.txt")).unwrap() {
            Preview::Text { text, truncated } => {
                assert!(!truncated);
                assert_eq!(text.len(), PREVIEW_LIMIT);
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn breadcrumbs_of_a_nested_path() {
        assert_eq!(
            breadcrumbs(Path::new("notes/2026/10")),
            vec![
                ("/".to_owned(), PathBuf::new()),
                ("notes".to_owned(), PathBuf::from("notes")),
                ("2026".to_owned(), PathBuf::from("notes/2026")),
                ("10".to_owned(), PathBuf::from("notes/2026/10")),
            ]
        );
        assert_eq!(
            breadcrumbs(Path::new("")),
            vec![("/".to_owned(), PathBuf::new())]
        );
    }
}
