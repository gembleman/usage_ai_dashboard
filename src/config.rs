//! Account configuration loaded from `config.toml`.
//!
//! The config file is looked up next to the executable first, then in the
//! current working directory. See `config.toml` in the repo root for the
//! schema. `~` in home paths expands to USERPROFILE (Windows) / HOME.

use std::path::{Path, PathBuf};

use serde::Deserialize;

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
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    codex_accounts: Vec<CodexAccountConfig>,
    #[serde(default)]
    claude_accounts: Vec<ClaudeAccountConfig>,
}

/// Fully-resolved account configuration.
#[derive(Debug, Default, Clone)]
pub struct Config {
    codex: Vec<(CodexAccount, bool)>,
    claude: Vec<(ClaudeAccount, bool)>,
}

impl Config {
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
                    },
                    c.dormant,
                )
            })
            .collect();

        Ok(Config { codex, claude })
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
