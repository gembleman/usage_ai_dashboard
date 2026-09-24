//! OpenCode session usage parser.
//!
//! Current OpenCode releases persist assistant messages in one or more
//! `opencode*.db` SQLite databases under the data directory. Older releases
//! used `storage/message/<session-id>/<message-id>.json`; both may coexist
//! after an upgrade, so this parser reads both and de-duplicates them.
//!
//! A fork can copy an already-billed assistant message under a new message ID.
//! OpenCode preserves the original timestamps and usage payload in that case,
//! so the usage fingerprint below prevents copied history from being billed a
//! second time while retaining genuinely new turns.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use time::OffsetDateTime;
use walkdir::WalkDir;

use crate::model::{Source, UsageRecord};

#[derive(Debug, Clone)]
pub struct OpenCodeAccount {
    pub name: String,
    /// Usually `~/.local/share/opencode`. A direct `.db` path is accepted too.
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OpenCodeCacheTokens {
    #[serde(default)]
    read: u64,
    #[serde(default)]
    write: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OpenCodeTokens {
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    input: u64,
    #[serde(default)]
    output: u64,
    #[serde(default)]
    reasoning: u64,
    #[serde(default)]
    cache: OpenCodeCacheTokens,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OpenCodeTime {
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    completed: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenCodeMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    time: OpenCodeTime,
    #[serde(default, rename = "modelID")]
    model_id: Option<String>,
    #[serde(default, rename = "providerID")]
    provider_id: Option<String>,
    #[serde(default)]
    cost: Option<f64>,
    #[serde(default)]
    tokens: Option<OpenCodeTokens>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UsageFingerprint {
    created_ms: i64,
    completed_ms: Option<i64>,
    provider: Option<String>,
    model: Option<String>,
    cost_bits: Option<u64>,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
}

#[derive(Debug, Default)]
struct Seen {
    ids: HashSet<String>,
    fingerprints: HashSet<UsageFingerprint>,
}

#[derive(Debug, Default)]
pub struct OpenCodeParseResult {
    pub records: Vec<UsageRecord>,
}

pub fn parse_account(account: &OpenCodeAccount) -> OpenCodeParseResult {
    let mut records = Vec::new();
    let mut seen = Seen::default();

    // DB first: when a message exists in both formats, the current database
    // representation is authoritative.
    for db_path in database_paths(&account.data_dir) {
        parse_database(&db_path, &account.name, &mut seen, &mut records);
    }

    let legacy_dir = if account.data_dir.is_file() {
        account
            .data_dir
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("storage")
            .join("message")
    } else {
        account.data_dir.join("storage").join("message")
    };
    parse_legacy_messages(&legacy_dir, &account.name, &mut seen, &mut records);

    records.sort_by_key(|r| r.timestamp);
    OpenCodeParseResult { records }
}

fn database_paths(data_dir: &Path) -> Vec<PathBuf> {
    if data_dir.is_file() {
        return vec![data_dir.to_path_buf()];
    }

    let mut paths = std::fs::read_dir(data_dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|e| e.to_str()) == Some("db")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|name| name == "opencode" || name.starts_with("opencode-"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn parse_database(path: &Path, account: &str, seen: &mut Seen, records: &mut Vec<UsageRecord>) {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = match Connection::open_with_flags(path, flags) {
        Ok(conn) => conn,
        Err(_) => return,
    };
    let mut stmt = match conn.prepare("SELECT id, time_created, data FROM message") {
        Ok(stmt) => stmt,
        Err(_) => return,
    };
    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return,
    };

    for row in rows.flatten() {
        let (id, created_ms, json) = row;
        parse_message_json(&json, Some(id), Some(created_ms), account, seen, records);
    }
}

fn parse_legacy_messages(
    messages_dir: &Path,
    account: &str,
    seen: &mut Seen,
    records: &mut Vec<UsageRecord>,
) {
    if !messages_dir.is_dir() {
        return;
    }
    let mut paths = WalkDir::new(messages_dir)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.into_path())
        .filter(|path| path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        let Ok(json) = std::fs::read_to_string(path) else {
            continue;
        };
        parse_message_json(&json, None, None, account, seen, records);
    }
}

fn parse_message_json(
    json: &str,
    fallback_id: Option<String>,
    fallback_created_ms: Option<i64>,
    account: &str,
    seen: &mut Seen,
    records: &mut Vec<UsageRecord>,
) {
    let Ok(message) = serde_json::from_str::<OpenCodeMessage>(json) else {
        return;
    };
    if message.role.as_deref() != Some("assistant") {
        return;
    }
    let Some(tokens) = message.tokens else {
        return;
    };
    let Some(created_ms) = message.time.created.or(fallback_created_ms) else {
        return;
    };
    let Ok(timestamp) =
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(created_ms) * 1_000_000)
    else {
        return;
    };

    let cost = message
        .cost
        .filter(|value| value.is_finite() && *value >= 0.0);
    let calculated_total = tokens
        .input
        .saturating_add(tokens.output)
        .saturating_add(tokens.reasoning)
        .saturating_add(tokens.cache.read)
        .saturating_add(tokens.cache.write);
    // Some in-progress rows briefly carry `total: 0`; use the categories if
    // they already contain completed usage.
    let total = tokens
        .total
        .filter(|total| *total > 0)
        .unwrap_or(calculated_total);
    if total == 0 && calculated_total == 0 && cost.unwrap_or(0.0) == 0.0 {
        return; // pending/failed assistant placeholders
    }

    let id = message.id.or(fallback_id);
    if id.as_ref().is_some_and(|id| seen.ids.contains(id)) {
        return;
    }
    let fingerprint = UsageFingerprint {
        created_ms,
        completed_ms: message.time.completed,
        provider: message.provider_id,
        model: message.model_id.clone(),
        cost_bits: cost.map(f64::to_bits),
        input: tokens.input,
        output: tokens.output,
        reasoning: tokens.reasoning,
        cache_read: tokens.cache.read,
        cache_write: tokens.cache.write,
    };
    if !seen.fingerprints.insert(fingerprint) {
        return;
    }
    if let Some(id) = id {
        seen.ids.insert(id);
    }

    records.push(UsageRecord {
        source: Source::OpenCode,
        account: account.to_string(),
        timestamp,
        model: message.model_id,
        // OpenCode calculates and persists this per request, including for
        // providers whose model prices are not in our config.
        cost_usd: cost,
        input_tokens: tokens.input,
        cached_input_tokens: tokens.cache.read,
        cache_creation_input_tokens: tokens.cache.write,
        output_tokens: tokens.output,
        reasoning_output_tokens: tokens.reasoning,
        total_tokens: total,
        is_subagent: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const MESSAGE: &str = r#"{"id":"msg_1","role":"assistant","time":{"created":1765915142201,"completed":1765915146360},"modelID":"z-ai/glm-4.6","providerID":"openrouter","cost":0.0025158,"tokens":{"total":11204,"input":2675,"output":28,"reasoning":1,"cache":{"read":7700,"write":800}}}"#;

    struct TempDb(PathBuf);

    impl TempDb {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "usage_ai_opencode_test_{}_{}.db",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            Self(path)
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn parses_logged_usage_and_cost() {
        let mut seen = Seen::default();
        let mut records = Vec::new();
        parse_message_json(MESSAGE, None, None, "user01", &mut seen, &mut records);

        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.source, Source::OpenCode);
        assert_eq!(r.model.as_deref(), Some("z-ai/glm-4.6"));
        assert_eq!(r.input_tokens, 2675);
        assert_eq!(r.output_tokens, 28);
        assert_eq!(r.reasoning_output_tokens, 1);
        assert_eq!(r.cached_input_tokens, 7700);
        assert_eq!(r.cache_creation_input_tokens, 800);
        assert_eq!(r.total_tokens, 11204);
        assert_eq!(r.cost_usd, Some(0.0025158));
    }

    #[test]
    fn reads_sqlite_message_data_with_column_fallbacks() {
        let db = TempDb::new();
        let conn = Connection::open(&db.0).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT PRIMARY KEY, time_created INTEGER NOT NULL, data TEXT NOT NULL);",
        )
        .unwrap();
        let without_id_or_time = r#"{"role":"assistant","modelID":"m","providerID":"p","cost":0.25,"tokens":{"input":10,"output":2,"reasoning":1,"cache":{"read":4,"write":0}}}"#;
        conn.execute(
            "INSERT INTO message (id, time_created, data) VALUES (?1, ?2, ?3)",
            ("msg_db", 1765915142201_i64, without_id_or_time),
        )
        .unwrap();
        drop(conn);

        let mut records = Vec::new();
        parse_database(&db.0, "account", &mut Seen::default(), &mut records);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model.as_deref(), Some("m"));
        assert_eq!(records[0].total_tokens, 17);
        assert_eq!(records[0].cost_usd, Some(0.25));
    }

    #[test]
    fn calculates_total_when_legacy_message_omits_it() {
        let json = r#"{"id":"msg_1","role":"assistant","time":{"created":1765915142201},"modelID":"m","cost":0,"tokens":{"input":10,"output":3,"reasoning":2,"cache":{"read":5,"write":1}}}"#;
        let mut records = Vec::new();
        parse_message_json(json, None, None, "a", &mut Seen::default(), &mut records);
        assert_eq!(records[0].total_tokens, 21);
    }

    #[test]
    fn calculates_total_when_in_progress_total_is_zero() {
        let json = r#"{"id":"msg_1","role":"assistant","time":{"created":1765915142201},"modelID":"m","cost":0,"tokens":{"total":0,"input":10,"output":3,"reasoning":2,"cache":{"read":5,"write":1}}}"#;
        let mut records = Vec::new();
        parse_message_json(json, None, None, "a", &mut Seen::default(), &mut records);
        assert_eq!(records[0].total_tokens, 21);
    }

    #[test]
    fn deduplicates_forked_history_with_new_message_id() {
        let forked = MESSAGE.replace("msg_1", "msg_2");
        let mut seen = Seen::default();
        let mut records = Vec::new();
        parse_message_json(MESSAGE, None, None, "a", &mut seen, &mut records);
        parse_message_json(&forked, None, None, "a", &mut seen, &mut records);
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn skips_user_and_empty_assistant_messages() {
        let mut seen = Seen::default();
        let mut records = Vec::new();
        parse_message_json(
            r#"{"id":"u","role":"user","time":{"created":1}}"#,
            None,
            None,
            "a",
            &mut seen,
            &mut records,
        );
        parse_message_json(
            r#"{"id":"a","role":"assistant","time":{"created":1},"cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}}}"#,
            None,
            None,
            "a",
            &mut seen,
            &mut records,
        );
        assert!(records.is_empty());
    }
}
