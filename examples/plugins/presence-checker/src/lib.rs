//! Example plugin (`step16.md`) proving `get-member-presence` capability
//! enforcement end-to-end: `on-event` calls it for `"local-user"` and logs
//! whichever outcome comes back (a real presence value if this plugin's
//! `presence-checker.toml` manifest grants `presence-read`, or the
//! capability-denial error string if it doesn't / the manifest is absent).
//! `crates/plugin-host`'s tests use two copies of this same `.wasm` --
//! one with a manifest, one without -- to prove both outcomes for real.

#[allow(warnings)]
mod bindings;

use bindings::workspace::plugins::host_services;
use bindings::Guest;

struct PresenceCheckerPlugin;

impl Guest for PresenceCheckerPlugin {
    fn initialize(_config: String) -> Result<(), String> {
        Ok(())
    }

    fn on_event(_event_type: String, _payload: String) -> Result<(), String> {
        match host_services::get_member_presence("local-user") {
            Ok(status) => {
                host_services::log("info", &format!("local-user presence: {status:?}"));
            }
            Err(e) => {
                host_services::log("info", &format!("get-member-presence denied: {e}"));
            }
        }
        Ok(())
    }

    fn shutdown() -> Result<(), String> {
        Ok(())
    }
}

bindings::export!(PresenceCheckerPlugin with_types_in bindings);
