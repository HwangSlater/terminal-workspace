# Platform Support Policy

The Terminal Workspace is a **Local First, Terminal First** platform with an explicit goal of running on Windows, macOS, and Linux without favoring one as a second-class target (`docs/01-product/product-requirements.md` §2, "OS Independence"). This document defines what "supported" means per platform, and is the source of truth `docs/06-development/development.md`'s CI rules and `.github/workflows/ci.yml` must stay consistent with.

> **Scope note**: everything below is about the **Contributor Experience** (building from source) — see `product-requirements.md` §2.2. It has no bearing on the **User Experience** (§2.1, "Zero Setup") — an end user running a prebuilt release binary never installs any of this.

> **Update (ADR-0014)**: earlier revisions of this document spent most of their length on why Windows contributors specifically needed to install Visual Studio Build Tools (a multi-GB, occasionally flaky install — see ADR-0014's Context for exactly how flaky). That requirement came entirely from `rusqlite`'s `bundled` SQLite needing a C compiler to build. Since the storage engine moved to `redb` (pure Rust, no C compiler, no build script), **no crate in this workspace needs a C toolchain to build, on any OS.** `rustup` alone is sufficient everywhere. The toolchain-installation content below is kept as reference for if/when a future dependency reintroduces a native build requirement — it is not something a contributor needs to act on today.

---

## 1. Support Tiers

| Platform | Toolchain | Support Level |
| :--- | :--- | :--- |
| Windows | MSVC (`x86_64-pc-windows-msvc`) | **Tier 1** |
| Linux | GNU (`x86_64-unknown-linux-gnu`) | **Tier 1** |
| macOS | Apple Clang (`x86_64-apple-darwin` / `aarch64-apple-darwin`) | **Tier 1** |
| Windows | GNU/MinGW (`x86_64-pc-windows-gnu`) | Experimental / best-effort |

**Tier 1** means: CI builds and runs the full test suite on every push/PR (see §2); a regression on a Tier 1 target blocks merge; release binaries are produced for it — the exact same four targets are declared in `dist-workspace.toml` (ADR-0015, `cargo-dist`); if this table ever changes, update that file too via `dist generate`, not by hand-editing the generated workflow.

**Experimental** means: not gated in CI, no release binaries produced, contributions welcome but not blocking.

The table is still worth keeping even with no current native-build requirement: `rustup`'s default host triple still differs per OS/toolchain, CI still needs to target something concrete, and if a future dependency ever does need a C compiler again, this tier ranking (MSVC preferred on Windows) is the default to fall back to rather than re-litigating it — see §3 (dormant) for why.

---

## 2. CI Enforcement

`.github/workflows/ci.yml` runs a 3-OS matrix — `windows-latest` (MSVC, the default Windows Rust target), `ubuntu-latest` (GNU), `macos-latest` (Apple Clang) — each executing:

1. `cargo fmt --all -- --check`
2. `cargo check --workspace --all-targets`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. `cargo test --workspace`

Running all three on every push/PR exists specifically to catch platform-specific breakage early — e.g. a path-handling bug that only surfaces on Windows — rather than discovering it after a contributor on another OS hits it. This is now a much lighter pipeline than it would have been under the SQLite-bundled design: no toolchain provisioning step is needed before these four commands can run.

---

## 3. Local Development Setup

Since no crate needs a C toolchain, setup is just: install Rust, clone, build.

```sh
# any OS
cargo check --workspace
cargo test --workspace
```

`scripts/setup.ps1` (Windows) / `scripts/setup.sh` (Linux/macOS) exist as a convenience one-liner for a first-time contributor: they confirm `rustup` is installed, then run `cargo check --workspace` and report pass/fail clearly, so "did I set this up right?" has an unambiguous answer without reading Cargo output by hand.

```powershell
# Windows (PowerShell)
powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1
```

```sh
# Linux / macOS
./scripts/setup.sh
```

### (Dormant) C toolchain installation reference

Kept for if a future dependency needs a native build step again — not required today (see the ADR-0014 update note at the top of this document).

- **Windows**: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/), "Desktop development with C++" workload, then `rustup default stable-x86_64-pc-windows-msvc`.
- **Linux**: your distro's package manager (e.g. `build-essential` on Debian/Ubuntu).
- **macOS**: `xcode-select --install` (Apple Clang).
