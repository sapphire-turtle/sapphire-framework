//! Replacing the running executable with a newer release.

use std::io::{BufRead, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::release::{Release, ReleaseSource, latest_release};
use crate::{Error, Result};

/// The most a download may weigh. A release binary is tens of megabytes; this only stops a
/// runaway response from filling memory.
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

/// How long one download may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The `update` subcommand's arguments: replace this executable with its newest release.
#[derive(Clone, Debug, Default, clap::Args)]
pub struct UpdateArgs {
    /// Only say whether a newer release exists; download nothing.
    #[arg(long)]
    pub check: bool,
    /// Install without asking first.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

/// How the running executable came to be where it is, which decides whether `update` may
/// replace it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallMethod {
    /// Downloaded from a release and placed by hand: `update` replaces it.
    Release,
    /// `cargo install`, under `$CARGO_HOME/bin`: cargo's records would no longer match a
    /// replaced file, so `update` says to run `cargo install` instead.
    Cargo,
    /// `cargo binstall`, under `$CARGO_HOME/bin` and recorded in
    /// `$CARGO_HOME/binstall/crates-v1.json`: `update` says to run `cargo binstall` again.
    Binstall {
        /// The crate it was installed from.
        crate_name: String,
        /// Where that crate came from.
        source: BinstallSource,
    },
    /// A system or package-manager location (`/usr/bin`, the Nix store, Homebrew): the
    /// package manager updates it.
    PackageManager,
}

/// Where `cargo binstall` took a crate from, which decides how to run it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BinstallSource {
    /// crates.io: `cargo binstall <crate>`.
    Registry,
    /// A git repository, by its URL without the commit: `cargo binstall --git <url> <crate>`.
    /// The crates that are only released on GitHub, such as the desktop apps.
    Git(String),
    /// A local manifest, or a source this does not know.
    Other,
}

/// How the executable at `exe` was installed, judged by where it is and, under
/// `$CARGO_HOME/bin`, by `cargo binstall`'s records. `cargo_home` is `$CARGO_HOME`, or
/// `~/.cargo` when unset.
pub fn install_method(exe: &Path, cargo_home: Option<&Path>) -> InstallMethod {
    if let Some(home) = cargo_home
        && exe.starts_with(home.join("bin"))
    {
        let records = std::fs::read_to_string(home.join("binstall").join("crates-v1.json"));
        let bin = exe.file_name().and_then(|name| name.to_str()).map(|name| {
            name.strip_suffix(std::env::consts::EXE_SUFFIX)
                .unwrap_or(name)
        });
        return match (records, bin) {
            (Ok(records), Some(bin)) => binstalled(&records, bin),
            _ => None,
        }
        .unwrap_or(InstallMethod::Cargo);
    }
    // `/usr/local` is where binaries placed by hand go; the rest of `/usr` is the
    // distribution's.
    if exe.starts_with("/usr/local") {
        return InstallMethod::Release;
    }
    const MANAGED: [&str; 7] = [
        "/usr",
        "/bin",
        "/sbin",
        "/nix",
        "/opt/homebrew",
        "/home/linuxbrew",
        "/snap",
    ];
    if MANAGED.iter().any(|dir| exe.starts_with(dir)) {
        return InstallMethod::PackageManager;
    }
    InstallMethod::Release
}

/// One crate in `$CARGO_HOME/binstall/crates-v1.json`, as far as this reads it.
#[derive(Deserialize)]
struct BinstallRecord {
    name: String,
    #[serde(default)]
    source: Option<BinstallRecordSource>,
    #[serde(default)]
    bins: Vec<String>,
}

#[derive(Deserialize)]
struct BinstallRecordSource {
    source_type: String,
    url: String,
}

/// How `cargo binstall` installed the binary `bin`, from `records`, the contents of
/// `$CARGO_HOME/binstall/crates-v1.json`: one JSON object per crate, one after another. `None`
/// when binstall did not install it, which leaves `cargo install`.
///
/// A binary binstall installed and `cargo install` replaced afterwards still reads as
/// binstall's; running binstall again puts the release back, which is what `update` is for.
fn binstalled(records: &str, bin: &str) -> Option<InstallMethod> {
    serde_json::Deserializer::from_str(records)
        .into_iter::<BinstallRecord>()
        .map_while(std::result::Result::ok)
        .filter(|record| record.bins.iter().any(|b| b == bin))
        .last()
        .map(|record| InstallMethod::Binstall {
            source: match record.source {
                Some(source) if source.source_type == "Registry" => BinstallSource::Registry,
                Some(source) if source.source_type == "Git" => {
                    let url = source
                        .url
                        .split_once('#')
                        .map_or(source.url.as_str(), |(u, _)| u);
                    BinstallSource::Git(url.to_owned())
                }
                _ => BinstallSource::Other,
            },
            crate_name: record.name,
        })
}

/// This machine as release files name it: `linux-x86_64`, `macos-aarch64`,
/// `windows-x86_64`.
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The file a release publishes `base` under for this machine:
/// `<base>-<os>-<arch>`, with `.exe` on Windows.
pub fn asset_name(base: &str) -> String {
    format!("{base}-{}{}", platform(), std::env::consts::EXE_SUFFIX)
}

/// Write `bytes` beside `exe` and hand the new file to `replace`, which puts it in `exe`'s
/// place. The staging file is removed afterwards, whatever `replace` did.
///
/// The staging file is in `exe`'s directory so the replacement is a rename on one file
/// system, never a half-written executable.
pub fn install(
    bytes: &[u8],
    exe: &Path,
    replace: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<()> {
    let dir = exe.parent().ok_or_else(|| {
        Error::Io(std::io::Error::other(format!(
            "{} has no directory",
            exe.display()
        )))
    })?;
    let mut staged = tempfile::Builder::new()
        .prefix(".update-")
        .tempfile_in(dir)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(staged.path(), std::fs::Permissions::from_mode(0o755))?;
    }
    replace(staged.path())?;
    Ok(())
}

/// Download `url`, refusing anything larger than [`MAX_DOWNLOAD`].
async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| Error::Http(format!("downloading {url}: {e}")))?;
    if response.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
        return Err(Error::Http(format!(
            "{url} is larger than {MAX_DOWNLOAD} bytes"
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| Error::Http(format!("downloading {url}: {e}")))?;
    if bytes.len() as u64 > MAX_DOWNLOAD {
        return Err(Error::Http(format!(
            "{url} is larger than {MAX_DOWNLOAD} bytes"
        )));
    }
    Ok(bytes.to_vec())
}

/// Download `release`'s file for this machine.
async fn fetch(
    client: &reqwest::Client,
    source: &ReleaseSource,
    release: &Release,
    base: &str,
) -> Result<Vec<u8>> {
    let name = asset_name(base);
    let binary = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| {
            Error::NoAsset(format!(
                "{} {} has no {name}",
                source.repository(),
                release.version
            ))
        })?;
    let bytes = download(client, &binary.download_url).await?;
    if bytes.is_empty() {
        return Err(Error::Http(format!("{name} downloaded empty")));
    }
    Ok(bytes)
}

/// Where the user installs what this binary is, by hand or with cargo, as advice.
fn manual_advice(source: &ReleaseSource, release: &Release, method: &InstallMethod) -> String {
    match (method, source.crate_name()) {
        (InstallMethod::Cargo, Some(name)) => {
            format!("installed with cargo; run: cargo install {name} --locked")
        }
        (InstallMethod::Cargo, None) => {
            "installed with cargo; update it with cargo install".to_owned()
        }
        (InstallMethod::Binstall { crate_name, source }, _) => match source {
            BinstallSource::Registry => {
                format!("installed with cargo binstall; run: cargo binstall {crate_name}")
            }
            BinstallSource::Git(url) => format!(
                "installed with cargo binstall; run: cargo binstall --git {url} {crate_name}"
            ),
            BinstallSource::Other => {
                "installed with cargo binstall; update it with cargo binstall".to_owned()
            }
        },
        (InstallMethod::PackageManager, _) => {
            "installed by a package manager; update it with that".to_owned()
        }
        (InstallMethod::Release, _) => format!("download it from {}", release.url),
    }
}

/// `$CARGO_HOME`, or `~/.cargo`.
fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".cargo")))
}

/// Carry out `update` for the binary `name` at `current`, whose releases `source` names;
/// returns the exit code.
///
/// Prints what it finds and does. Exit 0 is up to date, installed, or (with `--check`) a
/// newer release found; 1 is a newer release this binary does not install itself.
pub async fn run(
    args: &UpdateArgs,
    source: &ReleaseSource,
    name: &str,
    current: &str,
) -> Result<i32> {
    let current_v = semver::Version::parse(current).map_err(|e| {
        Error::NotConfigured(format!(
            "this binary's version {current:?} is not semver: {e}"
        ))
    })?;
    let client = crate::client(&format!("{name}/{current}"))?;
    let release = latest_release(&client, source).await?;
    if release.version <= current_v {
        println!(
            "{name} {current} is up to date (latest release {})",
            release.version
        );
        return Ok(0);
    }
    println!(
        "{name} {} is available (installed: {current}): {}",
        release.version, release.url
    );
    if args.check {
        return Ok(0);
    }

    let Some(base) = source.asset_base() else {
        println!(
            "{name} does not update itself; {}",
            manual_advice(source, &release, &InstallMethod::Release)
        );
        return Ok(1);
    };
    let exe = std::env::current_exe()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let method = install_method(&exe, cargo_home().as_deref());
    if method != InstallMethod::Release {
        println!(
            "{} is {}",
            exe.display(),
            manual_advice(source, &release, &method)
        );
        return Ok(1);
    }

    if !args.yes
        && !confirm(&format!(
            "replace {} with {}?",
            exe.display(),
            release.version
        ))?
    {
        println!("not updated");
        return Ok(1);
    }
    let bytes = fetch(&client, source, &release, base).await?;
    install(&bytes, &exe, |staged| self_replace::self_replace(staged))?;
    println!(
        "installed {name} {}; restart the {name} service to run it",
        release.version
    );
    Ok(0)
}

/// Ask `question` on the terminal; only `y` or `yes` is a yes.
fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    #[test]
    fn a_binary_is_replaced_only_where_it_was_placed_by_hand() {
        let home = Path::new("/home/u/.cargo");
        let method = |p: &str| install_method(Path::new(p), Some(home));
        assert_eq!(method("/home/u/.cargo/bin/app"), InstallMethod::Cargo);
        assert_eq!(method("/usr/bin/app"), InstallMethod::PackageManager);
        assert_eq!(
            method("/nix/store/abc-app/bin/app"),
            InstallMethod::PackageManager
        );
        assert_eq!(
            method("/opt/homebrew/bin/app"),
            InstallMethod::PackageManager
        );
        assert_eq!(method("/usr/local/bin/app"), InstallMethod::Release);
        assert_eq!(method("/home/u/.local/bin/app"), InstallMethod::Release);
    }

    #[test]
    fn binstall_s_records_name_the_crate_and_where_it_came_from() {
        let records = concat!(
            r#"{"name":"dioxus-cli","version_req":"*","current_version":"0.7.3","#,
            r#""source":{"source_type":"Registry","url":"https://github.com/rust-lang/crates.io-index"},"#,
            r#""target":"x86_64-unknown-linux-gnu","bins":["dx"]}"#,
            r#"{"name":"sapphire-journal-desktop","version_req":"*","current_version":"0.3.0","#,
            r#""source":{"source_type":"Git","url":"https://github.com/sapphire-turtle/sapphire-journal#b05d5c2"},"#,
            r#""target":"x86_64-unknown-linux-gnu","bins":["sapphire-journal-desktop"]}"#,
        );
        assert_eq!(
            binstalled(records, "dx"),
            Some(InstallMethod::Binstall {
                crate_name: "dioxus-cli".to_owned(),
                source: BinstallSource::Registry,
            })
        );
        assert_eq!(
            binstalled(records, "sapphire-journal-desktop"),
            Some(InstallMethod::Binstall {
                crate_name: "sapphire-journal-desktop".to_owned(),
                source: BinstallSource::Git(
                    "https://github.com/sapphire-turtle/sapphire-journal".to_owned()
                ),
            })
        );
        assert_eq!(binstalled(records, "sapphire-journal"), None);
        assert_eq!(binstalled("", "dx"), None);
    }

    #[test]
    fn a_binary_under_cargo_s_bin_is_binstall_s_only_when_it_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp
            .path()
            .join("bin")
            .join(format!("dx{}", std::env::consts::EXE_SUFFIX));
        assert_eq!(install_method(&exe, Some(tmp.path())), InstallMethod::Cargo);
        std::fs::create_dir(tmp.path().join("binstall")).unwrap();
        std::fs::write(
            tmp.path().join("binstall").join("crates-v1.json"),
            r#"{"name":"dioxus-cli","source":{"source_type":"Registry","url":""},"bins":["dx"]}"#,
        )
        .unwrap();
        assert_eq!(
            install_method(&exe, Some(tmp.path())),
            InstallMethod::Binstall {
                crate_name: "dioxus-cli".to_owned(),
                source: BinstallSource::Registry,
            }
        );
    }

    #[test]
    fn the_asset_names_this_machine() {
        let name = asset_name("sapphire-bridge");
        assert!(name.starts_with("sapphire-bridge-"));
        assert!(name.contains(std::env::consts::OS));
        assert!(name.contains(std::env::consts::ARCH));
    }

    #[test]
    fn install_puts_the_bytes_in_place_and_leaves_no_staging_file() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("app");
        std::fs::write(&exe, b"old").unwrap();
        install(b"new", &exe, |staged| {
            std::fs::copy(staged, &exe).map(|_| ())
        })
        .unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        let left: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(left.len(), 1, "the staging file is gone");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let staged_mode = std::fs::metadata(&exe).unwrap().permissions().mode();
            assert_eq!(
                staged_mode & 0o111,
                0o111,
                "the copy keeps the executable bit"
            );
        }
    }

    /// Serve the routes `routes` builds from the server's own root (path → body) on a
    /// loopback port until the test ends; returns the root.
    async fn serve(routes: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let routes = routes(&root);
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap();
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
                let (status, body) = match routes.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => ("200 OK", body.clone()),
                    None => ("404 Not Found", Vec::new()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        root
    }

    const RELEASES: &str = "/repos/o/r/releases?per_page=100";

    /// A repository with release 0.20.0 of `app`: its file for this machine.
    async fn repository(binary: &[u8]) -> String {
        let name = asset_name("app");
        let binary = binary.to_vec();
        serve(move |root| {
            let releases = format!(
                r#"[{{"tag_name":"v0.20.0","html_url":"{root}/r/v0.20.0","assets":[
                    {{"name":"{name}","browser_download_url":"{root}/dl/{name}"}}]}}]"#
            );
            vec![
                (RELEASES.to_owned(), releases.into_bytes()),
                (format!("/dl/{name}"), binary),
            ]
        })
        .await
    }

    #[tokio::test]
    async fn a_release_is_fetched_end_to_end() {
        let root = repository(b"new binary").await;
        let source = ReleaseSource::github("o", "r").api_base(&root).asset("app");
        let client = crate::client("test/0.0.0").unwrap();
        let release = latest_release(&client, &source).await.unwrap();
        assert_eq!(release.version, v("0.20.0"));
        let bytes = fetch(&client, &source, &release, "app").await.unwrap();
        assert_eq!(bytes, b"new binary");
    }

    #[tokio::test]
    async fn an_empty_download_is_refused() {
        let root = repository(b"").await;
        let source = ReleaseSource::github("o", "r").api_base(&root).asset("app");
        let client = crate::client("test/0.0.0").unwrap();
        let release = latest_release(&client, &source).await.unwrap();
        let err = fetch(&client, &source, &release, "app").await.unwrap_err();
        assert!(matches!(err, Error::Http(_)), "{err}");
    }

    #[tokio::test]
    async fn a_release_without_this_platforms_file_is_refused() {
        let root = serve(|_| {
            vec![(
                RELEASES.to_owned(),
                br#"[{"tag_name":"v0.20.0","html_url":"https://example.invalid","assets":[]}]"#
                    .to_vec(),
            )]
        })
        .await;
        let source = ReleaseSource::github("o", "r").api_base(&root).asset("app");
        let client = crate::client("test/0.0.0").unwrap();
        let release = latest_release(&client, &source).await.unwrap();
        let err = fetch(&client, &source, &release, "app").await.unwrap_err();
        assert!(matches!(err, Error::NoAsset(_)), "{err}");
    }
}
