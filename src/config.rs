//! Account configuration loaded from `config.toml`.
//!
//! The config file is looked up next to the executable first, then in the
//! current working directory. See `config.toml` in the repo root for the
//! schema. `~` in home paths expands to USERPROFILE (Windows) / HOME.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::claude_code::ClaudeAccount;
use crate::codex::CodexAccount;

#[derive(Debug, Deserialize)]
struct CodexAccountConfig {
    name: String,
    codex_home: String,
    #[serde(default)]
    dormant: bool,
}

#[derive(Debug, Deserialize)]
struct ClaudeAccountConfig {
    name: String,
    config_dir: String,
    #[serde(default)]
    dormant: bool,
    #[serde(default = "default_true")]
    include_subagents: bool,
}

/// Per-million-token prices in USD loaded from `config.toml`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct ModelPricing {
    pub input: f64,
    pub cached_input: f64,
    pub cache_creation_input: f64,
    pub output: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// Directory containing dashboard.html, styles.css, and the JavaScript files.
    /// Relative paths are resolved from the directory containing config.toml.
    pub frontend_dir: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 3000,
            frontend_dir: "src/frontend".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DashboardConfig {
    pub page_size: usize,
    pub model_chart_max_items: usize,
    pub auto_refresh_seconds: u64,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            page_size: 50,
            model_chart_max_items: 8,
            auto_refresh_seconds: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct CacheConfig {
    path: String,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            path: "cache.sqlite3".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TimeoutConfig {
    pub api_seconds: u64,
    pub refresh_seconds: u64,
    pub anthropic_seconds: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            api_seconds: 30,
            refresh_seconds: 120,
            anthropic_seconds: 8,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    // Backward compatibility with the old top-level `port` setting.
    port: Option<u16>,
    #[serde(default)]
    server: ServerConfig,
    #[serde(default)]
    dashboard: DashboardConfig,
    #[serde(default)]
    cache: CacheConfig,
    #[serde(default)]
    timeouts: TimeoutConfig,
    #[serde(default)]
    model_pricing: HashMap<String, ModelPricing>,
    #[serde(default)]
    codex_accounts: Vec<CodexAccountConfig>,
    #[serde(default)]
    claude_accounts: Vec<ClaudeAccountConfig>,
}

/// Fully-resolved account configuration.
#[derive(Debug, Default, Clone)]
pub struct Config {
    server: ServerConfig,
    dashboard: DashboardConfig,
    cache_path: PathBuf,
    frontend_dir: PathBuf,
    timeouts: TimeoutConfig,
    model_pricing: HashMap<String, ModelPricing>,
    codex: Vec<(CodexAccount, bool)>,
    claude: Vec<(ClaudeAccount, bool)>,
    /// Directory the loaded `config.toml` lives in, if any. Used to place
    /// the cache DB alongside it.
    config_dir: Option<PathBuf>,
}

impl Config {
    /// Server port from `config.toml`, if set.
    pub fn server(&self) -> &ServerConfig {
        &self.server
    }
    pub fn dashboard(&self) -> &DashboardConfig {
        &self.dashboard
    }
    pub fn cache_path(&self) -> &Path {
        &self.cache_path
    }
    pub fn frontend_dir(&self) -> &Path {
        &self.frontend_dir
    }
    pub fn timeouts(&self) -> &TimeoutConfig {
        &self.timeouts
    }

    /// Directory the loaded `config.toml` lives in, if any.
    pub fn config_dir(&self) -> Option<&Path> {
        self.config_dir.as_deref()
    }

    /// Model prices from `config.toml`.
    pub fn model_pricing(&self) -> &HashMap<String, ModelPricing> {
        &self.model_pricing
    }

    /// Codex accounts, filtered by dormant flag.
    pub fn codex_accounts(&self, include_dormant: bool) -> Vec<CodexAccount> {
        self.codex
            .iter()
            .filter(|(_, dormant)| include_dormant || !dormant)
            .map(|(a, _)| a.clone())
            .collect()
    }

    /// Claude Code accounts, filtered by dormant flag.
    pub fn claude_accounts(&self, include_dormant: bool) -> Vec<ClaudeAccount> {
        self.claude
            .iter()
            .filter(|(_, dormant)| include_dormant || !dormant)
            .map(|(a, _)| a.clone())
            .collect()
    }

    /// Load and resolve `config.toml`, exiting the process with a clear
    /// message if it cannot be found or parsed.
    pub fn load_or_exit() -> Config {
        match Self::load() {
            Ok(cfg) => cfg,
            Err(msg) => {
                eprintln!("Error: {msg}");
                std::process::exit(1);
            }
        }
    }

    fn load() -> Result<Config, String> {
        let path = Self::find_config_path().ok_or_else(|| {
            "config.toml not found (looked next to the executable and in the current directory). \
             See the repo's config.toml for the expected schema."
                .to_string()
        })?;

        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let raw: RawConfig = toml::from_str(&text)
            .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;

        for (model, pricing) in &raw.model_pricing {
            if model.trim().is_empty() {
                return Err("model_pricing contains an empty model ID".to_string());
            }
            if !pricing.input.is_finite()
                || pricing.input < 0.0
                || !pricing.cached_input.is_finite()
                || pricing.cached_input < 0.0
                || !pricing.cache_creation_input.is_finite()
                || pricing.cache_creation_input < 0.0
                || !pricing.output.is_finite()
                || pricing.output < 0.0
            {
                return Err(format!(
                    "model_pricing.{model} prices must be non-negative numbers"
                ));
            }
        }
        if raw.server.host.trim().is_empty() {
            return Err("server.host must not be empty".into());
        }
        if raw.server.frontend_dir.trim().is_empty() {
            return Err("server.frontend_dir must not be empty".into());
        }
        if raw.dashboard.page_size == 0 {
            return Err("dashboard.page_size must be greater than 0".into());
        }
        if raw.dashboard.model_chart_max_items < 2 {
            return Err("dashboard.model_chart_max_items must be at least 2".into());
        }
        if raw.timeouts.api_seconds == 0
            || raw.timeouts.refresh_seconds == 0
            || raw.timeouts.anthropic_seconds == 0
        {
            return Err("timeout values must be greater than 0".into());
        }

        let config_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let configured_cache = expand_home(&raw.cache.path);
        let cache_path = if configured_cache.is_absolute() {
            configured_cache
        } else {
            config_dir.join(configured_cache)
        };
        let configured_frontend = expand_home(&raw.server.frontend_dir);
        let frontend_dir = if configured_frontend.is_absolute() {
            configured_frontend
        } else {
            config_dir.join(configured_frontend)
        };
        let mut server = raw.server;
        if let Some(port) = raw.port {
            server.port = port;
        }

        let codex = raw
            .codex_accounts
            .into_iter()
            .map(|c| {
                (
                    CodexAccount {
                        name: c.name,
                        codex_home: expand_home(&c.codex_home),
                    },
                    c.dormant,
                )
            })
            .collect();
        let claude = raw
            .claude_accounts
            .into_iter()
            .map(|c| {
                (
                    ClaudeAccount {
                        name: c.name,
                        config_dir: expand_home(&c.config_dir),
                        include_subagents: c.include_subagents,
                    },
                    c.dormant,
                )
            })
            .collect();

        Ok(Config {
            server,
            dashboard: raw.dashboard,
            cache_path,
            frontend_dir,
            timeouts: raw.timeouts,
            model_pricing: raw.model_pricing,
            codex,
            claude,
            config_dir: path.parent().map(|p| p.to_path_buf()),
        })
    }

    fn find_config_path() -> Option<PathBuf> {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let candidate = dir.join("config.toml");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        let cwd = Path::new("config.toml");
        if cwd.is_file() {
            return Some(cwd.to_path_buf());
        }
        None
    }
}

fn default_true() -> bool {
    true
}

/// Expand a leading `~` to the user's home directory.
fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        home().join(rest)
    } else if path == "~" {
        home()
    } else {
        PathBuf::from(path)
    }
}

fn home() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
