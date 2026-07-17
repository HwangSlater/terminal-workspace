# ADR 0006: SecretProvider Chain Design

## Context
OAuth tokens and API secrets must not be stored in plain text inside `config.toml` or log streams. However, developers work in different environments where credential managers vary:
- **Desktop (macOS/Windows/Linux)**: OS Keyring is preferred.
- **Server / Remote SSH / Headless Containers**: Keyrings are unavailable; environment variables or HashiCorp Vault are preferred.
- **Zero-Dependency Local Executions**: A lightweight, keyring-independent local encrypted file fallback is needed.

---

## Decision
We create a **`SecretProvider` interface (trait)** in the Domain layer. Credentials will be resolved using a **Provider Chain Pattern**, where the platform queries providers sequentially until a secret is successfully retrieved.

```rust
pub trait SecretProvider: Send + Sync {
    fn get_secret(&self, key: &str) -> Option<secrecy::SecretString>;
}
```

We chain:
1. `EnvProvider`: Checked first (highest flexibility for containers/Dev).
2. `KeyringProvider`: Integrates with local OS keychain interfaces (Mac/Windows/Linux Desktop).
3. `EncryptedFileProvider`: Fallback files stored locally (`~/.config/terminal-workspace/secrets.enc`). Upon the first keyring failure, the user prompts a master passphrase to decrypt/encrypt this file. This ensures OS independence and keyring-free local usage.

---

## Alternatives Considered

### Storing Credentials in `config.toml`
- **Pros**: Easy setup.
- **Cons**: Severe security risk. Credentials would be accidentally committed to Git repositories. (Rejected).

### Mandating Keyring in all environments
- **Pros**: Strong desktop security.
- **Cons**: Breaks headless executions, Docker runs, and SSH remote development where DBus interfaces are missing. (Rejected).

---

## Consequences
- Security credentials are separated from general application configurations.
- The platform adapts dynamically across desktop, server, and CI/CD targets without changing integration adapter code.
- If keyring lookup fails on headless machines, the `EncryptedFileProvider` acts as a zero-setup local database vault.

---

## Amendment (Phase 2 Implementation Note)

`SecretProviderChain` in `crates/secrets` was already built as a `Vec<Box<dyn SecretProvider>>` (not a fixed set of fields), so new providers (Vault, AWS Secrets Manager, test mocks) can be registered via `add_provider` without a trait or struct change — matching the extensibility this ADR calls for.

Phase 2 adds one convenience constructor, `SecretProviderChain::default_chain()`, which assembles the canonical order decided above (`EnvProvider → KeyringProvider → EncryptedFileProvider`) so callers don't have to re-derive the ordering at every call site. It is additive only; the chain remains freely reconfigurable via `new`/`add_provider` for tests or alternate environments (e.g. injecting a mock provider first in unit tests).
