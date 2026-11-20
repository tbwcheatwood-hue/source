# Dev helper: start source-daemon then Source Tauri desktop (Windows 11).
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$Bind = if ($env:SOURCE_BIND) { $env:SOURCE_BIND } else { "127.0.0.1:7420" }
$env:SOURCE_DAEMON_URL = "http://$Bind"

Write-Host "==> Building/running source-daemon on $Bind"
$Daemon = Start-Process -FilePath "cargo" -ArgumentList @("run", "-p", "source-daemon", "--", "--bind", $Bind) -PassThru -NoNewWindow

try {
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
  if (-not $ok) { throw "daemon did not become healthy on $Bind" }

  Write-Host "==> pnpm --filter source-desktop tauri:dev"
  pnpm --filter source-desktop tauri:dev
} finally {
  if (-not $Daemon.HasExited) {
    Stop-Process -Id $Daemon.Id -Force -ErrorAction SilentlyContinue
  }
}
