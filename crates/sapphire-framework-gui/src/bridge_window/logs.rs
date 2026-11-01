//! The Logs page: the end of the bridge's newest log file.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How many lines the page shows.
pub const TAIL_LINES: usize = 500;

/// How often the file is read again while the page is shown.
pub const REFRESH: Duration = Duration::from_secs(2);

/// The last [`TAIL_LINES`] lines of the newest file in the bridge's log directory.
pub struct LogsView {
    dir: PathBuf,
    lines: Vec<String>,
    error: Option<String>,
    read_at: Option<Instant>,
}

impl LogsView {
    /// The view of the log files in `dir`.
    pub fn new(dir: PathBuf) -> LogsView {
        LogsView {
            dir,
            lines: Vec::new(),
            error: None,
            read_at: None,
        }
    }

    /// The lines, read again when [`REFRESH`] has passed; the error names the path.
    pub fn tail(&mut self) -> Result<&[String], String> {
        if self.read_at.is_none_or(|at| at.elapsed() >= REFRESH) {
            self.read_at = Some(Instant::now());
            match read_tail(&self.dir) {
                Ok(lines) => {
                    self.lines = lines;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        match &self.error {
            Some(error) => Err(error.clone()),
            None => Ok(&self.lines),
        }
    }

    /// Draw the page; it asks for a repaint so the lines keep up.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(crate::tr!(ui, "sapphire-bridge-page-logs"));
        if ui
            .button(crate::tr!(ui, "sapphire-bridge-logs-open-folder"))
            .clicked()
        {
            crate::platform::open_in_file_manager(&self.dir);
        }
        match self.tail() {
            Ok(lines) => {
                let text = lines.join("\n");
                egui::ScrollArea::both()
                    .stick_to_bottom(true)
                    .show(ui, |ui| ui.monospace(text));
            }
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        }
        ui.ctx().request_repaint_after(REFRESH);
    }
}

/// The last [`TAIL_LINES`] lines of the newest file in `dir`.
fn read_tail(dir: &std::path::Path) -> Result<Vec<String>, String> {
    let unreadable = |e: std::io::Error| format!("could not read {}: {e}", dir.display());
    let newest = std::fs::read_dir(dir)
        .map_err(unreadable)?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    let Some(newest) = newest else {
        return Ok(Vec::new());
    };
    let path = newest.path();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    Ok(lines[start..].iter().map(|l| (*l).to_owned()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_tail_reads_the_newest_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.log"), "old\n").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let body: String = (0..600).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.path().join("b.log"), body).unwrap();
        let mut view = LogsView::new(dir.path().to_owned());
        let lines = view.tail().unwrap();
        assert_eq!(lines.len(), TAIL_LINES);
        assert_eq!(lines.last().unwrap(), "line 599");
        assert_eq!(lines.first().unwrap(), "line 100");
    }

    #[test]
    fn an_unreadable_directory_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("logs");
        let err = LogsView::new(gone.clone()).tail().unwrap_err();
        assert!(err.contains(&gone.display().to_string()), "{err}");
    }
}
