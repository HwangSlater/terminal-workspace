# Platform Support Policy

The Terminal Workspace is a **Local First, Terminal First** platform with an explicit goal of running on Windows, macOS, and Linux without favoring one as a second-class target (`docs/01-product/product-requirements.md` §2, "OS Independence"). This document defines what "supported" means per platform, and is the source of truth `docs/06-development/development.md`'s CI rules and `.github/workflows/ci.yml` must stay consistent with.

> **Scope note**: everything below is about the **Contributor Experience** (building from source) — see `product-requirements.md` §2.2. It has no bearing on the **User Experience** (§2.1, "Zero Setup") — an end user running a prebuilt release binary never installs any of this.

> **Update (ADR-0014)**: earlier revisions of this document spent most of their length on why Windows contributors specifically needed to install Visual Studio Build Tools (a multi-GB, occasionally flaky install — see ADR-0014's Context for exactly how flaky). That requirement came entirely from `rusqlite`'s `bundled` SQLite needing a C compiler to build. Since the storage engine moved to `redb` (pure Rust, no C compiler, no build script), **no crate in this workspace needs a C *compiler* to build, on any OS.** The toolchain-installation content below is kept as reference for if/when a future dependency reintroduces a native build requirement.
>
> **Update (Phase 6/7, `step6.md`/`step7.md`)**: "no C compiler" is not quite the same as "nothing extra needed everywhere." Two later additions (`reqwest` for Slack's HTTP calls, `keyring` for OS credential storage) mean a bare `rustup` install is still sufficient on **Windows and macOS**, but **Linux** needs a linker plus the system's existing OpenSSL library + headers present for `reqwest`'s default `native-tls` backend to link against (it links the system's OpenSSL, it does not compile one — no C source of ours or a dependency's gets compiled either way). See the README's "Linux" section for exact package names per distro. `keyring`'s Linux backend (`zbus-secret-service-keyring-store`) is a pure-Rust DBus client and needs nothing extra to *build*; at *runtime* it needs a DBus Secret Service (gnome-keyring/kwallet) to actually store a credential, and falls back automatically to an encrypted local file when none is running (headless/server Linux) — not a build-time concern either way.

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

Since no crate needs a C *compiler*, setup is close to just "install Rust, clone, build" everywhere — Linux is the one exception, needing OpenSSL dev headers + `pkg-config` already present (see the Phase 6/7 update note above and the README's "Linux" section).

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

### (Dormant) C *compiler* installation reference

Kept for if a future dependency needs to actually compile C/C++ source again — not required today, on any OS (see the ADR-0014 update note at the top of this document). This is distinct from the Linux linker + system OpenSSL requirement noted in the Phase 6/7 update above — that's *linking against* something already present, not compiling anything.

- **Windows**: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/), "Desktop development with C++" workload, then `rustup default stable-x86_64-pc-windows-msvc`.
- **Linux**: your distro's package manager (e.g. `build-essential` on Debian/Ubuntu — this also happens to be one way to satisfy the linker half of the Phase 6/7 Linux requirement above, since it bundles `gcc`, but installing it for that reason alone is overkill; a linker alone plus `libssl-dev`/`pkg-config` is enough).
- **macOS**: `xcode-select --install` (Apple Clang) — same "Xcode Command Line Tools provide the system linker" baseline the README's macOS section describes; not specific to this project.
