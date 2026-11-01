//! `sapphire-bridge-desktop <subcommand>` runs the command line's own code.

use std::process::Command;

#[test]
fn a_subcommand_runs_the_cli() {
    let exe = env!("CARGO_BIN_EXE_sapphire-bridge-desktop");
    let version = Command::new(exe).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(env!("CARGO_PKG_VERSION")),
        "{version:?}"
    );

    // No bridge in an empty runtime directory: `status` says so, as `sapphire-bridge` does.
    let tmp = tempfile::tempdir().unwrap();
    let status = Command::new(exe)
        .arg("status")
        .env("SAPPHIRE_RUNTIME_DIR", tmp.path())
        .env("SAPPHIRE_BRIDGE_DIR", tmp.path().join("bridge"))
        .output()
        .unwrap();
    assert!(!status.status.success(), "{status:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(text.contains("sapphire-bridge"), "{text}");
}
