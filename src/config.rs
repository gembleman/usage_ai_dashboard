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
use crate::opencode::OpenCodeAccount;
use crate::pi::PiAccount;
use crate::token_api::TokenApiServer;

#[derive(Debug, Deserialize)]
struct TokenApiServerConfig {
    name: String,
    url: String,
    #[serde(default)]
    dormant: bool,
    #[serde(default = "default_true")]
    refresh: bool,
}

#[derive(Debug, Deserialize)]
struct CodexAccountConfig {
    name: String,
    codex_home: String,
    #[serde(default)]
    dormant: bool,
    #[serde(default = "default_true")]
    refresh: bool,
}

#[derive(Debug, Deserialize)]
struct PiAccountConfig {
    name: String,
    pi_home: String,
    #[serde(default)]
    dormant: bool,
    #[serde(default = "default_true")]
    refresh: bool,
}

#[derive(Debug, Deserialize)]
struct OpenCodeAccountConfig {
    name: String,
    data_dir: String,
    #[serde(default)]
    dormant: bool,
    #[serde(default = "default_true")]
    refresh: bool,
}

#[derive(Debug, Deserialize)]
struct ClaudeAccountConfig {
    name: String,
    config_dir: String,
    #[serde(default)]
    dormant: bool,
    #[serde(default = "default_true")]
    refresh: bool,
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
    #[serde(default)]
    pi_accounts: Vec<PiAccountConfig>,
    #[serde(default)]
    opencode_accounts: Vec<OpenCodeAccountConfig>,
    #[serde(default)]
    token_api_servers: Vec<TokenApiServerConfig>,
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
    codex: Vec<(CodexAccount, bool, bool)>,
    claude: Vec<(ClaudeAccount, bool, bool)>,
    pi: Vec<(PiAccount, bool, bool)>,
    opencode: Vec<(OpenCodeAccount, bool, bool)>,
    token_api: Vec<(TokenApiServer, bool, bool)>,
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

    /// Model prices from `config.toml`, with built-in defaults for GPT-6 Sol and Luna.
    pub fn model_pricing(&self) -> &HashMap<String, ModelPricing> {
        &self.model_pricing
    }

    /// Codex accounts enabled for refresh, filtered by dormant flag.
    pub fn codex_accounts(&self, include_dormant: bool) -> Vec<CodexAccount> {
        self.codex
            .iter()
            .filter(|(_, dormant, refresh)| *refresh && (include_dormant || !dormant))
            .map(|(a, _, _)| a.clone())
            .collect()
    }

    /// Claude Code accounts enabled for refresh, filtered by dormant flag.
    pub fn claude_accounts(&self, include_dormant: bool) -> Vec<ClaudeAccount> {
        self.claude
            .iter()
            .filter(|(_, dormant, refresh)| *refresh && (include_dormant || !dormant))
            .map(|(a, _, _)| a.clone())
            .collect()
    }

    /// pi accounts enabled for refresh, filtered by dormant flag.
    pub fn pi_accounts(&self, include_dormant: bool) -> Vec<PiAccount> {
        self.pi
            .iter()
            .filter(|(_, dormant, refresh)| *refresh && (include_dormant || !dormant))
            .map(|(a, _, _)| a.clone())
            .collect()
    }

    /// OpenCode accounts enabled for refresh, filtered by dormant flag.
    pub fn opencode_accounts(&self, include_dormant: bool) -> Vec<OpenCodeAccount> {
        self.opencode
            .iter()
            .filter(|(_, dormant, refresh)| *refresh && (include_dormant || !dormant))
            .map(|(a, _, _)| a.clone())
            .collect()
    }

    /// Remote token-usage API servers enabled for refresh, filtered by dormant flag.
    pub fn token_api_servers(&self, include_dormant: bool) -> Vec<TokenApiServer> {
        self.token_api
            .iter()
            .filter(|(_, dormant, refresh)| *refresh && (include_dormant || !dormant))
            .map(|(server, _, _)| server.clone())
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
        for server in &raw.token_api_servers {
            if server.name.trim().is_empty() {
                return Err("token_api_servers.name must not be empty".into());
            }
            if !server.url.starts_with("http://") && !server.url.starts_with("https://") {
                return Err(format!(
                    "token_api_servers.{} url must start with http:// or https://",
                    server.name
                ));
            }
        }

        let model_pricing = with_builtin_pricing(raw.model_pricing);

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
                    c.refresh,
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
                    c.refresh,
                )
            })
            .collect();

        let pi = raw
            .pi_accounts
            .into_iter()
            .map(|c| {
                (
                    PiAccount {
                        name: c.name,
                        pi_home: expand_home(&c.pi_home),
                    },
                    c.dormant,
                    c.refresh,
                )
            })
            .collect();

        let opencode = raw
            .opencode_accounts
            .into_iter()
            .map(|c| {
                (
                    OpenCodeAccount {
                        name: c.name,
                        data_dir: expand_home(&c.data_dir),
                    },
                    c.dormant,
                    c.refresh,
                )
            })
            .collect();

        let token_api = raw
            .token_api_servers
            .into_iter()
            .map(|server| {
                (
                    TokenApiServer {
                        name: server.name,
                        url: server.url.trim_end_matches('/').to_string(),
                    },
                    server.dormant,
                    server.refresh,
                )
            })
            .collect();

        Ok(Config {
            server,
            dashboard: raw.dashboard,
            cache_path,
            frontend_dir,
            timeouts: raw.timeouts,
            model_pricing,
            codex,
            claude,
            pi,
            opencode,
            token_api,
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

fn with_builtin_pricing(
    mut pricing: HashMap<String, ModelPricing>,
) -> HashMap<String, ModelPricing> {
    // Standard short-context USD prices per million tokens. A config entry
    // takes precedence so users can adjust the estimate for their service tier.
    pricing.entry("gpt-6-sol".into()).or_insert(ModelPricing {
        input: 2.0,
        cached_input: 0.2,
        cache_creation_input: 2.5,
        output: 10.0,
    });
    pricing.entry("gpt-6-luna".into()).or_insert(ModelPricing {
        input: 0.1,
        cached_input: 0.01,
        cache_creation_input: 0.125,
        output: 0.5,
    });
    pricing
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_gpt6_prices_fill_missing_entries_without_overriding_config() {
        let custom = ModelPricing {
            input: 3.0,
            cached_input: 0.3,
            cache_creation_input: 3.75,
            output: 15.0,
        };
        let pricing = with_builtin_pricing(HashMap::from([("gpt-6-sol".into(), custom)]));
        assert_eq!(pricing["gpt-6-sol"].input, 3.0);
        assert_eq!(pricing["gpt-6-luna"].cached_input, 0.01);
        assert_eq!(pricing["gpt-6-luna"].cache_creation_input, 0.125);
        assert_eq!(pricing["gpt-6-luna"].output, 0.5);
    }

    #[test]
    fn refresh_defaults_to_enabled_and_can_be_disabled_per_account() {
        let raw: RawConfig = toml::from_str(
            r#"
                [[codex_accounts]]
                name = "enabled"
                codex_home = "/enabled"

                [[codex_accounts]]
                name = "disabled"
                codex_home = "/disabled"
                refresh = false

                [[token_api_servers]]
                name = "remote"
                url = "http://localhost:8787"
                refresh = false
            "#,
        )
        .unwrap();

        assert!(raw.codex_accounts[0].refresh);
        assert!(!raw.codex_accounts[1].refresh);
        assert!(!raw.token_api_servers[0].refresh);
    }

    #[test]
    fn disabled_refresh_accounts_are_filtered_even_when_dormant_accounts_are_included() {
        let mut config = Config::default();
        config.codex = vec![
            (
                CodexAccount {
                    name: "enabled".into(),
                    codex_home: "/enabled".into(),
                },
                false,
                true,
            ),
            (
                CodexAccount {
                    name: "disabled".into(),
                    codex_home: "/disabled".into(),
                },
                false,
                false,
            ),
        ];

        let names: Vec<_> = config
            .codex_accounts(true)
            .into_iter()
            .map(|account| account.name)
            .collect();
        assert_eq!(names, ["enabled"]);
    }
}

fn home() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
