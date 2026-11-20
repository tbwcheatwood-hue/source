# Build Source Tauri desktop binaries for Windows 11 (NSIS/MSI + sidecar daemon).
# Prefer this on Windows; Git Bash users may still run ./scripts/build-desktop.sh.
# Never abandon the desktop target for HTML-only substitutes.
[CmdletBinding()]
param(
  [switch]$CheckDeps,
  [switch]$SkipDaemon,
  [switch]$Help
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

function Show-Help {
  @"
Usage: .\scripts\build-desktop.ps1 [-CheckDeps] [-SkipDaemon]

Builds the Source IDE Tauri v2 desktop app for Windows 11.
  -CheckDeps     Print Windows prerequisite checklist and exit.
  -SkipDaemon    Skip cargo build of source-daemon sidecar (use existing binary).

Prerequisites (Windows 11):
  - Rust (MSVC toolchain): rustup default stable-x86_64-pc-windows-msvc
  - Visual Studio Build Tools with "Desktop development with C++" (MSVC + Windows SDK)
  - Node.js 20+ and pnpm 9+
  - WebView2 Runtime (preinstalled on Windows 11)
  - git (shadow checkpoints / offline tools)

Artifacts:
  source-desktop\src-tauri\target\release\source-desktop.exe
  source-desktop\src-tauri\target\release\bundle\nsis\
  source-desktop\src-tauri\target\release\bundle\msi\
"@
}

function Test-CommandName([string]$Name) {
  return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function Show-WindowsDeps {
  Write-Host "==> Windows 11 Tauri / Source desktop prerequisites"
  $items = @(
    @{ Name = "rustc"; Ok = (Test-CommandName "rustc"); Hint = "Install rustup: https://rustup.rs/ then rustup default stable-x86_64-pc-windows-msvc" },
    @{ Name = "cargo"; Ok = (Test-CommandName "cargo"); Hint = "Comes with rustup" },
    @{ Name = "node"; Ok = (Test-CommandName "node"); Hint = "Install Node.js 20+ from https://nodejs.org/" },
    @{ Name = "pnpm"; Ok = (Test-CommandName "pnpm"); Hint = "npm install -g pnpm@9" },
    @{ Name = "git"; Ok = (Test-CommandName "git"); Hint = "Install Git for Windows" }
  )
  $missing = $false
  foreach ($i in $items) {
    if ($i.Ok) {
      Write-Host ("  [ok] {0}" -f $i.Name)
    } else {
      Write-Host ("  [MISSING] {0} — {1}" -f $i.Name, $i.Hint)
      $missing = $true
    }
  }
  Write-Host "  WebView2 Runtime: expected on Windows 11 (Evergreen bootstrapper if absent)."
  Write-Host "  MSVC + Windows SDK: required for Tauri (Visual Studio Build Tools)."
  if ($missing) { exit 2 }
  Write-Host "All checked CLI deps OK"
}

if ($Help) {
  Show-Help
  exit 0
}

if ($CheckDeps) {
  Show-WindowsDeps
  exit 0
}

Write-Host "==> Checking Windows CLI deps"
Show-WindowsDeps

$Triple = (rustc -vV | Select-String '^host:').ToString().Split(' ')[1]
if (-not $Triple) { $Triple = "x86_64-pc-windows-msvc" }
Write-Host "==> Host triple: $Triple"

$BinDir = Join-Path $Root "source-desktop\src-tauri\binaries"
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
$Sidecar = Join-Path $BinDir ("source-daemon-{0}.exe" -f $Triple)

if (-not $SkipDaemon) {
  Write-Host "==> Building source-daemon (sidecar)"
  cargo build -p source-daemon --release
  if ($LASTEXITCODE -ne 0) { throw "cargo build source-daemon failed" }
  $DaemonSrc = Join-Path $Root "target\release\source-daemon.exe"
  if (-not (Test-Path $DaemonSrc)) {
    throw "source-daemon.exe not found at $DaemonSrc"
  }
  Copy-Item -Force $DaemonSrc $Sidecar
  Write-Host "    staged $Sidecar"
} elseif (-not (Test-Path $Sidecar)) {
  throw "Sidecar missing at $Sidecar — omit -SkipDaemon or place the binary first"
}

Write-Host "==> Building source-web (Tauri frontendDist)"
pnpm install --frozen-lockfile=false
if ($LASTEXITCODE -ne 0) { throw "pnpm install failed" }
pnpm --filter source-web build
if ($LASTEXITCODE -ne 0) { throw "source-web build failed" }

Write-Host "==> Building Tauri desktop (NSIS + MSI)"
pnpm --filter source-desktop exec tauri build -- --bundles nsis,msi
if ($LASTEXITCODE -ne 0) {
  throw "Desktop build failed"
}

Write-Host "==> Desktop build complete"
$Bundle = Join-Path $Root "source-desktop\src-tauri\target\release\bundle"
if (Test-Path $Bundle) {
  Get-ChildItem -Recurse $Bundle | Select-Object FullName, Length
}
Write-Host "Installer(s) under source-desktop\src-tauri\target\release\bundle\{nsis,msi}"
