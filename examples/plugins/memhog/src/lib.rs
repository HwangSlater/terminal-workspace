//! Deliberately broken example plugin (`step14.md` Verification Plan):
//! `on-event` allocates far past the 64MB per-instance memory ceiling
//! (`docs/04-extensions/plugin-lifecycle.md` §3.2). Proves the memory
//! limiter actually stops unbounded guest allocation rather than letting
//! it grow the host process's memory without bound.

#[allow(warnings)]
mod bindings;

use bindings::Guest;

struct MemhogPlugin;

impl Guest for MemhogPlugin {
    fn initialize(_config: String) -> Result<(), String> {
        Ok(())
    }

    fn on_event(_event_type: String, _payload: String) -> Result<(), String> {
        // 200 x 1MB chunks = 200MB, well past the 64MB store limit.
        let mut chunks: Vec<Vec<u8>> = Vec::new();
        for _ in 0..200 {
            let chunk = vec![0u8; 1024 * 1024];
            std::hint::black_box(&chunk);
            chunks.push(chunk);
        }
        std::hint::black_box(&chunks);
        Ok(())
    }

    fn shutdown() -> Result<(), String> {
        Ok(())
    }
}

bindings::export!(MemhogPlugin with_types_in bindings);
