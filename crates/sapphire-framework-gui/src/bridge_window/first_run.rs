//! The bridge window's first run in a package: a welcome, then whether to keep Sapphire
//! Bridge running in the background.

use corundum_framework_gui::first_run::{FirstRunPage, PageAction, ServicePage};
use corundum_framework_service::ServiceSpec;

use crate::tr;

/// The welcome page: what Sapphire Bridge is for.
pub struct WelcomePage;

impl FirstRunPage for WelcomePage {
    fn title(&self, ctx: &egui::Context) -> String {
        tr!(ctx, "sapphire-bridge-welcome-title")
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> PageAction {
        ui.label(tr!(ui, "sapphire-bridge-welcome-body"));
        PageAction::Stay
    }
}

/// The first-run pages, in the language installed in `ctx`: the welcome, then corundum's
/// service page for `service`.
pub fn first_run_pages(
    ctx: &egui::Context,
    service: fn() -> ServiceSpec,
) -> Vec<Box<dyn FirstRunPage>> {
    crate::i18n::register(ctx, crate::RESOURCES);
    vec![
        Box::new(WelcomePage),
        Box::new(ServicePage::new(
            vec![service],
            tr!(ctx, "sapphire-bridge-service-question"),
            tr!(ctx, "sapphire-bridge-service-reason"),
        )),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc() -> ServiceSpec {
        ServiceSpec {
            app_name: "sapphire-bridge",
            description: String::new(),
            args: vec!["serve".into()],
            legacy_labels: Vec::new(),
            post_install: None,
        }
    }

    #[test]
    fn first_run_is_welcome_then_service() {
        let ctx = crate::test_support::english_ctx();
        let pages = first_run_pages(&ctx, svc);
        let titles: Vec<String> = pages.iter().map(|p| p.title(&ctx)).collect();
        assert_eq!(
            titles,
            [
                "Welcome to Sapphire Bridge",
                "Keep Sapphire Bridge running in the background?"
            ]
        );
    }
}
