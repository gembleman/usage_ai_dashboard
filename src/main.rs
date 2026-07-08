mod cache;
mod claude_code;
mod codex;
mod config;
mod model;
mod server;
#[cfg(test)]
mod test_util;

use std::collections::BTreeMap;

use chrono::Datelike;

use crate::config::Config;
use crate::model::{RateLimitSnapshot, Source, UsageRecord};

/// Aggregation key: source, account, date (YYYY-MM-DD), model.
pub type AggKey = (Source, String, String, String);

#[derive(Debug, Default, Clone, Copy)]
pub struct AggTotals {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    pub count: u64,
}

impl AggTotals {
    /// Add another totals value into this one, field by field.
    pub fn add(&mut self, other: &AggTotals) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.total_tokens += other.total_tokens;
        self.count += other.count;
    }
}

/// Collapse a full source x account x date x model aggregation down to
/// per (source, account) totals. Shared by the console printout and the
/// /api/accounts endpoint.
pub fn aggregate_per_account(
    map: &BTreeMap<AggKey, AggTotals>,
) -> BTreeMap<(Source, String), AggTotals> {
    let mut per_account: BTreeMap<(Source, String), AggTotals> = BTreeMap::new();
    for ((source, account, _date, _model), totals) in map {
        per_account
            .entry((*source, account.clone()))
            .or_default()
            .add(totals);
    }
    per_account
}

pub fn aggregate(records: &[UsageRecord]) -> BTreeMap<AggKey, AggTotals> {
    let mut map: BTreeMap<AggKey, AggTotals> = BTreeMap::new();
    for r in records {
        let date = format!(
            "{:04}-{:02}-{:02}",
            r.timestamp.year(),
            r.timestamp.month(),
            r.timestamp.day()
        );
        let model = r.model.clone().unwrap_or_else(|| "unknown".to_string());
        let key = (r.source, r.account.clone(), date, model);
        let entry = map.entry(key).or_default();
        entry.input_tokens += r.input_tokens;
        entry.cached_input_tokens += r.cached_input_tokens;
        entry.output_tokens += r.output_tokens;
        entry.reasoning_output_tokens += r.reasoning_output_tokens;
        entry.total_tokens += r.total_tokens;
        entry.count += 1;
    }
    map
}

fn print_aggregation_table(map: &BTreeMap<AggKey, AggTotals>) {
    println!(
        "{:<12} {:<8} {:<12} {:<24} {:>10} {:>12} {:>12} {:>12} {:>8}",
        "source", "account", "date", "model", "input", "cached_in", "output", "total", "turns"
    );
    println!("{}", "-".repeat(120));
    for ((source, account, date, model), totals) in map {
        println!(
            "{:<12} {:<8} {:<12} {:<24} {:>10} {:>12} {:>12} {:>12} {:>8}",
            source.to_string(),
            account,
            date,
            model,
            totals.input_tokens,
            totals.cached_input_tokens,
            totals.output_tokens,
            totals.total_tokens,
            totals.count
        );
    }
}

fn print_account_totals(map: &BTreeMap<AggKey, AggTotals>) {
    let per_account = aggregate_per_account(map);

    println!();
    println!("=== Account totals ===");
    println!(
        "{:<12} {:<8} {:>12} {:>12} {:>12} {:>12} {:>8}",
        "source", "account", "input", "cached_in", "output", "total", "turns"
    );
    println!("{}", "-".repeat(80));
    for ((source, account), totals) in &per_account {
        println!(
            "{:<12} {:<8} {:>12} {:>12} {:>12} {:>12} {:>8}",
            source.to_string(),
            account,
            totals.input_tokens,
            totals.cached_input_tokens,
            totals.output_tokens,
            totals.total_tokens,
            totals.count
        );
    }
}

fn print_rate_limit_snapshots(snapshots: &[RateLimitSnapshot]) {
    println!();
    println!("=== Rate limit snapshots (codex + claude_code) ===");
    if snapshots.is_empty() {
        println!("(none found)");
        return;
    }
    for snap in snapshots {
        println!(
            "source={} account={} observed_at={} limit_id={:?} plan_type={:?}",
            snap.source,
            snap.account,
            snap.observed_at.to_rfc3339(),
            snap.limit_id,
            snap.plan_type
        );
        if let Some(p) = &snap.primary {
            println!(
                "  primary   (5h / {}min): {:.1}% used, resets_at={} ({})",
                p.window_minutes,
                p.used_percent,
                p.resets_at,
                format_epoch(p.resets_at)
            );
        }
        if let Some(s) = &snap.secondary {
            println!(
                "  secondary (7d / {}min): {:.1}% used, resets_at={} ({})",
                s.window_minutes,
                s.used_percent,
                s.resets_at,
                format_epoch(s.resets_at)
            );
        }
        // Claude Code only: model-scoped weekly windows.
        for (label, window) in [
            ("opus 7d", &snap.seven_day_opus),
            ("sonnet 7d", &snap.seven_day_sonnet),
        ] {
            if let Some(w) = window {
                println!(
                    "  {:<9} ({}min): {:.1}% used, resets_at={} ({})",
                    label,
                    w.window_minutes,
                    w.used_percent,
                    w.resets_at,
                    format_epoch(w.resets_at)
                );
            }
        }
        // Claude Code only: extra-usage credits (present only when enabled).
        if let Some(extra) = &snap.extra_usage {
            println!(
                "  extra usage: {} / {} credits used ({}%)",
                extra
                    .used_credits
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "?".to_string()),
                extra
                    .monthly_limit
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_else(|| "?".to_string()),
                extra
                    .utilization
                    .map(|v| format!("{v:.1}"))
                    .unwrap_or_else(|| "?".to_string()),
            );
        }
        if let Some(rt) = &snap.rate_limit_reached_type {
            println!("  rate_limit_reached_type={}", rt);
        }
    }
}

fn format_epoch(secs: i64) -> String {
    match chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0) {
        Some(dt) => dt.to_rfc3339(),
        None => "invalid".to_string(),
    }
}

/// A one-line-per-account summary of a parse run, returned so callers can
/// decide whether/how to print progress (the CLI does; the server stays quiet).
#[derive(Debug, Clone)]
pub struct ParseSummary {
    /// (label, record_count), e.g. ("codex/user01", 42).
    pub per_account: Vec<(String, usize)>,
    pub total_records: usize,
}

/// Parse all configured Codex + Claude Code accounts and return the raw
/// records plus any Codex rate limit snapshots and a summary. Does no
/// stdout output — the CLI prints the summary, the server ignores it.
/// Shared by the console path and the web server (both initial load and
/// the /api/refresh endpoint).
pub fn parse_all(
    config: &Config,
    include_dormant_claude: bool,
) -> (Vec<UsageRecord>, Vec<RateLimitSnapshot>, ParseSummary) {
    let codex_accounts = config.codex_accounts(include_dormant_claude);
    let claude_accounts = config.claude_accounts(include_dormant_claude);

    // Accounts are independent of each other, so parse each on its own
    // thread. This matters most for Claude Code, where the rate-limit fetch
    // is a blocking HTTP call (8s timeout) that would otherwise serialize
    // per account. Joining in spawn order keeps the summary deterministic.
    let (codex_results, claude_results) = std::thread::scope(|s| {
        let codex_handles: Vec<_> = codex_accounts
            .iter()
            .map(|account| s.spawn(move || codex::parse_account(account)))
            .collect();
        let claude_handles: Vec<_> = claude_accounts
            .iter()
            .map(|account| {
                s.spawn(move || {
                    let result = claude_code::parse_account(account, true);
                    // Rate limits for Claude Code come from the Anthropic OAuth
                    // usage API (not the local transcripts). Any failure yields
                    // None and is skipped.
                    let snapshot = claude_code::fetch_rate_limit_snapshot(account);
                    (result, snapshot)
                })
            })
            .collect();
        (
            codex_handles
                .into_iter()
                .map(|h| h.join().expect("codex parse thread panicked"))
                .collect::<Vec<_>>(),
            claude_handles
                .into_iter()
                .map(|h| h.join().expect("claude parse thread panicked"))
                .collect::<Vec<_>>(),
        )
    });

    let mut all_records: Vec<UsageRecord> = Vec::new();
    let mut rate_limit_snapshots: Vec<RateLimitSnapshot> = Vec::new();
    let mut per_account: Vec<(String, usize)> = Vec::new();

    for (account, result) in codex_accounts.iter().zip(codex_results) {
        per_account.push((format!("codex/{}", account.name), result.records.len()));
        all_records.extend(result.records);
        if let Some(snap) = result.rate_limit_snapshot {
            rate_limit_snapshots.push(snap);
        }
    }

    for (account, (result, snapshot)) in claude_accounts.iter().zip(claude_results) {
        per_account.push((
            format!("claude_code/{}", account.name),
            result.records.len(),
        ));
        all_records.extend(result.records);
        if let Some(snap) = snapshot {
            rate_limit_snapshots.push(snap);
        }
    }

    let summary = ParseSummary {
        per_account,
        total_records: all_records.len(),
    };

    (all_records, rate_limit_snapshots, summary)
}

/// Print a parse summary to stdout (CLI / server-startup only).
pub fn print_parse_summary(summary: &ParseSummary) {
    println!("Parsing sessions...");
    for (label, count) in &summary.per_account {
        println!("  {}: {} records", label, count);
    }
    println!();
    println!("Total records: {}", summary.total_records);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let include_dormant_claude = args.iter().any(|a| a == "--include-dormant");
    let config = Config::load_or_exit();

    if args.iter().any(|a| a == "serve") {
        server::run(config, include_dormant_claude);
        return;
    }

    let (all_records, rate_limit_snapshots, summary) = parse_all(&config, include_dormant_claude);
    print_parse_summary(&summary);

    let agg = aggregate(&all_records);
    println!();
    println!("=== Account x Date x Model token totals ===");
    print_aggregation_table(&agg);
    print_account_totals(&agg);
    print_rate_limit_snapshots(&rate_limit_snapshots);
}
