//! A soft keyboard's text, turned into egui's events.
//!
//! On Android the keyboard edits a text buffer of GameActivity's (GameTextInput) rather than
//! sending keys, and winit 0.30 drops the events that report it. So the host reads the
//! buffer before every frame and hands egui the difference from the last read: characters
//! that went away as Backspaces, characters that arrived as [`egui::Event::Text`] (or an IME
//! commit, when they end a composition), and the part the keyboard is still composing — a
//! Japanese reading before conversion, say — as an IME preedit.
//!
//! The buffer is never rewritten while the keyboard types into it: a rewrite races the
//! keyboard, and what it typed in between is lost. It is cleared only when focus moves to
//! another field ([`TextInputBridge::refocus`]), when nobody is typing. Keys the keyboard
//! sends as keys (Backspace on an empty buffer, Enter) still arrive through winit.

/// The keyboard's buffer as the platform reports it. Spans are in UTF-16 code units, as
/// Java's strings count; a span that does not fall on characters is ignored.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Buffer {
    /// The buffer's text.
    pub text: String,
    /// The region the keyboard is composing, if any.
    pub compose: Option<(usize, usize)>,
}

/// Follows the keyboard's buffer between frames.
#[derive(Debug, Default)]
pub struct TextInputBridge {
    /// The buffer's settled text (everything before the composing region) as last read.
    settled: String,
    /// The composing text as last read; empty when nothing is composing.
    composing: String,
    /// After a refocus, how many more reads may still show the old field's text before the
    /// platform's clear is taken to have been lost. `None` when no clear is pending.
    awaiting_clear: Option<u32>,
}

/// How many reads (frames) a refocus waits for the platform to clear its buffer. The clear
/// is queued for Android's main thread and lands within a frame or two.
pub const REFOCUS_PATIENCE: u32 = 30;

impl TextInputBridge {
    /// What changed since the last read, as egui events.
    pub fn translate(&mut self, buffer: &Buffer) -> Vec<egui::Event> {
        // A clear is on its way: the buffer still holds the old field's text, which must be
        // neither typed into the new field nor deleted from it.
        if let Some(left) = self.awaiting_clear {
            if buffer.text.is_empty() {
                self.awaiting_clear = None;
                return Vec::new();
            }
            if left > 0 {
                self.awaiting_clear = Some(left - 1);
                return Vec::new();
            }
            // The clear never landed: what is there now is the baseline, not new text.
            self.awaiting_clear = None;
            self.adopt(buffer);
            return Vec::new();
        }
        let region = buffer
            .compose
            .and_then(|(a, b)| byte_range(&buffer.text, a.min(b), a.max(b)))
            .filter(|r| !r.is_empty());
        let (settled, composing) = match &region {
            Some(r) => (&buffer.text[..r.start], &buffer.text[r.start..r.end]),
            None => (buffer.text.as_str(), ""),
        };

        let mut events = Vec::new();
        let common = common_prefix(&self.settled, settled);
        let removed = self.settled[common..].chars().count();
        for _ in 0..removed {
            events.push(backspace(true));
            events.push(backspace(false));
        }
        let added = &settled[common..];
        let was_composing = !self.composing.is_empty();
        if !added.is_empty() {
            events.push(if was_composing {
                egui::Event::Ime(egui::ImeEvent::Commit(added.to_owned()))
            } else {
                egui::Event::Text(added.to_owned())
            });
        }
        if composing != self.composing {
            // An empty preedit is how egui hears a composition ended with nothing.
            if !(composing.is_empty() && !added.is_empty()) {
                events.push(egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: composing.to_owned(),
                    active_range_chars: None,
                }));
            }
        }
        self.settled = settled.to_owned();
        self.composing = composing.to_owned();
        events
    }

    /// Focus moved to another field: forget the buffer. The caller clears the platform's
    /// buffer too; that clear lands later, and until it reads back empty (or
    /// [`REFOCUS_PATIENCE`] reads pass) the old text is ignored.
    pub fn refocus(&mut self) {
        self.settled.clear();
        self.composing.clear();
        self.awaiting_clear = Some(REFOCUS_PATIENCE - 1);
    }

    /// Take `buffer` as the baseline without emitting anything.
    fn adopt(&mut self, buffer: &Buffer) {
        let region = buffer
            .compose
            .and_then(|(a, b)| byte_range(&buffer.text, a.min(b), a.max(b)))
            .filter(|r| !r.is_empty());
        match region {
            Some(r) => {
                self.settled = buffer.text[..r.start].to_owned();
                self.composing = buffer.text[r.start..r.end].to_owned();
            }
            None => {
                self.settled = buffer.text.clone();
                self.composing.clear();
            }
        }
    }
}

fn backspace(pressed: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Backspace,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// The length in bytes of the longest common prefix of `a` and `b`, on a character
/// boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map(|((i, _), _)| i)
        .unwrap_or_else(|| a.len().min(b.len()))
}

/// The byte range of the UTF-16 span `start..end` in `text`, if it falls on characters.
fn byte_range(text: &str, start: usize, end: usize) -> Option<std::ops::Range<usize>> {
    let mut units = 0;
    let mut from = None;
    let mut to = None;
    for (byte, ch) in text.char_indices() {
        if units == start {
            from = Some(byte);
        }
        if units == end {
            to = Some(byte);
        }
        units += ch.len_utf16();
    }
    if units == start {
        from = Some(text.len());
    }
    if units == end {
        to = Some(text.len());
    }
    Some(from?..to?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str, compose: Option<(usize, usize)>) -> Buffer {
        Buffer {
            text: text.into(),
            compose,
        }
    }

    fn text(s: &str) -> egui::Event {
        egui::Event::Text(s.into())
    }

    fn preedit(s: &str) -> egui::Event {
        egui::Event::Ime(egui::ImeEvent::Preedit {
            text: s.into(),
            active_range_chars: None,
        })
    }

    fn commit(s: &str) -> egui::Event {
        egui::Event::Ime(egui::ImeEvent::Commit(s.into()))
    }

    #[test]
    fn typing_gives_only_what_is_new() {
        let mut bridge = TextInputBridge::default();
        assert_eq!(bridge.translate(&buf("sapph", None)), vec![text("sapph")]);
        assert_eq!(
            bridge.translate(&buf("sapphire:", None)),
            vec![text("ire:")]
        );
        assert!(bridge.translate(&buf("sapphire:", None)).is_empty());
    }

    #[test]
    fn a_long_paste_arrives_whole() {
        let mut bridge = TextInputBridge::default();
        let ticket = format!("sapphire:{}", "X".repeat(300));
        assert_eq!(
            bridge.translate(&buf(&ticket[..150], None)),
            vec![text(&ticket[..150])]
        );
        assert_eq!(
            bridge.translate(&buf(&ticket, None)),
            vec![text(&ticket[150..])]
        );
    }

    #[test]
    fn deleted_characters_are_backspaces() {
        let mut bridge = TextInputBridge::default();
        bridge.translate(&buf("日本語", None));
        assert_eq!(
            bridge.translate(&buf("日本", None)),
            vec![backspace(true), backspace(false)]
        );
        // A correction: one character replaced by another.
        assert_eq!(
            bridge.translate(&buf("日a", None)),
            vec![backspace(true), backspace(false), text("a")]
        );
    }

    #[test]
    fn a_japanese_composition_is_a_preedit_then_a_commit() {
        let mut bridge = TextInputBridge::default();
        assert_eq!(
            bridge.translate(&buf("に", Some((0, 1)))),
            vec![preedit("に")]
        );
        assert_eq!(
            bridge.translate(&buf("にほん", Some((0, 3)))),
            vec![preedit("にほん")]
        );
        assert_eq!(bridge.translate(&buf("日本", None)), vec![commit("日本")]);
    }

    #[test]
    fn a_composition_after_settled_text_keeps_the_settled_text() {
        let mut bridge = TextInputBridge::default();
        bridge.translate(&buf("日本", None));
        // UTF-16 units: "日本" is 2, the composing "ご" is 1 more.
        assert_eq!(
            bridge.translate(&buf("日本ご", Some((2, 3)))),
            vec![preedit("ご")]
        );
        assert_eq!(bridge.translate(&buf("日本語", None)), vec![commit("語")]);
    }

    #[test]
    fn an_abandoned_composition_clears_the_preedit() {
        let mut bridge = TextInputBridge::default();
        bridge.translate(&buf("か", Some((0, 1))));
        assert_eq!(bridge.translate(&buf("", None)), vec![preedit("")]);
    }

    #[test]
    fn refocus_forgets_the_old_field() {
        let mut bridge = TextInputBridge::default();
        bridge.translate(&buf("ticket", None));
        bridge.refocus();
        // The platform applies the clear later: reads of the old text change nothing.
        assert!(bridge.translate(&buf("ticket", None)).is_empty());
        assert!(bridge.translate(&buf("ticket", None)).is_empty());
        // Once it reads back empty, typing starts afresh, deleting nothing.
        assert!(bridge.translate(&buf("", None)).is_empty());
        assert_eq!(bridge.translate(&buf("p", None)), vec![text("p")]);
    }

    #[test]
    fn a_clear_that_never_lands_is_given_up_without_retyping() {
        let mut bridge = TextInputBridge::default();
        bridge.translate(&buf("ticket", None));
        bridge.refocus();
        for _ in 0..REFOCUS_PATIENCE {
            assert!(bridge.translate(&buf("ticket", None)).is_empty());
        }
        // The stale text became the baseline: only what is typed after it is new.
        assert_eq!(bridge.translate(&buf("ticketx", None)), vec![text("x")]);
    }

    #[test]
    fn spans_count_utf16_units_not_bytes() {
        // An emoji is two UTF-16 units and four bytes.
        assert_eq!(byte_range("a😀b", 1, 3), Some(1..5));
        assert_eq!(byte_range("a😀b", 2, 3), None, "inside a surrogate pair");
        assert_eq!(byte_range("ab", 0, 5), None, "past the end");
    }

    #[test]
    fn the_common_prefix_stops_on_a_character_boundary() {
        assert_eq!(common_prefix("日本語", "日本人"), "日本".len());
        assert_eq!(common_prefix("abc", "ab"), 2);
        assert_eq!(common_prefix("", "x"), 0);
    }
}
