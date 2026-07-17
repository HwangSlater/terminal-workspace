# ADR 0002: WebAssembly (WASM) Sandboxed Plugin Runtime Selection

## Context
The Workspace must be highly customizable, allowing users to load third-party extensions. However, installing random binaries poses security and stability risks:
- A single null-pointer dereference or unhandled panic in a plugin must not crash the developer's entire workspace.
- Third-party code must not read sensitive files (e.g., SSH private keys) without explicit permission.
- Memory leakages in plugins must be bounded.

We compared three options for loading external plugins:
1. **WebAssembly (`wasmtime` engine)**: Capability-based sandboxing, multi-language support (Rust, TinyGo, Zig), resource limit enforcement.
2. **Native Dynamic Libraries (`.dll` / `.so` / `.dylib`)**: Loadable at runtime using `libloading`.
3. **Embedded Scripting Engine (Lua / Rhai)**: Safe embedded script interpreters.

---

## Decision
We select **WebAssembly (WASM)** utilizing the `wasmtime` runtime engine for executing plugins. Plugins will target the WASI (WebAssembly System Interface) component model with custom Host functions provided by the Workspace SDK.

---

## Alternatives Considered

### 1. Native Dynamic Libraries (`dylib`)
- *Pros*: Native performance, no memory copying overhead at the host-guest boundary.
- *Cons*: Zero security isolation. A plugin memory leak eventually consumes the host. Segfaults or panics immediately terminate the host. Compiling across Windows, Linux, and macOS requires complex platform-specific toolchains.

### 2. Embedded Scripting (e.g., Lua via `mlua`)
- *Pros*: Light runtime footprint, simple script authoring.
- *Cons*: Weak static analysis and type safety for large-scale plugin development. Script execution performance is slow compared to compiled WASM.

---

## Consequences

- **Fault Isolation**: If a WASM plugin panics or encounters an out-of-bounds error, the host catches the trap, logs the event, marks the plugin as `Suspended`, and continues running without interruption.
- **Resource Constraints**: We utilize `wasmtime`'s fuel consumption API to restrict maximum instructions executed per tick, preventing a rogue plugin from hanging the CPU in an infinite loop.
- **Security Sandboxing**: Plugins cannot connect to the internet or open files unless pre-opened file descriptors or domain socket hosts are linked during guest context assembly.
- **Data Boundary Overhead**: All data passed between the host and the plugin must undergo serialization (JSON strings mapped to linear memory). This is acceptable as data exchange rates are relatively low (triggered by human keystrokes or messaging notifications).
