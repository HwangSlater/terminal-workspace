# Capability Permission System Specification

This document details the capability authorization system that validates and intercepts resource usage by guest plugins.

> **Implementation Status (Phase 16, `step16.md`, amending the Phase 14 note below)**: one row of this page's permission map is now real — `presence-read` (gating `get-member-presence`), enforced by `PermissionManager::verify_capability` (`crates/plugin-host/src/lib.rs`, no longer a stub) against a per-plugin `<plugin-id>.toml` manifest (`capabilities = ["presence-read"]`) sitting next to the `.wasm` file. A missing manifest grants nothing, matching this page's "zero-privilege sandbox by default" framing for the first time. **Everything else on this page remains unbuilt**: `NetworkConnect`/`FsRead`/`FsWrite`/`SlackRead` (`PluginCapability`, `crates/plugin-sdk/src/lib.rs`) are still defined but unenforced — no host function exercises them yet. The manifest schema that's real (`capabilities = [...]`, a flat string list) is much smaller than this page's `[plugin]`/`[capabilities.network]`/`[capabilities.filesystem]` sketch — that fuller schema, the WASI `preopened_dir` filesystem restriction, and the manifest-approval database record (which would be `redb`, not SQLite, per ADR-0014 — this page predates that decision too) are all still just a design sketch for a future phase, wired in only if/when a real capability actually needs that much surface.

---

## 1. Capability Permission Map

Plugins run in a zero-privilege sandbox by default. Access must be requested in the manifest (`plugin.toml`) and verified by the host `PermissionManager`:

| Permission Key | Host Intercept Guard | Mitigation Action on Violation |
| :--- | :--- | :--- |
| `network:connect` | `host-services::publish-event` or socket bind hooks | Blocks domain resolve, logs security violation. |
| `fs:read` / `fs:write` | WASI virtual filesystem pre-opens | Block directory descriptors mapping. |
| `slack:read` / `slack:write` | Filter list in `PermissionManager` | Blocks event delivery, returns auth error. |
| `clipboard:access` | Linker API boundaries | Returns empty/denied result. |

---

## 2. Manifest Schema (`plugin.toml`)

```toml
[plugin]
id = "jira-issue-connector"
name = "Jira Integration Helper"
version = "0.1.0"

[capabilities]
network = ["api.atlassian.com"]
filesystem = [
    { path = "./cache", mode = "write" }
]
system = ["clipboard"]
```

---

## 3. Host Guard Execution (The Interceptor Pattern)

When a WASM plugin makes a request (e.g., trying to write to the clipboard), the call is handled by the Host Linker:

```text
       WASM Guest (invoke clipboard_write)
                 │
                 ▼
     [Host Linker Intercept]
                 │
                 ▼
       [PermissionManager] ──(Is "system:clipboard" granted?)
            /         \
         Yes           No
         /               \
        ▼                 ▼
   Write to OS       Log Security Alert,
   Clipboard         Return Error Code (AccessDenied)
```
- **Permission Check**: The `PermissionManager` holds an internal `HashSet<Permission>` per plugin instance loaded from the approved `plugin.toml` manifest database record in SQLite.
- **WASI Path Restriction**: Filesystem security is managed at the Wasmtime WASI context configuration stage:
  ```rust
  let mut wasi_ctx = WasiCtxBuilder::new();
  for path in allowed_paths {
      let file = std::fs::File::open(path)?;
      wasi_ctx.preopened_dir(file, path)?;
  }
  ```
  This restricts Guest directory access at the operating system file-descriptor layer.
