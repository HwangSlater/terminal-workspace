# ADR 0001: Core Architecture, Language, and Runtime Selection

## Context
The Terminal Workspace project aims to integrate multiple external services (Slack, GitHub, Gmail, Jira) into a unified console. The core requirements are:
- High execution performance and low CPU/memory footprints.
- Asynchronous integration workers executing in the background.
- Long-term maintainability over years of development with strict testability.
- Code robustness to avoid workspace crashes during integration failures.

We evaluated three potential languages for the runtime core:
1. **Rust**: High performance, memory-safe, outstanding async tooling (Tokio), and compiled native binaries.
2. **Go**: Excellent concurrency (goroutines), but higher runtime memory overhead and garbage collection pauses affecting TUI rendering stability.
3. **Node.js / TypeScript**: Easy service integration (HTTP calls), but large bundle sizes, single-threaded event loop constraints, and lacks low-level OS keyring integrations out of the box without native bindings.

---

## Decision
We select **Rust** as the core programming language and **Tokio** as the asynchronous runtime. The project follows **Clean Architecture** patterns separated into a multi-crate Cargo Workspace.

---

## Alternatives Considered

### 1. Go (Golang)
- *Pros*: Simple syntax, swift network IO.
- *Cons*: Garbage collection can cause micro-stuttering during TUI frames. Go's plugin loading (`plugin` package) is historically unstable, platform-restricted (no Windows support for plugins), and unsafe for multi-tenant extensions.

### 2. Node.js (TypeScript)
- *Pros*: Quick prototyping, rich NPM ecosystem.
- *Cons*: Consumes substantial memory (V8 overhead), making it unsuitable for a background developer daemon.

---

## Consequences

- **Safety & Performance**: Rust guarantees compile-time memory safety without a garbage collector.
- **Async Concurrency**: Tokio handles thousands of background service polling intervals and Event Bus dispatches on minimal thread footprints.
- **Architectural Isolation**: Strict layer isolation enforces that `core/domain` relies on zero libraries, facilitating easy mock-driven unit testing.
- **Complexity**: Rust has a steeper learning curve, meaning plugin developers need to adhere to strict SDK boundaries.
