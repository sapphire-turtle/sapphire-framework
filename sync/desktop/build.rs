//! Embeds Sapphire Sync's icon in the Windows executables (the window's and the console shim's),
//! which the installer's shortcuts, the taskbar and "Apps & features" show. Nothing
//! elsewhere: the other platforms take the icon from the bundle or the desktop file.

fn main() {
    corundum_framework_build::embed_icon("../docs/icon.svg");
}
