//! `packager-demo --version`, or `packager-demo update <endpoint>`: checks the endpoint's
//! JSON for a newer version, verifies its minisign signature with the public key built in
//! (`PACKAGER_DEMO_PUBKEY` at build time) and installs it.

use cargo_packager_updater::{Config, check_update, semver::Version, url::Url};

fn main() {
    let version = env!("CARGO_PKG_VERSION");
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--version") => println!("packager-demo {version}"),
        Some("update") => {
            let endpoint = args.get(2).expect("usage: packager-demo update <endpoint>");
            let config = Config {
                endpoints: vec![Url::parse(endpoint).expect("endpoint URL")],
                pubkey: env!("PACKAGER_DEMO_PUBKEY").to_owned(),
                windows: None,
            };
            match check_update(Version::parse(version).unwrap(), config) {
                Ok(Some(update)) => {
                    println!("found {} (format {:?})", update.version, update.format);
                    match update.download_and_install() {
                        Ok(()) => println!("installed {}", update.version),
                        Err(e) => {
                            eprintln!("install failed: {e}");
                            std::process::exit(2);
                        }
                    }
                }
                Ok(None) => println!("up to date ({version})"),
                Err(e) => {
                    eprintln!("check failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => println!("packager-demo {version}: --version | update <endpoint>"),
    }
}
