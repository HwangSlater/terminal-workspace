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

/// Toggles enabling individual third-party messaging sync services.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationsToggle {
    /// Sync Slack connection.
    pub slack_enabled: bool,
    /// Sync GitHub updates.
    pub github_enabled: bool,
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
                slack_enabled: false,
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
        let path = cli_config_path_override(&args).unwrap_or_else(standard_config_path);

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

const DEFAULT_TOML: &str = r#"# Terminal Workspace Core Settings
[core]
theme = "default-dark"
refresh_rate_ms = 100
log_level = "info"

[integrations]
slack_enabled = false
github_enabled = false
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
                        self.config.integrations.slack_enabled = b;
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
        assert!(!config.integrations.slack_enabled);
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
        assert!(config.integrations.slack_enabled);
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
}
