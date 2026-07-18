//! Deliberately malicious/broken example plugin (`step14.md` Verification
//! Plan): `on-event` busy-loops forever. Proves the fuel-budget limit
//! (`docs/04-extensions/plugin-lifecycle.md` §3.1) actually traps a
//! runaway guest rather than hanging the host -- "a plugin crash must not
//! crash the workspace" (ADR-0002) is the entire point of choosing WASM
//! sandboxing over dynamic libraries, and this is the one test in the
//! phase that actually proves it rather than asserting it.

#[allow(warnings)]
mod bindings;

use bindings::Guest;

struct LooperPlugin;

impl Guest for LooperPlugin {
    fn initialize(_config: String) -> Result<(), String> {
        Ok(())
    }

    fn on_event(_event_type: String, _payload: String) -> Result<(), String> {
        loop {
            std::hint::black_box(0);
        }
    }

    fn shutdown() -> Result<(), String> {
        Ok(())
    }
}

bindings::export!(LooperPlugin with_types_in bindings);
