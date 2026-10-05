use std::collections::HashMap;

use serde::Deserialize;

/// Top-level config — composed of domain-specific sub-structs.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub active_provider: String,
    pub providers: HashMap<String, ProviderConfig>,
    /// Reasoning effort: `low` / `medium` / `high`.
    /// Defaults to `medium` if unset.
    #[serde(default = "default_effort")]
    pub effort: String,
    /// Active color theme. Top-level `theme = "<name>"` in `config.toml`.
    /// `None` = built-in default. Names a file in `~/.config/rem/themes/`.
    #[serde(default)]
    pub theme: ThemeConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    #[serde(rename = "openai-compatible")]
    OpenAICompatible,
    #[serde(rename = "anthropic")]
    Anthropic,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub kind: ProviderKind,
    pub key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub default_model: String,
}

fn default_effort() -> String {
    "medium".to_string()
}

/// Active-theme pointer. A transparent wrapper so the TOML stays a plain
/// top-level string (`theme = "gruvbox"`) while the Rust side keeps the
/// same sub-struct pattern as `ApiConfig` / `ModelConfig`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(transparent)]
pub struct ThemeConfig {
    pub name: Option<String>,
}

impl Config {
    /// Canonical path to the config file (`~/.config/rem/config.toml`).
    /// Single source of truth — every module resolves config paths from here.
    pub fn path() -> Result<std::path::PathBuf, String> {
        let config_dir =
            dirs::config_dir().ok_or("could not determine platform config directory")?;
        Ok(config_dir.join("rem").join("config.toml"))
    }

    /// Load config from `~/.config/rem/config.toml`.
    pub fn from_file() -> Result<Self, String> {
        let config_path = Self::path()?;

        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("failed to read {}: {e}", config_path.display()))?;

        Self::from_str(&content, &config_path.display().to_string())
    }

    /// Parse config text, rejecting the legacy single-profile shape with a
    /// migration hint instead of a bare serde error.
    fn from_str(content: &str, origin: &str) -> Result<Self, String> {
        let value: toml::Value =
            toml::from_str(content).map_err(|e| format!("failed to parse {origin}: {e}"))?;
        if value.get("api").is_some() || value.get("model").is_some() {
            return Err(format!(
                "failed to parse {origin}: found legacy [api]/[model] sections; rewrite to active_provider + [providers.*] with key = \"env:VAR\" (see config.example.toml)"
            ));
        }
        value
            .try_into()
            .map_err(|e: toml::de::Error| format!("failed to parse {origin}: {e}"))
    }

    /// Resolve the provider named `active_provider`: unknown name,
    /// missing default model, and key/env failures are all fatal with a
    /// message naming the provider.
    pub fn active(&self) -> Result<ResolvedProvider, String> {
        self.provider(&self.active_provider)
    }

    /// Resolve one named provider entry into a usable key + endpoint +
    /// model triple. Keys must use `env:VAR` indirection — literals are
    /// rejected so secrets never sit in the config file.
    pub fn provider(&self, name: &str) -> Result<ResolvedProvider, String> {
        let entry = self
            .providers
            .get(name)
            .ok_or_else(|| format!("unknown provider \"{name}\""))?;
        let key = resolve_key(name, &entry.key)?;
        let base_url = match entry.kind {
            ProviderKind::Anthropic => Some(
                entry
                    .base_url
                    .clone()
                    .unwrap_or_else(|| ANTHROPIC_DEFAULT_BASE_URL.to_string()),
            ),
            ProviderKind::OpenAICompatible => entry.base_url.clone(),
        };
        Ok(ResolvedProvider {
            name: name.to_string(),
            kind: entry.kind.clone(),
            key,
            base_url,
            model: entry.default_model.clone(),
        })
    }
}

/// Default Anthropic endpoint, used when the provider entry omits `base_url`.
const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// One resolved provider: secret key material plus endpoint + model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProvider {
    pub name: String,
    pub kind: ProviderKind,
    pub key: String,
    pub base_url: Option<String>,
    pub model: String,
}

/// Resolve a `key = "env:VAR"` reference into secret material.
/// Anything else (literal keys, empty names, unset vars) fails closed.
fn resolve_key(provider: &str, raw: &str) -> Result<String, String> {
    let var = raw.strip_prefix("env:").ok_or_else(|| {
        format!("provider \"{provider}\" key must use env:VAR indirection, got a literal")
    })?;
    if var.is_empty() {
        return Err(format!(
            "provider \"{provider}\" key has empty env: reference"
        ));
    }
    std::env::var(var).map_err(|_| format!("provider \"{provider}\": env {var} not set"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn developer_can_declare_multiple_named_providers() {
        let toml_str = r#"
active_provider = "anthropic"

[providers.openai]
type = "openai-compatible"
base_url = "https://api.openai.com/v1"
key = "env:OPENAI_API_KEY"
default_model = "gpt-4o"

[providers.anthropic]
type = "anthropic"
key = "env:ANTHROPIC_API_KEY"
default_model = "claude-sonnet-4-5"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.active_provider, "anthropic");
        assert_eq!(config.providers["openai"].default_model, "gpt-4o");
        assert_eq!(
            config.providers["anthropic"].default_model,
            "claude-sonnet-4-5"
        );
    }

    #[test]
    fn developer_config_with_env_key_indirection_parses() {
        let toml_str = r#"
active_provider = "openai"

[providers.openai]
type = "openai-compatible"
base_url = "https://api.example.com/v1"
key = "env:OPENAI_API_KEY"
default_model = "gpt-4o"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.providers["openai"].key, "env:OPENAI_API_KEY");
        assert_eq!(
            config.providers["openai"].base_url.as_deref(),
            Some("https://api.example.com/v1")
        );
    }

    #[test]
    fn effort_defaults_to_medium_when_missing() {
        let toml_str = r#"
active_provider = "openai"

[providers.openai]
type = "openai-compatible"
base_url = "https://api.example.com/v1"
key = "env:OPENAI_API_KEY"
default_model = "gpt-4o"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.effort, "medium");
    }

    #[test]
    fn theme_defaults_to_none_when_missing() {
        let toml_str = r#"
active_provider = "openai"

[providers.openai]
type = "openai-compatible"
base_url = "https://api.example.com/v1"
key = "env:OPENAI_API_KEY"
default_model = "gpt-4o"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.theme.name, None);
    }

    #[test]
    fn theme_parses_top_level_string() {
        let toml_str = r#"
theme = "gruvbox"
active_provider = "openai"

[providers.openai]
type = "openai-compatible"
base_url = "https://api.example.com/v1"
key = "env:OPENAI_API_KEY"
default_model = "gpt-4o"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.theme.name.as_deref(), Some("gruvbox"));
    }

    #[test]
    fn config_fails_on_missing_required_fields() {
        let toml_str = r#"
active_provider = "openai"
"#;
        let result: Result<Config, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    fn multi_provider_toml() -> &'static str {
        r#"
active_provider = "anthropic"

[providers.openai]
type = "openai-compatible"
base_url = "https://api.openai.com/v1"
key = "env:REM_TEST_OPENAI_KEY"
default_model = "gpt-4o"

[providers.anthropic]
type = "anthropic"
key = "env:REM_TEST_ANTHROPIC_KEY"
default_model = "claude-sonnet-4-5"
"#
    }

    #[test]
    fn missing_secret_env_fails_closed_naming_provider_and_var() {
        // SAFETY: test-only env mutation, sequential test runner.
        unsafe {
            std::env::remove_var("REM_TEST_OPENAI_KEY");
            std::env::remove_var("REM_TEST_ANTHROPIC_KEY");
        }
        let config: Config = toml::from_str(multi_provider_toml()).unwrap();
        let err = config.active().expect_err("must fail without env set");
        assert!(
            err.contains("anthropic") && err.contains("REM_TEST_ANTHROPIC_KEY"),
            "got: {err}"
        );
    }

    #[test]
    fn literal_key_is_rejected_so_secrets_stay_out_of_config() {
        let toml_str = r#"
active_provider = "openai"

[providers.openai]
type = "openai-compatible"
base_url = "https://api.example.com/v1"
key = "sk-literal-secret"
default_model = "gpt-4o"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let err = config.active().expect_err("literal keys must fail");
        assert!(err.contains("env:VAR"), "got: {err}");
    }

    #[test]
    fn legacy_single_profile_shape_is_rejected_with_migration_hint() {
        let toml_str = r#"
[api]
key = "sk-test"
base_url = "https://api.example.com/v1"

[model]
default = "gpt-4o"
"#;
        let err = Config::from_str(toml_str, "config.toml").expect_err("legacy must fail");
        assert!(err.contains("[providers.*]"), "got: {err}");
    }

    #[test]
    fn anthropic_base_url_defaults_to_public_endpoint() {
        // SAFETY: single-threaded test env under `cargo test` default runner;
        // sequential tests only.
        unsafe { std::env::set_var("REM_TEST_ANTHROPIC_KEY", "sk-ant-test") };
        let config: Config = toml::from_str(multi_provider_toml()).unwrap();
        let resolved = config.provider("anthropic").unwrap();
        assert_eq!(
            resolved.base_url.as_deref(),
            Some("https://api.anthropic.com")
        );
        unsafe { std::env::remove_var("REM_TEST_ANTHROPIC_KEY") };
    }
}
