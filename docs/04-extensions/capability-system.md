# Capability Permission System Specification

This document details the capability authorization system that validates and intercepts resource usage by guest plugins.

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
