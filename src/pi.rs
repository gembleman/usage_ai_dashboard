//! pi code agent session log parser.
//!
//! Layout: `<pi_home>/agent/sessions/<cwd-slug>/<timestamp>_<uuid>.jsonl`.
//! Each line is an event carrying a `type` discriminator; only
//! `type == "message"` lines with an assistant `message.usage` object hold
//! token counts.
//!
//! Two rules matter for correctness:
//!
//! 1. **De-duplicate by record `id` across the whole account.** Resuming a
//!    session copies the prior conversation — including its assistant
//!    messages, verbatim with the same `id`, timestamp and usage — into the
//!    new session file. Summing every file independently over-counts by
//!    ~48% on real logs, so the account-wide `id` set below is what keeps
//!    the totals honest. (Claude Code's parser groups by `message.id`
//!    *within* a file; pi needs the wider scope.)
//! 2. **Trust the logged `cost`.** pi bills through third-party providers
//!    (opencode-go and friends), whose per-model prices `config.toml` does
//!    not track. pi already records the charged amount per message, so it is
//!    carried through as `cost_usd` instead of being re-derived from
//!    `model_pricing`.
//!
//! Unlike Codex and Claude Code, pi exposes no rate-limit or quota
//! information in its logs, so this module produces usage records only.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use walkdir::WalkDir;

use crate::model::{Source, UsageRecord};

/// One account's pi home directory (the parent of `agent/sessions`).
#[derive(Debug, Clone)]
pub struct PiAccount {
    pub name: String,
    pub pi_home: PathBuf,
}

/// Per-message cost breakdown pi computes from the provider's price list.
#[derive(Debug, Clone, Default, Deserialize)]
struct PiCost {
    #[serde(default)]
    total: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PiUsage {
    #[serde(default)]
    input: u64,
    #[serde(default)]
    output: u64,
    #[serde(default)]
    cache_read: u64,
    #[serde(default)]
    cache_write: u64,
    /// Thinking tokens; a subset of `output`, reported separately.
    #[serde(default)]
    reasoning: u64,
    #[serde(default)]
    total_tokens: u64,
    #[serde(default)]
    cost: Option<PiCost>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PiMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<PiUsage>,
}

#[derive(Debug, Deserialize)]
struct PiLine {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    timestamp: Option<DateTime<Utc>>,
    #[serde(default)]
    message: Option<PiMessage>,
}

#[derive(Debug, Default)]
pub struct PiParseResult {
    pub records: Vec<UsageRecord>,
}

pub fn parse_account(account: &PiAccount) -> PiParseResult {
    let sessions_dir = account.pi_home.join("agent").join("sessions");
    let mut records = Vec::new();

    if !sessions_dir.is_dir() {
        return PiParseResult { records };
    }

    // Shared across every file in the account: a resumed session re-emits the
    // messages it inherited, so the same `id` legitimately shows up in
    // several files and must only be counted once.
    let mut seen_ids: HashSet<String> = HashSet::new();

    // Resumed sessions copy earlier messages forward, so the *newest* file
    // holding a given id is a superset of the older ones. Walking in a
    // deterministic order (by path, which begins with the ISO timestamp)
    // keeps which copy wins stable from run to run — the usage payload is
    // identical either way.
    let mut paths: Vec<PathBuf> = WalkDir::new(&sessions_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .map(|e| e.into_path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect();
    paths.sort();

    for path in &paths {
        parse_file(path, &account.name, &mut seen_ids, &mut records);
    }

    PiParseResult { records }
}

fn parse_file(
    path: &Path,
    account: &str,
    seen_ids: &mut HashSet<String>,
    records: &mut Vec<UsageRecord>,
) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: PiLine = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // skip broken/partial lines (in-progress writes)
        };

        if parsed.kind.as_deref() != Some("message") {
            continue; // session / model_change / thinking_level_change
        }
        let Some(message) = parsed.message else {
            continue;
        };
        if message.role.as_deref() != Some("assistant") {
            continue; // user and tool messages carry no usage
        }
        let Some(usage) = message.usage else {
            continue;
        };
        let Some(timestamp) = parsed.timestamp else {
            continue;
        };

        // Records without an id cannot be de-duplicated; skipping them is the
        // safer default given resumes replay whole conversations.
        let Some(id) = parsed.id else {
            continue;
        };
        if !seen_ids.insert(id) {
            continue;
        }

        // Failed requests (e.g. provider 403s) are logged with an all-zero
        // usage block. They are not billed, so they would only add empty rows.
        if usage.total_tokens == 0 && usage.input == 0 && usage.output == 0 {
            continue;
        }

        records.push(UsageRecord {
            source: Source::Pi,
            account: account.to_string(),
            timestamp,
            model: message.model,
            cost_usd: usage.cost.as_ref().and_then(|c| c.total),
            // pi reports `input` exclusive of the cached portion (cacheRead
            // and cacheWrite are counted separately), which already matches
            // the convention the dashboard normalizes Codex records into.
            input_tokens: usage.input,
            cached_input_tokens: usage.cache_read,
            cache_creation_input_tokens: usage.cache_write,
            output_tokens: usage.output,
            reasoning_output_tokens: usage.reasoning,
            total_tokens: usage.total_tokens,
            // pi has no subagent concept in its session logs.
            is_subagent: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempFile;

    const MSG: &str = r#"{"type":"message","id":"a1","timestamp":"2026-08-05T01:30:00.000Z","message":{"role":"assistant","model":"deepseek-v4-flash","usage":{"input":1285,"output":253,"cacheRead":2432,"cacheWrite":10,"reasoning":83,"totalTokens":3970,"cost":{"input":0.0001799,"output":0.00007084,"cacheRead":0.0000068096,"cacheWrite":0,"total":0.0002575496}}}}"#;

    #[test]
    fn parses_usage_and_logged_cost() {
        let tmp = TempFile::new(&[MSG]);
        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", &mut HashSet::new(), &mut records);

        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.source, Source::Pi);
        assert_eq!(r.model.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(r.input_tokens, 1285);
        assert_eq!(r.cached_input_tokens, 2432);
        assert_eq!(r.cache_creation_input_tokens, 10);
        assert_eq!(r.output_tokens, 253);
        assert_eq!(r.reasoning_output_tokens, 83);
        assert_eq!(r.total_tokens, 3970);
        assert_eq!(r.cost_usd, Some(0.0002575496));
    }

    #[test]
    fn deduplicates_ids_across_files() {
        // Resuming a session copies earlier messages verbatim into the new
        // file. Counting both copies inflated real totals by ~48%.
        let first = TempFile::new(&[MSG]);
        let second = TempFile::new(&[MSG]);

        let mut seen = HashSet::new();
        let mut records = Vec::new();
        parse_file(&first.path, "user01", &mut seen, &mut records);
        parse_file(&second.path, "user01", &mut seen, &mut records);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].total_tokens, 3970);
    }

    #[test]
    fn skips_non_assistant_and_non_message_lines() {
        let lines = vec![
            r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-08-05T01:28:20.608Z","cwd":"C:\\x"}"#,
            r#"{"type":"model_change","id":"m1","timestamp":"2026-08-05T01:28:52.784Z","provider":"opencode-go","modelId":"kimi-k2.6"}"#,
            r#"{"type":"message","id":"u1","timestamp":"2026-08-05T01:29:00.000Z","message":{"role":"user","content":[]}}"#,
            MSG,
        ];
        let tmp = TempFile::new(&lines);
        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", &mut HashSet::new(), &mut records);
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn skips_zero_usage_error_responses() {
        // Provider errors are logged with an all-zero usage block and no charge.
        let lines = vec![
            r#"{"type":"message","id":"e1","timestamp":"2026-08-05T01:30:33.382Z","message":{"role":"assistant","model":"deepseek-v4-flash","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"total":0}},"stopReason":"error"}}"#,
        ];
        let tmp = TempFile::new(&lines);
        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", &mut HashSet::new(), &mut records);
        assert_eq!(records.len(), 0);
    }

    #[test]
    fn skips_broken_lines_and_continues() {
        let lines = vec![r#"{ not valid json"#, MSG];
        let tmp = TempFile::new(&lines);
        let mut records = Vec::new();
        parse_file(&tmp.path, "user01", &mut HashSet::new(), &mut records);
        assert_eq!(records.len(), 1);
    }
}
