//! Common normalized types shared between the Codex and Claude Code parsers.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Which CLI a usage record originated from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Codex,
    ClaudeCode,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Codex => write!(f, "codex"),
            Source::ClaudeCode => write!(f, "claude_code"),
        }
    }
}

impl std::str::FromStr for Source {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "codex" => Ok(Source::Codex),
            "claude_code" => Ok(Source::ClaudeCode),
            other => Err(format!("unknown source: {other}")),
        }
    }
}

/// A single normalized usage record: one turn (Codex) or one assistant
/// message (Claude Code), already de-duplicated / delta-resolved by the
/// source-specific parser.
#[derive(Debug, Clone, Serialize)]
pub struct UsageRecord {
    pub source: Source,
    pub account: String,
    pub timestamp: DateTime<Utc>,
    pub model: Option<String>,
    pub input_tokens: u64,
    /// Cache *read* input tokens (billed at ~0.1x the base input rate).
    /// Named for backwards compatibility with the DB column that once held
    /// creation+read merged; it now carries the read portion only.
    pub cached_input_tokens: u64,
    /// Cache *creation* input tokens (billed at ~1.25x the base input rate).
    /// Always 0 for Codex, which has no cache-creation concept.
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    /// Only meaningful for Claude Code; false for Codex.
    pub is_subagent: bool,
}

/// A single rate-limit window (e.g. 5h "primary" or 7d "secondary").
#[derive(Debug, Clone, Serialize)]
pub struct RateLimitWindowSnapshot {
    pub used_percent: f64,
    pub window_minutes: u64,
    /// Unix epoch seconds (UTC) at which the window resets.
    pub resets_at: i64,
}

/// Extra-usage ("pay-as-you-go beyond plan limits") credit state, reported by
/// the Anthropic OAuth usage API. Claude Code only; only stored when the
/// account has the feature enabled.
#[derive(Debug, Clone, Serialize)]
pub struct ExtraUsageSnapshot {
    /// Monthly credit cap, if configured (API reports it in plan currency).
    pub monthly_limit: Option<f64>,
    /// Credits consumed so far this month.
    pub used_credits: Option<f64>,
    /// Percent of the monthly cap consumed.
    pub utilization: Option<f64>,
}

/// Per-account rate-limit snapshot. Codex snapshots come from the session
/// JSONL `token_count` events; Claude Code snapshots come from the Anthropic
/// OAuth usage API (see design doc §4 — the local transcripts carry no
/// reset-time information). `source` disambiguates the two, since a Codex and
/// a Claude Code account can share the same display name (e.g. "user01").
#[derive(Debug, Clone, Serialize)]
pub struct RateLimitSnapshot {
    pub source: Source,
    pub account: String,
    /// Timestamp of the token_count event this snapshot was taken from.
    pub observed_at: DateTime<Utc>,
    pub limit_id: Option<String>,
    pub plan_type: Option<String>,
    pub rate_limit_reached_type: Option<String>,
    pub primary: Option<RateLimitWindowSnapshot>,
    pub secondary: Option<RateLimitWindowSnapshot>,
    /// Claude Code only: model-scoped weekly windows (null unless the plan
    /// enforces per-model caps). Always `None` for Codex.
    pub seven_day_opus: Option<RateLimitWindowSnapshot>,
    pub seven_day_sonnet: Option<RateLimitWindowSnapshot>,
    /// Claude Code only: extra-usage credits, present only when enabled.
    pub extra_usage: Option<ExtraUsageSnapshot>,
}
