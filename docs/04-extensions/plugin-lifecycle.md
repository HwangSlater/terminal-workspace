# Plugin Lifecycle Specification

This document defines the lifecycle states, WebAssembly hook transitions, and runtime boundary controls managed by the `PluginManager`.

---

## 1. Lifecycle State Diagram

The lifecycle of any WASM plugin is governed by the state machine below:

```mermaid
stateDiagram-v2
    [*] --> Unloaded
    Unloaded --> Loaded : Compile WASM (Host Linker)
    Loaded --> Initialized : Invoke guest initialize()
    Initialized --> Active : Register Command & UI Hooks
    Active --> Suspended : Trapped (Resource Limit / Panic)
    Suspended --> Active : Manual reload command
    Active --> Terminated : Invoke guest shutdown()
    Terminated --> Unloaded : Free WASM Instance memory
```

---

## 2. Transition Hook Executables

The `PluginManager` invokes Guest FFI bindings during lifecycle transitions:

### 1. `initialize()`
- **Host Action**: Allocates memory blocks and writes initial TOML/JSON configuration settings. Passes the configuration buffer pointer to the Wasm guest.
- **Guest Action**: Initializes internal memory caches, parses configurations, and allocates state.

### 2. `shutdown()`
- **Host Action**: Notifies the guest that the workspace is terminating.
- **Guest Action**: Flushes transient caches to pre-opened virtual files, closes pending sync timers, and prepares memory for unloading.

---

## 3. Resource & Instruction Limits (Wasmtime Runtime Bounds)

To prevent third-party plugins from locking the CPU or leaking memory, we enforce strict limits using `wasmtime` engine constraints:

### 1. CPU Execution Budget (Fuel)
- **Mechanism**: We enable Wasmtime instruction counting ("Fuel").
- **Limit**: Each event handled by the plugin is allocated a maximum of 1,000,000 instructions (fuel ticks).
- **Enforcement**: If the guest runs an infinite loop, the fuel is depleted. Wasmtime traps the execution, the `PluginManager` catches the exception, logs a `PLUGIN_CPU_LIMIT_EXCEEDED` error, suspends the plugin, and frees the instance.

### 2. Memory Limits (Linear Memory Limits)
- **Mechanism**: Configure Wasmtime's `Store` memory limiters.
- **Limit**: 64MB maximum heap allocation.
- **Enforcement**: Any guest allocation request that pushes memory usage past 64MB returns an out-of-memory error inside WASM or traps the execution.
