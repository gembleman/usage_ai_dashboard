//! SQLite-backed cache of accumulated `parse_all()` results.
//!
//! Parsing walks every account's local session log files, which is the
//! slow part of startup. This module persists the parsed `UsageRecord`s
//! and `RateLimitSnapshot`s so a restart can skip re-parsing and serve
//! stale-but-fast data immediately. Session logs rotate out (Claude Code
//! deletes transcripts after `cleanupPeriodDays`, 30 days by default), so
//! `save()` merges each parse into the cache rather than replacing it —
//! the cache keeps history the logs no longer cover.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection};

use crate::model::{
    ExtraUsageSnapshot, RateLimitSnapshot, RateLimitWindowSnapshot, Source, UsageRecord,
};

const DB_FILE_NAME: &str = "cache.sqlite3";

/// Cache schema version, tracked via `PRAGMA user_version`.
/// v1: codex rows store `input_tokens` excluding `cached_input_tokens`
/// (v0 rows stored OpenAI's cache-inclusive value).
/// v2: `rate_limits` gains a `source` column (codex / claude_code) so
/// same-named accounts from different CLIs no longer collide.
/// v3: `rate_limits` gains Claude Code-only columns: per-model weekly
/// windows (opus_*/sonnet_*) and extra-usage credits (extra_usage_*).
/// v4: `usage_records.timestamp` normalized to fixed-width UTC millis
/// ("YYYY-MM-DDTHH:MM:SS.sssZ") so plain string comparison is
/// chronological and the delete-window query can use the index instead
/// of wrapping both sides in `datetime()` (a full-table scan).
/// v5: `usage_records` gains a `cache_creation_input_tokens` column so
///     cache-creation and cache-read tokens are stored separately.
///     Previously they were merged into `cached_input_tokens`, which
///     caused cost estimates to under-report by ~20% (creation bills
///     at 1.25x vs read at 0.1x base input rate).
const SCHEMA_VERSION: i64 = 5;

const SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS usage_records (
        source TEXT NOT NULL,
        account TEXT NOT NULL,
        timestamp TEXT NOT NULL,
        model TEXT,
        input_tokens INTEGER NOT NULL,
        cached_input_tokens INTEGER NOT NULL,
        cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0,
        output_tokens INTEGER NOT NULL,
        reasoning_output_tokens INTEGER NOT NULL,
        total_tokens INTEGER NOT NULL,
        is_subagent INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rate_limits (
        source TEXT NOT NULL DEFAULT 'codex',
        account TEXT NOT NULL,
        observed_at TEXT NOT NULL,
        limit_id TEXT,
        plan_type TEXT,
        rate_limit_reached_type TEXT,
        primary_used_percent REAL,
        primary_window_minutes INTEGER,
        primary_resets_at INTEGER,
        secondary_used_percent REAL,
        secondary_window_minutes INTEGER,
        secondary_resets_at INTEGER,
        opus_used_percent REAL,
        opus_window_minutes INTEGER,
        opus_resets_at INTEGER,
        sonnet_used_percent REAL,
        sonnet_window_minutes INTEGER,
        sonnet_resets_at INTEGER,
        extra_usage_enabled INTEGER,
        extra_usage_monthly_limit REAL,
        extra_usage_used_credits REAL,
        extra_usage_utilization REAL
    );
    CREATE INDEX IF NOT EXISTS idx_usage_records_source_account_ts
        ON usage_records(source, account, timestamp);
";

/// Bring an existing DB up to `SCHEMA_VERSION`. Runs on every `open()`.
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 1 {
        // v0 codex rows stored OpenAI's cache-inclusive input_tokens, which
        // double-counts cached tokens in cost estimates. The parser now
        // stores the non-cached remainder; normalize old rows the same way
        // (total_tokens stays untouched, matching the parser).
        conn.execute(
            "UPDATE usage_records
             SET input_tokens = MAX(input_tokens - cached_input_tokens, 0)
             WHERE source = 'codex'",
            [],
        )?;
    }
    if version < 2 {
        // Add the rate_limits.source column to pre-v2 DBs. New CREATE TABLE
        // statements already include it, so guard against the "duplicate
        // column" error on freshly-created v2 tables by ignoring it.
        let _ = conn.execute(
            "ALTER TABLE rate_limits ADD COLUMN source TEXT NOT NULL DEFAULT 'codex'",
            [],
        );
    }
    if version < 3 {
        // Claude Code-only columns (model-scoped weekly windows + extra-usage
        // credits). Same duplicate-column-tolerant pattern as v2: fresh v3
        // tables already have them, so failures are ignored.
        for col in [
            "opus_used_percent REAL",
            "opus_window_minutes INTEGER",
            "opus_resets_at INTEGER",
            "sonnet_used_percent REAL",
            "sonnet_window_minutes INTEGER",
            "sonnet_resets_at INTEGER",
            "extra_usage_enabled INTEGER",
            "extra_usage_monthly_limit REAL",
            "extra_usage_used_credits REAL",
            "extra_usage_utilization REAL",
        ] {
            let _ = conn.execute(&format!("ALTER TABLE rate_limits ADD COLUMN {col}"), []);
        }
    }
    if version < 4 {
        // Rewrite pre-v4 timestamps (variable precision, "+00:00" suffix) to
        // the fixed-width millisecond UTC format `to_db_timestamp` writes.
        // COALESCE keeps any value strftime can't parse instead of NULLing it.
        conn.execute(
            "UPDATE usage_records
             SET timestamp = COALESCE(strftime('%Y-%m-%dT%H:%M:%fZ', timestamp), timestamp)",
            [],
        )?;
    }
    if version < 5 {
        // Split cache tokens: add the creation column. Existing rows get 0
        // (cache-creation was previously folded into cached_input_tokens;
        // there is no way to retroactively split them, but new parses will
        // populate both columns correctly going forward).
        let _ = conn.execute(
            "ALTER TABLE usage_records ADD COLUMN cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0",
            [],
        );
    }
    if version < SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

pub struct Cache {
    conn: Connection,
}

impl Cache {
    /// Open (creating if needed) the cache DB next to the given config path's
    /// directory, or the current directory if `config_dir` is `None`.
    pub fn open(config_dir: Option<&Path>) -> rusqlite::Result<Cache> {
        let path: PathBuf = match config_dir {
            Some(dir) => dir.join(DB_FILE_NAME),
            None => PathBuf::from(DB_FILE_NAME),
        };
        Cache::init(Connection::open(path)?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Cache> {
        // WAL + NORMAL trades a little durability-on-power-loss for much
        // faster writes; fine for a rebuildable cache. journal_mode returns a
        // row (and reports "memory" for in-memory DBs), so query it instead
        // of pragma_update.
        let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA_SQL)?;
        migrate(&conn)?;
        Ok(Cache { conn })
    }

    /// Merge a parse result into the cache.
    ///
    /// For each (source, account) present in `records`, rows at or after
    /// that account's earliest newly parsed timestamp are replaced; older
    /// rows and accounts absent from this parse are kept, so history
    /// survives session-log rotation. Rate limits are upserted per
    /// (source, account): a snapshot replaces that account's previous one,
    /// but accounts whose fetch failed this round (expired token, API
    /// error) keep their last successful snapshot — `observed_at` tells
    /// the UI how stale it is.
    pub fn save(
        &mut self,
        records: &[UsageRecord],
        rate_limits: &[RateLimitSnapshot],
    ) -> rusqlite::Result<()> {
        let mut earliest: HashMap<(Source, &str), DateTime<Utc>> = HashMap::new();
        for r in records {
            earliest
                .entry((r.source, r.account.as_str()))
                .and_modify(|t| *t = (*t).min(r.timestamp))
                .or_insert(r.timestamp);
        }

        let tx = self.conn.transaction()?;
        {
            let mut stmt =
                tx.prepare("DELETE FROM rate_limits WHERE source = ?1 AND account = ?2")?;
            for s in rate_limits {
                stmt.execute(params![s.source.to_string(), s.account])?;
            }
        }
        {
            // Timestamps are stored in a fixed-width UTC format (see
            // `to_db_timestamp` / v4 migration), so plain string comparison
            // is chronological and this DELETE can range-scan the
            // (source, account, timestamp) index instead of full-scanning
            // with datetime() on every row.
            let mut stmt = tx.prepare(
                "DELETE FROM usage_records
                 WHERE source = ?1 AND account = ?2 AND timestamp >= ?3",
            )?;
            for ((source, account), ts) in &earliest {
                stmt.execute(params![source.to_string(), account, to_db_timestamp(ts)])?;
            }
        }
        {
            let mut stmt = tx.prepare(
                "INSERT INTO usage_records (
                    source, account, timestamp, model, input_tokens, cached_input_tokens,
                    cache_creation_input_tokens,
                    output_tokens, reasoning_output_tokens, total_tokens, is_subagent
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for r in records {
                stmt.execute(params![
                    r.source.to_string(),
                    r.account,
                    to_db_timestamp(&r.timestamp),
                    r.model,
                    r.input_tokens as i64,
                    r.cached_input_tokens as i64,
                    r.cache_creation_input_tokens as i64,
                    r.output_tokens as i64,
                    r.reasoning_output_tokens as i64,
                    r.total_tokens as i64,
                    r.is_subagent as i64,
                ])?;
            }
        }
        {
            let mut stmt = tx.prepare(
                "INSERT INTO rate_limits (
                    source, account, observed_at, limit_id, plan_type, rate_limit_reached_type,
                    primary_used_percent, primary_window_minutes, primary_resets_at,
                    secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                    opus_used_percent, opus_window_minutes, opus_resets_at,
                    sonnet_used_percent, sonnet_window_minutes, sonnet_resets_at,
                    extra_usage_enabled, extra_usage_monthly_limit,
                    extra_usage_used_credits, extra_usage_utilization
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                          ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            )?;
            for s in rate_limits {
                stmt.execute(params![
                    s.source.to_string(),
                    s.account,
                    s.observed_at.to_rfc3339(),
                    s.limit_id,
                    s.plan_type,
                    s.rate_limit_reached_type,
                    s.primary.as_ref().map(|w| w.used_percent),
                    s.primary.as_ref().map(|w| w.window_minutes as i64),
                    s.primary.as_ref().map(|w| w.resets_at),
                    s.secondary.as_ref().map(|w| w.used_percent),
                    s.secondary.as_ref().map(|w| w.window_minutes as i64),
                    s.secondary.as_ref().map(|w| w.resets_at),
                    s.seven_day_opus.as_ref().map(|w| w.used_percent),
                    s.seven_day_opus.as_ref().map(|w| w.window_minutes as i64),
                    s.seven_day_opus.as_ref().map(|w| w.resets_at),
                    s.seven_day_sonnet.as_ref().map(|w| w.used_percent),
                    s.seven_day_sonnet.as_ref().map(|w| w.window_minutes as i64),
                    s.seven_day_sonnet.as_ref().map(|w| w.resets_at),
                    // Presence marker: extra_usage is only stored when enabled.
                    s.extra_usage.as_ref().map(|_| 1i64),
                    s.extra_usage.as_ref().and_then(|e| e.monthly_limit),
                    s.extra_usage.as_ref().and_then(|e| e.used_credits),
                    s.extra_usage.as_ref().and_then(|e| e.utilization),
                ])?;
            }
        }
        tx.commit()
    }

    /// Load the cached records and rate limits. Returns `None` if the cache
    /// is empty (e.g. first run).
    pub fn load(&self) -> rusqlite::Result<Option<(Vec<UsageRecord>, Vec<RateLimitSnapshot>)>> {
        let mut count_stmt = self.conn.prepare("SELECT COUNT(*) FROM usage_records")?;
        let record_count: i64 = count_stmt.query_row([], |row| row.get(0))?;
        if record_count == 0 {
            return Ok(None);
        }

        let mut stmt = self.conn.prepare(
            "SELECT source, account, timestamp, model, input_tokens, cached_input_tokens,
                    cache_creation_input_tokens,
                    output_tokens, reasoning_output_tokens, total_tokens, is_subagent
             FROM usage_records",
        )?;
        let records = stmt
            .query_map([], |row| {
                let source: String = row.get(0)?;
                let timestamp: String = row.get(2)?;
                let is_subagent: i64 = row.get(10)?;
                Ok(UsageRecord {
                    source: Source::from_str(&source).unwrap_or(Source::Codex),
                    account: row.get(1)?,
                    timestamp: parse_rfc3339(&timestamp),
                    model: row.get(3)?,
                    input_tokens: row.get::<_, i64>(4)? as u64,
                    cached_input_tokens: row.get::<_, i64>(5)? as u64,
                    cache_creation_input_tokens: row.get::<_, i64>(6)? as u64,
                    output_tokens: row.get::<_, i64>(7)? as u64,
                    reasoning_output_tokens: row.get::<_, i64>(8)? as u64,
                    total_tokens: row.get::<_, i64>(9)? as u64,
                    is_subagent: is_subagent != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut rl_stmt = self.conn.prepare(
            "SELECT source, account, observed_at, limit_id, plan_type, rate_limit_reached_type,
                    primary_used_percent, primary_window_minutes, primary_resets_at,
                    secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                    opus_used_percent, opus_window_minutes, opus_resets_at,
                    sonnet_used_percent, sonnet_window_minutes, sonnet_resets_at,
                    extra_usage_enabled, extra_usage_monthly_limit,
                    extra_usage_used_credits, extra_usage_utilization
             FROM rate_limits",
        )?;
        // Rebuild an optional window from its (used_percent, window_minutes,
        // resets_at) column triple; used_percent doubles as presence marker.
        let window = |used: Option<f64>, minutes: Option<i64>, resets: Option<i64>| {
            used.map(|used_percent| RateLimitWindowSnapshot {
                used_percent,
                window_minutes: minutes.unwrap_or(0) as u64,
                resets_at: resets.unwrap_or(0),
            })
        };
        let rate_limits = rl_stmt
            .query_map([], |row| {
                let source: String = row.get(0)?;
                let observed_at: String = row.get(2)?;
                let extra_enabled: Option<i64> = row.get(18)?;
                Ok(RateLimitSnapshot {
                    source: Source::from_str(&source).unwrap_or(Source::Codex),
                    account: row.get(1)?,
                    observed_at: parse_rfc3339(&observed_at),
                    limit_id: row.get(3)?,
                    plan_type: row.get(4)?,
                    rate_limit_reached_type: row.get(5)?,
                    primary: window(row.get(6)?, row.get(7)?, row.get(8)?),
                    secondary: window(row.get(9)?, row.get(10)?, row.get(11)?),
                    seven_day_opus: window(row.get(12)?, row.get(13)?, row.get(14)?),
                    seven_day_sonnet: window(row.get(15)?, row.get(16)?, row.get(17)?),
                    extra_usage: match extra_enabled {
                        Some(v) if v != 0 => Some(ExtraUsageSnapshot {
                            monthly_limit: row.get(19)?,
                            used_credits: row.get(20)?,
                            utilization: row.get(21)?,
                        }),
                        _ => None,
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Some((records, rate_limits)))
    }
}

/// Fixed-width UTC RFC3339 with millisecond precision, e.g.
/// "2026-06-01T12:00:00.000Z". Must match the v4 migration's
/// strftime('%Y-%m-%dT%H:%M:%fZ', ...) output so old and new rows compare
/// lexicographically == chronologically.
fn to_db_timestamp(t: &DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_rfc3339(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn open_mem() -> Cache {
        Cache::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn record(source: Source, account: &str, day: u32, input_tokens: u64) -> UsageRecord {
        UsageRecord {
            source,
            account: account.to_string(),
            timestamp: Utc.with_ymd_and_hms(2026, 6, day, 12, 0, 0).unwrap(),
            model: None,
            input_tokens,
            cached_input_tokens: 0,
            cache_creation_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens: input_tokens,
            is_subagent: false,
        }
    }

    #[test]
    fn save_keeps_history_older_than_new_parse() {
        let mut cache = open_mem();
        cache
            .save(
                &[
                    record(Source::Codex, "a", 1, 10),
                    record(Source::Codex, "a", 15, 20),
                ],
                &[],
            )
            .unwrap();
        // Second parse: the day-1 log has rotated out, day 15 was re-parsed
        // with a corrected value, and day 20 is new.
        cache
            .save(
                &[
                    record(Source::Codex, "a", 15, 25),
                    record(Source::Codex, "a", 20, 30),
                ],
                &[],
            )
            .unwrap();

        let (mut records, _) = cache.load().unwrap().unwrap();
        records.sort_by_key(|r| r.timestamp);
        let tokens: Vec<u64> = records.iter().map(|r| r.input_tokens).collect();
        assert_eq!(tokens, vec![10, 25, 30]);
    }

    #[test]
    fn save_leaves_other_accounts_untouched() {
        let mut cache = open_mem();
        cache
            .save(&[record(Source::ClaudeCode, "b", 1, 10)], &[])
            .unwrap();
        // A later parse that only sees account "a" must not delete "b"'s
        // older rows even though they predate the new parse window.
        cache
            .save(&[record(Source::Codex, "a", 20, 30)], &[])
            .unwrap();

        let (records, _) = cache.load().unwrap().unwrap();
        assert_eq!(records.len(), 2);
    }

    fn snapshot(source: Source, account: &str, used_percent: f64) -> RateLimitSnapshot {
        RateLimitSnapshot {
            source,
            account: account.to_string(),
            observed_at: Utc.with_ymd_and_hms(2026, 7, 8, 12, 0, 0).unwrap(),
            limit_id: None,
            plan_type: None,
            rate_limit_reached_type: None,
            primary: Some(RateLimitWindowSnapshot {
                used_percent,
                window_minutes: 300,
                resets_at: 0,
            }),
            secondary: None,
            seven_day_opus: None,
            seven_day_sonnet: None,
            extra_usage: None,
        }
    }

    #[test]
    fn save_keeps_rate_limits_for_accounts_missing_from_new_parse() {
        let mut cache = open_mem();
        cache
            .save(
                &[record(Source::ClaudeCode, "user01", 1, 10)],
                &[
                    snapshot(Source::ClaudeCode, "user01", 10.0),
                    snapshot(Source::ClaudeCode, "user02", 20.0),
                ],
            )
            .unwrap();
        // user01's fetch failed this round (e.g. expired token): only user02
        // came back. user01's last snapshot must survive; user02's must be
        // replaced, not duplicated.
        cache
            .save(
                &[record(Source::ClaudeCode, "user01", 2, 10)],
                &[snapshot(Source::ClaudeCode, "user02", 55.0)],
            )
            .unwrap();

        let (_, mut rate_limits) = cache.load().unwrap().unwrap();
        rate_limits.sort_by(|a, b| a.account.cmp(&b.account));
        assert_eq!(rate_limits.len(), 2);
        assert_eq!(rate_limits[0].account, "user01");
        assert_eq!(rate_limits[0].primary.as_ref().unwrap().used_percent, 10.0);
        assert_eq!(rate_limits[1].account, "user02");
        assert_eq!(rate_limits[1].primary.as_ref().unwrap().used_percent, 55.0);
    }

    #[test]
    fn rate_limit_snapshot_roundtrips_claude_code_fields() {
        let mut cache = open_mem();
        let snap = RateLimitSnapshot {
            source: Source::ClaudeCode,
            account: "user02".to_string(),
            observed_at: Utc.with_ymd_and_hms(2026, 7, 8, 12, 0, 0).unwrap(),
            limit_id: None,
            plan_type: Some("pro".to_string()),
            rate_limit_reached_type: None,
            primary: Some(RateLimitWindowSnapshot {
                used_percent: 33.0,
                window_minutes: 300,
                resets_at: 1783528199,
            }),
            secondary: None,
            seven_day_opus: None,
            seven_day_sonnet: Some(RateLimitWindowSnapshot {
                used_percent: 12.5,
                window_minutes: 10080,
                resets_at: 1783976399,
            }),
            extra_usage: Some(ExtraUsageSnapshot {
                monthly_limit: Some(5000.0),
                used_credits: Some(123.45),
                utilization: Some(2.5),
            }),
        };
        // load() returns None on an empty usage_records table, so store one row.
        cache
            .save(&[record(Source::ClaudeCode, "user02", 1, 10)], &[snap])
            .unwrap();

        let (_, rate_limits) = cache.load().unwrap().unwrap();
        assert_eq!(rate_limits.len(), 1);
        let s = &rate_limits[0];
        assert_eq!(s.source, Source::ClaudeCode);
        assert!(s.seven_day_opus.is_none());
        let sonnet = s.seven_day_sonnet.as_ref().unwrap();
        assert_eq!(sonnet.used_percent, 12.5);
        assert_eq!(sonnet.window_minutes, 10080);
        assert_eq!(sonnet.resets_at, 1783976399);
        let extra = s.extra_usage.as_ref().unwrap();
        assert_eq!(extra.monthly_limit, Some(5000.0));
        assert_eq!(extra.used_credits, Some(123.45));
        assert_eq!(extra.utilization, Some(2.5));
    }

    #[test]
    fn migration_v4_normalizes_timestamps_for_string_comparison() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        // Pre-v4 rows: "+00:00" offset form, with and without fractional
        // seconds. Left unmigrated these compare wrongly against the new
        // "Z"-suffixed fixed-width format ('+' < '.') and would never be
        // deleted by save()'s replace window, duplicating history.
        conn.execute(
            "INSERT INTO usage_records VALUES
                ('codex', 'a', '2026-06-01T12:00:00+00:00', NULL, 10, 0, 0, 0, 0, 10, 0),
                ('codex', 'a', '2026-06-20T08:30:00.123456+00:00', NULL, 20, 0, 0, 0, 0, 20, 0)",
            [],
        )
        .unwrap();
        let mut cache = Cache::init(conn).unwrap();

        let mut stmt = cache
            .conn
            .prepare("SELECT timestamp FROM usage_records ORDER BY timestamp")
            .unwrap();
        let ts: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(stmt);
        assert_eq!(
            ts,
            vec![
                "2026-06-01T12:00:00.000Z".to_string(),
                "2026-06-20T08:30:00.123Z".to_string(),
            ]
        );

        // New parse starting at day 15: the migrated day-20 row falls inside
        // the replace window (string >=) and must be dropped; day 1 survives.
        cache
            .save(&[record(Source::Codex, "a", 15, 99)], &[])
            .unwrap();
        let (mut records, _) = cache.load().unwrap().unwrap();
        records.sort_by_key(|r| r.timestamp);
        let tokens: Vec<u64> = records.iter().map(|r| r.input_tokens).collect();
        assert_eq!(tokens, vec![10, 99]);
    }

    #[test]
    fn migration_normalizes_v0_codex_input_tokens_once() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        // v0 rows: codex input_tokens included the cached portion.
        conn.execute(
            "INSERT INTO usage_records VALUES
                ('codex', 'a', '2026-06-01T00:00:00+00:00', NULL, 1000, 800, 0, 0, 0, 1000, 0),
                ('claude_code', 'b', '2026-06-01T00:00:00+00:00', NULL, 1000, 800, 0, 0, 0, 1800, 0)",
            [],
        )
        .unwrap();

        let cache = Cache::init(conn).unwrap();
        let (records, _) = cache.load().unwrap().unwrap();
        let codex = records.iter().find(|r| r.source == Source::Codex).unwrap();
        let claude = records
            .iter()
            .find(|r| r.source == Source::ClaudeCode)
            .unwrap();
        assert_eq!(codex.input_tokens, 200);
        // claude_code rows were always cache-exclusive: untouched.
        assert_eq!(claude.input_tokens, 1000);

        // user_version is now current, so re-running must not subtract again.
        migrate(&cache.conn).unwrap();
        let (records, _) = cache.load().unwrap().unwrap();
        let codex = records.iter().find(|r| r.source == Source::Codex).unwrap();
        assert_eq!(codex.input_tokens, 200);
    }
}
