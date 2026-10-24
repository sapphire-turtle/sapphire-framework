//! The file browser's egui half. Reads run on the runtime's blocking pool and land in a slot
//! the next frame picks up, so a slow disk never stalls a frame.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::model::{self, Entry, Preview};
use corundum_gui::COMPACT_WIDTH;
use corundum_gui::i18n::Message;
use corundum_gui::tr;

/// A read in flight or done, tagged with what it was for so a stale answer is dropped.
struct Slot<T> {
    for_path: PathBuf,
    result: Option<Result<T, Message>>,
}

type Shared<T> = Arc<Mutex<Option<Slot<T>>>>;

/// The workspace's files: one directory at a time, and the open file's preview.
///
/// Hold one per screen and call [`ui`](Self::ui) every frame with the workspace root (from
/// the server's snapshot); `None` shows a hint instead.
pub struct FileBrowser {
    runtime: tokio::runtime::Handle,
    repaint: Arc<dyn Fn() + Send + Sync>,
    root: Option<PathBuf>,
    /// The directory shown, relative to the root.
    dir: PathBuf,
    /// The open file, relative to the root.
    open: Option<PathBuf>,
    listing: Shared<Vec<Entry>>,
    preview: Shared<Preview>,
}

impl FileBrowser {
    /// A browser reading on `runtime`, calling `repaint` when a read lands.
    pub fn new(
        runtime: tokio::runtime::Handle,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> FileBrowser {
        FileBrowser {
            runtime,
            repaint,
            root: None,
            dir: PathBuf::new(),
            open: None,
            listing: Arc::default(),
            preview: Arc::default(),
        }
    }

    /// Render into `ui`, browsing `root`.
    pub fn ui(&mut self, ui: &mut egui::Ui, root: Option<&Path>) {
        super::super::register(ui);
        if self.root.as_deref() != root {
            // Another workspace (or none): start again at its top.
            self.root = root.map(Path::to_path_buf);
            self.dir = PathBuf::new();
            self.open = None;
            *lock(&self.preview) = None;
            self.load_listing();
        }
        if self.root.is_none() {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 3.0);
                    ui.heading(tr!(ui, "sapphire-files-title"));
                    ui.label(tr!(ui, "sapphire-files-no-workspace"));
                });
            });
            return;
        };

        if ui.available_width() < COMPACT_WIDTH {
            egui::CentralPanel::default().show(ui, |ui| {
                if self.open.is_some() {
                    if ui.button(tr!(ui, "sapphire-files-back")).clicked() {
                        self.open = None;
                    }
                    ui.separator();
                    self.preview_ui(ui);
                } else {
                    self.list_ui(ui);
                }
            });
        } else {
            egui::Panel::left("files_list")
                .resizable(true)
                .default_size(280.0)
                .show(ui, |ui| self.list_ui(ui));
            egui::CentralPanel::default().show(ui, |ui| {
                if self.open.is_some() {
                    self.preview_ui(ui);
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() / 3.0);
                        ui.weak(tr!(ui, "sapphire-files-select"));
                    });
                }
            });
        }
    }

    fn list_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (i, (label, path)) in model::breadcrumbs(&self.dir).into_iter().enumerate() {
                if i > 1 {
                    ui.label("/");
                }
                if ui.link(label).clicked() && path != self.dir {
                    self.dir = path;
                    self.load_listing();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("⟳")
                    .on_hover_text(tr!(ui, "sapphire-files-refresh"))
                    .clicked()
                {
                    self.load_listing();
                    if self.open.is_some() {
                        self.load_preview();
                    }
                }
            });
        });
        ui.separator();

        let listing = lock(&self.listing);
        let result = listing
            .as_ref()
            .filter(|slot| slot.for_path == self.dir)
            .and_then(|slot| slot.result.clone());
        drop(listing);
        match result {
            None => {
                ui.spinner();
            }
            Some(Err(err)) => {
                ui.colored_label(
                    crate::theme::current(ui.ctx()).text(crate::theme::Swatch::DANGER),
                    err.text(ui.ctx()),
                );
            }
            Some(Ok(entries)) if entries.is_empty() => {
                ui.weak(tr!(ui, "sapphire-files-empty"));
            }
            Some(Ok(entries)) => {
                let mut go_into = None;
                let mut open = None;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for entry in &entries {
                            let selected = self
                                .open
                                .as_ref()
                                .is_some_and(|o| *o == self.dir.join(&entry.name));
                            let label = entry_label(entry);
                            let response = ui.selectable_label(selected, label);
                            if entry.is_symlink {
                                response.on_hover_text(tr!(ui, "sapphire-files-symlink-hover"));
                            } else if response.clicked() {
                                if entry.is_dir {
                                    go_into = Some(self.dir.join(&entry.name));
                                } else {
                                    open = Some(self.dir.join(&entry.name));
                                }
                            }
                        }
                    });
                if let Some(dir) = go_into {
                    self.dir = dir;
                    self.load_listing();
                }
                if let Some(file) = open {
                    self.open = Some(file);
                    self.load_preview();
                }
            }
        }
    }

    fn preview_ui(&mut self, ui: &mut egui::Ui) {
        let Some(open) = self.open.clone() else {
            return;
        };
        ui.strong(open.display().to_string());
        ui.separator();
        let preview = lock(&self.preview);
        let result = preview
            .as_ref()
            .filter(|slot| slot.for_path == open)
            .and_then(|slot| slot.result.clone());
        drop(preview);
        match result {
            None => {
                ui.spinner();
            }
            Some(Err(err)) => {
                ui.colored_label(
                    crate::theme::current(ui.ctx()).text(crate::theme::Swatch::DANGER),
                    err.text(ui.ctx()),
                );
            }
            Some(Ok(Preview::Binary { size })) => {
                ui.weak(tr!(ui, "sapphire-files-binary", size = human_size(size)));
            }
            Some(Ok(Preview::Text { text, truncated })) => {
                if truncated {
                    ui.weak(tr!(
                        ui,
                        "sapphire-files-truncated",
                        size = human_size(model::PREVIEW_LIMIT as u64)
                    ));
                }
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(text).monospace())
                                .selectable(true)
                                .wrap(),
                        );
                    });
            }
        }
    }

    fn load_listing(&self) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let dir = self.dir.clone();
        self.spawn(&self.listing, dir.clone(), move || {
            model::list_dir(&root, &dir)
        });
    }

    fn load_preview(&self) {
        let (Some(root), Some(file)) = (self.root.clone(), self.open.clone()) else {
            return;
        };
        self.spawn(&self.preview, file.clone(), move || {
            model::read_preview(&root, &file)
        });
    }

    /// Read on the blocking pool into `slot`, replacing whatever it held.
    fn spawn<T: Send + 'static>(
        &self,
        slot: &Shared<T>,
        for_path: PathBuf,
        read: impl FnOnce() -> Result<T, Message> + Send + 'static,
    ) {
        *lock(slot) = Some(Slot {
            for_path: for_path.clone(),
            result: None,
        });
        let slot = Arc::clone(slot);
        let repaint = Arc::clone(&self.repaint);
        self.runtime.spawn_blocking(move || {
            let result = read();
            let mut guard = lock(&slot);
            // A later request replaced this one: its answer is the one to show.
            if let Some(current) = guard.as_mut()
                && current.for_path == for_path
            {
                current.result = Some(result);
            }
            drop(guard);
            repaint();
        });
    }
}

fn lock<T>(slot: &Shared<T>) -> std::sync::MutexGuard<'_, Option<Slot<T>>> {
    slot.lock().unwrap_or_else(|e| e.into_inner())
}

impl<T: Clone> Clone for Slot<T> {
    fn clone(&self) -> Self {
        Slot {
            for_path: self.for_path.clone(),
            result: self.result.clone(),
        }
    }
}

fn entry_label(entry: &Entry) -> String {
    if entry.is_dir {
        format!("🗀 {}", entry.name)
    } else if entry.is_symlink {
        format!("↪ {}", entry.name)
    } else {
        format!("🗋 {}  ({})", entry.name, human_size(entry.size))
    }
}

/// `1.2 KiB`-style sizes.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_people_write_them() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(1024 * 1024), "1.0 MiB");
    }
}
