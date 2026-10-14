//! Looking for newer releases of the running server.
//!
//! An application names where its releases are published with
//! [`AppServer::update_check`](crate::AppServer::update_check); the server then asks
//! GitHub for the latest release once at start and once a day after, and answers
//! [`SERVER_UPDATE`](sapphire_backend::protocol::SERVER_UPDATE) with what it learnt.
//! `status` and the GUI show it.
//!
//! This only looks. Nothing is downloaded, verified or installed here: installing a
//! release — and trusting it — stays with the user and their package manager, and so
//! does restarting the server afterwards. `[update] check = false` in `server.toml`
//! turns the looking off.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sapphire_backend::protocol::{ReleaseInfo, UpdateStatus};
use serde::Deserialize;

/// How often the server looks, after the first look at start.
const DEFAULT_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// How long one look may take before it counts as failed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Where an application's releases are published: a GitHub repository whose releases are
/// tagged `<prefix><version>`.
#[derive(Clone, Debug)]
pub struct UpdateCheck {
    owner: String,
    repo: String,
    tag_prefix: String,
    api_base: String,
    interval: Duration,
}

impl UpdateCheck {
    /// The releases of `github.com/<owner>/<repo>`, tagged `v<version>`.
    pub fn github(owner: impl Into<String>, repo: impl Into<String>) -> UpdateCheck {
        UpdateCheck {
            owner: owner.into(),
            repo: repo.into(),
            tag_prefix: "v".to_owned(),
            api_base: "https://api.github.com".to_owned(),
            interval: DEFAULT_INTERVAL,
        }
    }

    /// Release tags are `<prefix><version>` rather than `v<version>`, e.g.
    /// `sapphire-journal-v`.
    pub fn tag_prefix(mut self, prefix: impl Into<String>) -> UpdateCheck {
        self.tag_prefix = prefix.into();
        self
    }

    /// Ask another GitHub API root than `https://api.github.com`: for fixtures.
    pub fn api_base(mut self, base: impl Into<String>) -> UpdateCheck {
        self.api_base = base.into();
        self
    }

    /// Look this often after the first look, rather than once a day.
    pub fn interval(mut self, interval: Duration) -> UpdateCheck {
        self.interval = interval;
        self
    }

    fn latest_url(&self) -> String {
        format!(
            "{}/repos/{}/{}/releases/latest",
            self.api_base.trim_end_matches('/'),
            self.owner,
            self.repo
        )
    }
}

/// The part of GitHub's release object a look reads.
#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
}

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
    check: UpdateCheck,
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
    let client = match reqwest::Client::builder()
        .user_agent(format!("{app}/{current}"))
        .timeout(REQUEST_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            first.last_error = Some(format!("no HTTP client: {e}"));
            *status.lock().expect("update status lock") = first;
            return None;
        }
    };
    *status.lock().expect("update status lock") = first;
    Some(Checking(tokio::spawn(async move {
        let mut ticks = tokio::time::interval(check.interval);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            let found = look(&client, &check).await;
            apply(&status, &current, found);
        }
    })))
}

/// Ask GitHub for the latest release once.
async fn look(client: &reqwest::Client, check: &UpdateCheck) -> Result<ReleaseInfo, String> {
    let response = client
        .get(check.latest_url())
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("asking {}: {e}", check.latest_url()))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!(
            "{}/{} has no published release",
            check.owner, check.repo
        ));
    }
    let release: GithubRelease = response
        .error_for_status()
        .map_err(|e| format!("asking {}: {e}", check.latest_url()))?
        .json()
        .await
        .map_err(|e| format!("reading the latest release: {e}"))?;
    release_info(&release, &check.tag_prefix)
}

/// The release a tag names, or why the tag is not `<prefix><semver>`.
fn release_info(release: &GithubRelease, prefix: &str) -> Result<ReleaseInfo, String> {
    let version = release.tag_name.strip_prefix(prefix).ok_or_else(|| {
        format!(
            "the latest release's tag {:?} does not start with {prefix:?}",
            release.tag_name
        )
    })?;
    semver::Version::parse(version).map_err(|e| {
        format!(
            "the latest release's tag {:?} is not a version: {e}",
            release.tag_name
        )
    })?;
    Ok(ReleaseInfo {
        version: version.to_owned(),
        url: release.html_url.clone(),
    })
}

/// Record one look's outcome. A failure keeps the last release found.
fn apply(status: &SharedStatus, current: &semver::Version, found: Result<ReleaseInfo, String>) {
    let mut status = status.lock().expect("update status lock");
    match found {
        Ok(release) => {
            if let Ok(latest) = semver::Version::parse(&release.version) {
                status.update_available = latest > *current;
                if status.update_available {
                    tracing::info!("{} is available: {}", release.version, release.url);
                }
            }
            status.latest = Some(release);
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
    use super::stub::one_response;
    use super::*;

    fn release(tag: &str) -> GithubRelease {
        GithubRelease {
            tag_name: tag.to_owned(),
            html_url: format!("https://example.invalid/{tag}"),
        }
    }

    fn shared() -> SharedStatus {
        Arc::new(Mutex::new(UpdateStatus {
            enabled: true,
            ..UpdateStatus::default()
        }))
    }

    #[test]
    fn a_tag_is_its_prefix_and_a_version() {
        let info = release_info(&release("v0.20.0"), "v").unwrap();
        assert_eq!(info.version, "0.20.0");
        let info = release_info(&release("sapphire-journal-v1.2.3"), "sapphire-journal-v").unwrap();
        assert_eq!(info.version, "1.2.3");
        assert!(release_info(&release("0.20.0"), "v").is_err());
        assert!(release_info(&release("vnext"), "v").is_err());
    }

    #[test]
    fn only_a_newer_release_is_an_update() {
        let current = semver::Version::parse("0.19.0").unwrap();
        let status = shared();
        apply(&status, &current, release_info(&release("v0.19.0"), "v"));
        assert!(!status.lock().unwrap().update_available);
        apply(&status, &current, release_info(&release("v0.18.2"), "v"));
        assert!(!status.lock().unwrap().update_available);
        apply(&status, &current, release_info(&release("v0.20.0"), "v"));
        let status = status.lock().unwrap();
        assert!(status.update_available);
        assert_eq!(status.latest.as_ref().unwrap().version, "0.20.0");
    }

    #[test]
    fn a_failed_look_keeps_the_last_release_and_a_success_clears_the_error() {
        let current = semver::Version::parse("0.19.0").unwrap();
        let status = shared();
        apply(&status, &current, release_info(&release("v0.20.0"), "v"));
        apply(&status, &current, Err("offline".to_owned()));
        {
            let status = status.lock().unwrap();
            assert!(status.update_available);
            assert_eq!(status.last_error.as_deref(), Some("offline"));
        }
        apply(&status, &current, release_info(&release("v0.20.0"), "v"));
        assert_eq!(status.lock().unwrap().last_error, None);
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("update-test/0.0.0")
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn a_look_reads_the_latest_release_from_the_api() {
        let base = one_response(
            "200 OK",
            r#"{"tag_name":"v0.20.0","html_url":"https://github.com/o/r/releases/tag/v0.20.0","draft":false}"#,
        )
        .await;
        let check = UpdateCheck::github("o", "r").api_base(base);
        let info = look(&client(), &check).await.unwrap();
        assert_eq!(info.version, "0.20.0");
        assert_eq!(info.url, "https://github.com/o/r/releases/tag/v0.20.0");
    }

    #[tokio::test]
    async fn a_repository_without_releases_is_a_failed_look() {
        let base = one_response("404 Not Found", r#"{"message":"Not Found"}"#).await;
        let check = UpdateCheck::github("o", "r").api_base(base);
        let err = look(&client(), &check).await.unwrap_err();
        assert!(err.contains("no published release"), "{err}");
    }

    #[tokio::test]
    async fn a_server_version_that_is_not_semver_reports_and_does_not_look() {
        let status = shared();
        let checking = start(
            UpdateCheck::github("o", "r").api_base("http://127.0.0.1:9"),
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
