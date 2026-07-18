<#
.SYNOPSIS
  One-command dev environment verification for the Terminal Workspace on Windows.

.DESCRIPTION
  Confirms `rustup` is installed and runs `cargo check --workspace` so a new
  contributor gets an unambiguous "you're ready to build" or "here's what's
  wrong" answer. Storage (`redb`) and every crate except `crates/plugin-host`
  are pure Rust and need nothing beyond `rustup`; see
  docs/06-development/platform-support.md and ADR-0014 for why this script
  used to do a lot more than this. Since Phase 14 (ADR-0017),
  `crates/plugin-host` (`wasmtime`) needs a real C compiler -- MSVC Build
  Tools or the README's WinLibs MinGW -- and `cargo check --workspace` below
  will fail with a clear `cc`/linker error if neither is present. This
  script doesn't pre-detect that (Windows has no single reliable "is there a
  C compiler on PATH" check the way `command -v cc` does on Linux/macOS) --
  see platform-support.md §3.1 if `cargo check` fails here.

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
