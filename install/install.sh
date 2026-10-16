#!/bin/sh
# Install sapphire applications from their GitHub releases, together with
# sapphire-bridge, the host-wide daemon every one of them syncs through.
#
#   curl -fsSL https://raw.githubusercontent.com/sapphire-turtle/sapphire-framework/main/install/install.sh | sh -s -- journal
#
# Run with --help for the options. Linux (x86_64, aarch64) and macOS (aarch64);
# Windows has install.ps1.
#
# Files go to ~/.local/bin, where the `update` command of each binary
# recognises a release install and can replace it later. A binary installed by
# cargo or a package manager is left alone, as `update` would.
set -eu

# Every application this script knows: name, repository, release file name
# before `-<os>-<arch>`, and the prefix of its release tags. Keep in step with
# install.ps1 and with each application's release-plz.toml.
APPS="
bridge          sapphire-framework  sapphire-bridge          v
agent           sapphire-agent      sapphire-agent           v
agent-desktop   sapphire-agent      sapphire-agent-desktop   desktop-v
journal         sapphire-journal    sapphire-journal         v
journal-desktop sapphire-journal    sapphire-journal-desktop desktop-v
sync            sapphire-sync       sapphire-sync            v
sync-desktop    sapphire-sync       sapphire-sync-desktop    desktop-v
"
# Names an application no longer has: name, and the app that replaces it.
RETIRED="
agent-cli       agent
"
OWNER="sapphire-turtle"

usage() {
  cat <<EOF
Usage: install.sh [options] <app>...

Installs each <app> from its latest release, and sapphire-bridge with them.

Apps:
$(printf '%s\n' "$APPS" | awk 'NF { printf "  %s\n", $1 }')

Options:
  --dir <dir>          Install into <dir> (default: ~/.local/bin)
  --version <x.y.z>    Install this version instead of the latest (one app only)
  --no-bridge          Do not install or update sapphire-bridge
  -h, --help           Show this help
EOF
}

say() { printf '%s\n' "$*"; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }

INSTALL_DIR="${HOME}/.local/bin"
VERSION=""
WITH_BRIDGE=1
REQUESTED=""

while [ $# -gt 0 ]; do
  case "$1" in
    --dir) [ $# -ge 2 ] || err "--dir needs a directory"; INSTALL_DIR="$2"; shift 2 ;;
    --version) [ $# -ge 2 ] || err "--version needs a version"; VERSION="${2#v}"; shift 2 ;;
    --no-bridge) WITH_BRIDGE=0; shift ;;
    -h|--help) usage; exit 0 ;;
    -*) err "unknown option: $1 (see --help)" ;;
    *) REQUESTED="$REQUESTED $1"; shift ;;
  esac
done

[ -n "$REQUESTED" ] || { usage >&2; exit 2; }
# Unquoted on purpose: REQUESTED is a list of app names.
# shellcheck disable=SC2086
set -- $REQUESTED
if [ -n "$VERSION" ] && [ $# -gt 1 ]; then
  err "--version applies to one app at a time"
fi
for app in $REQUESTED; do
  now=$(printf '%s\n' "$RETIRED" | awk -v app="$app" '$1 == app { print $2 }')
  [ -z "$now" ] || err "$app is now part of $now: install $now instead"
  printf '%s\n' "$APPS" | awk -v app="$app" '$1 == app { found = 1 } END { exit !found }' \
    || err "unknown app: $app (see --help)"
done

command -v curl >/dev/null 2>&1 || err "curl is required"

case "$(uname -s)" in
  Linux*)  OS="linux" ;;
  Darwin*) OS="macos" ;;
  *) err "unsupported OS: $(uname -s) (on Windows, use install.ps1)" ;;
esac
case "$(uname -m)" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) err "unsupported architecture: $(uname -m)" ;;
esac
if [ "$OS" = "macos" ] && [ "$ARCH" = "x86_64" ]; then
  err "no release is built for Intel macOS; build from source with cargo"
fi

# app_field <app> <column>: one column of the app's row in APPS.
app_field() {
  printf '%s\n' "$APPS" | awk -v app="$1" -v col="$2" '$1 == app { print $col }'
}

# github_api <path>: a GitHub API response. Unauthenticated requests share a
# low per-address rate limit (CI runners hit it); GITHUB_TOKEN or GH_TOKEN, when
# set, lifts it.
github_api() {
  token="${GITHUB_TOKEN:-${GH_TOKEN:-}}"
  if [ -n "$token" ]; then
    curl -fsSL -H "Authorization: Bearer $token" "https://api.github.com/$1"
  else
    curl -fsSL "https://api.github.com/$1"
  fi
}

# latest_version <repo> <tag prefix>: the highest x.y.z among the repository's
# releases tagged <prefix>x.y.z. Not `releases/latest`: a repository that tags
# several binaries has one "latest" among all of them. Pre-release versions
# (x.y.z-…) are skipped, as `update` skips them.
latest_version() {
  releases=$(github_api "repos/${OWNER}/$1/releases?per_page=100") \
    || err "could not list the releases of ${OWNER}/$1 (rate-limited? set GITHUB_TOKEN)"
  printf '%s\n' "$releases" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' \
    | awk -v prefix="$2" '
        index($0, prefix) == 1 {
          v = substr($0, length(prefix) + 1)
          if (v ~ /^[0-9]+\.[0-9]+\.[0-9]+$/) print v
        }' \
    | sort -t . -k 1,1n -k 2,2n -k 3,3n \
    | tail -n 1
}

# install_method <path>: how the binary at <path> was installed, judged by
# where it is, as sapphire-framework-update's `install_method` does: cargo,
# package-manager or release. Only a release install may be replaced.
install_method() {
  cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
  case "$1" in
    "$cargo_bin"/*) echo cargo ;;
    /usr/local/*) echo release ;;
    /usr/*|/bin/*|/sbin/*|/nix/*|/opt/homebrew/*|/home/linuxbrew/*|/snap/*) echo package-manager ;;
    *) echo release ;;
  esac
}

INSTALLED=""

# The download in flight, removed if the script stops before it is renamed
# into place.
TMP=""
trap 'rm -f "$TMP"' EXIT
trap 'exit 130' INT TERM

# install_app <app> <version or empty>
install_app() {
  repo=$(app_field "$1" 2)
  base=$(app_field "$1" 3)
  prefix=$(app_field "$1" 4)

  # The binary is installed under its release file name, which is also the
  # name the application's own commands use for it.
  existing=$(command -v "$base" 2>/dev/null || true)
  if [ -n "$existing" ]; then
    method=$(install_method "$existing")
    if [ "$method" != release ]; then
      say "Leaving $existing alone: it was installed by $method. Update it the same way."
      return 0
    fi
  fi

  version="$2"
  if [ -z "$version" ]; then
    version=$(latest_version "$repo" "$prefix")
    [ -n "$version" ] || err "$repo has no ${prefix}x.y.z release of $base yet"
  fi
  tag="${prefix}${version}"
  asset="${base}-${OS}-${ARCH}"
  url="https://github.com/${OWNER}/${repo}/releases/download/${tag}/${asset}"
  dest="${INSTALL_DIR}/${base}"

  say "Installing $base $version ($OS-$ARCH) to $INSTALL_DIR"
  mkdir -p "$INSTALL_DIR"
  # Download beside the destination and rename it into place: the rename is
  # atomic, so a running copy (the bridge, typically) keeps its old file and
  # nothing ever sees a half-written executable.
  TMP=$(mktemp "${INSTALL_DIR}/.${base}.XXXXXX")
  curl -fsSL "$url" -o "$TMP" || err "could not download $url"
  chmod 755 "$TMP"
  mv -f "$TMP" "$dest"
  TMP=""

  if [ -n "$existing" ] && [ "$existing" != "$dest" ]; then
    say "Note: $existing comes first on PATH and was not replaced; remove it, or install"
    say "      with --dir $(dirname "$existing") to replace it."
  fi
  INSTALLED="$INSTALLED $base"
}

# The bridge first, so an application started right after finds it. It is
# installed at its latest version unless it is the one app asked for.
case " $REQUESTED " in
  *" bridge "*) WITH_BRIDGE=0 ;;
esac
if [ "$WITH_BRIDGE" -eq 1 ]; then
  install_app bridge ""
fi
for app in $REQUESTED; do
  install_app "$app" "$VERSION"
done

[ -n "$INSTALLED" ] || exit 0
say ""
say "Installed:$INSTALLED"

case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    say ""
    say "$INSTALL_DIR is not on PATH. Add it in your shell profile:"
    say "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac

case "$INSTALLED" in
  *" sapphire-bridge"*)
    say ""
    say "Run the bridge as a service, so it starts with your session:"
    say "  sapphire-bridge service install"
    say "If the service was already running, restart it to run the new version."
    ;;
esac
