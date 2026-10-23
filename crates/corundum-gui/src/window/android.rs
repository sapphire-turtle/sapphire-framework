//! The window on an Android phone: GameActivity's, through eframe, with the app kept clear
//! of the system's bars and the keyboard, and fed the keyboard's text.

use std::time::{Duration, Instant};

pub use android_activity::AndroidApp;
use android_activity::input::{TextInputState, TextSpan};

use super::text_input::{Buffer, TextInputBridge};
use super::{WindowOptions, open};
use crate::gpu;

/// Open the app's window on a phone and run it until it closes.
///
/// Installs [`gpu::install_panic_hook`], gives [`platform`](crate::platform) the activity
/// (for the insets), opens the window on wgpu with [`gpu::wgpu_options`], and builds the
/// app with `create` once the window exists, inside the phone's frame: kept clear of the
/// system's bars and the keyboard, and fed the keyboard's text, which winit does not
/// deliver. Of `options`, the title and the fonts apply; a phone's window has no size.
pub fn run_android(
    android_app: AndroidApp,
    options: WindowOptions,
    create: impl FnOnce(&eframe::CreationContext<'_>) -> Box<dyn eframe::App>,
) -> eframe::Result<()> {
    let WindowOptions { title, fonts, .. } = options;
    gpu::install_panic_hook();
    crate::platform::set_android_activity(android_app.activity_as_ptr());
    let keyboard = android_app.clone();
    let native = eframe::NativeOptions {
        android_app: Some(android_app),
        wgpu_options: gpu::wgpu_options(),
        ..Default::default()
    };
    open(&title, native, fonts, move |cc| {
        Box::new(PhoneWindow::new(create(cc), keyboard))
    })
}

/// How often the window asks for the insets: they change when the keyboard opens or the
/// phone turns, and asking costs a few JNI calls.
const INSETS_EVERY: Duration = Duration::from_millis(250);

/// The app's window as a phone needs it around the app: kept clear of the system's bars and
/// the keyboard, and fed the keyboard's text, which winit does not deliver.
struct PhoneWindow {
    app: Box<dyn eframe::App>,
    android: AndroidApp,
    text: TextInputBridge,
    /// The field the keyboard types into, as egui last reported it.
    focused: Option<egui::Id>,
    insets: [i32; 4],
    insets_read: Option<Instant>,
}

impl PhoneWindow {
    fn new(app: Box<dyn eframe::App>, android: AndroidApp) -> PhoneWindow {
        PhoneWindow {
            app,
            android,
            text: TextInputBridge::default(),
            focused: None,
            insets: [0; 4],
            insets_read: None,
        }
    }

    fn refresh_insets(&mut self) {
        if self
            .insets_read
            .is_some_and(|at| at.elapsed() < INSETS_EVERY)
        {
            return;
        }
        self.insets_read = Some(Instant::now());
        if let Some(insets) = crate::platform::window_insets() {
            self.insets = insets;
        }
    }
}

impl eframe::App for PhoneWindow {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Another field took focus: start its typing from an empty buffer, so the old
        // field's text is neither re-sent nor deleted from the new one.
        let focused = ctx.memory(|m| m.focused());
        if focused != self.focused {
            self.focused = focused;
            self.text.refocus();
            self.android.set_text_input_state(TextInputState {
                text: String::new(),
                selection: TextSpan { start: 0, end: 0 },
                compose_region: None,
            });
        }
        let state = self.android.text_input_state();
        let buffer = Buffer {
            text: state.text,
            compose: state.compose_region.map(|span| (span.start, span.end)),
        };
        raw_input.events.extend(self.text.translate(&buffer));
        // The keyboard's edits arrive as nothing winit wakes the loop for: look again soon
        // while a field has the keyboard.
        if ctx.egui_wants_keyboard_input() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.app.raw_input_hook(ctx, raw_input);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.refresh_insets();
        let ppp = ui.ctx().pixels_per_point();
        let [left, top, right, bottom] = self.insets.map(|px| px.max(0) as f32 / ppp);
        let full = ui.max_rect();
        let inner = egui::Rect::from_min_max(
            full.min + egui::vec2(left, top),
            full.max - egui::vec2(right, bottom),
        );
        // The strips under the bars take the panels' colour, not the clear colour.
        ui.painter().rect_filled(full, 0.0, ui.visuals().panel_fill);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            self.app.ui(ui, frame);
        });
        // The insets change without an event winit reports while the keyboard animates.
        ui.ctx().request_repaint_after(INSETS_EVERY);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.save(storage);
    }

    fn on_exit(&mut self) {
        self.app.on_exit();
    }
}
