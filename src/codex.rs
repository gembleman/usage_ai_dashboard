//! Codex CLI session rollout parser.
//!
//! See docs/codex-usage-parsing-design.md for the full spec. Key rules:
//! - Only `last_token_usage` (delta) is summed, never `total_token_usage`
//!   (cumulative) — mixing the two double-counts.
//! - The current model comes from the most recent `turn_context` event.
//! - `info: null` token_count events are skipped.
//! - Per-account rate limit snapshot = latest non-null `rate_limits` seen
//!   across all sessions.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;
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
pub struct RateLimitWindow {
    #[serde(default)]
    pub used_percent: f64,
    #[serde(default)]
    pub window_minutes: u64,
    #[serde(default)]
    pub resets_at: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RateLimits {
    #[serde(default)]
    pub limit_id: Option<String>,
    #[serde(default)]
    pub primary: Option<RateLimitWindow>,
    #[serde(default)]
    pub secondary: Option<RateLimitWindow>,
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub rate_limit_reached_type: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TokenCountInfo {
    #[serde(default)]
    last_token_usage: Option<TokenUsage>,
    #[allow(dead_code)]
    #[serde(default)]
    total_token_usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
struct EventMsgPayload {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    info: Option<TokenCountInfo>,
    #[serde(default)]
    rate_limits: Option<RateLimits>,
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
    let mut latest_snapshot: Option<RateLimitSnapshot> = None;

    if !sessions_dir.is_dir() {
        return CodexParseResult {
            records,
            rate_limit_snapshot: latest_snapshot,
        };
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

        parse_file(path, &account.name, &mut records, &mut latest_snapshot);
    }

    CodexParseResult {
        records,
        rate_limit_snapshot: latest_snapshot,
    }
}

fn parse_file(
    path: &Path,
    account: &str,
    records: &mut Vec<UsageRecord>,
    latest_snapshot: &mut Option<RateLimitSnapshot>,
) {
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
                    records.push(UsageRecord {
                        source: Source::Codex,
                        account: account.to_string(),
                        timestamp,
                        model: current_model.clone(),
                        input_tokens: delta.input_tokens,
                        cached_input_tokens: delta.cached_input_tokens,
                        output_tokens: delta.output_tokens,
                        reasoning_output_tokens: delta.reasoning_output_tokens,
                        total_tokens: delta.total_tokens,
                        is_subagent: false,
                    });
                }

                if let Some(rl) = ev.rate_limits {
                    // Only keep non-null primary/secondary snapshots, and
                    // only if this event is newer than what we have.
                    if rl.primary.is_some() || rl.secondary.is_some() {
                        let is_newer = latest_snapshot
                            .as_ref()
                            .map(|s| timestamp > s.observed_at)
                            .unwrap_or(true);
                        if is_newer {
                            *latest_snapshot = Some(RateLimitSnapshot {
                                account: account.to_string(),
                                observed_at: timestamp,
                                limit_id: rl.limit_id,
                                plan_type: rl.plan_type,
                                rate_limit_reached_type: rl.rate_limit_reached_type,
                                primary: rl.primary.map(|w| RateLimitWindowSnapshot {
                                    used_percent: w.used_percent,
                                    window_minutes: w.window_minutes,
                                    resets_at: w.resets_at,
                                }),
                                secondary: rl.secondary.map(|w| RateLimitWindowSnapshot {
                                    used_percent: w.used_percent,
                                    window_minutes: w.window_minutes,
                                    resets_at: w.resets_at,
                                }),
                            });
                        }
                    }
                }
            }
            _ => {} // response_item, session_meta, and anything else is ignored
        }
    }
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
        let mut snapshot = None;
        parse_file(&tmp.path, "user01", &mut records, &mut snapshot);

        assert_eq!(records.len(), 2);
        let total_input: u64 = records.iter().map(|r| r.input_tokens).sum();
        let total_total: u64 = records.iter().map(|r| r.total_tokens).sum();
        // Sum of deltas: 100 + 150 = 250, NOT the cumulative 280.
        assert_eq!(total_input, 250);
        assert_eq!(total_total, 280);
        assert_eq!(records[0].model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn skips_null_info() {
        let lines = vec![
            r#"{"timestamp":"2026-06-26T15:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":null}}"#,
        ];
        let tmp = write_temp_jsonl(&lines);
        let mut records = Vec::new();
        let mut snapshot = None;
        parse_file(&tmp.path, "user01", &mut records, &mut snapshot);
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
        let mut snapshot = None;
        parse_file(&tmp.path, "user01", &mut records, &mut snapshot);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_tokens, 5);
    }

    #[test]
    fn captures_latest_rate_limit_snapshot() {
        let lines = vec![
            r#"{"timestamp":"2026-06-26T15:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":1}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":10.0,"window_minutes":300,"resets_at":1000},"secondary":{"used_percent":20.0,"window_minutes":10080,"resets_at":2000},"plan_type":"team"}}}"#,
            r#"{"timestamp":"2026-06-26T16:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":1}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":44.0,"window_minutes":300,"resets_at":1500},"secondary":{"used_percent":99.0,"window_minutes":10080,"resets_at":2500},"plan_type":"team"}}}"#,
            r#"{"timestamp":"2026-06-26T17:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":1}},"rate_limits":{"limit_id":"premium","primary":null,"secondary":null,"credits":{},"plan_type":"premium"}}}"#,
        ];
        let tmp = write_temp_jsonl(&lines);
        let mut records = Vec::new();
        let mut snapshot = None;
        parse_file(&tmp.path, "user01", &mut records, &mut snapshot);
        let snap = snapshot.unwrap();
        // The premium null-limits event is newer but must be skipped;
        // last non-null snapshot (16:00) wins.
        assert_eq!(snap.primary.unwrap().used_percent, 44.0);
        assert_eq!(snap.secondary.unwrap().used_percent, 99.0);
    }
}
