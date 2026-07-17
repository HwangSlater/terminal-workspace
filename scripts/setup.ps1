<#
.SYNOPSIS
  One-command dev environment verification for the Terminal Workspace on Windows.

.DESCRIPTION
  Confirms `rustup` is installed and runs `cargo check --workspace` so a new
  contributor gets an unambiguous "you're ready to build" or "here's what's
  wrong" answer. No native/C toolchain install is needed for this workspace
  today — every dependency (including storage, via `redb`) is pure Rust; see
  docs/06-development/platform-support.md and ADR-0014 for why this script
  used to do a lot more than this.

.EXAMPLE
  .\scripts\setup.ps1
#>

$ErrorActionPreference = "Stop"

Write-Host "== Terminal Workspace: dev environment check ==" -ForegroundColor Cyan

if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    Write-Error "rustup not found. Install Rust first: https://rustup.rs then re-run this script."
    exit 1
}

Write-Host "rustup found." -ForegroundColor Green
Write-Host "Verifying with 'cargo check --workspace'..."

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    cargo check --workspace
    if ($LASTEXITCODE -ne 0) {
        Write-Error "cargo check failed. See output above."
        exit 1
    }
}
finally {
    Pop-Location
}

Write-Host ""
Write-Host "Setup complete. Try: cargo run -p app" -ForegroundColor Green
