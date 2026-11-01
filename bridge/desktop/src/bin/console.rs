//! The console shim the Windows package puts on `PATH` as `sapphire-bridge.exe` (#363 part
//! C): `sapphire-bridge-desktop.exe` is built for the GUI subsystem, so a command typed in a
//! terminal would print nothing. This runs it with the same arguments and streams, waits,
//! and exits with its code (corundum-framework-package's `console::forward`).

#[cfg(windows)]
fn main() -> ! {
    corundum_framework_package::console::forward("sapphire-bridge-desktop.exe")
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("sapphire-bridge-console is the Windows package's command; run sapphire-bridge-desktop");
    std::process::ExitCode::FAILURE
}
