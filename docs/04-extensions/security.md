# Security Architecture Specification

Maintaining the confidentiality of developer credentials (OAuth tokens, API keys) and securing the execution of third-party code are absolute priorities. This document outlines the core security controls integrated into the Terminal Workspace.

---

## 1. Credentials Storage (System Keyring Integration)

Storing raw OAuth tokens or API secret keys in plain-text configuration files (`config.toml`) or local databases (`workspace.redb`) is strictly prohibited.

- **System Keyring Adapter**: The Workspace utilizes the Rust `keyring` crate to interact directly with OS-native credential managers:
  - **macOS**: Keychain Services
  - **Linux**: Secret Service API via DBus (gnome-keyring / kwallet)
  - **Windows**: Credential Manager
- **Token Retrieval Flow**:
  1. On startup, the configuration service reads token identifiers (e.g., `keyring_service_name = "terminal-workspace"`, `keyring_key = "slack_token"`).
  2. The integration service requests the token from the OS keyring asynchronously.
  3. The token is stored in memory as an ephemeral `secrecy::SecretString` wrapper.
  4. Memory allocations containing the decrypted token are zeroized on drop.

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
