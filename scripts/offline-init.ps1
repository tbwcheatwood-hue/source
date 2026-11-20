# Seed %USERPROFILE%\.source\ (or $env:SOURCE_HOME\.source) with templates, caches, offline docs.
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

function Write-Utf8NoBom([string]$Path, [string]$Content) {
  $encoding = New-Object System.Text.UTF8Encoding $false
  [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

$HomeBase = if ($env:SOURCE_HOME) { $env:SOURCE_HOME } else { $env:USERPROFILE }
$SourceRoot = Join-Path $HomeBase ".source"

Write-Host "==> Seeding Source offline layout at $SourceRoot"

@(
  "offline-docs",
  "templates",
  "extensions",
  "cache\rag",
  "cache\vendor\npm",
  "cache\vendor\pnpm",
  "cache\vendor\cargo",
  "cache\vendor\wheels",
  "checkpoints"
) | ForEach-Object {
  New-Item -ItemType Directory -Force -Path (Join-Path $SourceRoot $_) | Out-Null
}

$Doc = Join-Path $SourceRoot "offline-docs\source-overview.md"
if (-not (Test-Path $Doc)) {
  Write-Utf8NoBom $Doc @"
# Source offline docs

Source is a local-first agent IDE.

## Network tiers

- LOCAL_ONLY - loopback only; mesh disabled
- LOCAL_AND_LAN - RFC1918 + Tailscale CGNAT; mesh allowed
- UNRESTRICTED_CONFIGURABLE - allowlist + approval for public hosts

## Plan / Act

Plan mode is read-only. Act mode requires approval gates for File Edits, Shell, Network, MCP, and Cross-Node.

## Mesh

Pair peers with a 6-digit PIN. Register hosts by IP or Tailscale MagicDNS.
"@
}

$DesktopDoc = Join-Path $SourceRoot "offline-docs\desktop-apps.md"
Write-Utf8NoBom $DesktopDoc @"
# Desktop apps (HARD RULE)

NEVER refuse native/desktop requests with "this environment can't", "use Electron later", or "HTML-only instead".

When the user asks for a desktop app:

1. Seed offline layout (`offline-init.ps1` / `offline-init.sh`) so templates\tauri-v2-react exists.
2. Create via ``templates_ops`` ``create_desktop_app`` (or ``source/templates/scaffold`` with ``tauri-v2-react``).
3. Build with ``.\scripts\build-desktop.ps1`` on Windows or ``./scripts/build-desktop.sh`` on Linux / ``pnpm tauri build``.
4. On Linux, if WebKitGTK/-dev packages are missing, install apt packages or run ``--install-deps``.
5. On Windows 11, install MSVC Build Tools + WebView2 (usually present), then run ``.\scripts\build-desktop.ps1``.

HTML-only is allowed only when the user explicitly asked for a web page.

## Windows 11 deps

- Rust MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`)
- Visual Studio Build Tools (C++ workload + Windows SDK)
- Node 20+, pnpm 9+, git
- WebView2 Runtime

## Linux deps

``````bash
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf \
  pkg-config build-essential curl wget file libssl-dev \
  libayatana-appindicator3-dev
``````
"@

$ExtDir = Join-Path $SourceRoot "extensions\sample-echo"
$ExtManifest = Join-Path $ExtDir "mcp.json"
if (-not (Test-Path $ExtManifest)) {
  New-Item -ItemType Directory -Force -Path $ExtDir | Out-Null
  # PS 5.1 Set-Content -Encoding utf8 writes a BOM that breaks serde_json.
  Write-Utf8NoBom $ExtManifest @"
{
  "id": "sample-echo",
  "name": "Sample Echo",
  "transport": "stdio",
  "command": ["python", "-c", "import sys; print('{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[]}}', flush=True); sys.stdin.readline()"],
  "offline_capable": true,
  "requires_wan": false,
  "enabled": false
}
"@
}

Write-Host "==> Bootstrapping daemon to seed templates..."
$env:SOURCE_HOME = $HomeBase
cargo build -q -p source-daemon
if ($LASTEXITCODE -ne 0) { throw "cargo build source-daemon failed" }

$Bind = "127.0.0.1:17421"
$Daemon = Join-Path $Root "target\debug\source-daemon.exe"
$LogOut = Join-Path $env:TEMP "source-offline-init.out.log"
$LogErr = Join-Path $env:TEMP "source-offline-init.err.log"
# Start-Process requires distinct stdout/stderr redirect paths.
$Proc = Start-Process -FilePath $Daemon -ArgumentList @("--bind", $Bind) -RedirectStandardOutput $LogOut -RedirectStandardError $LogErr -PassThru -WindowStyle Hidden

$ok = $false
for ($i = 0; $i -lt 80; $i++) {
  try {
    Invoke-WebRequest -Uri "http://$Bind/health" -UseBasicParsing -TimeoutSec 1 | Out-Null
    $ok = $true
    break
  } catch {
    Start-Sleep -Milliseconds 250
  }
}

if (-not $ok) {
  if (-not $Proc.HasExited) { Stop-Process -Id $Proc.Id -Force -ErrorAction SilentlyContinue }
  throw "daemon failed during offline-init; see $LogOut and $LogErr"
}

try {
  Invoke-RestMethod -Method Post -Uri "http://$Bind/rpc" -ContentType "application/json" -Body '{"jsonrpc":"2.0","id":1,"method":"source/templates/list","params":{}}' | ConvertTo-Json -Depth 6
  Invoke-RestMethod -Method Post -Uri "http://$Bind/rpc" -ContentType "application/json" -Body '{"jsonrpc":"2.0","id":2,"method":"source/rag/index","params":{"force":true}}' | ConvertTo-Json -Depth 6
} finally {
  if (-not $Proc.HasExited) { Stop-Process -Id $Proc.Id -Force -ErrorAction SilentlyContinue }
  try { Wait-Process -Id $Proc.Id -TimeoutSec 5 -ErrorAction SilentlyContinue } catch {}
}

Write-Host "==> Offline init complete"
Write-Host "    data root: $SourceRoot"
Get-ChildItem (Join-Path $SourceRoot "templates") -ErrorAction SilentlyContinue
Get-ChildItem (Join-Path $SourceRoot "offline-docs") -ErrorAction SilentlyContinue
