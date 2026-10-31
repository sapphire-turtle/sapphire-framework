//! The apps' `workgroup`, `device` and `embedding` moved to `sapphire-bridge` (#363): an
//! app's command line no longer has them, and still has `config`.

use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct Run {
    code: i32,
    err: String,
}

async fn cli(tmp: &std::path::Path, args: &[&str]) -> Run {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_server-test-app"))
        .arg(tmp.join("run"))
        .arg(tmp.join("state"))
        .args(args)
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(DEADLINE, output)
        .await
        .expect("the CLI finished in time")
        .unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        err: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[tokio::test]
async fn the_bridge_s_subcommands_are_gone() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    for args in [
        &["workgroup", "show"][..],
        &["device", "list"],
        &["embedding", "show"],
    ] {
        let run = cli(tmp.path(), args).await;
        assert_ne!(run.code, 0, "{args:?}: {run:?}");
        assert!(
            run.err.contains("unrecognized subcommand"),
            "{args:?}: {run:?}"
        );
    }
}

#[tokio::test]
async fn config_is_still_there() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let run = cli(tmp.path(), &["config", "list"]).await;
    assert_eq!(run.code, 0, "{run:?}");
}
