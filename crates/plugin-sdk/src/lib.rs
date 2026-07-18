//! Plugin SDK: the crate a guest plugin author's `cargo-component`-built
//! crate depends on. Generates real WASM Component Model bindings from
//! `wit/plugin-sdk.wit` (ADR-0009, `step14.md`) — replaces an earlier,
//! plain `#[async_trait] GuestPlugin` trait that was never going to be
//! what a sandboxed WASM guest actually implements (component exports are
//! synchronous calls across the canonical ABI, not native Rust `async`).

wit_bindgen::generate!({
    world: "developer-workspace-plugin",
    path: "wit/plugin-sdk.wit",
    // Generates the `export!` macro invocation-target trait as `Guest`
    // and the host-service imports as free functions -- see this crate's
    // re-exports below for the stable names plugin authors actually use.
});

// `Guest` (the trait a plugin author implements for `initialize`/
// `on_event`/`shutdown`) and the `export!` macro are already generated at
// this crate's root by the `generate!` call above -- `initialize`/
// `on-event`/`shutdown` are exported directly by the world (not through a
// named interface), so wit-bindgen doesn't nest them under `exports::`.
// `log`/`publish_event` (host-services, imported by the world) resolve
// via their package/interface path -- re-exported here so plugin authors
// write `plugin_sdk::log(...)` rather than reaching into this crate's
// generated module layout, which isn't a stability guarantee.
pub use workspace::plugins::host_services::{
    get_member_presence, log, publish_event, PresenceStatus,
};

/// Host capabilities a plugin's manifest can request. `PresenceRead` is
/// **enforced for real** as of `step16.md` (`crates/plugin-host`'s
/// `PermissionManager::verify_capability`, checked against a
/// `<plugin-id>.toml` manifest) — the first of these to be. The other
/// three remain defined but **not yet enforced**: nothing in any example
/// plugin exercises filesystem/network access yet. Wire real enforcement
/// for one of them once a real plugin actually needs it gated, the same
/// way `PresenceRead` got wired once `get-member-presence` needed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCapability {
    /// Connect to remote networks.
    NetworkConnect(String),
    /// Read local folders.
    FsRead(String),
    /// Write local folders.
    FsWrite(String),
    /// Request Slack reading.
    SlackRead,
    /// Query a team member's presence status (`get-member-presence`).
    PresenceRead,
}
