# ADR 0009: Plugin SDK WIT Component Model Selection

## Context
WASM modules cannot naturally share complex structures (like structs or lists) with the Host without FFI boundary translation rules. We need a secure, typed, and maintainable FFI model.

---

## Decision
We adopt the **WebAssembly Component Model** utilizing **WIT (WebAssembly Interface Type)** definitions. Pointers and memory allocations are managed automatically using generated bindings via `wit-bindgen`.

---

## Alternatives Considered

### Raw Pointer FFI & JSON Serialization
- **Pros**: Easy to implement initially.
- **Cons**: Severe lack of type safety. Guest plugins must manually allocate linear memory, pass raw `*mut u8` pointers to the host, and invoke unsafe functions. This defeats Rust's memory safety guarantees and leads to memory leaks if deallocation is skipped.

---

## Consequences
- **Static Type Safety**: WIT interface definitions ensure compile-time check matches across the host and guest, eliminating runtime FFI mismatch crashes.
- **WASM Component Target**: Plugins must compile to the WASM Component target, which requires Rust toolchains supporting the component model.
