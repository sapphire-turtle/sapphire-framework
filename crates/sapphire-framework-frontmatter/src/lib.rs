//! YAML front matter for Markdown files.
//!
//! A sapphire workspace is plain text that people and agents both edit, and much of it is
//! Markdown that opens with a block of YAML:
//!
//! ```text
//! ---
//! title: Morning call
//! enabled: true
//! ---
//!
//! The body.
//! ```
//!
//! The front matter starts on the file's first line, `---`, and ends at the next line that
//! holds only `---`. Both LF and CRLF line endings are accepted.
//!
//! - [`split`] finds the block and the body, without parsing anything.
//! - [`parse`] / [`parse_optional`] read the block into a type of your own; [`from_yaml`]
//!   reads a block already split off.
//! - [`render`] writes a file back from a value and a body; [`to_yaml`] writes the block
//!   alone.
//! - [`set_top_level_key`] changes one key and leaves every other byte alone — the comments,
//!   the order of the keys and the line endings a person wrote.
//!
//! YAML is read and written with [`serde-saphyr`](serde_saphyr). Values whose shape is not
//! known in advance — the keys an application keeps but does not interpret — are
//! [`Value`] and [`Mapping`], which are `serde_json`'s: front matter keys are strings, so
//! nothing YAML can say there is lost.

#![warn(missing_docs)]

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

/// A front matter value of no particular shape.
pub use serde_json::Value;

/// Front matter as a map of keys to values, in the order they were read.
pub type Mapping = serde_json::Map<String, Value>;

/// The line that opens and closes the front matter.
pub const FENCE: &str = "---";

/// Why front matter could not be read or written.
#[derive(Debug, Error)]
pub enum Error {
    /// The file does not open with a `---` line.
    #[error("missing front matter: the file does not start with a `---` line")]
    Missing,
    /// The file opens a front matter block and never closes it.
    #[error("front matter is not closed: no `---` line after the opening one")]
    Unclosed,
    /// The front matter is not YAML of the expected shape.
    #[error("front matter: {0}")]
    Parse(#[from] serde_saphyr::Error),
    /// The value could not be written as YAML.
    #[error("front matter: {0}")]
    Serialize(#[from] serde_saphyr::ser::Error),
    /// [`set_top_level_key`] was given a value that does not fit on the key's line.
    #[error("the value for `{0}` is not a single-line scalar")]
    NotAScalar(String),
}

/// The crate's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Split `raw` into its front matter and its body.
///
/// The front matter is the text between the fences, with its line endings; the body is
/// everything after the closing fence's line, verbatim — a blank line after the fence is
/// the body's first line. `None` when the file has no front matter, or opens one and never
/// closes it.
pub fn split(raw: &str) -> Option<(&str, &str)> {
    locate(raw).ok()
}

/// Read the front matter of `raw` as a `T`, and return it with the body.
///
/// A file without front matter is [`Error::Missing`]; one whose block never closes is
/// [`Error::Unclosed`]. An empty block reads as an empty mapping, so a `T` whose fields all
/// have defaults reads from it. The body is returned as [`split`] returns it.
pub fn parse<T: DeserializeOwned>(raw: &str) -> Result<(T, &str)> {
    let (front, body) = locate(raw)?;
    Ok((from_yaml(front)?, body))
}

/// As [`parse`], with a file that has no front matter at all read as `None` and the whole
/// file as its body. A block that opens and never closes is still [`Error::Unclosed`].
pub fn parse_optional<T: DeserializeOwned>(raw: &str) -> Result<(Option<T>, &str)> {
    match locate(raw) {
        Ok((front, body)) => Ok((Some(from_yaml(front)?), body)),
        Err(Error::Missing) => Ok((None, raw)),
        Err(err) => Err(err),
    }
}

/// Read front matter already split off — the text between the fences.
///
/// An empty or blank block reads as an empty mapping.
pub fn from_yaml<T: DeserializeOwned>(front: &str) -> Result<T> {
    if front.trim().is_empty() {
        return Ok(serde_saphyr::from_str("{}")?);
    }
    Ok(serde_saphyr::from_str(front)?)
}

/// Write `meta` as the text between the fences, ending with a newline.
pub fn to_yaml<T: Serialize>(meta: &T) -> Result<String> {
    let mut yaml = serde_saphyr::to_string(meta)?;
    if !yaml.ends_with('\n') {
        yaml.push('\n');
    }
    Ok(yaml)
}

/// Write a file from its front matter and its body.
///
/// `---`, the YAML, `---`, then — when the body has any — one blank line and the body with
/// its leading line breaks removed, so a file read with [`parse`] and written back keeps a
/// single blank line after the fence. Line endings are LF.
pub fn render<T: Serialize>(meta: &T, body: &str) -> Result<String> {
    let yaml = to_yaml(meta)?;
    let body = body.trim_start_matches(['\n', '\r']);
    let mut out = String::with_capacity(yaml.len() + body.len() + 9);
    out.push_str(FENCE);
    out.push('\n');
    out.push_str(&yaml);
    out.push_str(FENCE);
    out.push('\n');
    if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
    }
    Ok(out)
}

/// Set the top-level `key` of `raw`'s front matter to `value`, and change nothing else.
///
/// Not a [`parse`] and [`render`] round trip: that drops every comment and may reorder the
/// keys, so an agent asked to flip one switch would rewrite a person's file. Here the key's
/// line is replaced where it stands — together with the lines that continued its old value
/// (indented lines, and a block sequence's `-` lines) — or, when the key is absent, added
/// at the end of the block. The file's line endings are kept.
///
/// `value` must be a scalar that fits on one line (a bool, a number, a short string). `None`
/// when `raw` has no front matter: a file that is not a definition at all is the caller's
/// to refuse, not this function's to repair by inventing one. The result is checked to
/// read back with `key` holding `value`.
pub fn set_top_level_key<V: Serialize>(raw: &str, key: &str, value: &V) -> Result<Option<String>> {
    let Ok((front, _)) = locate(raw) else {
        return Ok(None);
    };
    let scalar = serde_saphyr::to_string(value)?;
    let scalar = scalar.trim_end_matches(['\n', '\r']);
    if scalar.is_empty() || scalar.contains('\n') {
        return Err(Error::NotAScalar(key.to_owned()));
    }

    // Keep the file's own line endings: rewriting a CRLF file as LF turns a one-line change
    // into a whole-file diff.
    let head_len = if raw.starts_with("---\r\n") { 5 } else { 4 };
    let nl = if head_len == 5 { "\r\n" } else { "\n" };
    let line = format!("{key}: {scalar}{nl}");
    let prefix = format!("{key}:");

    let mut out = String::with_capacity(raw.len() + line.len());
    out.push_str(&raw[..head_len]);
    let mut replaced = false;
    let mut skipping = false;
    for current in front.split_inclusive('\n') {
        let text = current.trim_end_matches(['\n', '\r']);
        if skipping {
            // The old value's continuation: indented lines, and a block sequence written at
            // the key's own indentation.
            if text.starts_with([' ', '\t']) || text == "-" || text.starts_with("- ") {
                continue;
            }
            skipping = false;
        }
        // Top level only: an indented line is a nested key, not this one.
        let is_key = !replaced
            && text
                .strip_prefix(&prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']));
        if is_key {
            out.push_str(&line);
            replaced = true;
            skipping = true;
        } else {
            out.push_str(current);
        }
    }
    if !replaced {
        if !out.ends_with('\n') {
            out.push_str(nl);
        }
        out.push_str(&line);
    }
    // From the closing fence onwards, verbatim.
    out.push_str(&raw[head_len + front.len()..]);

    // The edit is textual; make sure it says what was asked.
    let (front, _) = locate(&out)?;
    let read: Mapping = from_yaml(front)?;
    let expected = serde_json::to_value(value).ok();
    if expected.is_some() && read.get(key) != expected.as_ref() {
        return Err(Error::NotAScalar(key.to_owned()));
    }
    Ok(Some(out))
}

/// Find the front matter and the body, or say why there is none.
fn locate(raw: &str) -> Result<(&str, &str)> {
    let rest = raw
        .strip_prefix("---\n")
        .or_else(|| raw.strip_prefix("---\r\n"))
        .ok_or(Error::Missing)?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == FENCE {
            return Ok((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    Err(Error::Unclosed)
}

#[cfg(test)]
mod tests;
