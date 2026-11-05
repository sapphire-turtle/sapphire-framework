//! Writes Sapphire Bridge's icon as `icon.ico`, `icon.icns` and `icon.png` (512 px) into the
//! directory given, for the package's icons in the release workflow (`release-bridge.yml`).
//! The icns is drawn from `icon-macos.svg`: the mark on the brand's tile.

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn main() -> std::io::Result<()> {
    use corundum_framework_platform::icon;

    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: icons <dir>"));
    std::fs::create_dir_all(&dir)?;
    let svg = include_bytes!("../../headless/docs/icon.svg");
    // macOS shows the icon as it is drawn, so its own takes the brand's tile; a mark that
    // does not fill the icon grid would be set on the system's plate instead.
    let macos_svg = include_bytes!("../../headless/docs/icon-macos.svg");
    std::fs::write(dir.join("icon.ico"), icon::ico(svg)?)?;
    std::fs::write(dir.join("icon.icns"), icon::icns(macos_svg)?)?;
    std::fs::write(dir.join("icon.png"), icon::render_png(svg, 512)?)?;
    Ok(())
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn main() {}
