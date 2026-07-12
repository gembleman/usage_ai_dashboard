//! Codex CLI session rollout parser.
//!
//! See docs/codex-usage-parsing-design.md for the full spec. Key rules:
//! - Only `last_token_usage` (delta) is summed, never `total_token_usage`
//!   (cumulative) — mixing the two double-counts.
//! - The current model comes from the most recent `turn_context` event.
//! - `info: null` token_count events are skipped.
//! - Token usage comes exclusively from the local session logs.
//! - Rate limits from session logs are retained and merged with app-server data.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::model::{RateLimitSnapshot, RateLimitWindowSnapshot, Source, UsageRecord};

/// One account's CODEX_HOME configuration.
#[derive(Debug, Clone)]
pub struct CodexAccount {
    pub name: String,
    pub codex_home: std::path::PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TokenCountInfo {
    #[serde(default)]
    last_token_usage: Option<TokenUsage>,
    #[allow(dead_code)]
    #[serde(default)]
    total_token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LogRateLimitWindow {
    #[serde(default)] used_percent: f64,
    #[serde(default)] window_minutes: u64,
    #[serde(default)] resets_at: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LogRateLimits {
    #[serde(default)] limit_id: Option<String>,
    #[serde(default)] plan_type: Option<String>,
    #[serde(default)] rate_limit_reached_type: Option<String>,
    #[serde(default)] primary: Option<LogRateLimitWindow>,
    #[serde(default)] secondary: Option<LogRateLimitWindow>,
}

#[derive(Debug, Deserialize)]
struct EventMsgPayload {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    info: Option<TokenCountInfo>,
    #[serde(default)]
    rate_limits: Option<LogRateLimits>,
    /// turn_context payload field
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RolloutLine {
    #[serde(default)]
    timestamp: Option<DateTime<Utc>>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

/// Result of parsing all sessions for one account.
#[derive(Debug, Default)]
pub struct CodexParseResult {
    pub records: Vec<UsageRecord>,
    pub rate_limit_snapshot: Option<RateLimitSnapshot>,
}

pub fn parse_account(account: &CodexAccount) -> CodexParseResult {
    let sessions_dir = account.codex_home.join("sessions");
    let mut records = Vec::new();
    let mut rate_limit_snapshot = None;

    if !sessions_dir.is_dir() {
        return CodexParseResult { records, rate_limit_snapshot };
    }

    for entry in WalkDir::new(&sessions_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let fname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !(fname.starts_with("rollout-") && fname.ends_with(".jsonl")) {
            continue;
        }

        parse_file(path, &account.name, &mut records, &mut rate_limit_snapshot);
    }

    CodexParseResult { records, rate_limit_snapshot }
}

fn parse_file(path: &Path, account: &str, records: &mut Vec<UsageRecord>, latest_snapshot: &mut Option<RateLimitSnapshot>) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = BufReader::new(file);

    let mut current_model: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: RolloutLine = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // skip broken/partial lines
        };

        let Some(timestamp) = parsed.timestamp else {
            continue;
        };

        match parsed.kind.as_deref() {
            Some("turn_context") => {
                if let Some(payload) = parsed.payload {
                    if let Ok(tc) = serde_json::from_value::<EventMsgPayload>(payload) {
                        if let Some(model) = tc.model {
                            current_model = Some(model);
                        }
                    }
                }
            }
            Some("event_msg") => {
                let Some(payload) = parsed.payload else {
                    continue;
                };
                let ev: EventMsgPayload = match serde_json::from_value(payload) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if ev.kind.as_deref() != Some("token_count") {
                    continue;
                }

                // info: null -> skip (immediately-terminated session)
                let Some(info) = ev.info else {
                    continue;
                };
                if let Some(delta) = info.last_token_usage {
                    // OpenAI's `input_tokens` is cache-inclusive (cached_input_tokens
                    // is a subset, not additive) — e.g. input_tokens=16296,
                    // cached_input_tokens=11648, output_tokens=776,
                    // total_tokens=17072=input+output. Normalize to the
                    // claude_code convention (input_tokens excludes cache) so
                    // downstream cost estimation doesn't double-count the
                    // cached portion at both full price and cache-read price.
                    let non_cached_input =
                        delta.input_tokens.saturating_sub(delta.cached_input_tokens);
                    records.push(UsageRecord {
                        source: Source::Codex,
                        account: account.to_string(),
                        timestamp,
                        model: current_model.clone(),
                        input_tokens: non_cached_input,
                        cached_input_tokens: delta.cached_input_tokens,
                        // Codex has no cache-creation concept; its cache tokens
                        // are all reads (cached_input_tokens above).
                        cache_creation_input_tokens: 0,
                        output_tokens: delta.output_tokens,
                        reasoning_output_tokens: delta.reasoning_output_tokens,
                        total_tokens: delta.total_tokens,
                        is_subagent: false,
                    });
                }

                if let Some(rl) = ev.rate_limits {
                    if (rl.primary.is_some() || rl.secondary.is_some())
                        && latest_snapshot.as_ref().map(|s| timestamp > s.observed_at).unwrap_or(true)
                    {
                        let window = |w: LogRateLimitWindow| RateLimitWindowSnapshot {
                            used_percent: w.used_percent,
                            window_minutes: w.window_minutes,
                            resets_at: w.resets_at,
                        };
                        let incoming = RateLimitSnapshot {
                            source: Source::Codex,
                            account: account.to_string(),
                            observed_at: timestamp,
                            limit_id: rl.limit_id,
                            plan_type: rl.plan_type,
                            rate_limit_reached_type: rl.rate_limit_reached_type,
                            primary: rl.primary.map(window),
                            secondary: rl.secondary.map(window),
                            seven_day_opus: None,
                            seven_day_sonnet: None,
                            extra_usage: None,
                        };
                        // Newer Codex versions can emit a partial snapshot
                        // containing only the weekly window. Keep windows of
                        // other durations from the preceding log snapshot so
                        // a weekly-only event does not erase the 5-hour quota.
                        *latest_snapshot = merge_rate_limit_snapshots(
                            latest_snapshot.take(),
                            Some(incoming),
                        );
                    }
                }
            }
            _ => {} // response_item, session_meta, and anything else is ignored
        }
    }
}

/// Merge app-server data with the latest session-log snapshot. Some current
/// app-server versions expose only the weekly window, while logs still carry
/// both the 5-hour and weekly quotas.
pub fn merge_rate_limit_snapshots(
    log: Option<RateLimitSnapshot>,
    live: Option<RateLimitSnapshot>,
) -> Option<RateLimitSnapshot> {
    let mut merged = live.or(log.clone())?;
    if let Some(log) = log {
        let mut windows = [merged.primary.take(), merged.secondary.take()]
            .into_iter().flatten().collect::<Vec<_>>();
        for w in [log.primary, log.secondary].into_iter().flatten() {
            if !windows.iter().any(|existing| existing.window_minutes == w.window_minutes) {
                windows.push(w);
            }
        }
        windows.sort_by_key(|w| w.window_minutes);
        merged.primary = windows.first().cloned();
        merged.secondary = windows.get(1).cloned();
    }
    Some(merged)
}

#[derive(Debug, Serialize)]
struct AppServerRequest<'a> {
    id: u64,
    method: &'a str,
    params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppServerRateLimitWindow {
    used_percent: f64,
    window_duration_mins: Option<u64>,
    resets_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppServerRateLimits {
    limit_id: Option<String>,
    plan_type: Option<String>,
    rate_limit_reached_type: Option<String>,
    primary: Option<AppServerRateLimitWindow>,
    secondary: Option<AppServerRateLimitWindow>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppServerRateLimitsResponse {
    rate_limits: AppServerRateLimits,
}

fn app_server_snapshot(
    account: &CodexAccount,
    response: AppServerRateLimitsResponse,
) -> Option<RateLimitSnapshot> {
    let rl = response.rate_limits;
    if rl.primary.is_none() && rl.secondary.is_none() {
        return None;
    }
    let window = |w: AppServerRateLimitWindow| RateLimitWindowSnapshot {
        used_percent: w.used_percent,
        window_minutes: w.window_duration_mins.unwrap_or_default(),
        resets_at: w.resets_at.unwrap_or_default(),
    };
    Some(RateLimitSnapshot {
        source: Source::Codex,
        account: account.name.clone(),
        observed_at: Utc::now(),
        limit_id: rl.limit_id,
        plan_type: rl.plan_type,
        rate_limit_reached_type: rl.rate_limit_reached_type,
        primary: rl.primary.map(window),
        secondary: rl.secondary.map(window),
        seven_day_opus: None,
        seven_day_sonnet: None,
        extra_usage: None,
    })
}

/// Fetch the current ChatGPT Codex rate-limit windows through the local Codex
/// app-server protocol. Token usage is deliberately not read through this API.
pub fn fetch_rate_limit_snapshot(
    account: &CodexAccount,
    timeout_seconds: u64,
) -> Option<RateLimitSnapshot> {
    let mut child = match Command::new("codex")
        .args(["app-server", "--stdio"])
        .env("CODEX_HOME", &account.codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("codex/{}: failed to start app-server: {e}", account.name);
            return None;
        }
    };

    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let initialize = AppServerRequest {
        id: 1,
        method: "initialize",
        params: serde_json::json!({
            "clientInfo": {
                "name": "usage_ai_dashboard",
                "title": "Usage AI Dashboard",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    };
    let read_limits = AppServerRequest {
        id: 2,
        method: "account/rateLimits/read",
        params: serde_json::Value::Null,
    };

    use std::io::Write;
    let write_result = (|| -> std::io::Result<()> {
        serde_json::to_writer(&mut stdin, &initialize)?;
        writeln!(stdin)?;
        writeln!(stdin, "{{\"method\":\"initialized\"}}")?;
        serde_json::to_writer(&mut stdin, &read_limits)?;
        writeln!(stdin)?;
        stdin.flush()
    })();
    if let Err(e) = write_result {
        eprintln!("codex/{}: failed to query app-server: {e}", account.name);
        let _ = child.kill();
        return None;
    }

    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_seconds);
    let mut snapshot = None;
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(remaining) {
            Ok(Ok(line)) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if value.get("id").and_then(|id| id.as_u64()) != Some(2) {
                    continue;
                }
                if let Some(error) = value.get("error") {
                    eprintln!(
                        "codex/{}: app-server rate-limit error: {error}",
                        account.name
                    );
                    break;
                }
                if let Some(result) = value.get("result") {
                    match serde_json::from_value::<AppServerRateLimitsResponse>(result.clone()) {
                        Ok(response) => snapshot = app_server_snapshot(account, response),
                        Err(e) => eprintln!(
                            "codex/{}: invalid app-server rate-limit response: {e}",
                            account.name
                        ),
                    }
                }
                break;
            }
            Ok(Err(e)) => {
                eprintln!(
                    "codex/{}: failed reading app-server response: {e}",
                    account.name
                );
                break;
            }
            Err(_) => {
                eprintln!(
                    "codex/{}: app-server rate-limit query timed out",
                    account.name
                );
                break;
            }
        }
    }
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempFile;

    fn write_temp_jsonl(lines: &[&str]) -> TempFile {
        TempFile::new(lines)
    }

    #[test]
    fn sums_delta_not_cumulative() {
        let lines = vec![
            r#"{"timestamp":"2026-06-26T15:00:00.000Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
            r#"{"timestamp":"2026-06-26T15:01:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":0,"total_tokens":110},"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":0,"total_tokens":110}},"rate_limits":null}}"#,
            r#"{"timestamp":"2026-06-26T15:02:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":250,"cached_input_tokens":0,"output_tokens":30,"reasoning_output_tokens":0,"total_tokens":280},"last_token_usage":{"input_tokens":150,"cached_input_tokens":0,"output_tokens":20,"reasoning_output_tokens":0,"total_tokens":170}},"rate_limits":null}}"#,
        ];
        let lines_ref: Vec<&str> = lines.iter().map(|s| s.as_ref()).collect();
        let tmp = write_temp_jsonl(&lines_ref);

        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", &mut records, &mut None);

        assert_eq!(records.len(), 2);
        let total_input: u64 = records.iter().map(|r| r.input_tokens).sum();
        let total_total: u64 = records.iter().map(|r| r.total_tokens).sum();
        // Sum of deltas: 100 + 150 = 250, NOT the cumulative 280.
        assert_eq!(total_input, 250);
        assert_eq!(total_total, 280);
        assert_eq!(records[0].model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn normalizes_input_tokens_to_exclude_cache() {
        // OpenAI's input_tokens is cache-inclusive: cached_input_tokens is a
        // subset, not additive (input_tokens=16296, cached=11648,
        // output=776, total=17072=input+output). We store input_tokens as
        // the non-cached remainder so cost estimation doesn't double-count.
        let lines = vec![
            r#"{"timestamp":"2026-06-26T15:01:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":16296,"cached_input_tokens":11648,"output_tokens":776,"reasoning_output_tokens":516,"total_tokens":17072}},"rate_limits":null}}"#,
        ];
        let tmp = write_temp_jsonl(&lines);
        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", &mut records, &mut None);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_tokens, 16296 - 11648);
        assert_eq!(records[0].cached_input_tokens, 11648);
        assert_eq!(records[0].total_tokens, 17072); // original total left untouched
    }

    #[test]
    fn skips_null_info() {
        let lines = vec![
            r#"{"timestamp":"2026-06-26T15:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":null}}"#,
        ];
        let tmp = write_temp_jsonl(&lines);
        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", &mut records, &mut None);
        assert_eq!(records.len(), 0);
    }

    #[test]
    fn skips_broken_lines_and_continues() {
        let lines = vec![
            r#"{ this is not valid json"#,
            r#"{"timestamp":"2026-06-26T15:01:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":5,"cached_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":6}},"rate_limits":null}}"#,
        ];
        let tmp = write_temp_jsonl(&lines);
        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", &mut records, &mut None);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_tokens, 5);
    }

    #[test]
    fn newer_weekly_only_log_snapshot_preserves_session_window() {
        let lines = vec![
            r#"{"timestamp":"2026-06-26T15:01:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":5,"cached_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":6}},"rate_limits":{"limit_id":"codex","plan_type":"team","primary":{"used_percent":44.0,"window_minutes":300,"resets_at":1500},"secondary":{"used_percent":22.0,"window_minutes":10080,"resets_at":2500}}}}"#,
            r#"{"timestamp":"2026-06-26T15:02:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":5,"cached_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":6}},"rate_limits":{"limit_id":"codex","plan_type":"team","primary":{"used_percent":23.0,"window_minutes":10080,"resets_at":2600},"secondary":null}}}"#,
        ];
        let tmp = write_temp_jsonl(&lines);
        let mut records = Vec::new();
        let mut snapshot = None;
        parse_file(&tmp.path, "user02", &mut records, &mut snapshot);

        let snapshot = snapshot.unwrap();
        assert_eq!(snapshot.observed_at.to_rfc3339(), "2026-06-26T15:02:00+00:00");
        assert_eq!(snapshot.primary.as_ref().unwrap().window_minutes, 300);
        assert_eq!(snapshot.primary.as_ref().unwrap().used_percent, 44.0);
        assert_eq!(snapshot.secondary.as_ref().unwrap().window_minutes, 10080);
        assert_eq!(snapshot.secondary.as_ref().unwrap().used_percent, 23.0);
    }

    #[test]
    fn converts_app_server_rate_limit_response() {
        let account = CodexAccount {
            name: "user01".into(),
            codex_home: ".".into(),
        };
        let response: AppServerRateLimitsResponse = serde_json::from_value(serde_json::json!({
            "rateLimits": {
                "limitId": "codex",
                "planType": "team",
                "primary": { "usedPercent": 44, "windowDurationMins": 300, "resetsAt": 1500 },
                "secondary": { "usedPercent": 99, "windowDurationMins": 10080, "resetsAt": 2500 }
            }
        }))
        .unwrap();
        let snap = app_server_snapshot(&account, response).unwrap();
        assert_eq!(snap.limit_id.as_deref(), Some("codex"));
        assert_eq!(snap.primary.unwrap().used_percent, 44.0);
        assert_eq!(snap.secondary.unwrap().window_minutes, 10080);
    }
}
