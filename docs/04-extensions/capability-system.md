# Capability Permission System Specification

This document details the capability authorization system that validates and intercepts resource usage by guest plugins.

> **Implementation Status (Phase 14, `step14.md` Decision 3)**: **nothing on this page is built.** The real `PermissionManager::verify_capability` (`crates/plugin-host/src/lib.rs`) is a stub that always returns `true`, honestly labeled as such in its own doc comment — no permission map, no `plugin.toml` manifest file/schema, no WASI `preopened_dir` filesystem restriction, no manifest-approval database record (which would be `redb`, not SQLite, per ADR-0014 — this page predates that decision too). `PluginCapability` (`crates/plugin-sdk/src/lib.rs`) defines four variants (`NetworkConnect`, `FsRead`, `FsWrite`, `SlackRead`) as a plain Rust enum a plugin's manifest *could* eventually use, but nothing reads or enforces them yet. This was a deliberate scope decision, not an oversight: `step14.md`'s Decision 3 chose to ship the sandbox/lifecycle/resource-limit plumbing first and defer capability enforcement until a real plugin actually needs one of these gated (the example plugins request none of them). Treat this whole document as a design sketch for that future phase, not a description of current behavior.

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
