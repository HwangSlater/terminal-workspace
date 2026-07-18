# ADR 0017: Plugin Runtime Reintroduces a C Compiler Requirement — Accepted, Scoped Exception to ADR-0014

## Status

Amends [ADR-0014](0014-storage-engine-reconsideration.md) in scope only — ADR-0014's decision (pure-Rust `redb` for storage) is unchanged and unaffected. This ADR narrows ADR-0014's *consequence* ("no crate in this workspace needs a C compiler") to no longer hold workspace-wide, while keeping it true everywhere except the plugin runtime.

## Context

`step14.md` (Phase 14, Plugin Runtime) needed to actually build on top of ADR-0002 (WASM via `wasmtime`) and ADR-0009 (WebAssembly Component Model / WIT via `wit-bindgen`), both decided before any implementation existed. Adding `wasmtime` for real (`cargo add wasmtime -p plugin-host`, then `cargo tree -p plugin-host -i cc`) showed it pulls in the `cc` crate as a **build-dependency**:

```
cc v1.2.67
[build-dependencies]
└── wasmtime v46.0.1
    └── plugin-host v0.1.0
```

This directly contradicts the workspace-wide principle ADR-0014 established: "no crate in this workspace needs a C compiler, on any OS." Every phase since ADR-0014 has been built to preserve that property — the Phase 6 `rustls`→`native-tls` swap (avoiding `ring`'s C/assembly build step) and the Phase 12 `ical`/`rrule` pure-Rust verification (before committing to either dependency) were both decisions made specifically to keep it true.

The pure-Rust alternative, `wasmi`, was verified the same way (`cargo add wasmi -p plugin-host`, `cargo tree -i cc` → nothing) and builds with zero C dependency. But `wasmi` has no WebAssembly Component Model support — adopting it would mean reopening ADR-0009 as well, and falling back to the raw-pointer-plus-JSON-serialization FFI boundary ADR-0009 explicitly rejected: *"Guest plugins must manually allocate linear memory, pass raw `*mut u8` pointers to the host... This defeats Rust's memory safety guarantees and leads to memory leaks if deallocation is skipped."* Trading a build-time-only, contributor-only cost for a permanent guest-authoring safety regression was judged the wrong trade.

## Decision

Stay on `wasmtime` (ADR-0002/0009 unchanged). Accept that building `crates/plugin-host` from source now requires a real C compiler, on every OS. This is treated as a **scoped exception**, not a reversal: ADR-0014's reasoning (a KV store doesn't need SQL, so don't pay for a C toolchain to get SQL) doesn't transfer here — WASM sandboxing genuinely has no mature pure-Rust Component Model implementation, unlike the storage case where the C-requiring option (SQLite) was solving a problem (`relational queries`) the codebase didn't actually have.

Critically, **this only affects contributors building from source**. End users running a prebuilt release binary (`cargo-dist`, ADR-0015) are entirely unaffected: `wasmtime`'s JIT (Cranelift) is statically linked into the shipped `terminal-workspace` binary and compiles guest `.wasm` bytecode to native code *at runtime*, inside the already-running process — no external compiler is ever invoked on an end user's machine. The C-compiler cost is paid once, by whoever compiles the workspace, exactly like every other "Contributor Experience: One-Time Setup" cost `product-requirements.md` §2.2 already carves out as distinct from "Zero Setup" (§2.1, the end-user promise).

The plugin system is also **default-off** (`[plugins].enabled`, mirroring every integration's toggle) — a contributor not working on plugins never needs to exercise the feature at runtime even after paying the one-time build cost of compiling `wasmtime` in.

## Alternatives Considered

### Switch to `wasmi` (pure Rust), reopen ADR-0009
Rejected — see Context above. Trades a one-time, contributor-only, well-precedented cost (every OS already needs *some* toolchain for Rust itself; macOS/Windows already document exactly this class of requirement for other reasons) for a permanent regression in guest-plugin memory safety, which is the actual point ADR-0009 was written to secure.

### Defer the entire Plugin Runtime phase indefinitely
Rejected as a non-decision — `v0.5 Plugins` is on the roadmap and the user explicitly confirmed proceeding with `wasmtime` after being shown the trade-off (`step14.md` Context). Deferring doesn't resolve the underlying tension, it just postpones the same choice.

### Vendor a minimal Cranelift-only build path avoiding the `cc`-requiring parts of `wasmtime`
Investigated only at the `cargo tree` level, not pursued further — `wasmtime`'s `cc` build-dependency comes from low-level runtime primitives (stack-switching/fiber support, JIT trampolines), not an optional feature that can be feature-flagged away without losing Component Model / async support this project needs. Not a real option without forking `wasmtime` itself, which is out of proportion to the problem.

## Consequences

- **`docs/06-development/platform-support.md`** gets a "Phase 14" amendment: a real C compiler is now required to build `crates/plugin-host` specifically, on every OS — Linux's "no crate needs a C compiler, only a linker" claim (true since Phase 2, reaffirmed after the Phase 6/7 OpenSSL-linking amendment) no longer holds workspace-wide, though it remains true for every crate *except* `plugin-host`.
- **`README.md`**'s per-OS setup sections gain a plugin-runtime-specific note: macOS needs nothing new (Xcode Command Line Tools already provides a real C compiler); Windows can reuse the already-documented WinLibs MinGW install (or MSVC Build Tools); Linux gains its first genuine "install an actual C compiler" requirement in this project, not just a linker.
- **`scripts/setup.sh`/`setup.ps1`** detection logic needs to check for an actual C compiler on Linux going forward, not just a linker — the Phase 6/7 "linker + OpenSSL headers, no compiler" check is no longer sufficient once `crates/plugin-host` is part of `cargo check --workspace`.
- **No change to end-user-facing claims**: `product-requirements.md` §2.1 ("Zero Setup") and the release pipeline (ADR-0015) are unaffected — this is purely a `cargo check --workspace`-from-source concern.
