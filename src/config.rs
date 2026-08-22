//! Configuration loading and defaults for minimax-cli.

use std::collections::HashMap;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::features::{Features, FeaturesToml, is_known_feature_key};
use crate::hooks::HooksConfig;

/// Built-in fallback text model used when config/env does not provide one.
pub const DEFAULT_TEXT_MODEL: &str = "MiniMax-M2.5";
/// Default provider name when none is configured.
pub const DEFAULT_PROVIDER_NAME: &str = "minimax";
/// Maximum supported concurrent subagents.
pub const MAX_SUBAGENTS: usize = 5;

/// Chat API protocol used by a configured LLM provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProviderApi {
    #[default]
    Anthropic,
    #[serde(alias = "openai-compat", alias = "openai_compatible")]
    OpenAi,
}

impl ProviderApi {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
        }
    }
}

/// One named LLM provider from config (`[providers.<name>]`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProviderEntry {
    pub api: Option<ProviderApi>,
    pub url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
}

/// Resolved active provider used to construct the text client.
#[derive(Debug, Clone)]
pub struct ActiveProvider {
    pub name: String,
    pub api: ProviderApi,
    pub url: String,
    pub api_key: String,
    pub default_model: String,
}

impl ActiveProvider {
    /// Messages endpoint for Anthropic-compatible APIs.
    #[must_use]
    pub fn anthropic_messages_url(&self) -> String {
        anthropic_messages_url(&self.url)
    }

    /// Chat completions endpoint for OpenAI-compatible APIs.
    #[must_use]
    pub fn openai_chat_completions_url(&self) -> String {
        openai_chat_completions_url(&self.url)
    }

    /// Models list endpoint for OpenAI-compatible APIs.
    #[must_use]
    pub fn openai_models_url(&self) -> String {
        openai_models_url(&self.url)
    }

    /// Models list endpoint for Anthropic-compatible APIs.
    #[must_use]
    pub fn anthropic_models_url(&self) -> String {
        anthropic_models_url(&self.url)
    }
}

// === Types ===

/// Raw retry configuration loaded from config files.
#[derive(Debug, Clone, Deserialize)]
pub struct RetryConfig {
    pub enabled: Option<bool>,
    pub max_retries: Option<u32>,
    pub initial_delay: Option<f64>,
    pub max_delay: Option<f64>,
    pub exponential_base: Option<f64>,
    pub request_timeout: Option<f64>,
    pub total_timeout: Option<f64>,
}

/// RLM configuration loaded from config files.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RlmConfig {
    pub max_context_chars: Option<usize>,
    pub max_search_results: Option<usize>,
    pub default_chunk_size: Option<usize>,
    pub default_overlap: Option<usize>,
    pub session_dir: Option<String>,
}

/// Duo configuration loaded from config files.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DuoConfig {
    pub max_turns: Option<u32>,
    pub approval_threshold: Option<f64>,
    pub default_max_tokens: Option<u32>,
    pub coach_temperature: Option<f32>,
    pub player_temperature: Option<f32>,
    pub coach_model: Option<String>,
    pub coach_base_url: Option<String>,
    pub coach_api_key: Option<String>,
    pub player_model: Option<String>,
    pub player_base_url: Option<String>,
    pub player_api_key: Option<String>,
}

/// Compaction configuration loaded from config files.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CompactionSettings {
    pub enabled: Option<bool>,
    pub token_threshold: Option<usize>,
    pub message_threshold: Option<usize>,
    pub keep_recent: Option<usize>,
    pub model: Option<String>,
    pub cache_summary: Option<bool>,
    pub compact_prompt: Option<String>,
    pub model_auto_compact_token_limit: Option<usize>,
}

/// Resolved retry policy with defaults applied.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub enabled: bool,
    pub max_retries: u32,
    pub initial_delay: f64,
    pub max_delay: f64,
    pub exponential_base: f64,
    /// Per-request timeout in seconds (0 = no timeout).
    pub request_timeout: f64,
    /// Total timeout across all retry attempts in seconds (0 = no total timeout).
    pub total_timeout: f64,
}

impl RetryPolicy {
    /// Compute the backoff delay for a retry attempt.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> std::time::Duration {
        let exponent = i32::try_from(attempt).unwrap_or(i32::MAX);
        let delay = self.initial_delay * self.exponential_base.powi(exponent);
        let delay = delay.min(self.max_delay);
        std::time::Duration::from_secs_f64(delay)
    }
}

/// Resolved CLI configuration, including defaults and environment overrides.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub default_text_model: Option<String>,
    pub default_image_model: Option<String>,
    pub default_video_model: Option<String>,
    pub default_audio_model: Option<String>,
    pub default_music_model: Option<String>,

    /// Active LLM provider name (key under `[providers]`).
    pub provider: Option<String>,
    /// Named LLM providers (`name` / `url` / `api_key` / `api`).
    pub providers: Option<HashMap<String, ProviderEntry>>,

    // === Coding API Configuration ===
    /// Second API key for MiniMax Coding API (optional, falls back to primary)
    pub api_key_2: Option<String>,
    /// Base URL for MiniMax Coding API (optional, falls back to primary)
    pub base_url_2: Option<String>,
    /// Default model for coding tasks
    pub default_coding_model: Option<String>,

    // === RLM Configuration ===
    pub rlm: Option<RlmConfig>,

    // === Duo Configuration ===
    pub duo: Option<DuoConfig>,

    // === Compaction Configuration ===
    pub compaction: Option<CompactionSettings>,

    // === Standard Configuration ===
    pub output_dir: Option<String>,
    pub tools_file: Option<String>,
    pub skills_dir: Option<String>,
    pub mcp_config_path: Option<String>,
    pub notes_path: Option<String>,
    pub memory_path: Option<String>,
    pub allow_shell: Option<bool>,
    pub max_subagents: Option<usize>,
    pub retry: Option<RetryConfig>,
    pub features: Option<FeaturesToml>,

    /// Lifecycle hooks configuration
    #[serde(default)]
    pub hooks: Option<HooksConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ConfigFile {
    #[serde(flatten)]
    base: Config,
    profiles: Option<HashMap<String, Config>>,
}

// === Config Loading ===

impl Config {
    /// Load configuration from disk and merge with environment overrides.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// # use crate::config::Config;
    /// let config = Config::load(None, None)?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn load(path: Option<PathBuf>, profile: Option<&str>) -> Result<Self> {
        let path = path.or_else(default_config_path);
        let mut config = if let Some(path) = path.as_ref() {
            if path.exists() {
                let contents = fs::read_to_string(path)
                    .with_context(|| format!("Failed to read config file: {}", path.display()))?;
                let parsed: ConfigFile = toml::from_str(&contents)
                    .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
                apply_profile(parsed, profile)?
            } else {
                Config::default()
            }
        } else {
            Config::default()
        };

        apply_env_overrides(&mut config);
        config.validate()?;
        Ok(config)
    }

    /// Validate that critical config fields are present.
    pub fn validate(&self) -> Result<()> {
        if let Some(ref key) = self.api_key
            && key.trim().is_empty()
        {
            anyhow::bail!("api_key cannot be empty string");
        }
        // If a non-default provider is selected, it must exist.
        if let Some(name) = self
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let names = self.provider_names();
            let known = names.iter().any(|n| n == name)
                || (name == DEFAULT_PROVIDER_NAME && self.providers.is_none());
            if !known {
                anyhow::bail!(
                    "Unknown provider '{name}'. Available providers: {}",
                    names.join(", ")
                );
            }
            // If the provider entry exists, ensure required fields when a key is expected.
            if let Some(entry) = self.providers.as_ref().and_then(|m| m.get(name))
                && let Err(err) = resolve_provider_entry(name, entry, self)
            {
                // Allow missing api_key during onboarding (no key anywhere yet).
                let has_any_key = self
                    .api_key
                    .as_ref()
                    .is_some_and(|k| !k.trim().is_empty())
                    || self.providers.as_ref().is_some_and(|m| {
                        m.values()
                            .any(|e| e.api_key.as_ref().is_some_and(|k| !k.trim().is_empty()))
                    });
                if has_any_key {
                    return Err(err);
                }
            }
        }
        if let Some(features) = &self.features {
            for key in features.entries.keys() {
                if !is_known_feature_key(key) {
                    anyhow::bail!("Unknown feature flag: {key}");
                }
            }
        }
        Ok(())
    }

    /// List configured provider names (includes implicit `minimax` when using legacy config).
    #[must_use]
    pub fn provider_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .providers
            .as_ref()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default();
        if names.is_empty() {
            names.push(DEFAULT_PROVIDER_NAME.to_string());
        } else if self.api_key.is_some() && !names.iter().any(|n| n == DEFAULT_PROVIDER_NAME) {
            // Legacy top-level key still usable as minimax when not shadowed.
            names.push(DEFAULT_PROVIDER_NAME.to_string());
        }
        names.sort();
        names.dedup();
        names
    }

    /// Active provider name (config / env), defaulting to `minimax`.
    #[must_use]
    pub fn active_provider_name(&self) -> String {
        self.provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_PROVIDER_NAME)
            .to_string()
    }

    /// Resolve the active LLM provider (name, API, URL, key, default model).
    pub fn active_provider(&self) -> Result<ActiveProvider> {
        let name = self.active_provider_name();
        if let Some(entry) = self.providers.as_ref().and_then(|m| m.get(&name)) {
            return resolve_provider_entry(&name, entry, self);
        }

        // Implicit / legacy MiniMax provider when `[providers]` is absent or name is minimax.
        if name == DEFAULT_PROVIDER_NAME {
            let api_key = self
                .api_key
                .clone()
                .filter(|k| !k.trim().is_empty())
                .context(
                    "Failed to load API key: set api_key / MINIMAX_API_KEY or configure [providers.minimax].",
                )?;
            let url = self
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.minimax.io".to_string());
            let default_model = self
                .default_text_model
                .clone()
                .unwrap_or_else(|| DEFAULT_TEXT_MODEL.to_string());
            return Ok(ActiveProvider {
                name,
                api: ProviderApi::Anthropic,
                url: normalize_base_url(&url),
                api_key,
                default_model,
            });
        }

        let available = self.provider_names().join(", ");
        anyhow::bail!("Unknown provider '{name}'. Available providers: {available}")
    }

    /// Select an active provider by name (mutates `provider` field).
    pub fn set_active_provider(&mut self, name: &str) -> Result<ActiveProvider> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Provider name cannot be empty");
        }
        self.provider = Some(trimmed.to_string());
        self.active_provider()
    }

    /// Return the `MiniMax` base URL (normalized).
    #[must_use]
    pub fn minimax_base_url(&self) -> String {
        let base = self
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.minimax.io".to_string());
        normalize_base_url(&base)
    }

    /// Return the MiniMax text API base URL (normalized).
    ///
    /// MiniMax text chat currently uses the `/anthropic` compatibility route.
    #[allow(dead_code)]
    #[must_use]
    pub fn minimax_text_base_url(&self) -> String {
        // Prefer active anthropic provider URL when it is MiniMax.
        if let Ok(active) = self.active_provider()
            && active.api == ProviderApi::Anthropic
        {
            let root = normalize_base_url(&active.url);
            if is_minimax_host(&root) {
                return format!("{}/anthropic", root.trim_end_matches('/'));
            }
            return root;
        }
        let root = normalize_base_url(
            &self
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.minimax.io".to_string()),
        );
        format!("{}/anthropic", root.trim_end_matches('/'))
    }

    // === Coding API Methods ===

    /// Return the MiniMax Coding API base URL (normalized).
    #[must_use]
    pub fn coding_base_url(&self) -> String {
        let base = self
            .base_url_2
            .clone()
            .unwrap_or_else(|| "https://api.minimax.io".to_string());
        normalize_base_url(&base)
    }

    /// Read the MiniMax Coding API key from config/environment.
    pub fn coding_api_key(&self) -> Result<String> {
        // Try api_key_2 first, fall back to primary api_key
        if let Some(ref key) = self.api_key_2
            && !key.trim().is_empty()
        {
            return Ok(key.clone());
        }
        // Fall back to primary API key
        self.api_key
            .clone()
            .context("Failed to load MiniMax API key: MINIMAX_API_KEY or MINIMAX_API_KEY_2 missing. Set it in config.toml or environment.")
    }

    /// Return the default coding model, or fall back to text model if not set.
    #[must_use]
    pub fn coding_model(&self) -> String {
        self.default_coding_model
            .clone()
            .or_else(|| self.default_text_model.clone())
            .unwrap_or_else(|| "MiniMax-M2.5-Coding".to_string())
    }

    /// Check if coding API is configured with a separate key.
    #[allow(dead_code)]
    #[must_use]
    pub fn has_separate_coding_api_key(&self) -> bool {
        self.api_key_2.is_some()
            && self
                .api_key_2
                .as_ref()
                .is_some_and(|k| !k.trim().is_empty())
    }

    /// Get the appropriate API key and base URL for a given mode (text or coding).
    #[allow(dead_code)]
    pub fn api_for_mode(&self, is_coding: bool) -> (String, String) {
        if is_coding {
            (
                self.coding_api_key()
                    .unwrap_or_else(|_| self.api_key.clone().unwrap_or_default()),
                self.coding_base_url(),
            )
        } else {
            (
                self.minimax_api_key().unwrap_or_default(),
                self.minimax_base_url(),
            )
        }
    }

    /// Read the `MiniMax` API key from config/environment.
    pub fn minimax_api_key(&self) -> Result<String> {
        self.api_key
            .clone()
            .context(
                "Failed to load MiniMax API key: MINIMAX_API_KEY missing. Set it in config.toml or environment.",
            )
    }

    #[allow(dead_code)]
    pub fn minimax_text_api_key(&self) -> Result<String> {
        if let Ok(active) = self.active_provider() {
            return Ok(active.api_key);
        }
        self.minimax_api_key()
    }

    /// Default text model for the active provider (falls back to legacy / built-in).
    #[must_use]
    pub fn resolved_default_text_model(&self) -> String {
        self.active_provider()
            .map(|p| p.default_model)
            .ok()
            .or_else(|| self.default_text_model.clone())
            .unwrap_or_else(|| DEFAULT_TEXT_MODEL.to_string())
    }

    /// Return whether auto-compaction should be enabled from config (if explicitly set).
    #[must_use]
    pub fn auto_compact_enabled(&self) -> Option<bool> {
        self.compaction.as_ref().and_then(|cfg| cfg.enabled)
    }

    /// Resolve compaction configuration with defaults, optionally deriving a model-aware token limit.
    #[must_use]
    pub fn compaction_config_for_model(
        &self,
        active_model: &str,
    ) -> crate::compaction::CompactionConfig {
        let mut cfg = crate::compaction::CompactionConfig::default();

        let Some(compaction) = &self.compaction else {
            return cfg;
        };

        if let Some(enabled) = compaction.enabled {
            cfg.enabled = enabled;
        }
        if let Some(message_threshold) = compaction.message_threshold {
            cfg.message_threshold = message_threshold.max(1);
        }
        if let Some(keep_recent) = compaction.keep_recent {
            cfg.keep_recent = keep_recent.max(1);
        }
        if let Some(cache_summary) = compaction.cache_summary {
            cfg.cache_summary = cache_summary;
        }
        if let Some(model) = compaction
            .model
            .as_ref()
            .map(|m| m.trim())
            .filter(|m| !m.is_empty())
        {
            cfg.model = Some(model.to_string());
        }
        if let Some(prompt) = compaction
            .compact_prompt
            .as_ref()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
        {
            cfg.compact_prompt = Some(prompt.to_string());
        }

        let model_for_limit = cfg.model.as_deref().unwrap_or(active_model);
        let has_explicit_limit = compaction.token_threshold.is_some()
            || compaction.model_auto_compact_token_limit.is_some();
        cfg.derive_token_threshold_from_model = !has_explicit_limit;
        let token_threshold = compaction
            .token_threshold
            .or(compaction.model_auto_compact_token_limit)
            .or_else(|| {
                crate::models::context_window_for_model(model_for_limit)
                    .map(|window| ((u64::from(window) * 9) / 10) as usize)
            });
        if let Some(token_threshold) = token_threshold {
            cfg.token_threshold = token_threshold.max(1);
        }

        cfg
    }

    /// Resolve enabled features from defaults and config entries.
    #[must_use]
    pub fn features(&self) -> Features {
        let mut features = Features::with_defaults();
        if let Some(table) = &self.features {
            features.apply_map(&table.entries);
        }
        features
    }

    pub fn set_feature(&mut self, key: &str, enabled: bool) -> Result<()> {
        if !is_known_feature_key(key) {
            anyhow::bail!("Unknown feature flag: {key}");
        }
        let table = self.features.get_or_insert_with(FeaturesToml::default);
        table.entries.insert(key.to_string(), enabled);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn output_dir(&self) -> PathBuf {
        self.output_dir
            .as_deref()
            .map(expand_path)
            .unwrap_or_else(|| PathBuf::from("./outputs"))
    }

    /// Resolve the skills directory path.
    #[must_use]
    pub fn skills_dir(&self) -> PathBuf {
        self.skills_dir
            .as_deref()
            .map(expand_path)
            .or_else(default_skills_dir)
            .unwrap_or_else(|| PathBuf::from("./skills"))
    }

    /// Resolve the MCP config path.
    #[must_use]
    pub fn mcp_config_path(&self) -> PathBuf {
        self.mcp_config_path
            .as_deref()
            .map(expand_path)
            .or_else(default_mcp_config_path)
            .unwrap_or_else(|| PathBuf::from("./mcp.json"))
    }

    /// Resolve the notes file path.
    #[must_use]
    pub fn notes_path(&self) -> PathBuf {
        self.notes_path
            .as_deref()
            .map(expand_path)
            .or_else(default_notes_path)
            .unwrap_or_else(|| PathBuf::from("./notes.txt"))
    }

    /// Resolve the memory file path.
    #[must_use]
    pub fn memory_path(&self) -> PathBuf {
        self.memory_path
            .as_deref()
            .map(expand_path)
            .or_else(default_memory_path)
            .unwrap_or_else(|| PathBuf::from("./memory.md"))
    }

    /// Return whether shell execution is allowed.
    #[must_use]
    pub fn allow_shell(&self) -> bool {
        self.allow_shell.unwrap_or(false)
    }

    /// Return the maximum number of concurrent sub-agents.
    #[must_use]
    pub fn max_subagents(&self) -> usize {
        self.max_subagents.unwrap_or(5).clamp(1, 5)
    }

    // === RLM Configuration Methods ===

    /// Resolve the effective RLM configuration with defaults applied.
    #[allow(dead_code)]
    #[must_use]
    pub fn rlm_config(&self) -> RlmConfig {
        let defaults = RlmConfig {
            max_context_chars: Some(10_000_000),
            max_search_results: Some(100),
            default_chunk_size: Some(2_000),
            default_overlap: Some(200),
            session_dir: Some("~/.minimax/rlm".to_string()),
        };

        let Some(cfg) = &self.rlm else {
            return defaults;
        };

        RlmConfig {
            max_context_chars: cfg.max_context_chars.or(defaults.max_context_chars),
            max_search_results: cfg.max_search_results.or(defaults.max_search_results),
            default_chunk_size: cfg.default_chunk_size.or(defaults.default_chunk_size),
            default_overlap: cfg.default_overlap.or(defaults.default_overlap),
            session_dir: cfg.session_dir.clone().or(defaults.session_dir),
        }
    }

    /// Get the RLM session directory path.
    #[allow(dead_code)]
    #[must_use]
    pub fn rlm_session_dir(&self) -> PathBuf {
        self.rlm_config()
            .session_dir
            .as_deref()
            .map(expand_path)
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .map(|home| home.join(".minimax").join("rlm"))
                    .unwrap_or_else(|| PathBuf::from(".minimax/rlm"))
            })
    }

    /// Get the maximum context characters for RLM.
    #[allow(dead_code)]
    #[must_use]
    pub fn rlm_max_context_chars(&self) -> usize {
        self.rlm_config().max_context_chars.unwrap_or(10_000_000)
    }

    /// Get the maximum search results for RLM.
    #[allow(dead_code)]
    #[must_use]
    pub fn rlm_max_search_results(&self) -> usize {
        self.rlm_config().max_search_results.unwrap_or(100)
    }

    // === Duo Configuration Methods ===

    /// Resolve the effective Duo configuration with defaults applied.
    #[allow(dead_code)]
    #[must_use]
    pub fn duo_config(&self) -> DuoConfig {
        let defaults = DuoConfig {
            max_turns: Some(10),
            approval_threshold: Some(0.9),
            default_max_tokens: Some(8192),
            coach_temperature: Some(0.3),
            player_temperature: Some(0.7),
            coach_model: None,
            coach_base_url: None,
            coach_api_key: None,
            player_model: None,
            player_base_url: None,
            player_api_key: None,
        };

        let Some(cfg) = &self.duo else {
            return defaults;
        };

        DuoConfig {
            max_turns: cfg.max_turns.or(defaults.max_turns),
            approval_threshold: cfg.approval_threshold.or(defaults.approval_threshold),
            default_max_tokens: cfg.default_max_tokens.or(defaults.default_max_tokens),
            coach_temperature: cfg.coach_temperature.or(defaults.coach_temperature),
            player_temperature: cfg.player_temperature.or(defaults.player_temperature),
            coach_model: cfg.coach_model.clone(),
            coach_base_url: cfg.coach_base_url.clone(),
            coach_api_key: cfg.coach_api_key.clone(),
            player_model: cfg.player_model.clone(),
            player_base_url: cfg.player_base_url.clone(),
            player_api_key: cfg.player_api_key.clone(),
        }
    }

    /// Get the maximum turns for Duo mode.
    #[allow(dead_code)]
    #[must_use]
    pub fn duo_max_turns(&self) -> u32 {
        self.duo_config().max_turns.unwrap_or(10)
    }

    /// Get the approval threshold for Duo mode.
    #[allow(dead_code)]
    #[must_use]
    pub fn duo_approval_threshold(&self) -> f64 {
        self.duo_config().approval_threshold.unwrap_or(0.9)
    }

    /// Get the default max tokens for Duo requests.
    #[allow(dead_code)]
    #[must_use]
    pub fn duo_default_max_tokens(&self) -> u32 {
        self.duo_config().default_max_tokens.unwrap_or(8192)
    }

    /// Get the coach temperature for Duo validation.
    #[allow(dead_code)]
    #[must_use]
    pub fn duo_coach_temperature(&self) -> f32 {
        self.duo_config().coach_temperature.unwrap_or(0.3)
    }

    /// Get the player temperature for Duo implementation.
    #[allow(dead_code)]
    #[must_use]
    pub fn duo_player_temperature(&self) -> f32 {
        self.duo_config().player_temperature.unwrap_or(0.7)
    }

    /// Get hooks configuration, returning default if not configured.
    pub fn hooks_config(&self) -> HooksConfig {
        self.hooks.clone().unwrap_or_default()
    }

    /// Resolve the effective retry policy with defaults applied.
    #[must_use]
    pub fn retry_policy(&self) -> RetryPolicy {
        let defaults = RetryPolicy {
            enabled: true,
            max_retries: 3,
            initial_delay: 1.0,
            max_delay: 60.0,
            exponential_base: 2.0,
            request_timeout: 0.0,
            total_timeout: 0.0,
        };

        let Some(cfg) = &self.retry else {
            return defaults;
        };

        RetryPolicy {
            enabled: cfg.enabled.unwrap_or(defaults.enabled),
            max_retries: cfg.max_retries.unwrap_or(defaults.max_retries),
            initial_delay: cfg.initial_delay.unwrap_or(defaults.initial_delay),
            max_delay: cfg.max_delay.unwrap_or(defaults.max_delay),
            exponential_base: cfg.exponential_base.unwrap_or(defaults.exponential_base),
            request_timeout: cfg.request_timeout.unwrap_or(defaults.request_timeout),
            total_timeout: cfg.total_timeout.unwrap_or(defaults.total_timeout),
        }
    }
}

// === Defaults ===

fn default_config_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MINIMAX_CONFIG_PATH")
        && !path.trim().is_empty()
    {
        return Some(PathBuf::from(path));
    }
    dirs::home_dir().map(|home| home.join(".minimax").join("config.toml"))
}

fn expand_path(path: &str) -> PathBuf {
    let expanded = shellexpand::tilde(path);
    PathBuf::from(expanded.as_ref())
}

fn default_skills_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".minimax").join("skills"))
}

fn default_mcp_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".minimax").join("mcp.json"))
}

fn default_notes_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".minimax").join("notes.txt"))
}

fn default_memory_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".minimax").join("memory.md"))
}

// === Environment Overrides ===

fn apply_env_overrides(config: &mut Config) {
    if let Ok(value) = std::env::var("MINIMAX_API_KEY") {
        config.api_key = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_API_KEY_2") {
        config.api_key_2 = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_BASE_URL") {
        config.base_url = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_BASE_URL_2") {
        config.base_url_2 = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_PROVIDER") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            config.provider = Some(trimmed.to_string());
        }
    }
    if let Ok(value) = std::env::var("MINIMAX_DEFAULT_CODING_MODEL") {
        config.default_coding_model = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_OUTPUT_DIR") {
        config.output_dir = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_SKILLS_DIR") {
        config.skills_dir = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_MCP_CONFIG") {
        config.mcp_config_path = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_NOTES_PATH") {
        config.notes_path = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_MEMORY_PATH") {
        config.memory_path = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_ALLOW_SHELL") {
        config.allow_shell = Some(value == "1" || value.eq_ignore_ascii_case("true"));
    }
    if let Ok(value) = std::env::var("MINIMAX_MAX_SUBAGENTS")
        && let Ok(parsed) = value.parse::<usize>()
    {
        config.max_subagents = Some(parsed.clamp(1, 5));
    }
    if let Ok(value) = std::env::var("MINIMAX_AUTO_COMPACT") {
        let enabled = value == "1" || value.eq_ignore_ascii_case("true");
        let compaction = config
            .compaction
            .get_or_insert_with(CompactionSettings::default);
        compaction.enabled = Some(enabled);
    }
    if let Ok(value) = std::env::var("MINIMAX_COMPACTION_TOKEN_THRESHOLD")
        && let Ok(parsed) = value.parse::<usize>()
    {
        let compaction = config
            .compaction
            .get_or_insert_with(CompactionSettings::default);
        compaction.token_threshold = Some(parsed.max(1));
    }
    if let Ok(value) = std::env::var("MINIMAX_COMPACTION_MESSAGE_THRESHOLD")
        && let Ok(parsed) = value.parse::<usize>()
    {
        let compaction = config
            .compaction
            .get_or_insert_with(CompactionSettings::default);
        compaction.message_threshold = Some(parsed.max(1));
    }
    if let Ok(value) = std::env::var("MINIMAX_COMPACTION_KEEP_RECENT")
        && let Ok(parsed) = value.parse::<usize>()
    {
        let compaction = config
            .compaction
            .get_or_insert_with(CompactionSettings::default);
        compaction.keep_recent = Some(parsed.max(1));
    }
    if let Ok(value) = std::env::var("MINIMAX_COMPACT_PROMPT")
        && !value.trim().is_empty()
    {
        let compaction = config
            .compaction
            .get_or_insert_with(CompactionSettings::default);
        compaction.compact_prompt = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_AUTO_COMPACT_TOKEN_LIMIT")
        && let Ok(parsed) = value.parse::<usize>()
    {
        let compaction = config
            .compaction
            .get_or_insert_with(CompactionSettings::default);
        compaction.model_auto_compact_token_limit = Some(parsed.max(1));
    }
    if let Ok(value) = std::env::var("MINIMAX_COACH_MODEL") {
        let duo = config.duo.get_or_insert_with(DuoConfig::default);
        duo.coach_model = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_COACH_BASE_URL") {
        let duo = config.duo.get_or_insert_with(DuoConfig::default);
        duo.coach_base_url = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_COACH_API_KEY") {
        let duo = config.duo.get_or_insert_with(DuoConfig::default);
        duo.coach_api_key = Some(value);
    } else if let Ok(value) = std::env::var("ZAI_API_KEY") {
        let duo = config.duo.get_or_insert_with(DuoConfig::default);
        duo.coach_api_key = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_PLAYER_MODEL") {
        let duo = config.duo.get_or_insert_with(DuoConfig::default);
        duo.player_model = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_PLAYER_BASE_URL") {
        let duo = config.duo.get_or_insert_with(DuoConfig::default);
        duo.player_base_url = Some(value);
    }
    if let Ok(value) = std::env::var("MINIMAX_PLAYER_API_KEY") {
        let duo = config.duo.get_or_insert_with(DuoConfig::default);
        duo.player_api_key = Some(value);
    } else if let Ok(value) = std::env::var("ZAI_API_KEY") {
        let duo = config.duo.get_or_insert_with(DuoConfig::default);
        duo.player_api_key = Some(value);
    }
}

fn normalize_base_url(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if is_minimax_host(trimmed) {
        return trimmed
            .trim_end_matches("/anthropic")
            .trim_end_matches("/v1")
            .to_string();
    }
    trimmed.to_string()
}

pub(crate) fn is_minimax_host(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("api.minimax.io") || lower.contains("api.minimaxi.com")
}

/// Build Anthropic Messages URL from a provider base URL.
#[must_use]
pub fn anthropic_messages_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let lower = trimmed.to_lowercase();
    if lower.ends_with("/v1/messages") {
        return trimmed.to_string();
    }
    if lower.contains("/anthropic") {
        if lower.ends_with("/v1") {
            return format!("{trimmed}/messages");
        }
        return format!("{trimmed}/v1/messages");
    }
    if is_minimax_host(trimmed) {
        let root = normalize_base_url(trimmed);
        return format!("{}/anthropic/v1/messages", root.trim_end_matches('/'));
    }
    if lower.ends_with("/v1") {
        return format!("{trimmed}/messages");
    }
    format!("{trimmed}/v1/messages")
}

/// Build OpenAI Chat Completions URL from a provider base URL.
#[must_use]
pub fn openai_chat_completions_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let lower = trimmed.to_lowercase();
    if lower.ends_with("/chat/completions") {
        return trimmed.to_string();
    }
    if lower.ends_with("/v1") {
        return format!("{trimmed}/chat/completions");
    }
    format!("{trimmed}/v1/chat/completions")
}

/// Build the models list URL for OpenAI-compatible APIs from a provider base URL.
#[must_use]
pub fn openai_models_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let lower = trimmed.to_lowercase();
    let base = if lower.ends_with("/chat/completions") {
        trimmed.trim_end_matches("/chat/completions").to_string()
    } else {
        trimmed.to_string()
    };
    if base.to_lowercase().ends_with("/v1") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

/// Build the models list URL for Anthropic-compatible APIs from a provider base URL.
#[must_use]
pub fn anthropic_models_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let lower = trimmed.to_lowercase();
    if lower.ends_with("/v1/models") {
        return trimmed.to_string();
    }
    if lower.contains("/anthropic") {
        if lower.ends_with("/v1") {
            return format!("{trimmed}/models");
        }
        return format!("{trimmed}/v1/models");
    }
    if is_minimax_host(trimmed) {
        let root = normalize_base_url(trimmed);
        return format!("{}/anthropic/v1/models", root.trim_end_matches('/'));
    }
    if lower.ends_with("/v1") {
        return format!("{trimmed}/models");
    }
    format!("{trimmed}/v1/models")
}

fn resolve_provider_entry(
    name: &str,
    entry: &ProviderEntry,
    config: &Config,
) -> Result<ActiveProvider> {
    let api = entry.api.unwrap_or(if name == DEFAULT_PROVIDER_NAME {
        ProviderApi::Anthropic
    } else {
        // Non-minimax providers default to openai-compatible unless specified.
        ProviderApi::OpenAi
    });

    let api_key = entry
        .api_key
        .clone()
        .filter(|k| !k.trim().is_empty())
        .or_else(|| {
            if name == DEFAULT_PROVIDER_NAME {
                config.api_key.clone().filter(|k| !k.trim().is_empty())
            } else {
                None
            }
        })
        .with_context(|| {
            format!("Provider '{name}' is missing api_key. Set it under [providers.{name}].")
        })?;

    let url = entry
        .url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .or_else(|| {
            if name == DEFAULT_PROVIDER_NAME {
                config.base_url.clone()
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            if name == DEFAULT_PROVIDER_NAME {
                "https://api.minimax.io".to_string()
            } else {
                String::new()
            }
        });
    if url.trim().is_empty() {
        anyhow::bail!("Provider '{name}' is missing url. Set it under [providers.{name}].");
    }

    let default_model = entry
        .default_model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .or_else(|| {
            if name == DEFAULT_PROVIDER_NAME {
                config.default_text_model.clone()
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            if name == DEFAULT_PROVIDER_NAME {
                DEFAULT_TEXT_MODEL.to_string()
            } else {
                "gpt-4o".to_string()
            }
        });

    let url = if api == ProviderApi::Anthropic && is_minimax_host(&url) {
        normalize_base_url(&url)
    } else {
        url.trim().trim_end_matches('/').to_string()
    };

    Ok(ActiveProvider {
        name: name.to_string(),
        api,
        url,
        api_key,
        default_model,
    })
}

fn apply_profile(config: ConfigFile, profile: Option<&str>) -> Result<Config> {
    if let Some(profile_name) = profile {
        let profiles = config.profiles.as_ref();
        match profiles.and_then(|profiles| profiles.get(profile_name)) {
            Some(override_cfg) => Ok(merge_config(config.base, override_cfg.clone())),
            None => {
                let available = profiles
                    .map(|profiles| {
                        let mut keys = profiles.keys().cloned().collect::<Vec<_>>();
                        keys.sort();
                        if keys.is_empty() {
                            "none".to_string()
                        } else {
                            keys.join(", ")
                        }
                    })
                    .unwrap_or_else(|| "none".to_string());
                anyhow::bail!(
                    "Profile '{}' not found. Available profiles: {}",
                    profile_name,
                    available
                )
            }
        }
    } else {
        Ok(config.base)
    }
}

fn merge_config(base: Config, override_cfg: Config) -> Config {
    let providers = match (base.providers, override_cfg.providers) {
        (Some(mut base_map), Some(over_map)) => {
            for (k, v) in over_map {
                base_map.insert(k, v);
            }
            Some(base_map)
        }
        (None, Some(over_map)) => Some(over_map),
        (Some(base_map), None) => Some(base_map),
        (None, None) => None,
    };

    Config {
        api_key: override_cfg.api_key.or(base.api_key),
        base_url: override_cfg.base_url.or(base.base_url),
        default_text_model: override_cfg.default_text_model.or(base.default_text_model),
        default_image_model: override_cfg
            .default_image_model
            .or(base.default_image_model),
        default_video_model: override_cfg
            .default_video_model
            .or(base.default_video_model),
        default_audio_model: override_cfg
            .default_audio_model
            .or(base.default_audio_model),
        default_music_model: override_cfg
            .default_music_model
            .or(base.default_music_model),

        provider: override_cfg.provider.or(base.provider),
        providers,

        // Coding API configuration
        api_key_2: override_cfg.api_key_2.or(base.api_key_2),
        base_url_2: override_cfg.base_url_2.or(base.base_url_2),
        default_coding_model: override_cfg
            .default_coding_model
            .or(base.default_coding_model),

        // RLM configuration
        rlm: override_cfg.rlm.or(base.rlm),

        // Duo configuration
        duo: match (override_cfg.duo, base.duo) {
            (Some(o), Some(b)) => Some(DuoConfig {
                max_turns: o.max_turns.or(b.max_turns),
                approval_threshold: o.approval_threshold.or(b.approval_threshold),
                default_max_tokens: o.default_max_tokens.or(b.default_max_tokens),
                coach_temperature: o.coach_temperature.or(b.coach_temperature),
                player_temperature: o.player_temperature.or(b.player_temperature),
                coach_model: o.coach_model.or(b.coach_model),
                coach_base_url: o.coach_base_url.or(b.coach_base_url),
                coach_api_key: o.coach_api_key.or(b.coach_api_key),
                player_model: o.player_model.or(b.player_model),
                player_base_url: o.player_base_url.or(b.player_base_url),
                player_api_key: o.player_api_key.or(b.player_api_key),
            }),
            (Some(o), None) => Some(o),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        },
        compaction: override_cfg.compaction.or(base.compaction),

        // Standard configuration
        output_dir: override_cfg.output_dir.or(base.output_dir),
        tools_file: override_cfg.tools_file.or(base.tools_file),
        skills_dir: override_cfg.skills_dir.or(base.skills_dir),
        mcp_config_path: override_cfg.mcp_config_path.or(base.mcp_config_path),
        notes_path: override_cfg.notes_path.or(base.notes_path),
        memory_path: override_cfg.memory_path.or(base.memory_path),
        allow_shell: override_cfg.allow_shell.or(base.allow_shell),
        max_subagents: override_cfg.max_subagents.or(base.max_subagents),
        retry: override_cfg.retry.or(base.retry),
        features: override_cfg.features.or(base.features),
        hooks: override_cfg.hooks.or(base.hooks),
    }
}

pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }
    Ok(())
}

/// Save an API key to the config file. Creates the file if it doesn't exist.
pub fn save_api_key(api_key: &str) -> Result<PathBuf> {

    let config_path = default_config_path()
        .context("Failed to resolve config path: home directory not found.")?;

    ensure_parent_dir(&config_path)?;

    let content = if config_path.exists() {
        // Read existing config and update the api_key line
        let existing = fs::read_to_string(&config_path)?;
        if existing.contains("api_key") {
            // Replace existing api_key line
            let mut result = String::new();
            for line in existing.lines() {
                if is_api_key_assignment(line) {
                    let _ = writeln!(result, "api_key = \"{api_key}\"");
                } else {
                    result.push_str(line);
                    result.push('\n');
                }
            }
            result
        } else {
            // Prepend api_key to existing config
            format!("api_key = \"{api_key}\"\n{existing}")
        }
    } else {
        // Create new minimal config
        format!(
            r#"# MiniMax CLI Configuration
# Get your API key from https://platform.minimax.io

api_key = "{api_key}"

# Base URL (default: https://api.minimax.io)
# base_url = "https://api.minimax.io"

# Default model
default_text_model = "MiniMax-M2.5"
"#
        )
    };

    fs::write(&config_path, content)
        .with_context(|| format!("Failed to write config to {}", config_path.display()))?;

    Ok(config_path)
}

/// True when a line assigns to `api_key` (not `api_key_2` or similar).
fn is_api_key_assignment(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("api_key")
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Escape a value for a TOML basic string.
fn escape_toml_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Save a provider API key under `[providers.<name>]` in the config file.
///
/// The default `minimax` provider maps to the top-level `api_key` key
/// (legacy form). Creates the config file, the `[providers]` table, or the
/// provider entry as needed, and preserves existing formatting and comments.
pub fn save_provider_api_key(provider: &str, api_key: &str) -> Result<PathBuf> {
    let name = provider.trim();
    anyhow::ensure!(!name.is_empty(), "Provider name cannot be empty");
    anyhow::ensure!(
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "Invalid provider name '{name}'. Use letters, digits, '-', or '_'."
    );
    if name == DEFAULT_PROVIDER_NAME {
        return save_api_key(api_key.trim());
    }

    let config_path = default_config_path()
        .context("Failed to resolve config path: home directory not found.")?;
    ensure_parent_dir(&config_path)?;

    let existing = if config_path.exists() {
        fs::read_to_string(&config_path)?
    } else {
        String::new()
    };

    let section = format!("[providers.{name}]");
    let assignment = format!("api_key = \"{}\"", escape_toml_string(api_key.trim()));

    // Locate the provider section and any existing api_key line inside it.
    let mut section_start: Option<usize> = None;
    let mut key_line: Option<usize> = None;
    let mut in_section = false;
    for (index, line) in existing.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == section;
            if in_section && section_start.is_none() {
                section_start = Some(index);
            }
            continue;
        }
        if in_section && key_line.is_none() && is_api_key_assignment(trimmed) {
            key_line = Some(index);
        }
    }

    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();
    match (key_line, section_start) {
        (Some(index), _) => lines[index] = assignment,
        (None, Some(header)) => lines.insert(header + 1, assignment),
        (None, None) => {
            // No section yet: append a fresh one, separated by a blank line.
            if !lines.is_empty() && !lines.last().is_some_and(String::is_empty) {
                lines.push(String::new());
            }
            lines.push(section);
            lines.push(assignment);
        }
    }

    let mut content = lines.join("\n");
    content.push('\n');
    fs::write(&config_path, content)
        .with_context(|| format!("Failed to write config to {}", config_path.display()))?;

    Ok(config_path)
}

fn is_provider_field_assignment(line: &str, field: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with(field) {
        return false;
    }
    let rest = trimmed[field.len()..].trim_start();
    rest.starts_with('=')
}

/// Save or update a provider entry under `[providers.<name>]` in the config file.
///
/// Creates the config file or provider section as needed, writes `api`, `url`,
/// and optional `api_key` and `default_model` entries, and preserves existing
/// formatting and comments.
pub fn save_provider(
    name: &str,
    api: ProviderApi,
    url: &str,
    api_key: &str,
    default_model: &str,
) -> Result<PathBuf> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "Provider name cannot be empty");
    anyhow::ensure!(
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "Invalid provider name '{name}'. Use letters, digits, '-', or '_'."
    );
    if name == DEFAULT_PROVIDER_NAME {
        anyhow::bail!("Cannot add default provider 'minimax' via /add-provider. Use /login or configure top-level api_key.");
    }

    let config_path = default_config_path()
        .context("Failed to resolve config path: home directory not found.")?;
    ensure_parent_dir(&config_path)?;

    let existing = if config_path.exists() {
        fs::read_to_string(&config_path)?
    } else {
        String::new()
    };

    let section = format!("[providers.{name}]");

    // Find the range of the existing section if present.
    let mut section_start: Option<usize> = None;
    let mut section_end: Option<usize> = None;
    let mut in_section = false;

    let raw_lines: Vec<String> = existing.lines().map(str::to_string).collect();
    for (index, line) in raw_lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_section {
                section_end = Some(index);
                break;
            }
            if trimmed == section {
                in_section = true;
                section_start = Some(index);
            }
        }
    }
    if in_section && section_end.is_none() {
        section_end = Some(raw_lines.len());
    }

    let mut lines = raw_lines;

    let desired_api = format!("api = \"{}\"", api.as_str());
    let desired_url = format!("url = \"{}\"", escape_toml_string(url.trim()));
    let desired_api_key = if !api_key.trim().is_empty() {
        Some(format!("api_key = \"{}\"", escape_toml_string(api_key.trim())))
    } else {
        None
    };
    let desired_default_model = if !default_model.trim().is_empty() {
        Some(format!("default_model = \"{}\"", escape_toml_string(default_model.trim())))
    } else {
        None
    };

    match (section_start, section_end) {
        (Some(start), Some(end)) => {
            // Section exists: update existing field lines or append missing fields within section
            let mut api_idx = None;
            let mut url_idx = None;
            let mut api_key_idx = None;
            let mut default_model_idx = None;

            for idx in (start + 1)..end {
                let trimmed = lines[idx].trim();
                if is_provider_field_assignment(trimmed, "api") {
                    api_idx = Some(idx);
                } else if is_provider_field_assignment(trimmed, "url") {
                    url_idx = Some(idx);
                } else if is_provider_field_assignment(trimmed, "api_key") {
                    api_key_idx = Some(idx);
                } else if is_provider_field_assignment(trimmed, "default_model") {
                    default_model_idx = Some(idx);
                }
            }

            // Replace or remove in reverse index order if modifying, or record fields to insert
            // Simple approach: update lines in-place where found, remove unwanted key/model lines, insert missing
            if let Some(idx) = api_idx {
                lines[idx] = desired_api.clone();
            }
            if let Some(idx) = url_idx {
                lines[idx] = desired_url.clone();
            }
            if let Some(idx) = api_key_idx {
                if let Some(ref val) = desired_api_key {
                    lines[idx] = val.clone();
                }
            }
            if let Some(idx) = default_model_idx {
                if let Some(ref val) = desired_default_model {
                    lines[idx] = val.clone();
                }
            }

            // Insert missing fields right after header
            let mut insert_pos = start + 1;
            if api_idx.is_none() {
                lines.insert(insert_pos, desired_api);
                insert_pos += 1;
            }
            if url_idx.is_none() {
                lines.insert(insert_pos, desired_url);
                insert_pos += 1;
            }
            if api_key_idx.is_none() {
                if let Some(val) = desired_api_key {
                    lines.insert(insert_pos, val);
                    insert_pos += 1;
                }
            }
            if default_model_idx.is_none() {
                if let Some(val) = desired_default_model {
                    lines.insert(insert_pos, val);
                }
            }
        }
        _ => {
            // Section does not exist: append fresh section
            if !lines.is_empty() && !lines.last().is_some_and(String::is_empty) {
                lines.push(String::new());
            }
            lines.push(section);
            lines.push(desired_api);
            lines.push(desired_url);
            if let Some(val) = desired_api_key {
                lines.push(val);
            }
            if let Some(val) = desired_default_model {
                lines.push(val);
            }
        }
    }

    let mut content = lines.join("\n");
    content.push('\n');
    fs::write(&config_path, content)
        .with_context(|| format!("Failed to write config to {}", config_path.display()))?;

    Ok(config_path)
}

/// Check if an API key is configured (either in config or environment)
pub fn has_api_key(config: &Config) -> bool {
    config.active_provider().is_ok()
        || config
            .api_key
            .as_ref()
            .is_some_and(|k| !k.trim().is_empty())
}

/// Clear the API key from the config file
pub fn clear_api_key() -> Result<()> {
    let config_path = default_config_path()
        .context("Failed to resolve config path: home directory not found.")?;

    if !config_path.exists() {
        return Ok(());
    }

    let existing = fs::read_to_string(&config_path)?;
    let mut result = String::new();

    for line in existing.lines() {
        if !line.trim_start().starts_with("api_key") {
            result.push_str(line);
            result.push('\n');
        }
    }

    fs::write(&config_path, result)
        .with_context(|| format!("Failed to write config to {}", config_path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::env;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use parking_lot::Mutex;
    use std::sync::OnceLock;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct EnvGuard {
        home: Option<OsString>,
        userprofile: Option<OsString>,
        minimax_config_path: Option<OsString>,
    }

    impl EnvGuard {
        fn new(home: &Path) -> Self {
            let home_str = OsString::from(home.as_os_str());
            let config_path = home.join(".minimax").join("config.toml");
            let config_str = OsString::from(config_path.as_os_str());
            let home_prev = env::var_os("HOME");
            let userprofile_prev = env::var_os("USERPROFILE");
            let minimax_config_prev = env::var_os("MINIMAX_CONFIG_PATH");
            // Safety: test-only environment mutation guarded by a global mutex.
            unsafe {
                env::set_var("HOME", &home_str);
                env::set_var("USERPROFILE", &home_str);
                env::set_var("MINIMAX_CONFIG_PATH", &config_str);
            }
            Self {
                home: home_prev,
                userprofile: userprofile_prev,
                minimax_config_path: minimax_config_prev,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.home.take() {
                // Safety: test-only environment mutation guarded by a global mutex.
                unsafe {
                    env::set_var("HOME", value);
                }
            } else {
                // Safety: test-only environment mutation guarded by a global mutex.
                unsafe {
                    env::remove_var("HOME");
                }
            }
            if let Some(value) = self.userprofile.take() {
                // Safety: test-only environment mutation guarded by a global mutex.
                unsafe {
                    env::set_var("USERPROFILE", value);
                }
            } else {
                // Safety: test-only environment mutation guarded by a global mutex.
                unsafe {
                    env::remove_var("USERPROFILE");
                }
            }
            if let Some(value) = self.minimax_config_path.take() {
                // Safety: test-only environment mutation guarded by a global mutex.
                unsafe {
                    env::set_var("MINIMAX_CONFIG_PATH", value);
                }
            } else {
                // Safety: test-only environment mutation guarded by a global mutex.
                unsafe {
                    env::remove_var("MINIMAX_CONFIG_PATH");
                }
            }
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn save_api_key_writes_config() -> Result<()> {
        let _lock = env_lock().lock();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_root =
            env::temp_dir().join(format!("minimax-cli-test-{}-{}", std::process::id(), nanos));
        fs::create_dir_all(&temp_root)?;
        let _guard = EnvGuard::new(&temp_root);

        let path = save_api_key("test-key")?;
        let expected = temp_root.join(".minimax").join("config.toml");
        assert_eq!(path, expected);

        let contents = fs::read_to_string(&path)?;
        assert!(contents.contains("api_key = \"test-key\""));
        Ok(())
    }

    #[test]
    fn test_tilde_expansion_in_paths() -> Result<()> {
        let _lock = env_lock().lock();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_root = env::temp_dir().join(format!(
            "minimax-cli-tilde-test-{}-{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&temp_root)?;
        let _guard = EnvGuard::new(&temp_root);

        let config = Config {
            skills_dir: Some("~/.minimax/skills".to_string()),
            ..Default::default()
        };
        let expected_home = dirs::home_dir().expect("home dir not found");
        let expected_skills = expected_home.join(".minimax").join("skills");
        let actual_skills = config.skills_dir();
        assert_eq!(
            actual_skills.components().collect::<Vec<_>>(),
            expected_skills.components().collect::<Vec<_>>()
        );

        let absolute_path = temp_root.join("absolute-path");
        let absolute_str = absolute_path.to_string_lossy().to_string();
        let config = Config {
            output_dir: Some(absolute_str.clone()),
            ..Default::default()
        };
        assert_eq!(config.output_dir(), PathBuf::from(absolute_str));

        let config = Config {
            output_dir: Some("./relative/path".to_string()),
            ..Default::default()
        };
        assert_eq!(config.output_dir(), PathBuf::from("./relative/path"));
        Ok(())
    }

    #[test]
    fn test_nonexistent_profile_error() {
        let mut profiles = HashMap::new();
        profiles.insert("work".to_string(), Config::default());
        let config = ConfigFile {
            base: Config::default(),
            profiles: Some(profiles),
        };

        let err = apply_profile(config, Some("nonexistent")).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("Profile 'nonexistent' not found"));
        assert!(message.contains("Available profiles"));
        assert!(message.contains("work"));
    }

    #[test]
    fn test_profile_with_no_profiles_section() {
        let config = ConfigFile {
            base: Config::default(),
            profiles: None,
        };

        let err = apply_profile(config, Some("missing")).unwrap_err();
        assert!(err.to_string().contains("Available profiles: none"));
    }

    #[test]
    fn test_save_api_key_doesnt_match_similar_keys() -> Result<()> {
        let _lock = env_lock().lock();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_root = env::temp_dir().join(format!(
            "minimax-cli-api-key-test-{}-{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&temp_root)?;
        let _guard = EnvGuard::new(&temp_root);

        let config_path = temp_root.join(".minimax").join("config.toml");
        ensure_parent_dir(&config_path)?;
        fs::write(
            &config_path,
            "api_key_backup = \"old\"\napi_key = \"current\"\n",
        )?;

        let path = save_api_key("new-key")?;
        assert_eq!(path, config_path);

        let contents = fs::read_to_string(&config_path)?;
        assert!(contents.contains("api_key_backup = \"old\""));
        assert!(contents.contains("api_key = \"new-key\""));
        Ok(())
    }

    #[test]
    fn save_provider_api_key_appends_new_section() -> Result<()> {
        let _lock = env_lock().lock();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_root =
            env::temp_dir().join(format!("minimax-login-append-{nanos}"));
        fs::create_dir_all(&temp_root)?;
        let _guard = EnvGuard::new(&temp_root);

        let config_path = temp_root.join(".minimax").join("config.toml");
        fs::create_dir_all(config_path.parent().unwrap())?;
        fs::write(
            &config_path,
            "api_key = \"top-level\"\n\n[providers.deepseek]\nurl = \"https://api.deepseek.com/v1\"\n",
        )?;

        let saved = save_provider_api_key("kimi", "sk-kimi")?;
        assert_eq!(saved, config_path);

        let contents = fs::read_to_string(&config_path)?;
        assert!(
            contents.contains("[providers.kimi]\napi_key = \"sk-kimi\""),
            "appended section missing:\n{contents}"
        );
        // Existing content untouched
        assert!(contents.contains("api_key = \"top-level\""));
        assert!(contents.contains("[providers.deepseek]"));

        // Round-trips through the config loader
        let config = Config::load(Some(config_path), None)?;
        let providers = config.providers.expect("providers table");
        assert_eq!(
            providers.get("kimi").and_then(|p| p.api_key.as_deref()),
            Some("sk-kimi")
        );
        fs::remove_dir_all(&temp_root).ok();
        Ok(())
    }

    #[test]
    fn save_provider_api_key_updates_existing_section() -> Result<()> {
        let _lock = env_lock().lock();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_root =
            env::temp_dir().join(format!("minimax-login-update-{nanos}"));
        fs::create_dir_all(&temp_root)?;
        let _guard = EnvGuard::new(&temp_root);

        let config_path = temp_root.join(".minimax").join("config.toml");
        fs::create_dir_all(config_path.parent().unwrap())?;
        fs::write(
            &config_path,
            "[providers.deepseek]\nurl = \"https://api.deepseek.com/v1\"\napi_key = \"old-key\"\n\n[providers.zai]\nurl = \"https://api.z.ai/api/anthropic\"\n",
        )?;

        // Replace an existing key line
        save_provider_api_key("deepseek", "new-key")?;
        // Insert a key into a section that has none
        save_provider_api_key("zai", "sk-zai")?;

        let contents = fs::read_to_string(&config_path)?;
        assert!(
            contents
                .contains("[providers.deepseek]\nurl = \"https://api.deepseek.com/v1\"\napi_key = \"new-key\""),
            "deepseek section wrong:\n{contents}"
        );
        assert!(
            contents.contains("[providers.zai]")
                && contents.contains("url = \"https://api.z.ai/api/anthropic\"")
                && contents.contains("api_key = \"sk-zai\""),
            "zai section wrong:\n{contents}"
        );
        assert!(!contents.contains("old-key"));

        let config = Config::load(Some(config_path), None)?;
        let providers = config.providers.expect("providers table");
        assert_eq!(
            providers.get("deepseek").and_then(|p| p.api_key.as_deref()),
            Some("new-key")
        );
        assert_eq!(
            providers.get("zai").and_then(|p| p.api_key.as_deref()),
            Some("sk-zai")
        );
        fs::remove_dir_all(&temp_root).ok();
        Ok(())
    }

    #[test]
    fn save_provider_api_key_minimax_writes_top_level() -> Result<()> {
        let _lock = env_lock().lock();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_root =
            env::temp_dir().join(format!("minimax-login-minimax-{nanos}"));
        fs::create_dir_all(&temp_root)?;
        let _guard = EnvGuard::new(&temp_root);

        let config_path = temp_root.join(".minimax").join("config.toml");
        fs::create_dir_all(config_path.parent().unwrap())?;
        fs::write(&config_path, "api_key = \"old\"\n")?;

        save_provider_api_key("minimax", "sk-mini")?;

        let contents = fs::read_to_string(&config_path)?;
        assert!(contents.contains("api_key = \"sk-mini\""));
        assert!(!contents.contains("[providers.minimax]"));
        fs::remove_dir_all(&temp_root).ok();
        Ok(())
    }

    #[test]
    fn test_empty_api_key_rejected() {
        let config = Config {
            api_key: Some("   ".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_missing_api_key_allowed() -> Result<()> {
        let config = Config::default();
        config.validate()?;
        Ok(())
    }

    #[test]
    fn test_implicit_minimax_provider() {
        let config = Config {
            api_key: Some("sk-test".into()),
            base_url: Some("https://api.minimax.io".into()),
            default_text_model: Some("MiniMax-M3".into()),
            ..Default::default()
        };
        let active = config.active_provider().unwrap();
        assert_eq!(active.name, "minimax");
        assert_eq!(active.api, ProviderApi::Anthropic);
        assert_eq!(active.default_model, "MiniMax-M3");
        assert_eq!(
            active.anthropic_messages_url(),
            "https://api.minimax.io/anthropic/v1/messages"
        );
    }

    #[test]
    fn test_named_openai_provider() {
        let mut providers = HashMap::new();
        providers.insert(
            "openai".to_string(),
            ProviderEntry {
                api: Some(ProviderApi::OpenAi),
                url: Some("https://api.openai.com/v1".into()),
                api_key: Some("sk-oai".into()),
                default_model: Some("gpt-4.1".into()),
            },
        );
        let config = Config {
            provider: Some("openai".into()),
            providers: Some(providers),
            api_key: Some("sk-mini".into()),
            ..Default::default()
        };
        let active = config.active_provider().unwrap();
        assert_eq!(active.name, "openai");
        assert_eq!(active.api, ProviderApi::OpenAi);
        assert_eq!(
            active.openai_chat_completions_url(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_unknown_provider_errors() {
        let config = Config {
            provider: Some("nope".into()),
            api_key: Some("sk".into()),
            ..Default::default()
        };
        let err = config.active_provider().unwrap_err().to_string();
        assert!(err.contains("Unknown provider 'nope'"));
    }

    #[test]
    fn test_anthropic_and_openai_url_helpers() {
        assert_eq!(
            anthropic_messages_url("https://api.minimax.io"),
            "https://api.minimax.io/anthropic/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            openai_chat_completions_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            openai_chat_completions_url("https://proxy.example.com"),
            "https://proxy.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_models_url_helpers() {
        // OpenAI-compatible base URLs
        assert_eq!(
            openai_models_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            openai_models_url("https://proxy.example.com"),
            "https://proxy.example.com/v1/models"
        );
        // Full chat-completions URL collapses back to the models endpoint
        assert_eq!(
            openai_models_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/models"
        );

        // Anthropic-compatible base URLs
        assert_eq!(
            anthropic_models_url("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            anthropic_models_url("https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            anthropic_models_url("https://api.minimax.io"),
            "https://api.minimax.io/anthropic/v1/models"
        );
        assert_eq!(
            anthropic_models_url("https://api.minimax.io/anthropic/v1"),
            "https://api.minimax.io/anthropic/v1/models"
        );
    }

    #[test]
    fn test_coding_plan_provider_urls() {
        // Kimi Coding Plan — separate api.kimi.com service
        assert_eq!(
            anthropic_messages_url("https://api.kimi.com/coding/v1"),
            "https://api.kimi.com/coding/v1/messages"
        );
        assert_eq!(
            anthropic_models_url("https://api.kimi.com/coding/v1"),
            "https://api.kimi.com/coding/v1/models"
        );
        assert_eq!(
            anthropic_models_url("https://api.moonshot.ai/anthropic"),
            "https://api.moonshot.ai/anthropic/v1/models"
        );

        // Z.ai Coding Plan — Anthropic-compatible
        assert_eq!(
            anthropic_messages_url("https://api.z.ai/api/anthropic"),
            "https://api.z.ai/api/anthropic/v1/messages"
        );
        assert_eq!(
            anthropic_models_url("https://api.z.ai/api/anthropic"),
            "https://api.z.ai/api/anthropic/v1/models"
        );

        // DeepSeek — OpenAI-compatible
        assert_eq!(
            openai_chat_completions_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            openai_models_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/models"
        );
    }

    #[test]
    fn test_save_provider_fresh_config() -> Result<()> {
        let _lock = env_lock().lock();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_root =
            env::temp_dir().join(format!("minimax-save-provider-{nanos}"));
        fs::create_dir_all(&temp_root)?;
        let _guard = EnvGuard::new(&temp_root);

        let config_path = temp_root.join(".minimax").join("config.toml");

        // Save openrouter without api_key and default_model
        let saved = save_provider(
            "openrouter",
            ProviderApi::OpenAi,
            "https://openrouter.ai/api/v1",
            "",
            "",
        )?;
        assert_eq!(saved, config_path);

        let contents = fs::read_to_string(&config_path)?;
        assert!(
            contents.contains("[providers.openrouter]"),
            "missing section header:\n{contents}"
        );
        assert!(
            contents.contains("api = \"openai\""),
            "missing api line:\n{contents}"
        );
        assert!(
            contents.contains("url = \"https://openrouter.ai/api/v1\""),
            "missing url line:\n{contents}"
        );
        assert!(
            !contents.contains("api_key"),
            "api_key should not be present:\n{contents}"
        );
        assert!(
            !contents.contains("default_model"),
            "default_model should not be present:\n{contents}"
        );

        // Error on minimax name
        let err = save_provider("minimax", ProviderApi::OpenAi, "https://example.com", "", "").unwrap_err();
        assert!(err.to_string().contains("Cannot add default provider 'minimax'"));
        // Error on name with a newline (section-header injection)
        let err = save_provider("foo\nbar", ProviderApi::OpenAi, "https://example.com", "", "").unwrap_err();
        assert!(err.to_string().contains("Invalid provider name"));

        // Round-trips through Config::load
        let config = Config::load(Some(config_path), None)?;
        let providers = config.providers.expect("providers table");
        let entry = providers.get("openrouter").expect("openrouter entry");
        assert_eq!(entry.api, Some(ProviderApi::OpenAi));
        assert_eq!(entry.url.as_deref(), Some("https://openrouter.ai/api/v1"));
        assert_eq!(entry.api_key, None);

        fs::remove_dir_all(&temp_root).ok();
        Ok(())
    }
}
