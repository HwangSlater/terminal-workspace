# Security Architecture Specification

Maintaining the confidentiality of developer credentials (OAuth tokens, API keys) and securing the execution of third-party code are absolute priorities. This document outlines the core security controls integrated into the Terminal Workspace.

---

## 1. Credentials Storage (System Keyring Integration)

> **Implementation Status (Phase 7)**: `crates/secrets` implements this section for real (`KeyringProvider`, `EncryptedFileProvider`) — see `step7.md`. Two corrections against the original spec below: there is no separate `keyring_service_name`/`keyring_key` config scheme — the service name is a fixed constant (`"terminal-workspace"`) and the lookup key is the secret's own name (e.g. `SLACK_BOT_TOKEN`), so nothing about credentials appears in `config.toml` at all. And storage isn't keyring-only: a local AES-256-GCM encrypted file (`EncryptedFileProvider`) is a fallback for environments with no reachable keyring backend (headless Linux, some containers) — see its honest limitation noted where it's implemented (the encryption key sits in a plain file next to the ciphertext; this is not a security-equivalent alternative to a real OS keyring, only a fallback for when one is unavailable).

Storing raw OAuth tokens or API secret keys in plain-text configuration files (`config.toml`) or local databases (`workspace.redb`) is strictly prohibited.

- **System Keyring Adapter**: The Workspace utilizes the Rust `keyring` crate to interact directly with OS-native credential managers:
  - **macOS**: Keychain Services
  - **Linux**: Secret Service API via DBus (gnome-keyring / kwallet) — via a pure-Rust DBus client (`zbus`), not the C-binding `libdbus` flavor, keeping ADR-0014's "no C compiler required" property intact.
  - **Windows**: Credential Manager
- **Token Retrieval Flow** (`SecretProviderChain`, ADR-0006):
  1. On startup, the chain tries the `SLACK_BOT_TOKEN` environment variable first, then the OS keyring, then the encrypted file.
  2. The token is stored in memory as an ephemeral `secrecy::SecretString` wrapper.
  3. Memory allocations containing the decrypted token are zeroized on drop (`secrecy`'s guarantee).
- **Token Write Flow** (new in Phase 7 — a token entered through the in-app setup overlay, `Ctrl+S`): the chain tries the OS keyring first, falling back to the encrypted file if no keyring backend is reachable. The environment variable is never a write target — setting a process env var from inside the app wouldn't survive a restart, defeating the point of durable storage.

---

## 2. Dynamic Log Scrubbing (Secret Masking)

To prevent leaking API tokens or private user data into text log files (`app.log`), the logging pipeline incorporates a custom filter:

- **Regex Masking**: All standard logs pass through a parser that intercepts patterns resembling OAuth tokens (`xoxb-[a-zA-Z0-9-]+`, `ghp_[a-zA-Z0-9]{36}`) or bearer Authorization headers and replaces them with `[REDACTED_SECRET]`.
- **Level Constraint**: Production deployments run at `info` level, which disables logging of raw HTTP request and response payloads.

---

## 3. Network Transport Security

- **TLS Enforced**: All outgoing integration HTTP requests are encrypted using TLS, via `reqwest`'s default `native-tls` backend — **not** `rustls` as originally specified here. `rustls`'s default crypto provider (`ring`) compiles C/assembly source at build time, reintroducing exactly the C-toolchain requirement ADR-0014 eliminated by switching storage to `redb`; discovered and corrected while building the first real HTTP-using adapter (`SlackAdapter`, Phase 6 — see `step6.md`). `native-tls` uses each OS's built-in TLS stack instead (SChannel on Windows, Secure Transport on macOS — neither needs a C compiler; Linux links system OpenSSL via `openssl-sys`, needing only pre-installed dev headers, not a compiler).
- **Certificate Pinning**: (Optional) For high-security environments, the client can be configured to pin certificate authorities for Slack and GitHub API domains to prevent Man-in-the-Middle (MitM) inspection.

---

## 4. Audit Logging

All administrative events are recorded in the local database under `failed_events` or write-only log channels:
- Authentication changes (token registration / removal).
- Plugin installs, updates, or crashes.
- Configuration edits.
- Permission escalations.
