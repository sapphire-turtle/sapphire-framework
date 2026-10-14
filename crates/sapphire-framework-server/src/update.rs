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

    /// The repository's newest releases. Not `releases/latest`: a repository that tags
    /// several binaries (`cli-v…`, `desktop-v…`) has one "latest" among all of them.
    fn releases_url(&self) -> String {
        format!(
            "{}/repos/{}/{}/releases?per_page=100",
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
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
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

/// Ask GitHub for the newest release once.
async fn look(client: &reqwest::Client, check: &UpdateCheck) -> Result<ReleaseInfo, String> {
    let url = check.releases_url();
    let releases: Vec<GithubRelease> = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("asking {url}: {e}"))?
        .json()
        .await
        .map_err(|e| format!("reading the releases: {e}"))?;
    newest(&releases, &check.tag_prefix).ok_or_else(|| {
        format!(
            "{}/{} has no published release tagged {}<version>",
            check.owner, check.repo, check.tag_prefix
        )
    })
}

/// The newest published, non-prerelease release tagged `<prefix><semver>`. Tags of the
/// repository's other binaries, drafts, prereleases and tags that are not versions are
/// skipped.
fn newest(releases: &[GithubRelease], prefix: &str) -> Option<ReleaseInfo> {
    releases
        .iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let version = semver::Version::parse(r.tag_name.strip_prefix(prefix)?).ok()?;
            version.pre.is_empty().then_some((version, r))
        })
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(version, r)| ReleaseInfo {
            version: version.to_string(),
            url: r.html_url.clone(),
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
            draft: false,
            prerelease: false,
        }
    }

    /// The release `tag` names, as a look that found only it reports.
    fn found(tag: &str) -> Result<ReleaseInfo, String> {
        newest(&[release(tag)], "v").ok_or_else(|| format!("no release in {tag}"))
    }

    fn shared() -> SharedStatus {
        Arc::new(Mutex::new(UpdateStatus {
            enabled: true,
            ..UpdateStatus::default()
        }))
    }

    #[test]
    fn the_newest_release_is_the_highest_version_with_the_prefix() {
        // sapphire-journal tags its CLI and its desktop app in one repository, and the
        // desktop app's release can be the newest of all.
        let releases = [
            release("desktop-v0.1.0"),
            release("cli-v0.12.0"),
            release("cli-v0.9.3"),
            release("cli-v0.11.0"),
        ];
        let info = newest(&releases, "cli-v").unwrap();
        assert_eq!(info.version, "0.12.0");
        assert_eq!(info.url, "https://example.invalid/cli-v0.12.0");
        assert_eq!(newest(&releases, "desktop-v").unwrap().version, "0.1.0");
        // `v` does not reach into `desktop-v…`, nor `cli-v` into `sapphire-agent-cli-v…`.
        assert_eq!(newest(&releases, "v"), None);
        assert_eq!(
            newest(&[release("sapphire-agent-cli-v0.8.0")], "cli-v"),
            None
        );
    }

    #[test]
    fn drafts_prereleases_and_tags_that_are_not_versions_are_skipped() {
        let mut draft = release("v0.22.0");
        draft.draft = true;
        let mut flagged = release("v0.21.0");
        flagged.prerelease = true;
        let releases = [
            draft,
            flagged,
            release("v0.21.0-rc.1"),
            release("vnext"),
            release("v0.20.0"),
        ];
        assert_eq!(newest(&releases, "v").unwrap().version, "0.20.0");
    }

    #[test]
    fn only_a_newer_release_is_an_update() {
        let current = semver::Version::parse("0.19.0").unwrap();
        let status = shared();
        apply(&status, &current, found("v0.19.0"));
        assert!(!status.lock().unwrap().update_available);
        apply(&status, &current, found("v0.18.2"));
        assert!(!status.lock().unwrap().update_available);
        apply(&status, &current, found("v0.20.0"));
        let status = status.lock().unwrap();
        assert!(status.update_available);
        assert_eq!(status.latest.as_ref().unwrap().version, "0.20.0");
    }

    #[test]
    fn a_failed_look_keeps_the_last_release_and_a_success_clears_the_error() {
        let current = semver::Version::parse("0.19.0").unwrap();
        let status = shared();
        apply(&status, &current, found("v0.20.0"));
        apply(&status, &current, Err("offline".to_owned()));
        {
            let status = status.lock().unwrap();
            assert!(status.update_available);
            assert_eq!(status.last_error.as_deref(), Some("offline"));
        }
        apply(&status, &current, found("v0.20.0"));
        assert_eq!(status.lock().unwrap().last_error, None);
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("update-test/0.0.0")
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn a_look_reads_the_newest_release_from_the_api() {
        let base = one_response(
            "200 OK",
            r#"[{"tag_name":"desktop-v0.30.0","html_url":"https://github.com/o/r/releases/tag/desktop-v0.30.0","draft":false,"prerelease":false},
                {"tag_name":"v0.20.0","html_url":"https://github.com/o/r/releases/tag/v0.20.0","draft":false,"prerelease":false}]"#,
        )
        .await;
        let check = UpdateCheck::github("o", "r").api_base(base);
        let info = look(&client(), &check).await.unwrap();
        assert_eq!(info.version, "0.20.0");
        assert_eq!(info.url, "https://github.com/o/r/releases/tag/v0.20.0");
    }

    #[tokio::test]
    async fn a_repository_without_releases_is_a_failed_look() {
        let base = one_response("200 OK", "[]").await;
        let check = UpdateCheck::github("o", "r").api_base(base);
        let err = look(&client(), &check).await.unwrap_err();
        assert!(
            err.contains("no published release tagged v<version>"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_missing_repository_is_a_failed_look() {
        let base = one_response("404 Not Found", r#"{"message":"Not Found"}"#).await;
        let check = UpdateCheck::github("o", "r").api_base(base);
        let err = look(&client(), &check).await.unwrap_err();
        assert!(err.contains("404"), "{err}");
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
