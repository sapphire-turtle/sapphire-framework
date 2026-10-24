//! Pure renderers: each takes a [`Snapshot`](crate::client::Snapshot) and returns the
//! [`Command`](crate::client::Command) the user asked for this frame, if any.

pub mod model;

pub(crate) mod appearance;
mod banner;
pub(crate) mod config;
mod devices;
mod embedding;
mod external;
pub mod files;
mod workgroup;
mod workspaces;

pub use banner::ServiceStatusBanner;
pub use devices::{DeviceList, InviteDialog};
pub use embedding::EmbeddingView;
pub use external::ExternalDeviceList;
pub use files::FileBrowser;
pub use workgroup::{JoinDialog, WorkgroupView};
pub use workspaces::WorkspacePicker;

use crate::client::Snapshot;
use crate::i18n::Message;

/// What every view reads.
pub struct ViewCtx<'a> {
    /// The latest snapshot.
    pub snapshot: &'a Snapshot,
    /// This application's name (filters the ledger, names the marker).
    pub app_name: &'a str,
    /// A command this view sent is still running: disable its buttons.
    pub busy: bool,
}

/// Make this crate's strings available in `ui`'s context; each view's `ui` starts with it
/// so a view used outside [`Settings`](crate::Settings) is translated too.
pub(crate) fn register(ui: &egui::Ui) {
    crate::i18n::register(ui.ctx(), crate::RESOURCES);
}

/// An error from the backend or the bridge, shown as it came: not translated.
pub(crate) fn backend_error(text: &str) -> Message {
    Message::new("sapphire-error-text").arg("text", text)
}

/// [`error_line`] for an error held as text, shown as it came: not translated.
pub(crate) fn error_text_line(ui: &mut egui::Ui, error: &mut Option<String>) {
    let mut message = error.as_deref().map(backend_error);
    error_line(ui, &mut message);
    if message.is_none() {
        *error = None;
    }
}

/// The dismissible red error line every view uses. The message is resolved here, each
/// frame, so a language switch reaches an error already on screen.
pub(crate) fn error_line(ui: &mut egui::Ui, error: &mut Option<Message>) {
    let Some(msg) = error.as_ref().map(|m| m.text(ui.ctx())) else {
        return;
    };
    // Wrapped: a phone is narrower than most messages.
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("×").clicked() {
            *error = None;
        }
        ui.add(
            egui::Label::new(
                egui::RichText::new(msg)
                    .color(crate::theme::current(ui.ctx()).text(crate::theme::Swatch::DANGER)),
            )
            .wrap(),
        );
    });
}

/// The one-line hint a view shows when the process it needs is not up. `what` is the
/// message saying which.
pub(crate) fn unavailable(ui: &mut egui::Ui, what: Message) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.label(what.text(ui.ctx()));
        ui.small(corundum_gui::tr!(ui, "sapphire-unavailable-hint"));
    });
}

/// The bridge is not up.
pub(crate) fn bridge_unavailable(ui: &mut egui::Ui) {
    unavailable(ui, Message::new("sapphire-unavailable-bridge"));
}

/// `text` with the first `name` in it in bold. The text is translated, so the name sits
/// wherever the language puts it.
pub(crate) fn with_strong_name(ui: &mut egui::Ui, text: &str, name: &str) {
    match text.find(name).filter(|_| !name.is_empty()) {
        Some(at) => {
            let (before, after) = (&text[..at], &text[at + name.len()..]);
            ui.horizontal_wrapped(|ui| {
                if !before.trim().is_empty() {
                    ui.label(before.trim_end());
                }
                ui.strong(name);
                if !after.trim().is_empty() {
                    ui.label(after.trim_start());
                }
            });
        }
        None => {
            ui.label(text);
        }
    }
}

/// "Type `name` to confirm:", the name in bold.
pub(crate) fn type_to_confirm(ui: &mut egui::Ui, name: &str) {
    let text = corundum_gui::tr!(ui, "sapphire-confirm-type", name = name);
    with_strong_name(ui, &text, name);
}
