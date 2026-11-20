# Build this Tauri v2 desktop app on Windows 11 (NSIS/MSI).
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

Write-Host "==> Installing JS deps"
if (Get-Command pnpm -ErrorAction SilentlyContinue) {
  pnpm install --frozen-lockfile=$false
  if ($LASTEXITCODE -ne 0) { throw "pnpm install failed" }
  Write-Host "==> tauri build (nsis,msi)"
  # Do not insert "--" before --bundles (pnpm/npx would forward it incorrectly).
  pnpm exec tauri build --bundles nsis,msi
} else {
  npm install
  if ($LASTEXITCODE -ne 0) { throw "npm install failed" }
  npx tauri build --bundles nsis,msi
}
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
Write-Host "Desktop build complete -> src-tauri\target\release\bundle"
