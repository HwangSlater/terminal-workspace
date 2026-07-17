# Configuration Specification

The Terminal Workspace is configured via a single **TOML** file (`config.toml`) situated in the user's standard configuration directory. TOML was selected for its clean syntax, strong typing support, and seamless integration with Rust's `serde` framework.

> **Implementation Status (Phase 2, amended Phase 6)**: `crates/config` implements `[core]`, a real nested `[integrations.slack]` section (`enabled`, `sync_interval_secs`, `channel_ids`, `watched_user_ids` — see `docs/04-extensions/integrations/slack.md`), and a still-flat `github_enabled` toggle (no GitHub adapter exists yet). This was a breaking change to the on-disk schema (`[integrations] slack_enabled = ...` → `[integrations.slack] enabled = ...`) — acceptable pre-v1.0 with no public users yet (`step6.md`); a user with an old `config.toml` gets the new default file layout by deleting it and letting Zero-Config regenerate it. The richer schema shown in §1 beyond Slack (`repositories`, `[plugins]`, `[keybindings]`, etc.) remains the target shape for later phases, not built yet.

---

## 1. Complete Configuration Schema (`config.toml`)

Below is the structured layout and description of the settings file:

```toml
# General Core Settings
[core]
theme = "nord-dark"             # UI Theme stylesheet name
refresh_rate_ms = 100           # TUI frame polling interval
log_level = "info"              # debug, info, warn, error
auto_away_timeout_mins = 15     # Auto-switch Slack status to Away after inactivity

# Slack Integration Settings
[integrations.slack]
enabled = true
sync_interval_secs = 30
channel_ids = ["C0123456789"]        # Channels polled for messages (conversations.history)
watched_user_ids = ["U0123456789"]   # Teammates polled for presence (users.getPresence) --
                                      # not the whole workspace roster; see docs/04-extensions/integrations/slack.md
# SLACK_BOT_TOKEN is read from the environment (SecretProviderChain, ADR-0006). Never stored in this file.

# GitHub Integration Settings
[integrations.github]
enabled = true
sync_interval_secs = 60
repositories = [
    "google/terminal-workspace",
    "rust-lang/rust"
]

# Google Calendar Settings
[integrations.calendar]
enabled = true
sync_interval_secs = 300
calendar_ids = ["primary"]

# Active Plugin Configuration
[plugins]
directory = "~/.local/share/terminal-workspace/plugins"
allowed_list = [
    "todo-tracker",
    "pomodoro-timer"
]

# Custom Keybindings mapping overrides
[keybindings]
focus_next_pane = "tab"
quit = "ctrl+q"
```

---

## 2. Configuration Struct & Validation (Rust)

The configuration module parses this file into strongly-typed Rust structures using `serde` and performs validation on startup.

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct AppConfig {
    pub core: CoreSettings,
    pub integrations: IntegrationsSettings,
    pub plugins: PluginsSettings,
    pub keybindings: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoreSettings {
    pub theme: String,
    #[serde(default = "default_refresh_rate")]
    pub refresh_rate_ms: u64,
    pub log_level: String,
    pub auto_away_timeout_mins: u32,
}

fn default_refresh_rate() -> u64 { 100 }

impl AppConfig {
    /// Validates the configuration values.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.core.refresh_rate_ms < 16 {
            return Err(ConfigError::InvalidValue("refresh_rate_ms must be >= 16ms (60 FPS limit)".into()));
        }
        if self.core.auto_away_timeout_mins == 0 {
            return Err(ConfigError::InvalidValue("auto_away_timeout_mins must be > 0".into()));
        }
        Ok(())
    }
}
```

---

## 3. Configuration Layering (`ConfigBuilder`)

Non-secret settings (`[core]`, `[integrations]` toggles, etc.) and secret values (OAuth tokens, API keys) follow **two distinct precedence chains**. Conflating them under one list previously caused ambiguity; they are kept separate here.

### 3.1 Config Value Precedence

`crates/config::ConfigBuilder` merges four layers, each overriding only the fields it explicitly sets — later layers win:

```text
Default
    ↓
config.toml   (merge_file)
    ↓
Environment   (merge_env)
    ↓
CLI Option    (merge_cli)
    ↓
AppConfig
```

- **Default**: `AppConfig::default()` — always the base; guarantees Zero Configuration (see §4).
- **File** (`merge_file`): parses `config.toml` if present; missing file is not an error, defaults are bootstrapped to disk instead (see §4).
- **Environment** (`merge_env`): any key can be overridden by variables prefixed `TERM_WS_`, mapped `SECTION_FIELD` → `TERM_WS_SECTION_FIELD`:
  - `TERM_WS_CORE_THEME` → `core.theme`
  - `TERM_WS_CORE_REFRESH_RATE_MS` → `core.refresh_rate_ms`
  - `TERM_WS_CORE_LOG_LEVEL` → `core.log_level`
  - `TERM_WS_INTEGRATIONS_SLACK_ENABLED` → `integrations.slack.enabled`
  - `TERM_WS_INTEGRATIONS_GITHUB_ENABLED` → `integrations.github_enabled`
- **CLI Option** (`merge_cli`): highest precedence, for one-off overrides at invocation time: `--theme`, `--log-level`, `--refresh-rate-ms`, `--config <path>`. Implemented as a small hand-rolled `--key value` scan rather than a CLI-parsing dependency (e.g. `clap`) — the flag surface is intentionally tiny today (4 flags) and the project favors a minimal dependency graph until the CLI surface actually grows (e.g. plugin subcommands in a later phase). Revisit this choice via ADR if/when that happens.
- `AppConfig::load_or_create_default()` is a convenience wrapper: `ConfigBuilder::new().merge_file(<standard path>).merge_env().merge_cli(std::env::args()).build()`.

### 3.2 Secret Value Precedence

Secrets are **never** part of `config.toml` or the layers above — they are resolved separately by `SecretProviderChain` (ADR-0006):

1. **Environment Variables**: Checked first (useful for dev container/headless server environments). e.g. `TERM_WS_SLACK_TOKEN` / `SLACK_BOT_TOKEN`.
2. **System Keyring (Keytar/DBus)**: Checked second for sensitive tokens/credentials.
3. **Encrypted Local File**: Checked last as a keyring-free, zero-dependency fallback (`~/.config/terminal-workspace/secrets.enc`).

See `docs/06-development/decisions/0006-secret-provider.md` for the full rationale and `SecretProviderChain::default_chain()` for the canonical wiring.

---

## 4. Zero-Configuration First Run

The platform targets Terminal First / Local First / Cross Platform / Zero Configuration. A first run must not require the user to hand-author any file or set any environment variable:

```text
$ tw
```

On startup, `AppConfig::load_or_create_default()`:
1. Resolves the OS-standard config directory (`$HOME/.config/terminal-workspace` or `%USERPROFILE%\.config\terminal-workspace`), creating it if missing.
2. If `config.toml` does not exist, writes a default file there (so the user has something inspectable/editable afterward) and proceeds with in-memory defaults for this run.
3. Applies the Environment and CLI layers on top (§3.1), then validates and returns `AppConfig`.

No required flags, no required env vars — the layered `ConfigBuilder` degrades gracefully to `AppConfig::default()` at every layer that isn't present.
