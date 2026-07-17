#!/usr/bin/env bash
# One-command dev environment verification for the Terminal Workspace on
# Linux/macOS. Confirms `rustup` is installed and runs
# `cargo check --workspace`. No native/C toolchain install is needed for
# this workspace today — every dependency (including storage, via `redb`)
# is pure Rust; see docs/06-development/platform-support.md and ADR-0014
# for why this script used to do a lot more than this.
#
# Usage: ./scripts/setup.sh
set -euo pipefail

echo "== Terminal Workspace: dev environment check =="

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup not found. Install Rust first: https://rustup.rs then re-run this script." >&2
  exit 1
fi

echo "rustup found."
echo "Verifying with 'cargo check --workspace'..."
cd "$(dirname "$0")/.."
if cargo check --workspace; then
  echo ""
  echo "Setup complete. Try: cargo run -p app"
else
  echo ""
  echo "cargo check failed. See output above." >&2
  exit 1
fi
