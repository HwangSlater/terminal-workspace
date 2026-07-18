#!/usr/bin/env bash
# One-command dev environment verification for the Terminal Workspace on
# Linux/macOS. Confirms `rustup` is installed and runs
# `cargo check --workspace`. Storage (`redb`, ADR-0014) is pure Rust and
# needs nothing extra on any platform. HTTP (`reqwest`'s default
# native-tls backend, README's macOS/Linux sections) is not fully
# toolchain-free on Linux specifically: it links the system's existing
# OpenSSL rather than compiling one, so `libssl-dev`/`pkg-config` (or your
# distro's equivalent) need to already be present -- if `cargo check`
# fails below with an `openssl-sys`/`pkg-config` error, that's what's
# missing, not a problem with this script or `rustup`.
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
  echo "On Linux, an openssl-sys/pkg-config error usually means libssl-dev" >&2
  echo "(Debian/Ubuntu) or openssl-devel (Fedora/RHEL) isn't installed --" >&2
  echo "see this project's README for the exact package names." >&2
  exit 1
fi
