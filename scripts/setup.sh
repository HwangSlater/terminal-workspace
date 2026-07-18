#!/usr/bin/env bash
# One-command dev environment check for the Terminal Workspace on
# Linux/macOS: confirms `rustup` is installed, detects whether the
# platform-specific extras `reqwest`'s native-tls backend needs are
# present (see docs/06-development/platform-support.md's Phase 6/7 update
# and the README's macOS/Linux sections), then runs `cargo check --workspace`.
#
# Deliberately does NOT install anything on your behalf (no `sudo apt
# install ...` run by this script) -- an earlier version of this project's
# setup tooling auto-installed native toolchains, and that caused real
# pain (a stuck, silent multi-GB Windows install someone had to kill
# manually; see ADR-0014's Context for the full story). This script only
# detects what's missing and prints the exact command for your specific
# package manager -- you run it yourself.
#
# Usage: ./scripts/setup.sh
set -uo pipefail

echo "== Terminal Workspace: dev environment check =="

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup not found. Install Rust first: https://rustup.rs then re-run this script." >&2
  exit 1
fi
echo "[ok] rustup found."

os="$(uname -s)"

if [ "$os" = "Darwin" ]; then
  if xcode-select -p >/dev/null 2>&1; then
    echo "[ok] Xcode Command Line Tools found."
  else
    echo "[missing] Xcode Command Line Tools (provides the system linker -- not specific to this project, every Rust build on macOS needs it)."
    echo "  Install with: xcode-select --install"
  fi
elif [ "$os" = "Linux" ]; then
  has_linker=0
  command -v cc >/dev/null 2>&1 && has_linker=1
  command -v gcc >/dev/null 2>&1 && has_linker=1
  command -v clang >/dev/null 2>&1 && has_linker=1

  has_openssl=0
  if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists openssl 2>/dev/null; then
    has_openssl=1
  fi

  if [ "$has_linker" = "1" ] && [ "$has_openssl" = "1" ]; then
    echo "[ok] Linker + OpenSSL dev headers + pkg-config found."
  else
    echo "[missing] A linker and/or OpenSSL dev headers + pkg-config."
    echo "  reqwest's default native-tls backend links the system's existing"
    echo "  OpenSSL on Linux (it does not compile one) -- see the README's"
    echo "  \"Linux\" section for why. Install with:"
    if command -v apt-get >/dev/null 2>&1; then
      echo "    sudo apt install build-essential libssl-dev pkg-config"
    elif command -v dnf >/dev/null 2>&1; then
      echo "    sudo dnf install gcc openssl-devel pkgconf-pkg-config"
    elif command -v pacman >/dev/null 2>&1; then
      echo "    sudo pacman -S base-devel openssl pkgconf"
    elif command -v zypper >/dev/null 2>&1; then
      echo "    sudo zypper install gcc libopenssl-devel pkg-config"
    else
      echo "    (unrecognized package manager -- install a C linker, OpenSSL"
      echo "     development headers, and pkg-config via your distro's tool)"
    fi
  fi
fi

echo ""
echo "Verifying with 'cargo check --workspace'..."
cd "$(dirname "$0")/.."
if cargo check --workspace; then
  echo ""
  echo "Setup complete. Try: cargo run -p app"
else
  echo ""
  echo "cargo check failed. See output above." >&2
  echo "If it mentions openssl-sys or pkg-config, install the package(s) noted above and re-run this script." >&2
  exit 1
fi
