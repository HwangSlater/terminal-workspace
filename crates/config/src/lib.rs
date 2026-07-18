//! Configuration loader parsing settings from TOML file.
//!
//! See `docs/05-operations/configuration.md` §3 for the full layering rationale.

use common::{Result, WorkspaceError};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Main application configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Core runtime configs.
    pub core: CoreSettings,
    /// Enabled integrations switch.
    #[serde(default)]
    pub integrations: IntegrationsToggle,
}

/// Core runtime configuration fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreSettings {
    /// TUI theme styles selector.
    pub theme: String,
    /// Milliseconds between rendering loops.
    pub refresh_rate_ms: u64,
    /// System log level.
    pub log_level: String,
}

/// Per-integration settings. `slack` is a real nested table (Phase 6); `github_enabled`
/// stays a flat toggle since no GitHub adapter exists yet.
///
/// Every field here is `#[serde(default)]`: a `config.toml` written before
/// a given field existed (or before the `[integrations.slack]` table
/// existed at all — the schema this superseded, ADR-0014-adjacent lesson
/// from `step6.md`) must still parse, not hard-fail the whole app on
/// startup. Zero Configuration (`product-requirements.md` §2.1) applies to
/// config *evolution*, not just first run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntegrationsToggle {
    /// Slack integration settings (`docs/04-extensions/integrations/slack.md`).
    #[serde(default)]
    pub slack: SlackSettings,
    /// Sync GitHub updates.
    #[serde(default)]
    pub github_enabled: bool,
}

/// `[integrations.slack]` settings. The Bot Token itself is never here —
/// it's resolved via `SecretProviderChain` (ADR-0006), never `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackSettings {
    /// Whether the Slack adapter starts at all.
    #[serde(default)]
    pub enabled: bool,
    /// Seconds between poll cycles.
    #[serde(default = "default_slack_sync_interval")]
    pub sync_interval_secs: u64,
    /// Channels polled for messages (`conversations.history`).
    #[serde(default)]
    pub channel_ids: Vec<String>,
    /// Teammates polled for presence (`users.getPresence`) — a configured
    /// watch-list, not the whole workspace roster.
    #[serde(default)]
    pub watched_user_ids: Vec<String>,
}

impl Default for SlackSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            sync_interval_secs: default_slack_sync_interval(),
            channel_ids: Vec::new(),
            watched_user_ids: Vec::new(),
        }
    }
}

fn default_slack_sync_interval() -> u64 {
    30
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            core: CoreSettings {
                theme: "default-dark".into(),
                refresh_rate_ms: 100,
                log_level: "info".into(),
            },
            integrations: IntegrationsToggle {
                slack: SlackSettings {
                    enabled: false,
                    sync_interval_secs: default_slack_sync_interval(),
                    channel_ids: Vec::new(),
                    watched_user_ids: Vec::new(),
                },
                github_enabled: false,
            },
        }
    }
}

impl AppConfig {
    /// Load configuration by checking default folders or creating defaults if missing.
    /// Hierarchy: Default -> `config.toml` -> Environment -> CLI Option (see `ConfigBuilder`).
    pub fn load_or_create_default() -> Result<Self> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let path = resolve_config_path();

        ConfigBuilder::new()
            .merge_file(&path)?
            .merge_env()
            .merge_cli(&args)
            .build()
    }

    /// Parse TOML configurations from target string buffer.
    pub fn parse(toml_str: &str) -> Result<Self> {
        if toml_str.trim().is_empty() {
            return Ok(Self::default());
        }
        let config: AppConfig =
            toml::from_str(toml_str).map_err(|e| WorkspaceError::Configuration(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Overwrite `path` with this config, serialized as TOML. Used by the
    /// Slack channel/user picker (`step8.md`) to persist a selection —
    /// **not** part of the normal boot path, which stays read-only.
    ///
    /// Round-trips through `serde`, so hand-added comments/formatting in an
    /// existing file are lost on save. Accepted limitation, same category
    /// as `EncryptedFileProvider`'s honest tradeoff (`crates/secrets`) —
    /// pre-v1.0 with no public users yet, not hidden.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| WorkspaceError::Configuration(e.to_string()))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| WorkspaceError::Configuration(e.to_string()))?;
        }
        fs::write(path, toml_str).map_err(|e| WorkspaceError::Configuration(e.to_string()))
    }

    /// Perform configurations validation constraints.
    pub fn validate(&self) -> Result<()> {
        if self.core.refresh_rate_ms < 16 {
            return Err(WorkspaceError::Configuration(
                "refresh_rate_ms cannot be below 16ms (60 FPS max)".into(),
            ));
        }
        Ok(())
    }
}

/// Resolve the OS-standard configuration directory path for `config.toml`,
/// creating the parent directory if it does not yet exist.
fn standard_config_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    let mut path = PathBuf::from(home);
    path.push(".config");
    path.push("terminal-workspace");

    if let Err(e) = fs::create_dir_all(&path) {
        tracing::warn!(
            "Failed to create config directory: {:?}. Using defaults.",
            e
        );
    }

    path.push("config.toml");
    path
}

/// Scan raw CLI args for a `--config <path>` override, used to pick which
/// file `merge_file` reads before the rest of the CLI layer is applied.
fn cli_config_path_override(args: &[String]) -> Option<PathBuf> {
    args.iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
}

/// The exact path `load_or_create_default` reads from (a `--config`
/// override if given, else the OS-standard location) — exposed separately
/// so a caller that needs to write back later (`AppConfig::save_to`, the
/// picker in `step8.md`) targets the identical file rather than guessing.
#[must_use]
pub fn resolve_config_path() -> PathBuf {
    let args: Vec<String> = std::env::args().skip(1).collect();
    cli_config_path_override(&args).unwrap_or_else(standard_config_path)
}

const DEFAULT_TOML: &str = r#"# Terminal Workspace Core Settings
[core]
theme = "default-dark"
refresh_rate_ms = 100
log_level = "info"

[integrations]
github_enabled = false

[integrations.slack]
enabled = false
sync_interval_secs = 30
channel_ids = []
watched_user_ids = []
"#;

/// Builds an `AppConfig` by layering Default -> File -> Environment -> CLI,
/// where each later layer only overrides the fields it explicitly sets.
///
/// See `docs/05-operations/configuration.md` §3.1 for the field/precedence contract.
pub struct ConfigBuilder {
    config: AppConfig,
}

impl ConfigBuilder {
    /// Start a new builder seeded with `AppConfig::default()`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: AppConfig::default(),
        }
    }

    /// Merge in the `config.toml` layer. If `path` does not exist, bootstraps
    /// a default file there (Zero Configuration first run) and leaves the
    /// builder's in-memory defaults untouched for this run.
    pub fn merge_file(mut self, path: &Path) -> Result<Self> {
        if !path.exists() {
            if let Err(e) = fs::write(path, DEFAULT_TOML) {
                tracing::warn!(
                    "Failed to write default config file: {:?}. Using defaults.",
                    e
                );
            }
            return Ok(self);
        }

        let content =
            fs::read_to_string(path).map_err(|e| WorkspaceError::Configuration(e.to_string()))?;
        if !content.trim().is_empty() {
            self.config = toml::from_str(&content)
                .map_err(|e| WorkspaceError::Configuration(e.to_string()))?;
        }
        Ok(self)
    }

    /// Merge in the `TERM_WS_*` environment variable layer.
    #[must_use]
    pub fn merge_env(self) -> Self {
        self.merge_env_from(&std::env::vars().collect::<Vec<_>>())
    }

    /// Merge environment overrides from an explicit `(key, value)` list.
    /// Split out from [`Self::merge_env`] so tests don't need to mutate
    /// process-global environment state.
    #[must_use]
    pub fn merge_env_from(mut self, vars: &[(String, String)]) -> Self {
        for (key, value) in vars {
            match key.as_str() {
                "TERM_WS_CORE_THEME" => self.config.core.theme = value.clone(),
                "TERM_WS_CORE_LOG_LEVEL" => self.config.core.log_level = value.clone(),
                "TERM_WS_CORE_REFRESH_RATE_MS" => {
                    if let Ok(n) = value.parse() {
                        self.config.core.refresh_rate_ms = n;
                    } else {
                        tracing::warn!("Ignoring invalid TERM_WS_CORE_REFRESH_RATE_MS={value}");
                    }
                }
                "TERM_WS_INTEGRATIONS_SLACK_ENABLED" => {
                    if let Ok(b) = value.parse() {
                        self.config.integrations.slack.enabled = b;
                    } else {
                        tracing::warn!(
                            "Ignoring invalid TERM_WS_INTEGRATIONS_SLACK_ENABLED={value}"
                        );
                    }
                }
                "TERM_WS_INTEGRATIONS_GITHUB_ENABLED" => {
                    if let Ok(b) = value.parse() {
                        self.config.integrations.github_enabled = b;
                    } else {
                        tracing::warn!(
                            "Ignoring invalid TERM_WS_INTEGRATIONS_GITHUB_ENABLED={value}"
                        );
                    }
                }
                _ => {}
            }
        }
        self
    }

    /// Merge in the CLI Option layer (highest precedence). Accepts a plain
    /// arg slice (e.g. `std::env::args().skip(1)`), recognizing `--theme`,
    /// `--log-level`, `--refresh-rate-ms`. `--config <path>` is recognized
    /// (and skipped) but has already been consumed by the caller to select
    /// which file `merge_file` read.
    #[must_use]
    pub fn merge_cli<S: AsRef<str>>(mut self, args: &[S]) -> Self {
        let mut i = 0;
        while i < args.len() {
            match args[i].as_ref() {
                "--theme" => {
                    if let Some(v) = args.get(i + 1) {
                        self.config.core.theme = v.as_ref().to_string();
                        i += 1;
                    }
                }
                "--log-level" => {
                    if let Some(v) = args.get(i + 1) {
                        self.config.core.log_level = v.as_ref().to_string();
                        i += 1;
                    }
                }
                "--refresh-rate-ms" => {
                    if let Some(v) = args.get(i + 1) {
                        if let Ok(n) = v.as_ref().parse() {
                            self.config.core.refresh_rate_ms = n;
                        } else {
                            tracing::warn!("Ignoring invalid --refresh-rate-ms {}", v.as_ref());
                        }
                        i += 1;
                    }
                }
                "--config" => {
                    // Already consumed by the caller to select the file layer's path.
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        self
    }

    /// Validate and produce the final `AppConfig`.
    pub fn build(self) -> Result<AppConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layer_alone_is_valid() {
        let config = ConfigBuilder::new()
            .build()
            .expect("defaults must validate");
        assert_eq!(config.core.theme, "default-dark");
        assert_eq!(config.core.refresh_rate_ms, 100);
        assert!(!config.integrations.slack.enabled);
        assert_eq!(config.integrations.slack.sync_interval_secs, 30);
    }

    #[test]
    fn env_layer_overrides_defaults() {
        let vars = vec![
            ("TERM_WS_CORE_THEME".to_string(), "solarized".to_string()),
            (
                "TERM_WS_INTEGRATIONS_SLACK_ENABLED".to_string(),
                "true".to_string(),
            ),
        ];
        let config = ConfigBuilder::new()
            .merge_env_from(&vars)
            .build()
            .expect("env overrides must validate");
        assert_eq!(config.core.theme, "solarized");
        assert!(config.integrations.slack.enabled);
        // Untouched fields keep their default.
        assert_eq!(config.core.log_level, "info");
    }

    #[test]
    fn cli_layer_overrides_env_layer() {
        let vars = vec![("TERM_WS_CORE_THEME".to_string(), "solarized".to_string())];
        let args = vec!["--theme".to_string(), "nord".to_string()];
        let config = ConfigBuilder::new()
            .merge_env_from(&vars)
            .merge_cli(&args)
            .build()
            .expect("cli overrides must validate");
        assert_eq!(config.core.theme, "nord");
    }

    #[test]
    fn invalid_env_refresh_rate_is_ignored_not_fatal() {
        let vars = vec![(
            "TERM_WS_CORE_REFRESH_RATE_MS".to_string(),
            "not-a-number".to_string(),
        )];
        let config = ConfigBuilder::new()
            .merge_env_from(&vars)
            .build()
            .expect("garbage input must be ignored, not panic");
        assert_eq!(config.core.refresh_rate_ms, 100);
    }

    #[test]
    fn validate_rejects_refresh_rate_below_16ms() {
        let args = vec!["--refresh-rate-ms".to_string(), "5".to_string()];
        let result = ConfigBuilder::new().merge_cli(&args).build();
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_string_returns_defaults() {
        let config = AppConfig::parse("").expect("empty input should fall back to defaults");
        assert_eq!(config.core.theme, "default-dark");
    }

    #[test]
    fn a_pre_phase_6_config_toml_still_parses_instead_of_crashing_on_startup() {
        // Reproduces a real bug report: a config.toml written before Phase 6
        // (flat `[integrations] slack_enabled = ...`, no `[integrations.slack]`
        // table at all) hard-failed AppConfig::load_or_create_default() with
        // "missing field `slack`", killing the app on every startup instead of
        // defaulting -- exactly the failure mode Zero Configuration promises
        // not to have (docs/05-operations/configuration.md's Phase 6 amendment).
        let old_toml = r#"
            [core]
            theme = "default-dark"
            refresh_rate_ms = 100
            log_level = "info"

            [integrations]
            slack_enabled = false
            github_enabled = false
        "#;
        let config = AppConfig::parse(old_toml).expect("an old config.toml must not crash startup");
        assert!(!config.integrations.slack.enabled);
        assert_eq!(config.integrations.slack.sync_interval_secs, 30);
        assert!(!config.integrations.github_enabled);
    }

    #[test]
    fn the_bootstrapped_default_toml_itself_parses_back_correctly() {
        // Guards against the DEFAULT_TOML constant (written by hand, not
        // generated from the struct) drifting out of sync with AppConfig's
        // actual shape -- a first-run user gets exactly this file written
        // to disk (docs/05-operations/configuration.md §4).
        let config = AppConfig::parse(DEFAULT_TOML).expect("DEFAULT_TOML must parse");
        assert!(!config.integrations.slack.enabled);
        assert_eq!(config.integrations.slack.sync_interval_secs, 30);
        assert!(config.integrations.slack.channel_ids.is_empty());
        assert!(config.integrations.slack.watched_user_ids.is_empty());
        assert!(!config.integrations.github_enabled);
    }

    #[test]
    fn parses_real_slack_channel_and_watch_list_config() {
        let toml = r#"
            [core]
            theme = "default-dark"
            refresh_rate_ms = 100
            log_level = "info"

            [integrations]
            github_enabled = false

            [integrations.slack]
            enabled = true
            sync_interval_secs = 45
            channel_ids = ["C0123456789"]
            watched_user_ids = ["U0123456789", "U0987654321"]
        "#;
        let config = AppConfig::parse(toml).expect("valid Slack config must parse");
        assert!(config.integrations.slack.enabled);
        assert_eq!(config.integrations.slack.sync_interval_secs, 45);
        assert_eq!(config.integrations.slack.channel_ids, vec!["C0123456789"]);
        assert_eq!(
            config.integrations.slack.watched_user_ids,
            vec!["U0123456789", "U0987654321"]
        );
    }

    #[test]
    fn save_to_then_parse_round_trips_a_picker_selection() {
        let dir = std::env::temp_dir().join(format!("tw_config_test_{}", uuid::Uuid::new_v4()));
        let path = dir.join("config.toml");

        let mut config = AppConfig::default();
        config.integrations.slack.enabled = true;
        config.integrations.slack.channel_ids = vec!["C123".to_string()];
        config.integrations.slack.watched_user_ids = vec!["U123".to_string(), "U456".to_string()];
        config.save_to(&path).expect("save_to must succeed");

        let reloaded_toml = std::fs::read_to_string(&path).unwrap();
        let reloaded = AppConfig::parse(&reloaded_toml).expect("saved config must parse back");

        assert!(reloaded.integrations.slack.enabled);
        assert_eq!(reloaded.integrations.slack.channel_ids, vec!["C123"]);
        assert_eq!(
            reloaded.integrations.slack.watched_user_ids,
            vec!["U123", "U456"]
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
