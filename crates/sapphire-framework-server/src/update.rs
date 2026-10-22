//! Looking for newer releases of the running server.
//!
//! An application names where its releases are published with
//! [`AppServer::update_check`](crate::AppServer::update_check); the server then asks
//! GitHub for the newest release once at start and once a day after, and answers
//! [`SERVER_UPDATE`](sapphire_backend::protocol::SERVER_UPDATE) with what it learnt.
//! `status` and the GUI show it.
//!
//! The server only looks. Installing is the `update` command's
//! ([`sapphire_framework_update::run`]), run by the user; restarting the server afterwards
//! is the user's too. `[update] check = false` in `config.toml` turns the looking off.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sapphire_backend::protocol::{ReleaseInfo, UpdateStatus};
use sapphire_framework_update::{Release, ReleaseSource};

/// How often the server looks, after the first look at start.
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// The shared [`UpdateStatus`] the looking task writes and `server.update` reads.
pub(crate) type SharedStatus = Arc<Mutex<UpdateStatus>>;

/// The looking task: aborted when dropped, so it ends with the server.
pub(crate) struct Checking(tokio::task::JoinHandle<()>);

impl Drop for Checking {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Start looking for releases newer than `current`, writing what is found into `status`.
///
/// A `current` that is not a semantic version cannot be compared with anything: the
/// check is reported as failed, and no task starts.
pub(crate) fn start(
    source: ReleaseSource,
    app: &str,
    current: &str,
    status: SharedStatus,
) -> Option<Checking> {
    let mut first = UpdateStatus {
        enabled: true,
        ..UpdateStatus::default()
    };
    let current = match semver::Version::parse(current) {
        Ok(version) => version,
        Err(e) => {
            first.last_error = Some(format!(
                "this server's version {current:?} is not semver: {e}"
            ));
            *status.lock().expect("update status lock") = first;
            return None;
        }
    };
    let client = match sapphire_framework_update::client(&format!("{app}/{current}")) {
        Ok(client) => client,
        Err(e) => {
            first.last_error = Some(e.to_string());
            *status.lock().expect("update status lock") = first;
            return None;
        }
    };
    *status.lock().expect("update status lock") = first;
    Some(Checking(tokio::spawn(async move {
        let mut ticks = tokio::time::interval(INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            let found = sapphire_framework_update::latest_release(&client, &source)
                .await
                .map_err(|e| e.to_string());
            apply(&status, &current, found);
        }
    })))
}

/// Record one look's outcome. A failure keeps the last release found.
fn apply(status: &SharedStatus, current: &semver::Version, found: Result<Release, String>) {
    let mut status = status.lock().expect("update status lock");
    match found {
        Ok(release) => {
            status.update_available = release.version > *current;
            if status.update_available {
                tracing::info!("{} is available: {}", release.version, release.url);
            }
            status.latest = Some(ReleaseInfo {
                version: release.version.to_string(),
                url: release.url,
            });
            status.last_error = None;
        }
        Err(e) => {
            tracing::debug!("looking for a newer release failed: {e}");
            status.last_error = Some(e);
        }
    }
}

/// A stand-in for the GitHub API, for tests here and in `command`.
#[cfg(test)]
pub(crate) mod stub {
    /// Serve one canned HTTP response on a loopback port, and return the API root.
    pub(crate) async fn one_response(status_line: &'static str, body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        format!("http://{addr}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> SharedStatus {
        Arc::new(Mutex::new(UpdateStatus {
            enabled: true,
            ..UpdateStatus::default()
        }))
    }

    /// A look that found `version`.
    fn found(version: &str) -> Result<Release, String> {
        Ok(Release {
            version: semver::Version::parse(version).unwrap(),
            url: format!("https://example.invalid/v{version}"),
            assets: Vec::new(),
        })
    }

    #[test]
    fn only_a_newer_release_is_an_update() {
        let current = semver::Version::parse("0.19.0").unwrap();
        let status = shared();
        apply(&status, &current, found("0.19.0"));
        assert!(!status.lock().unwrap().update_available);
        apply(&status, &current, found("0.18.2"));
        assert!(!status.lock().unwrap().update_available);
        apply(&status, &current, found("0.20.0"));
        let status = status.lock().unwrap();
        assert!(status.update_available);
        assert_eq!(status.latest.as_ref().unwrap().version, "0.20.0");
    }

    #[test]
    fn a_failed_look_keeps_the_last_release_and_a_success_clears_the_error() {
        let current = semver::Version::parse("0.19.0").unwrap();
        let status = shared();
        apply(&status, &current, found("0.20.0"));
        apply(&status, &current, Err("offline".to_owned()));
        {
            let status = status.lock().unwrap();
            assert!(status.update_available);
            assert_eq!(status.last_error.as_deref(), Some("offline"));
        }
        apply(&status, &current, found("0.20.0"));
        assert_eq!(status.lock().unwrap().last_error, None);
    }

    #[tokio::test]
    async fn a_server_version_that_is_not_semver_reports_and_does_not_look() {
        let status = shared();
        let checking = start(
            ReleaseSource::github("o", "r").api_base("http://127.0.0.1:9"),
            "app",
            "dev",
            Arc::clone(&status),
        );
        assert!(checking.is_none());
        let status = status.lock().unwrap();
        assert!(status.enabled);
        assert!(status.last_error.as_deref().unwrap().contains("not semver"));
    }
}
