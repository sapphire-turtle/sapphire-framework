//! Helpers for rendering the components in unit tests.

use std::sync::Arc;
use std::time::Duration;

use sapphire_ipc::Endpoint;

use crate::client::{AppIdentity, ClientConfig, Endpoints, FrameworkClient, ServiceExes};

/// A client of nothing: its endpoints are in a directory no process listens in, so the
/// snapshot reads both processes as absent. Keep the runtime for as long as the client.
pub(crate) fn client() -> (FrameworkClient, tokio::runtime::Runtime) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let dir = std::env::temp_dir().join(format!("sapphire-gui-test-{}", std::process::id()));
    let config = ClientConfig {
        app: AppIdentity {
            app_name: "sapphire-gui-test",
            version: "0.0.0",
        },
        endpoints: Endpoints {
            bridge: Endpoint::in_dir("bridge", dir.clone()),
            app: Endpoint::in_dir("app", dir.clone()),
        },
        service_exes: ServiceExes {
            bridge: dir.join("sapphire-bridge"),
            app: dir.join("app"),
        },
        refresh: Duration::from_secs(60),
        fetch_timeout: Duration::from_secs(1),
        command_timeout: Duration::from_secs(1),
        install_timeout: Duration::from_secs(1),
    };
    let client = FrameworkClient::spawn(runtime.handle(), config, Arc::new(|| {}));
    (client, runtime)
}

/// Run `ui` over two frames of a `size` screen (the first lays out, the second draws).
pub(crate) fn render(size: [f32; 2], mut ui: impl FnMut(&mut egui::Ui)) {
    let ctx = egui::Context::default();
    for _ in 0..2 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size[0], size[1]),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, &mut ui);
        // Unapplied texture deltas panic on drop; there is no renderer here.
        output.textures_delta.clear();
    }
}
