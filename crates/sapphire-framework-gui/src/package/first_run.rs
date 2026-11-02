//! An app's first run in a package: whether to keep its app server running in the
//! background.

use corundum_framework_gui::first_run::{FirstRunPage, ServicePage};
use corundum_framework_service::ServiceSpec;

use crate::tr;

/// The first-run pages for the app `name`, in the language installed in `ctx`: corundum's
/// service page for `service`, Yes by default.
pub fn app_first_run_pages(
    ctx: &egui::Context,
    name: &str,
    service: fn() -> ServiceSpec,
) -> Vec<Box<dyn FirstRunPage>> {
    crate::i18n::register(ctx, crate::RESOURCES);
    vec![Box::new(ServicePage::new(
        vec![service],
        tr!(ctx, "sapphire-app-service-question", app = name),
        tr!(ctx, "sapphire-app-service-reason"),
    ))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_asks_to_keep_the_app_running() {
        let ctx = crate::test_support::english_ctx();
        let pages = app_first_run_pages(&ctx, "Sapphire Example", crate::package::tests::svc);
        let titles: Vec<String> = pages.iter().map(|p| p.title(&ctx)).collect();
        assert_eq!(titles, ["Keep Sapphire Example running in the background?"]);
    }
}
