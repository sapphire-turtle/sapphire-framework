//! What the framework is made of, stated where a reviewer will see it.

#[test]
fn the_http_sync_stack_is_gone() {
    let manifest = include_str!("../../../Cargo.toml");
    for gone in [
        "sapphire-framework-rpc",
        "sapphire-framework-remote-client",
        "sapphire-framework-remote-server",
        "sapphire-framework-blob",
    ] {
        assert!(
            !manifest.contains(gone),
            "{gone} was replaced by the process architecture; see \
             docs/superpowers/specs/2026-09-16-process-architecture-design.md §6"
        );
    }
}

#[test]
fn the_crates_that_replaced_it_are_present() {
    let manifest = include_str!("../../../Cargo.toml");
    for present in [
        "sapphire-framework-ipc",
        "sapphire-framework-server",
        "sapphire-framework-session",
        "sapphire-framework-bridge",
        "sapphire-framework-keys",
    ] {
        assert!(
            manifest.contains(present),
            "{present} is missing from the workspace"
        );
    }
}
