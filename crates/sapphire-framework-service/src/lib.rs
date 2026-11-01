//! Deprecated (#381): an application's OS service registration is
//! [`corundum-framework-service`](corundum_framework_service), which this crate re-exports
//! for one release. Depend on that crate, or on `sapphire_framework::service`, instead.
//!
//! What changed for an application:
//!
//! - [`ServiceSpec`] has `legacy_labels`: the macOS labels an earlier version registered
//!   under, which install, uninstall and status take off launchd. sapphire's first one is
//!   [`legacy_label`]; corundum retires its own earlier `net.sapphire-turtle.<app>` itself.
//! - The macOS label is `org.sapphire-turtle.<app>` (#385). The systemd unit
//!   (`<app>.service`) and the Windows task (`<app>`) are named as before.

#![allow(deprecated)]

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub use corundum_framework_service::*;

/// The label sapphire's first LaunchAgents were registered under:
/// `net.fireturtle.sapphire.<app>`. Put it in [`ServiceSpec::legacy_labels`].
#[deprecated(note = "pass it in corundum_framework_service::ServiceSpec::legacy_labels")]
pub fn legacy_label(app_name: &str) -> String {
    format!("net.fireturtle.sapphire.{app_name}")
}

/// Where an agent under the [`legacy_label`] was installed.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
#[deprecated(note = "corundum_framework_service::labelled_agent_path(&legacy_label(app), home)")]
pub fn legacy_agent_path(app_name: &str, home: &std::path::Path) -> std::path::PathBuf {
    corundum_framework_service::labelled_agent_path(&legacy_label(app_name), home)
}

#[cfg(all(test, not(any(target_os = "ios", target_os = "android"))))]
mod tests {
    use super::*;

    #[test]
    fn the_re_exports_and_helpers_stay() {
        let spec = ServiceSpec {
            app_name: "sapphire-agent",
            description: String::new(),
            args: vec!["serve".into()],
            legacy_labels: vec![legacy_label("sapphire-agent")],
            post_install: None,
        };
        assert_eq!(
            spec.legacy_labels,
            ["net.fireturtle.sapphire.sapphire-agent"]
        );
        assert_eq!(
            label("sapphire-agent"),
            "org.sapphire-turtle.sapphire-agent"
        );
        assert!(
            legacy_agent_path("sapphire-agent", std::path::Path::new("/Users/a"))
                .ends_with("net.fireturtle.sapphire.sapphire-agent.plist")
        );
        let _: fn() -> ServiceCommand = || ServiceCommand::Status;
    }
}
