//! `tracing` to logcat: on Android a process's stderr goes nowhere.

use std::ffi::CString;
use std::io::Write;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

const ANDROID_LOG_INFO: i32 = 4;

/// Send this process's events to logcat, tagged with the app's name, at the framework's
/// default filter (or `RUST_LOG`). The first subscriber wins: the framework's own console
/// subscriber, installed later, finds this one in place and changes nothing.
pub(super) fn install(app_name: &'static str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!(
            "warn,{},sapphire_framework_gui=info",
            sapphire_workspace::logging::default_log_filter(app_name)
        ))
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .without_time()
        .with_writer(Logcat {
            tag: CString::new(app_name).unwrap_or_default(),
        })
        .try_init();
}

#[derive(Clone)]
struct Logcat {
    tag: CString,
}

impl<'a> MakeWriter<'a> for Logcat {
    type Writer = Line;

    fn make_writer(&'a self) -> Line {
        Line {
            tag: self.tag.clone(),
            buf: Vec::new(),
        }
    }
}

/// One event's text, written to logcat when the formatter is done with it.
struct Line {
    tag: CString,
    buf: Vec<u8>,
}

impl Write for Line {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        while self.buf.last() == Some(&b'\n') {
            self.buf.pop();
        }
        self.buf.retain(|&b| b != 0);
        let Ok(text) = CString::new(std::mem::take(&mut self.buf)) else {
            return;
        };
        // SAFETY: both pointers are NUL-terminated strings that outlive the call.
        unsafe {
            android_log_sys::__android_log_write(
                ANDROID_LOG_INFO,
                self.tag.as_ptr(),
                text.as_ptr(),
            );
        }
    }
}
