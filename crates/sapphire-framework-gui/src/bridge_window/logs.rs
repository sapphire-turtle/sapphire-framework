//! The Logs page: the end of the bridge's newest log file.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How many lines the page shows.
pub const TAIL_LINES: usize = 500;

/// How often the file is read again while the page is shown.
pub const REFRESH: Duration = Duration::from_secs(2);

/// The bridge's log file in its log directory (the bridge's `logging::LOG_FILE`).
pub const LOG_FILE: &str = "node.log";

/// How much of the end of the file is read: the file rotates at 10 MiB, and a frame must
/// not read that much.
pub const READ_BYTES: u64 = 256 * 1024;

/// The last [`TAIL_LINES`] lines of the bridge's log file ([`LOG_FILE`]).
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

/// The last [`TAIL_LINES`] lines of [`LOG_FILE`] in `dir`, from at most its last
/// [`READ_BYTES`]; a byte that is not UTF-8 shows as a replacement character.
fn read_tail(dir: &std::path::Path) -> Result<Vec<String>, String> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let path = dir.join(LOG_FILE);
    let unreadable = |e: std::io::Error| format!("could not read {}: {e}", path.display());
    let mut file = std::fs::File::open(&path).map_err(unreadable)?;
    let len = file.metadata().map_err(unreadable)?.len();
    let start = len.saturating_sub(READ_BYTES);
    file.seek(SeekFrom::Start(start)).map_err(unreadable)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(unreadable)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    // Read from the middle of a line: that line is not whole.
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    let first = lines.len().saturating_sub(TAIL_LINES);
    Ok(lines[first..].iter().map(|l| (*l).to_owned()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_tail_reads_the_end_of_node_log() {
        let dir = tempfile::tempdir().unwrap();
        let body: String = (0..600).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.path().join("node.log"), body).unwrap();
        // A rotated file written later, as a coarse clock can make it seem.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.path().join("node.log.1"), "rotated\n").unwrap();
        let mut view = LogsView::new(dir.path().to_owned());
        let lines = view.tail().unwrap();
        assert_eq!(lines.len(), TAIL_LINES);
        assert_eq!(lines.last().unwrap(), "line 599");
        assert_eq!(lines.first().unwrap(), "line 100");
    }

    #[test]
    fn a_byte_that_is_not_utf8_does_not_hide_the_log() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("node.log"), b"broken \xff\nfine\n").unwrap();
        let mut view = LogsView::new(dir.path().to_owned());
        assert_eq!(view.tail().unwrap().last().unwrap(), "fine");
    }

    #[test]
    fn only_the_end_of_a_large_log_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let line = "x".repeat(1000);
        let body: String = (0..4000).map(|i| format!("{i} {line}\n")).collect();
        std::fs::write(dir.path().join("node.log"), &body).unwrap();
        let mut view = LogsView::new(dir.path().to_owned());
        let lines = view.tail().unwrap();
        assert!(lines.last().unwrap().starts_with("3999 "));
        // At most READ_BYTES were read: fewer whole lines than TAIL_LINES fit.
        assert!(lines.len() <= READ_BYTES as usize / 1000);
        assert!(
            lines.iter().all(|l| l.ends_with(&line)),
            "no partial first line"
        );
    }

    #[test]
    fn an_unreadable_directory_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("logs");
        let err = LogsView::new(gone.clone()).tail().unwrap_err();
        assert!(err.contains(&gone.display().to_string()), "{err}");
    }
}
