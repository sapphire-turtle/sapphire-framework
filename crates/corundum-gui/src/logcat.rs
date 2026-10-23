//! `tracing` to logcat: on Android a process's stderr goes nowhere.

/// The filter a phone logs with when `RUST_LOG` says nothing: the app's own `app_filter`,
/// corundum-gui's start-up lines (the renderer, the font), every other target's warnings —
/// and winit's Android backend at errors only, because its warning for each keyboard event
/// it drops carries the whole text typed (an invite ticket, say) into a log any debugging
/// tool can read.
pub fn phone_log_filter(app_filter: &str) -> String {
    format!("warn,{app_filter},corundum_gui=info,winit::platform_impl::android=error")
}

#[cfg(all(target_os = "android", feature = "android"))]
pub use android::install;

#[cfg(all(target_os = "android", feature = "android"))]
mod android {
    use std::ffi::CString;
    use std::io::Write;

    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::MakeWriter;

    const ANDROID_LOG_INFO: i32 = 4;

    /// Send this process's events to logcat, tagged `tag`, at `RUST_LOG` or
    /// [`phone_log_filter(app_filter)`](super::phone_log_filter). The first subscriber
    /// wins: one installed later finds this one in place and changes nothing.
    pub fn install(tag: &str, app_filter: &str) {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(super::phone_log_filter(app_filter)));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .without_time()
            .with_writer(Logcat {
                tag: CString::new(tag).unwrap_or_default(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phone_filter_wraps_the_apps_and_keeps_typed_text_out() {
        let filter = phone_log_filter("my_app=info");
        assert!(filter.starts_with("warn,my_app=info,"));
        assert!(filter.contains(",corundum_gui=info,"));
        assert!(filter.ends_with(",winit::platform_impl::android=error"));
    }
}
