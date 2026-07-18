# Plugin SDK & WIT Interface Specification

This document details the WebAssembly Interface Type (WIT) schemas defining the host imports and guest exports for the plugin boundary.

> **Implementation Status (Phase 16, `step16.md`, amending the Phase 14/ADR-0017 note below)**: `get-member-presence` is now real — restored to `crates/plugin-sdk/wit/plugin-sdk.wit`'s `host-services` interface, gated behind the `presence-read` capability (see `docs/04-extensions/capability-system.md`'s own updated status note). It returns a `presence-status` enum (mapping `domain::PresenceStatus`'s five variants), not the fuller `types` interface (`notification`, etc.) §1 below also sketches — that part remains unbuilt, nothing needs it yet. §2's code sketches still have the wrong file paths — see `crates/plugin-sdk/src/lib.rs` (guest) and `crates/plugin-host/src/lib.rs` (host, inside a `mod bindings` — the macro's generated code collides with `common::Result` at crate-root scope otherwise) for the real invocations.

---

## 1. WebAssembly Interface Type Definition (`plugin-sdk.wit`)

We use the standard WASM Component Model WIT format to generate bindings automatically via `wit-bindgen`:

```wit
package workspace:plugins;

interface types {
    enum presence-status {
        active,
        away,
        offline,
        meeting,
        lunch
    }

    record notification {
        id: string,
        source: string,
        title: string,
        body: string,
        priority: string,
    }
}

interface host-services {
    use types.{notification, presence-status};

    /// Publish a strongly-typed event back to the Host Event Bus
    publish-event: func(event-type: string, payload: string) -> result<_, string>;

    /// Print a message to the unified tracing logger
    log: func(level: string, message: string);

    /// Query current presence status of a team member
    get-member-presence: func(user-id: string) -> result<presence-status, string>;
}

world developer-workspace-plugin {
    import host-services;
    
    export initialize: func(config: string) -> result<_, string>;
    export on-event: func(event-type: string, payload: string) -> result<_, string>;
    export shutdown: func() -> result<_, string>;
}
```

---

## 2. Binding Code Generation (Host & Guest)

### Host Generation (Rust):
```rust
// In core/src/application/plugin_manager.rs
wasmtime::component::bindgen!({
    world: "developer-workspace-plugin",
    path: "docs/plugin-sdk.wit",
});
```

### Guest Generation (Rust Plugin SDK):
```rust
// In plugins/sdk/src/lib.rs
wit_bindgen::generate!({
    world: "developer-workspace-plugin",
    path: "plugin-sdk.wit",
});
```
This generates safe, typed interfaces. Both host and guest interact using native Rust types rather than raw pointers, eliminating FFI memory leaks and compilation drift.
