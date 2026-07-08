//! Claude Code session transcript parser.
//!
//! See docs/claude-code-usage-parsing-design.md for the full spec. Key rule
//! (§3.1, the most important one): a single API response (same
//! `message.id`) is split across multiple JSONL lines (one per content
//! block). `input_tokens`/`cache_*` are identical across the group;
//! `output_tokens` grows monotonically until the final line. We must group
//! by `message.id` and keep only the line with the maximum `output_tokens`
//! per group — summing all lines would over-count input tokens by the
//! number of lines in the group.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use walkdir::WalkDir;

use crate::model::{
    ExtraUsageSnapshot, RateLimitSnapshot, RateLimitWindowSnapshot, Source, UsageRecord,
};

/// One account's CLAUDE_CONFIG_DIR configuration.
#[derive(Debug, Clone)]
pub struct ClaudeAccount {
    pub name: String,
    pub config_dir: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClaudeUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub service_tier: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct MessageBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stop_reason: Option<serde_json::Value>,
    #[serde(default)]
    usage: Option<ClaudeUsage>,
}

#[derive(Debug, Deserialize)]
struct TranscriptLine {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    timestamp: Option<DateTime<Utc>>,
    #[serde(default, rename = "isSidechain")]
    is_sidechain: bool,
    #[serde(default)]
    message: Option<MessageBody>,
}

/// One candidate line for a message.id group, kept until we know it's the
/// max-output_tokens line in the group.
struct GroupCandidate {
    message_id: String,
    output_tokens: u64,
    record: UsageRecord,
}

#[derive(Debug, Default)]
pub struct ClaudeParseResult {
    pub records: Vec<UsageRecord>,
}

pub fn parse_account(account: &ClaudeAccount, include_subagents: bool) -> ClaudeParseResult {
    let projects_dir = account.config_dir.join("projects");
    let mut records = Vec::new();

    if !projects_dir.is_dir() {
        return ClaudeParseResult { records };
    }

    for entry in WalkDir::new(&projects_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }

        let is_subagent_file = path
            .components()
            .any(|c| c.as_os_str() == "subagents");
        if is_subagent_file && !include_subagents {
            continue;
        }

        parse_file(path, &account.name, is_subagent_file, &mut records);
    }

    ClaudeParseResult { records }
}

fn parse_file(
    path: &Path,
    account: &str,
    is_subagent_file: bool,
    records: &mut Vec<UsageRecord>,
) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = BufReader::new(file);

    let mut seen_message_ids: HashSet<String> = HashSet::new();
    let mut current_group: Option<GroupCandidate> = None;

    let flush = |group: Option<GroupCandidate>, records: &mut Vec<UsageRecord>| {
        if let Some(g) = group {
            records.push(g.record);
        }
    };

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: TranscriptLine = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // skip broken/partial lines (in-progress writes)
        };

        if parsed.kind.as_deref() != Some("assistant") {
            continue;
        }
        let Some(message) = parsed.message else {
            continue;
        };
        let Some(message_id) = message.id.clone() else {
            continue;
        };
        let Some(usage) = message.usage.clone() else {
            continue;
        };
        let timestamp = match parsed.timestamp {
            Some(t) => t,
            None => continue,
        };

        // If this message_id was already fully flushed earlier in the file
        // (should not normally happen — groups are contiguous — but guard
        // against re-appearance per design §5.2).
        if current_group
            .as_ref()
            .map(|g| g.message_id != message_id)
            .unwrap_or(false)
        {
            // New group starting: flush previous.
            let prev = current_group.take();
            if let Some(g) = &prev {
                seen_message_ids.insert(g.message_id.clone());
            }
            flush(prev, records);
        }

        if seen_message_ids.contains(&message_id) {
            // Already flushed this group earlier (non-contiguous
            // re-appearance) — skip to avoid double counting.
            continue;
        }

        let is_subagent = is_subagent_file || parsed.is_sidechain;
        let output_tokens = usage.output_tokens;

        let candidate_record = UsageRecord {
            source: Source::ClaudeCode,
            account: account.to_string(),
            timestamp,
            model: message.model.clone(),
            input_tokens: usage.input_tokens,
            cached_input_tokens: usage
                .cache_creation_input_tokens
                .saturating_add(usage.cache_read_input_tokens),
            output_tokens,
            reasoning_output_tokens: 0,
            total_tokens: usage
                .input_tokens
                .saturating_add(usage.cache_creation_input_tokens)
                .saturating_add(usage.cache_read_input_tokens)
                .saturating_add(output_tokens),
            is_subagent,
        };

        match &mut current_group {
            Some(g) if g.message_id == message_id => {
                // Same group: keep whichever line has the larger output_tokens.
                if output_tokens >= g.output_tokens {
                    g.output_tokens = output_tokens;
                    g.record = candidate_record;
                }
            }
            _ => {
                current_group = Some(GroupCandidate {
                    message_id,
                    output_tokens,
                    record: candidate_record,
                });
            }
        }
    }

    // Flush the final group at EOF.
    flush(current_group, records);
}

// ---------------------------------------------------------------------------
// Rate-limit snapshot via the Anthropic OAuth usage API.
//
// Claude Code does not persist rate-limit / reset-time information in its local
// transcripts (see docs/claude-code-usage-parsing-design.md §4), so unlike
// Codex we cannot derive a snapshot from disk. Instead we read the OAuth access
// token from `<config_dir>/.credentials.json` and call the undocumented
// `GET https://api.anthropic.com/api/oauth/usage` endpoint, which returns the
// 5-hour ("session") and 7-day ("weekly") window utilization plus reset times.
//
// Hard rules (do NOT relax without re-reading the task brief):
//   * Never refresh the token and never write to `.credentials.json`. A refresh
//     would rotate the refresh token and break the user's Claude Code login.
//   * If the token is expired, skip the call entirely (no snapshot).
//   * Any network / HTTP / parse failure is swallowed → no snapshot, so a
//     transient API outage never fails the whole dashboard parse.
//   * Never log or otherwise surface the token value.
// ---------------------------------------------------------------------------

/// 5-hour ("session") window length in minutes, mirrored into the shared
/// `RateLimitWindowSnapshot` so the frontend can render a window label.
const FIVE_HOUR_WINDOW_MINUTES: u64 = 5 * 60;
/// 7-day ("weekly") window length in minutes.
const SEVEN_DAY_WINDOW_MINUTES: u64 = 7 * 24 * 60;

/// Shape of `<config_dir>/.credentials.json`. We only read the fields we need;
/// everything else is ignored. We never write this file back.
#[derive(Debug, Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<OauthCredentials>,
}

#[derive(Debug, Deserialize)]
struct OauthCredentials {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    /// Expiry as a millisecond epoch.
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

/// One usage window in the `/api/oauth/usage` response. `resets_at` is an
/// ISO-8601 string here (Codex uses epoch seconds); we normalize on mapping.
#[derive(Debug, Deserialize)]
struct UsageWindow {
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
}

/// `extra_usage` block of the `/api/oauth/usage` response: pay-as-you-go
/// credits consumed once the plan windows are exhausted.
#[derive(Debug, Deserialize)]
struct ExtraUsage {
    #[serde(default)]
    is_enabled: bool,
    #[serde(default)]
    monthly_limit: Option<f64>,
    #[serde(default)]
    used_credits: Option<f64>,
    #[serde(default)]
    utilization: Option<f64>,
}

/// Subset of the `/api/oauth/usage` response we consume. The endpoint returns
/// many more (mostly-null) window keys; serde ignores unknown fields. Parse
/// leniently: Enterprise accounts can have `five_hour`/`seven_day` null with
/// only `extra_usage` populated (cship issue #173), and per-model weekly
/// windows are null unless the plan enforces them.
#[derive(Debug, Deserialize)]
struct OauthUsageResponse {
    #[serde(default)]
    five_hour: Option<UsageWindow>,
    #[serde(default)]
    seven_day: Option<UsageWindow>,
    #[serde(default)]
    seven_day_opus: Option<UsageWindow>,
    #[serde(default)]
    seven_day_sonnet: Option<UsageWindow>,
    #[serde(default)]
    extra_usage: Option<ExtraUsage>,
}

/// Read the OAuth access token (and plan metadata) for an account. Returns
/// `None` if the file is missing/unreadable, has no `claudeAiOauth` block, has
/// no access token, or the token is already expired.
fn read_oauth_credentials(account: &ClaudeAccount) -> Option<OauthCredentials> {
    let path = account.config_dir.join(".credentials.json");
    let text = std::fs::read_to_string(&path).ok()?;
    let parsed: CredentialsFile = serde_json::from_str(&text).ok()?;
    let oauth = parsed.claude_ai_oauth?;
    oauth.access_token.as_ref()?;

    if let Some(expires_at_ms) = oauth.expires_at {
        let now_ms = Utc::now().timestamp_millis();
        if expires_at_ms <= now_ms {
            // Expired: do not attempt the call (and never refresh).
            eprintln!(
                "claude_code/{}: OAuth token expired; skipping rate-limit fetch (no refresh by design).",
                account.name
            );
            return None;
        }
    }

    Some(oauth)
}

/// Map one API `UsageWindow` to the shared `RateLimitWindowSnapshot`, converting
/// the ISO-8601 `resets_at` to epoch seconds. Returns `None` if the window has
/// no utilization value.
fn to_window_snapshot(w: &UsageWindow, window_minutes: u64) -> Option<RateLimitWindowSnapshot> {
    let used_percent = w.utilization?;
    let resets_at = w
        .resets_at
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp())
        .unwrap_or(0);
    Some(RateLimitWindowSnapshot {
        used_percent,
        window_minutes,
        resets_at,
    })
}

/// Fetch a Claude Code rate-limit snapshot for one account from the Anthropic
/// OAuth usage API. Returns `None` on any failure (missing/expired token,
/// network error, non-2xx status, unparseable body) — callers treat that as
/// "no snapshot" rather than an error.
pub fn fetch_rate_limit_snapshot(account: &ClaudeAccount) -> Option<RateLimitSnapshot> {
    let oauth = read_oauth_credentials(account)?;
    let token = oauth.access_token.as_deref()?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;

    // The `User-Agent: claude-code/...` header matters: without it the endpoint
    // buckets requests aggressively and returns persistent 429s.
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "claude-code/1.0.0")
        .header("Content-Type", "application/json")
        .send()
        .ok()?;

    if !resp.status().is_success() {
        eprintln!(
            "claude_code/{}: OAuth usage API returned {}; skipping rate-limit snapshot.",
            account.name,
            resp.status()
        );
        return None;
    }

    let usage: OauthUsageResponse = resp.json().ok()?;

    let primary = usage
        .five_hour
        .as_ref()
        .and_then(|w| to_window_snapshot(w, FIVE_HOUR_WINDOW_MINUTES));
    let secondary = usage
        .seven_day
        .as_ref()
        .and_then(|w| to_window_snapshot(w, SEVEN_DAY_WINDOW_MINUTES));
    // Model-scoped weekly windows: null unless the plan enforces per-model caps.
    let seven_day_opus = usage
        .seven_day_opus
        .as_ref()
        .and_then(|w| to_window_snapshot(w, SEVEN_DAY_WINDOW_MINUTES));
    let seven_day_sonnet = usage
        .seven_day_sonnet
        .as_ref()
        .and_then(|w| to_window_snapshot(w, SEVEN_DAY_WINDOW_MINUTES));
    // Extra-usage credits: only surfaced when the account has them enabled.
    let extra_usage = usage
        .extra_usage
        .as_ref()
        .filter(|e| e.is_enabled)
        .map(|e| ExtraUsageSnapshot {
            monthly_limit: e.monthly_limit,
            used_credits: e.used_credits,
            utilization: e.utilization,
        });

    // Nothing usable came back — don't emit an empty snapshot. (Enterprise
    // accounts may have only extra_usage populated, which still counts.)
    if primary.is_none()
        && secondary.is_none()
        && seven_day_opus.is_none()
        && seven_day_sonnet.is_none()
        && extra_usage.is_none()
    {
        return None;
    }

    // plan_type: prefer subscriptionType (e.g. "pro"/"max"), fall back to the
    // rateLimitTier so there's always something to show.
    let plan_type = oauth
        .subscription_type
        .clone()
        .or_else(|| oauth.rate_limit_tier.clone());

    Some(RateLimitSnapshot {
        source: Source::ClaudeCode,
        account: account.name.clone(),
        observed_at: Utc::now(),
        limit_id: None,
        plan_type,
        rate_limit_reached_type: None,
        primary,
        secondary,
        seven_day_opus,
        seven_day_sonnet,
        extra_usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempFile;

    #[test]
    fn dedups_by_message_id_keeping_max_output_tokens() {
        let lines = vec![
            r#"{"isSidechain":false,"sessionId":"s1","timestamp":"2026-07-08T09:25:11.000Z","type":"assistant","message":{"id":"msg_1","model":"claude-fable-5","stop_reason":null,"usage":{"input_tokens":4557,"cache_creation_input_tokens":4173,"cache_read_input_tokens":16084,"output_tokens":1}}}"#,
            r#"{"isSidechain":false,"sessionId":"s1","timestamp":"2026-07-08T09:25:12.000Z","type":"assistant","message":{"id":"msg_1","model":"claude-fable-5","stop_reason":null,"usage":{"input_tokens":4557,"cache_creation_input_tokens":4173,"cache_read_input_tokens":16084,"output_tokens":500}}}"#,
            r#"{"isSidechain":false,"sessionId":"s1","timestamp":"2026-07-08T09:25:13.000Z","type":"assistant","message":{"id":"msg_1","model":"claude-fable-5","stop_reason":"tool_use","usage":{"input_tokens":4557,"cache_creation_input_tokens":4173,"cache_read_input_tokens":16084,"output_tokens":1155}}}"#,
        ];
        let tmp = TempFile::new(&lines);
        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", false, &mut records);

        assert_eq!(records.len(), 1, "all 3 lines are one message.id group");
        assert_eq!(records[0].output_tokens, 1155);
        assert_eq!(records[0].input_tokens, 4557);
    }

    #[test]
    fn handles_multiple_groups_and_broken_lines() {
        let lines = vec![
            r#"{"type":"user","timestamp":"2026-07-08T09:00:00.000Z"}"#, // non-assistant, skip
            r#"{ broken json"#,                                          // broken, skip
            r#"{"isSidechain":false,"sessionId":"s1","timestamp":"2026-07-08T09:25:11.000Z","type":"assistant","message":{"id":"msg_1","model":"m1","usage":{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":5}}}"#,
            r#"{"isSidechain":false,"sessionId":"s1","timestamp":"2026-07-08T09:26:00.000Z","type":"assistant","message":{"id":"msg_2","model":"m1","usage":{"input_tokens":20,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":8}}}"#,
        ];
        let tmp = TempFile::new(&lines);
        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", false, &mut records);

        assert_eq!(records.len(), 2);
        let total_input: u64 = records.iter().map(|r| r.input_tokens).sum();
        assert_eq!(total_input, 30);
    }

    #[test]
    fn tags_subagent_via_file_and_sidechain() {
        let lines_sidechain = vec![
            r#"{"isSidechain":true,"sessionId":"s1","timestamp":"2026-07-08T09:25:11.000Z","type":"assistant","message":{"id":"msg_1","model":"m1","usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1}}}"#,
        ];
        let tmp = TempFile::new(&lines_sidechain);
        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", false, &mut records);
        assert!(records[0].is_subagent);

        let mut records2 = Vec::new();
        parse_file(&tmp.path, "user01", true, &mut records2);
        assert!(records2[0].is_subagent);
    }

    #[test]
    fn parses_oauth_usage_response_and_maps_windows() {
        // Trimmed real /api/oauth/usage payload (extra window keys omitted;
        // serde ignores unknown fields).
        let body = r#"{
            "five_hour": {"utilization": 99.0, "resets_at": "2026-07-08T14:20:00.472088+00:00"},
            "seven_day": {"utilization": 49.0, "resets_at": "2026-07-14T00:00:00.472119+00:00"},
            "seven_day_opus": null,
            "seven_day_sonnet": {"utilization": 12.5, "resets_at": "2026-07-14T00:00:00+00:00"},
            "extra_usage": {"is_enabled": false, "monthly_limit": null, "used_credits": null, "utilization": null}
        }"#;
        let usage: OauthUsageResponse = serde_json::from_str(body).unwrap();

        let primary = usage
            .five_hour
            .as_ref()
            .and_then(|w| to_window_snapshot(w, FIVE_HOUR_WINDOW_MINUTES))
            .expect("five_hour window");
        assert_eq!(primary.used_percent, 99.0);
        assert_eq!(primary.window_minutes, 300);
        // 2026-07-08T14:20:00Z -> epoch seconds.
        assert_eq!(
            primary.resets_at,
            DateTime::parse_from_rfc3339("2026-07-08T14:20:00.472088+00:00")
                .unwrap()
                .timestamp()
        );

        let secondary = usage
            .seven_day
            .as_ref()
            .and_then(|w| to_window_snapshot(w, SEVEN_DAY_WINDOW_MINUTES))
            .expect("seven_day window");
        assert_eq!(secondary.used_percent, 49.0);
        assert_eq!(secondary.window_minutes, 10080);

        // Model-scoped weekly windows: opus null → skipped, sonnet mapped.
        assert!(usage.seven_day_opus.is_none());
        let sonnet = usage
            .seven_day_sonnet
            .as_ref()
            .and_then(|w| to_window_snapshot(w, SEVEN_DAY_WINDOW_MINUTES))
            .expect("seven_day_sonnet window");
        assert_eq!(sonnet.used_percent, 12.5);

        // extra_usage with is_enabled=false must be dropped.
        assert!(!usage.extra_usage.as_ref().unwrap().is_enabled);
    }

    #[test]
    fn parses_enabled_extra_usage_and_tolerates_null_windows() {
        // Enterprise-style payload: plan windows null, only extra_usage set
        // (see cship issue #173) — parsing must stay lenient.
        let body = r#"{
            "five_hour": null,
            "seven_day": null,
            "extra_usage": {"is_enabled": true, "monthly_limit": 5000, "used_credits": 123.45, "utilization": 2.5}
        }"#;
        let usage: OauthUsageResponse = serde_json::from_str(body).unwrap();
        assert!(usage.five_hour.is_none());
        assert!(usage.seven_day.is_none());
        let extra = usage.extra_usage.as_ref().unwrap();
        assert!(extra.is_enabled);
        assert_eq!(extra.monthly_limit, Some(5000.0));
        assert_eq!(extra.used_credits, Some(123.45));
        assert_eq!(extra.utilization, Some(2.5));
    }

    #[test]
    fn window_without_utilization_is_skipped() {
        let w = UsageWindow {
            utilization: None,
            resets_at: Some("2026-07-08T14:20:00+00:00".to_string()),
        };
        assert!(to_window_snapshot(&w, FIVE_HOUR_WINDOW_MINUTES).is_none());
    }
}
