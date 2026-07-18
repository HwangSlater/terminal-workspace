# Platform Support Policy

The Terminal Workspace is a **Local First, Terminal First** platform with an explicit goal of running on Windows, macOS, and Linux without favoring one as a second-class target (`docs/01-product/product-requirements.md` §2, "OS Independence"). This document defines what "supported" means per platform, and is the source of truth `docs/06-development/development.md`'s CI rules and `.github/workflows/ci.yml` must stay consistent with.

> **Scope note**: everything below is about the **Contributor Experience** (building from source) — see `product-requirements.md` §2.2. It has no bearing on the **User Experience** (§2.1, "Zero Setup") — an end user running a prebuilt release binary never installs any of this.

> **Update (ADR-0014)**: earlier revisions of this document spent most of their length on why Windows contributors specifically needed to install Visual Studio Build Tools (a multi-GB, occasionally flaky install — see ADR-0014's Context for exactly how flaky). That requirement came entirely from `rusqlite`'s `bundled` SQLite needing a C compiler to build. Since the storage engine moved to `redb` (pure Rust, no C compiler, no build script), **no crate in this workspace needs a C *compiler* to build, on any OS.** The toolchain-installation content below is kept as reference for if/when a future dependency reintroduces a native build requirement.
>
> **Update (Phase 6/7, `step6.md`/`step7.md`)**: "no C compiler" is not quite the same as "nothing extra needed everywhere." Two later additions (`reqwest` for Slack's HTTP calls, `keyring` for OS credential storage) mean a bare `rustup` install is still sufficient on **Windows and macOS**, but **Linux** needs a linker plus the system's existing OpenSSL library + headers present for `reqwest`'s default `native-tls` backend to link against (it links the system's OpenSSL, it does not compile one — no C source of ours or a dependency's gets compiled either way). See the README's "Linux" section for exact package names per distro. `keyring`'s Linux backend (`zbus-secret-service-keyring-store`) is a pure-Rust DBus client and needs nothing extra to *build*; at *runtime* it needs a DBus Secret Service (gnome-keyring/kwallet) to actually store a credential, and falls back automatically to an encrypted local file when none is running (headless/server Linux) — not a build-time concern either way.
>
> **Update (Phase 14, `step14.md`, ADR-0017)**: the "no crate needs a C *compiler*" claim above no longer holds workspace-wide. `crates/plugin-host` depends on `wasmtime` (the WASM sandbox for the plugin runtime), which pulls in the `cc` crate as a genuine build-dependency — confirmed via `cargo tree -p plugin-host -i cc`, not assumed. Unlike the Linux OpenSSL case above (linking against something already present), this is real C/assembly compilation happening as part of `wasmtime`'s own build. ADR-0017 covers the full reasoning for accepting this rather than reverting to a pure-Rust WASM runtime; the short version is it's the one place in this workspace a C compiler is genuinely unavoidable (no mature pure-Rust WebAssembly Component Model implementation exists), the cost lands on contributors building from source only (never end users of a prebuilt release binary — `wasmtime`'s JIT is statically linked in and compiles guest `.wasm` entirely inside the already-running process, no external compiler invoked), and the plugin system stays default-off. See §3 below for what this means per OS.

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

Running all three on every push/PR exists specifically to catch platform-specific breakage early — e.g. a path-handling bug that only surfaces on Windows — rather than discovering it after a contributor on another OS hits it. This was a much lighter pipeline than it would have been under the SQLite-bundled design (no toolchain provisioning step needed) until Phase 14 (ADR-0017) added `crates/plugin-host`'s `wasmtime` dependency back into `cargo check --workspace`/`test --workspace`. In practice this hasn't required adding an explicit provisioning step to `ci.yml`: GitHub-hosted `ubuntu-latest`/`windows-latest`/`macos-latest` runners all ship a C compiler preinstalled (`build-essential`, MSVC Build Tools, and Xcode Command Line Tools respectively, as part of the standard runner images) — but this is worth re-verifying against a real CI run rather than assumed, the same way the Phase 5-9 cross-platform build was actually verified via GitHub Actions rather than just reasoned about locally.

---

## 3. Local Development Setup

Building everything **except** `crates/plugin-host` is close to just "install Rust, clone, build" everywhere — Linux is the one exception even there, needing OpenSSL dev headers + `pkg-config` already present (see the Phase 6/7 update note above and the README's "Linux" section). Since Phase 14 (ADR-0017), `crates/plugin-host` — and therefore `cargo check --workspace`/`cargo test --workspace` as run below, which include it — needs a real C compiler; see §3.1 for what that means per OS. If you aren't working on the plugin runtime, you can skip §3.1 and build every other crate individually (`cargo check -p <crate>`) without it.

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

### 3.1. C *compiler* installation reference (live again since Phase 14, `crates/plugin-host` only)

No longer dormant — `wasmtime` (the plugin runtime's WASM sandbox) needs a real C compiler to build (ADR-0017), not just the linker the Phase 6/7 Linux note above describes. This is distinct from that Linux linker + system OpenSSL requirement — that's *linking against* something already present, not compiling anything; this is genuine C/assembly compilation as part of `wasmtime`'s own build.

- **Windows**: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/), "Desktop development with C++" workload, then `rustup default stable-x86_64-pc-windows-msvc` — **or** the already-documented WinLibs MinGW-w64 GCC (README's Windows section), which also happens to satisfy this.
- **Linux**: your distro's package manager, e.g. `build-essential` on Debian/Ubuntu (this also happens to satisfy the linker half of the Phase 6/7 Linux requirement above, since it bundles `gcc`, but before Phase 14 that alone would have been overkill for that requirement — now it's the actual requirement, not incidental coverage).
- **macOS**: `xcode-select --install` (Apple Clang) — already required regardless (the README's macOS section describes this as the system linker baseline every macOS Rust build needs); Phase 14 doesn't add a new step here, `wasmtime` just uses more of what's already there.
