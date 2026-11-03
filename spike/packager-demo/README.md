# packager-demo (spike, not for merging)

Does cargo-packager do what Velopack does for the sapphire desktop apps? The workflow
`.github/workflows/spike-packager.yml` packages this app on Linux (AppImage), macOS (.app and
.dmg, signed and notarized) and Windows (NSIS, per user), then updates an installed 0.1.0 to
0.2.0 from a static `latest.json` with a minisign signature, the way GitHub Releases would
serve it.

Locally on Linux: `cargo packager --release -f appimage` with `CARGO_PACKAGER_SIGN_PRIVATE_KEY`
and its password set, and `PACKAGER_DEMO_PUBKEY` (the public key) when building.
