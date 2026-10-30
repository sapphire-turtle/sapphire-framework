//! The executable a service manager is told to start: the running one, as the outside sees
//! it.
//!
//! Usually that is [`std::env::current_exe`]. Inside an AppImage it is not: the image is
//! mounted afresh under `/tmp/.mount_*` at every start, so the path of the running binary
//! is gone by the next boot. The one path that lasts is the image itself, which the
//! AppImage runtime puts in `$APPIMAGE` (and the mount in `$APPDIR`); starting the image
//! with `serve` starts the same binary with `serve` (#366).
//!
//! The variables are inherited by whatever the image starts, so they only count for a
//! binary inside `$APPDIR`: a `sapphire-bridge` started from an AppImage'd app is not the
//! image. corundum-framework-platform's `package::current()` will answer this for every
//! package kind once it exists (corundum-framework#51); until then, `$APPIMAGE` is read
//! here.

use std::path::{Path, PathBuf};

/// The executable a service manager should start to run this one again: `$APPIMAGE`
/// inside an AppImage, the running executable everywhere else.
pub fn service_exe() -> std::io::Result<PathBuf> {
    let current = std::env::current_exe()?;
    Ok(resolve(
        current,
        std::env::var_os("APPIMAGE").map(PathBuf::from),
        std::env::var_os("APPDIR").map(PathBuf::from),
    ))
}

/// [`service_exe`]'s rule, on values: the image when `current` runs from inside its mount.
fn resolve(current: PathBuf, appimage: Option<PathBuf>, appdir: Option<PathBuf>) -> PathBuf {
    match (appimage, appdir) {
        (Some(image), Some(dir)) if !image.as_os_str().is_empty() && inside(&current, &dir) => {
            image
        }
        _ => current,
    }
}

fn inside(path: &Path, dir: &Path) -> bool {
    !dir.as_os_str().is_empty() && path.starts_with(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn outside_an_appimage_it_is_the_running_executable() {
        assert_eq!(
            resolve(p("/usr/bin/sapphire-journal"), None, None),
            p("/usr/bin/sapphire-journal")
        );
    }

    #[test]
    fn inside_an_appimage_it_is_the_image() {
        assert_eq!(
            resolve(
                p("/tmp/.mount_SapphAb12/usr/bin/sapphire-journal"),
                Some(p("/home/u/Applications/SapphireJournal.AppImage")),
                Some(p("/tmp/.mount_SapphAb12")),
            ),
            p("/home/u/Applications/SapphireJournal.AppImage")
        );
    }

    #[test]
    fn a_process_started_from_an_appimage_is_not_the_image() {
        // The variables are inherited: a bridge the app started still sees them.
        assert_eq!(
            resolve(
                p("/home/u/.local/bin/sapphire-bridge"),
                Some(p("/home/u/Applications/SapphireJournal.AppImage")),
                Some(p("/tmp/.mount_SapphAb12")),
            ),
            p("/home/u/.local/bin/sapphire-bridge")
        );
    }

    #[test]
    fn empty_variables_count_for_nothing() {
        let current = p("/tmp/.mount_x/usr/bin/app");
        assert_eq!(
            resolve(current.clone(), Some(p("")), Some(p("/tmp/.mount_x"))),
            current
        );
        assert_eq!(
            resolve(current.clone(), Some(p("/a.AppImage")), Some(p(""))),
            current
        );
        assert_eq!(
            resolve(current.clone(), Some(p("/a.AppImage")), None),
            current
        );
    }
}
