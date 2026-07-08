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

use crate::model::{Source, UsageRecord};

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
}
