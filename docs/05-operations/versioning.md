# Versioning & Deprecation Policy

This document details the versioning strategies for the Terminal Workspace codebase, local database migrations, and WASM Plugin SDK schemas.

---

## 1. Core Platform Versioning (SemVer)
The project adheres strictly to Semantic Versioning 2.0.0 (MAJOR.MINOR.PATCH):
- **MAJOR**: Changes that break backwards compatibility (e.g., modifying the core `enum Event` layout or deleting SDK host import functions).
- **MINOR**: Additions of non-breaking features (e.g., adding a new integration adapter, adding a new panel view slot).
- **PATCH**: Bug fixes, performance optimizations.

---

## 2. Plugin SDK Compatibility & Deprecation

Since the Plugin SDK uses WASM Component Model (`wit` definitions), host-guest compatibility is governed by the following rules:

### 1. Interface Versioning
The SDK imports are exported under versioned namespaces in WIT (e.g., `workspace:plugins/events@1.0.0`).
- **Backwards Compatibility**: The host links both `@1.0.0` and `@1.1.0` endpoints if possible, allowing older plugin binaries to load without recompilation.
- **Breaking changes**: If a breaking change occurs, the host increments the WIT namespace to `@2.0.0`.

### 2. Deprecation Policy
- Deprecated SDK APIs are decorated with deprecation warnings in compilation headers.
- Deprecated APIs remain supported for one minor version release (e.g., deprecated in v1.2, fully removed in v1.3).
- On loading an outdated plugin, the `PluginManager` logs a warning to `app.log` indicating: `Plugin [Name] uses deprecated SDK v1.0. Upgrade recommended.`
