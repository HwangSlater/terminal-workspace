# Plugin Permission Model Specification

The Terminal Workspace implements a **Capability-Based Security Model** for WebAssembly plugins. Plugins cannot access system resources (network, filesystem, commands) by default. They must request capabilities in their manifest file, which the user explicitly approves during installation.

---

## 1. Capabilities Definitions

The host runtime recognizes six scopes of permissions:

| Permission Scope | Parameter | Description |
| :--- | :--- | :--- |
| `network:connect` | Allowed Domain Lists (e.g., `["api.github.com"]`) | Allows the plugin to make outbound HTTP requests to specified domains. Wildcards `*` are forbidden. |
| `fs:read` | Read-only Directory Paths | Allows the plugin to read files in specified directories. |
| `fs:write` | Read-write Directory Paths | Allows the plugin to modify/create files in specified directories. |
| `slack:read` | None | Allows the plugin to read Slack channels/messages routed via Event Bus. |
| `slack:write` | None | Allows the plugin to publish message sending commands to the Host. |
| `github:read` | None | Allows the plugin to read repository state and PR reviews. |

---

## 2. Plugin Manifest File (`plugin.toml`)

Every plugin package must include a manifest declaring its identifying metadata and requested permissions.

```toml
[plugin]
id = "todo-sync-plugin"
name = "Todo Sync Manager"
version = "1.2.0"
author = "Dev Corp"

[permissions]
# Requesting access to connect to specific sync backend
network = [
    "sync-service.devcorp.com"
]

# Read-write workspace files to extract todo comments
filesystem = [
    { path = "./src", mode = "read" },
    { path = "./target/todo_cache", mode = "write" }
]

# Requires integration scopes
slack = ["read", "write"]
github = ["read"]
```

---

## 3. Host Capability Enforcement

The WASM runtime (`wasmtime`) enforces requested permissions through Host Linker bindings.

- **Sandbox Initialization**:
  ```rust
  pub struct PluginCapabilities {
      pub allowed_domains: Vec<String>,
      pub read_paths: Vec<std::path::PathBuf>,
      pub write_paths: Vec<std::path::PathBuf>,
      pub slack_read_allowed: bool,
  }
  ```
- **Outbound HTTP Linker Enforcer**:
  When a guest plugin invokes `host_http_request(url_ptr, len)`, the host:
  1. Resolves the destination domain from the URL pointer.
  2. Verifies if the domain matches the `allowed_domains` list.
  3. If missing, blocks the execution and returns a permission-denied error code to the guest.
- **Wasi Virtual Filesystem Mapping**:
  `wasmtime_wasi` limits the plugin's file system calls to pre-opened directory descriptors. The host only links directories listed in the approved filesystem permissions. Any call to read outside these mappings is trapped at compilation/WASI syscall level.
