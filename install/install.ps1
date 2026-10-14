# Install sapphire applications from their GitHub releases, together with
# sapphire-bridge, the host-wide daemon every one of them syncs through.
#
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/sapphire-turtle/sapphire-framework/main/install/install.ps1))) journal
#
# Run with -Help for the options. Windows x86_64; Linux and macOS have install.sh.
#
# Files go to ~\.local\bin, where the `update` command of each binary
# recognises a release install and can replace it later. A binary installed by
# cargo is left alone, as `update` would.

# Only the app names are positional: without PositionalBinding = $false, a bare
# name binds to the first free parameter in declaration order (-Version, say)
# before -App ever sees it.
[CmdletBinding(PositionalBinding = $false)]
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $App = @(),
    [string] $Dir = (Join-Path $HOME '.local\bin'),
    [string] $Version = '',
    [switch] $NoBridge,
    [switch] $Help
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Owner = 'sapphire-turtle'

# Every application this script knows: repository, release file name before
# `-<os>-<arch>`, and the prefix of its release tags. Keep in step with
# install.sh and with each application's release-plz.toml.
$Apps = [ordered]@{
    'bridge'          = @('sapphire-framework', 'sapphire-bridge', 'v')
    'agent'           = @('sapphire-agent', 'sapphire-agent-server', 'sapphire-agent-server-v')
    'agent-cli'       = @('sapphire-agent', 'sapphire-agent-cli', 'sapphire-agent-cli-v')
    'agent-desktop'   = @('sapphire-agent', 'sapphire-agent-desktop', 'desktop-v')
    'journal'         = @('sapphire-journal', 'sapphire-journal', 'cli-v')
    'journal-desktop' = @('sapphire-journal', 'sapphire-journal-desktop', 'desktop-v')
    'sync'            = @('sapphire-sync', 'sapphire-sync', 'v')
    'sync-desktop'    = @('sapphire-sync', 'sapphire-sync-desktop', 'desktop-v')
}

function Show-Usage {
    Write-Host 'Usage: install.ps1 [-Dir <dir>] [-Version <x.y.z>] [-NoBridge] <app>...'
    Write-Host ''
    Write-Host 'Installs each <app> from its latest release, and sapphire-bridge with them.'
    Write-Host ''
    Write-Host 'Apps:'
    foreach ($name in $Apps.Keys) { Write-Host "  $name" }
    Write-Host ''
    Write-Host 'Options:'
    Write-Host '  -Dir <dir>        Install into <dir> (default: ~\.local\bin)'
    Write-Host '  -Version <x.y.z>  Install this version instead of the latest (one app only)'
    Write-Host '  -NoBridge         Do not install or update sapphire-bridge'
}

if ($Help) { Show-Usage; exit 0 }
if ($App.Count -eq 0) { Show-Usage; exit 2 }
$Version = $Version.TrimStart('v')
if ($Version -and $App.Count -gt 1) { throw '-Version applies to one app at a time' }
foreach ($name in $App) {
    if (-not $Apps.Contains($name)) { throw "unknown app: $name (see -Help)" }
}
if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
    throw 'releases are built for Windows x86_64 only; build from source with cargo'
}

# The highest x.y.z among the repository's releases tagged <prefix>x.y.z. Not
# `releases/latest`: a repository that tags several binaries has one "latest"
# among all of them. Pre-release versions (x.y.z-...) are skipped, as `update`
# skips them.
function Get-LatestVersion([string] $Repo, [string] $Prefix) {
    # Unauthenticated requests share a low per-address rate limit (CI runners
    # hit it); GITHUB_TOKEN or GH_TOKEN, when set, lifts it.
    $token = if ($env:GITHUB_TOKEN) { $env:GITHUB_TOKEN } else { $env:GH_TOKEN }
    $headers = @{}
    if ($token) { $headers['Authorization'] = "Bearer $token" }
    try {
        $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$Owner/$Repo/releases?per_page=100" -Headers $headers
    } catch {
        throw "could not list the releases of $Owner/$Repo (rate-limited? set GITHUB_TOKEN): $_"
    }
    $versions = foreach ($r in $releases) {
        if ($r.draft -or $r.prerelease) { continue }
        if (-not $r.tag_name.StartsWith($Prefix)) { continue }
        $v = $r.tag_name.Substring($Prefix.Length)
        if ($v -match '^\d+\.\d+\.\d+$') { [version] $v }
    }
    $latest = $versions | Sort-Object -Descending | Select-Object -First 1
    if ($latest) { "$latest" } else { $null }
}

# A binary under cargo's bin directory was installed by `cargo install`: leave
# it to cargo, as sapphire-framework-update's `install_method` does.
function Test-CargoInstall([string] $Path) {
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
    $cargoBin = (Join-Path $cargoHome 'bin') + '\'
    $Path.StartsWith($cargoBin, [StringComparison]::OrdinalIgnoreCase)
}

$Installed = @()

function Install-App([string] $Name, [string] $Want) {
    $repo, $base, $prefix = $Apps[$Name]

    $existing = (Get-Command "$base.exe" -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1).Source
    if ($existing -and (Test-CargoInstall $existing)) {
        Write-Host "Leaving $existing alone: it was installed by cargo. Update it the same way."
        return
    }

    $v = if ($Want) { $Want } else { Get-LatestVersion $repo $prefix }
    if (-not $v) { throw "$repo has no ${prefix}x.y.z release of $base yet" }
    $url = "https://github.com/$Owner/$repo/releases/download/$prefix$v/$base-windows-x86_64.exe"
    $dest = Join-Path $Dir "$base.exe"

    Write-Host "Installing $base $v (windows-x86_64) to $Dir"
    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    $tmp = Join-Path $Dir ".$base.$([guid]::NewGuid().ToString('N')).exe"
    try {
        Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing
    } catch {
        Remove-Item -Force -ErrorAction SilentlyContinue $tmp
        throw "could not download ${url}: $_"
    }
    # A running executable cannot be overwritten on Windows, but it can be
    # renamed: move the old file aside, then put the new one in its place. The
    # old file is removed when nothing runs it any more, or on the next install.
    Get-ChildItem -Path $Dir -Filter ".$base.old-*.exe" -ErrorAction SilentlyContinue |
        Remove-Item -Force -ErrorAction SilentlyContinue
    if (Test-Path $dest) {
        Move-Item -Force $dest (Join-Path $Dir ".$base.old-$([guid]::NewGuid().ToString('N')).exe")
    }
    Move-Item -Force $tmp $dest

    if ($existing -and $existing -ne $dest) {
        Write-Host "Note: $existing comes first on PATH and was not replaced; remove it, or install"
        Write-Host "      with -Dir $(Split-Path $existing) to replace it."
    }
    $script:Installed += $base
}

# The bridge first, so an application started right after finds it. It is
# installed at its latest version unless it is the one app asked for.
if (-not $NoBridge -and $App -notcontains 'bridge') {
    Install-App 'bridge' ''
}
foreach ($name in $App) {
    Install-App $name $Version
}

if ($Installed.Count -eq 0) { exit 0 }
Write-Host ''
Write-Host "Installed: $($Installed -join ' ')"

$userPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
if (($userPath -split ';') -notcontains $Dir) {
    [Environment]::SetEnvironmentVariable('PATH', "$Dir;$userPath", 'User')
    Write-Host ''
    Write-Host "Added $Dir to your PATH. Open a new terminal to use it."
}

if ($Installed -contains 'sapphire-bridge') {
    Write-Host ''
    Write-Host 'Run the bridge as a service, so it starts with your session:'
    Write-Host '  sapphire-bridge service install'
    Write-Host 'If the service was already running, restart it to run the new version.'
}
