//! SQLite-backed cache of the last `parse_all()` result.
//!
//! Parsing walks every account's local session log files, which is the
//! slow part of startup. This module persists the parsed `UsageRecord`s
//! and `RateLimitSnapshot`s so a restart can skip re-parsing and serve
//! stale-but-fast data immediately; `/api/refresh` still re-parses and
//! overwrites the cache.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::model::{RateLimitSnapshot, RateLimitWindowSnapshot, Source, UsageRecord};

const DB_FILE_NAME: &str = "cache.sqlite3";

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
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS usage_records (
                source TEXT NOT NULL,
                account TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                model TEXT,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                reasoning_output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                is_subagent INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS rate_limits (
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
                secondary_resets_at INTEGER
            );
            ",
        )?;
        Ok(Cache { conn })
    }

    /// Replace the entire cache contents with the given parse result.
    pub fn save(
        &mut self,
        records: &[UsageRecord],
        rate_limits: &[RateLimitSnapshot],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM usage_records", [])?;
        tx.execute("DELETE FROM rate_limits", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO usage_records (
                    source, account, timestamp, model, input_tokens, cached_input_tokens,
                    output_tokens, reasoning_output_tokens, total_tokens, is_subagent
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for r in records {
                stmt.execute(params![
                    r.source.to_string(),
                    r.account,
                    r.timestamp.to_rfc3339(),
                    r.model,
                    r.input_tokens,
                    r.cached_input_tokens,
                    r.output_tokens,
                    r.reasoning_output_tokens,
                    r.total_tokens,
                    r.is_subagent as i64,
                ])?;
            }
        }
        {
            let mut stmt = tx.prepare(
                "INSERT INTO rate_limits (
                    account, observed_at, limit_id, plan_type, rate_limit_reached_type,
                    primary_used_percent, primary_window_minutes, primary_resets_at,
                    secondary_used_percent, secondary_window_minutes, secondary_resets_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for s in rate_limits {
                stmt.execute(params![
                    s.account,
                    s.observed_at.to_rfc3339(),
                    s.limit_id,
                    s.plan_type,
                    s.rate_limit_reached_type,
                    s.primary.as_ref().map(|w| w.used_percent),
                    s.primary.as_ref().map(|w| w.window_minutes),
                    s.primary.as_ref().map(|w| w.resets_at),
                    s.secondary.as_ref().map(|w| w.used_percent),
                    s.secondary.as_ref().map(|w| w.window_minutes),
                    s.secondary.as_ref().map(|w| w.resets_at),
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
                    output_tokens, reasoning_output_tokens, total_tokens, is_subagent
             FROM usage_records",
        )?;
        let records = stmt
            .query_map([], |row| {
                let source: String = row.get(0)?;
                let timestamp: String = row.get(2)?;
                let is_subagent: i64 = row.get(9)?;
                Ok(UsageRecord {
                    source: Source::from_str(&source).unwrap_or(Source::Codex),
                    account: row.get(1)?,
                    timestamp: parse_rfc3339(&timestamp),
                    model: row.get(3)?,
                    input_tokens: row.get(4)?,
                    cached_input_tokens: row.get(5)?,
                    output_tokens: row.get(6)?,
                    reasoning_output_tokens: row.get(7)?,
                    total_tokens: row.get(8)?,
                    is_subagent: is_subagent != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut rl_stmt = self.conn.prepare(
            "SELECT account, observed_at, limit_id, plan_type, rate_limit_reached_type,
                    primary_used_percent, primary_window_minutes, primary_resets_at,
                    secondary_used_percent, secondary_window_minutes, secondary_resets_at
             FROM rate_limits",
        )?;
        let rate_limits = rl_stmt
            .query_map([], |row| {
                let observed_at: String = row.get(1)?;
                let primary_used: Option<f64> = row.get(5)?;
                let primary_window: Option<u64> = row.get(6)?;
                let primary_resets: Option<i64> = row.get(7)?;
                let secondary_used: Option<f64> = row.get(8)?;
                let secondary_window: Option<u64> = row.get(9)?;
                let secondary_resets: Option<i64> = row.get(10)?;
                Ok(RateLimitSnapshot {
                    account: row.get(0)?,
                    observed_at: parse_rfc3339(&observed_at),
                    limit_id: row.get(2)?,
                    plan_type: row.get(3)?,
                    rate_limit_reached_type: row.get(4)?,
                    primary: primary_used.map(|used_percent| RateLimitWindowSnapshot {
                        used_percent,
                        window_minutes: primary_window.unwrap_or(0),
                        resets_at: primary_resets.unwrap_or(0),
                    }),
                    secondary: secondary_used.map(|used_percent| RateLimitWindowSnapshot {
                        used_percent,
                        window_minutes: secondary_window.unwrap_or(0),
                        resets_at: secondary_resets.unwrap_or(0),
                    }),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Some((records, rate_limits)))
    }
}

fn parse_rfc3339(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}
