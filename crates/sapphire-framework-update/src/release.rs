//! Where a binary's releases are, and which is the newest.

use std::time::Duration;

use serde::Deserialize;

use crate::{Error, Result};

/// How long one request may take before it counts as failed. Downloads have their own,
/// longer budget.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Where a binary's releases are published: a GitHub repository whose releases are tagged
/// `<prefix><version>`, and — for the `update` command — the file in them.
#[derive(Clone, Debug)]
pub struct ReleaseSource {
    owner: String,
    repo: String,
    tag_prefix: String,
    api_base: String,
    asset: Option<String>,
    cargo_crate: Option<String>,
}

impl ReleaseSource {
    /// The releases of `github.com/<owner>/<repo>`, tagged `v<version>`.
    pub fn github(owner: impl Into<String>, repo: impl Into<String>) -> ReleaseSource {
        ReleaseSource {
            owner: owner.into(),
            repo: repo.into(),
            tag_prefix: "v".to_owned(),
            api_base: "https://api.github.com".to_owned(),
            asset: None,
            cargo_crate: None,
        }
    }

    /// Release tags are `<prefix><version>` rather than `v<version>`, e.g. `cli-v` in a
    /// repository that also tags a desktop app.
    pub fn tag_prefix(mut self, prefix: impl Into<String>) -> ReleaseSource {
        self.tag_prefix = prefix.into();
        self
    }

    /// Ask another GitHub API root than `https://api.github.com`: for fixtures.
    pub fn api_base(mut self, base: impl Into<String>) -> ReleaseSource {
        self.api_base = base.into();
        self
    }

    /// The binary is published as `<name>-<os>-<arch>` (`.exe` on Windows): what `update`
    /// downloads. Without it, `update` only reports.
    pub fn asset(mut self, name: impl Into<String>) -> ReleaseSource {
        self.asset = Some(name.into());
        self
    }

    /// The crate `cargo install` installs this binary from, for the advice `update` gives
    /// a binary it will not replace.
    pub fn cargo_crate(mut self, name: impl Into<String>) -> ReleaseSource {
        self.cargo_crate = Some(name.into());
        self
    }

    /// `<owner>/<repo>`.
    pub fn repository(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    pub(crate) fn asset_base(&self) -> Option<&str> {
        self.asset.as_deref()
    }

    pub(crate) fn crate_name(&self) -> Option<&str> {
        self.cargo_crate.as_deref()
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

/// One published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Its version, the tag without its prefix.
    pub version: semver::Version,
    /// Its web page, where its notes are.
    pub url: String,
    /// Its files.
    pub assets: Vec<Asset>,
}

/// One file of a release.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Asset {
    /// The file's name.
    pub name: String,
    /// Where it downloads from.
    #[serde(rename = "browser_download_url")]
    pub download_url: String,
}

/// The part of GitHub's release object this reads.
#[derive(Debug, Deserialize)]
pub(crate) struct GithubRelease {
    pub(crate) tag_name: String,
    pub(crate) html_url: String,
    #[serde(default)]
    pub(crate) draft: bool,
    #[serde(default)]
    pub(crate) prerelease: bool,
    #[serde(default)]
    pub(crate) assets: Vec<Asset>,
}

/// An HTTP client that names itself `user_agent` (GitHub refuses requests without one).
pub fn client(user_agent: &str) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| Error::Http(format!("no HTTP client: {e}")))
}

/// Ask GitHub for `source`'s newest release.
pub async fn latest_release(client: &reqwest::Client, source: &ReleaseSource) -> Result<Release> {
    let url = source.releases_url();
    let releases: Vec<GithubRelease> = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| Error::Http(format!("asking {url}: {e}")))?
        .json()
        .await
        .map_err(|e| Error::Http(format!("reading the releases: {e}")))?;
    newest(releases, &source.tag_prefix).ok_or_else(|| {
        Error::NoRelease(format!(
            "{} has no published release tagged {}<version>",
            source.repository(),
            source.tag_prefix
        ))
    })
}

/// The newest published, non-prerelease release tagged `<prefix><semver>`. Tags of the
/// repository's other binaries, drafts, prereleases and tags that are not versions are
/// skipped.
pub(crate) fn newest(
    releases: impl IntoIterator<Item = GithubRelease>,
    prefix: &str,
) -> Option<Release> {
    releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let version = semver::Version::parse(r.tag_name.strip_prefix(prefix)?).ok()?;
            version.pre.is_empty().then_some(Release {
                version,
                url: r.html_url,
                assets: r.assets,
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn release(tag: &str) -> GithubRelease {
        GithubRelease {
            tag_name: tag.to_owned(),
            html_url: format!("https://example.invalid/{tag}"),
            draft: false,
            prerelease: false,
            assets: Vec::new(),
        }
    }

    #[test]
    fn the_newest_release_is_the_highest_version_with_the_prefix() {
        // sapphire-journal tags its CLI and its desktop app in one repository, and the
        // desktop app's release can be the newest of all.
        let releases = || {
            [
                release("desktop-v0.1.0"),
                release("cli-v0.12.0"),
                release("cli-v0.9.3"),
                release("cli-v0.11.0"),
            ]
        };
        let info = newest(releases(), "cli-v").unwrap();
        assert_eq!(info.version.to_string(), "0.12.0");
        assert_eq!(info.url, "https://example.invalid/cli-v0.12.0");
        assert_eq!(
            newest(releases(), "desktop-v").unwrap().version.to_string(),
            "0.1.0"
        );
        // `v` does not reach into `desktop-v…`, nor `cli-v` into `sapphire-agent-cli-v…`.
        assert_eq!(newest(releases(), "v"), None);
        assert_eq!(
            newest([release("sapphire-agent-cli-v0.8.0")], "cli-v"),
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
        assert_eq!(newest(releases, "v").unwrap().version.to_string(), "0.20.0");
    }
}
