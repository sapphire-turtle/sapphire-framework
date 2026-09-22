//! The facade's features, and what each one brings.

#[test]
fn the_old_features_are_gone() {
    let manifest = include_str!("../Cargo.toml");
    for gone in ["remote-client", "remote-server", "\nrpc =", "\nblob ="] {
        assert!(
            !manifest.contains(gone),
            "the feature {gone:?} still exists"
        );
    }
}

#[test]
fn every_module_feature_has_a_matching_optional_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for feature in [
        "workspace",
        "retrieve",
        "track",
        "sync",
        "session",
        "ipc",
        "server",
        "bridge",
        "keys",
        "service",
        "registry",
        "backend",
        "gui",
    ] {
        assert!(
            manifest.contains(&format!("sapphire-framework-{feature} =")),
            "the feature {feature} has no dependency behind it"
        );
    }
}

#[test]
fn native_includes_what_a_host_needs() {
    let manifest = include_str!("../Cargo.toml");
    let line = manifest
        .lines()
        .find(|l| l.starts_with("native ="))
        .expect("a native feature");
    for part in ["workspace", "backend", "server", "bridge"] {
        assert!(line.contains(part), "native is missing {part}: {line}");
    }
}
