//! Local web dashboard for the usage data.
//!
//! Parses all accounts once at startup, serves a single embedded HTML page
//! plus a small JSON API, and exposes POST /api/refresh to re-parse on
//! demand (e.g. after new session logs have been written).

use std::sync::{Arc, RwLock};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::config::Config;
use crate::model::{RateLimitSnapshot, Source, UsageRecord};
use crate::{aggregate, aggregate_per_account, parse_all, AggTotals};

const DASHBOARD_HTML: &str = include_str!("dashboard.html");
const DEFAULT_PORT: u16 = 3000;

struct AppData {
    records: Vec<UsageRecord>,
    rate_limits: Vec<RateLimitSnapshot>,
    config: Config,
    include_dormant_claude: bool,
}

type SharedState = Arc<RwLock<AppData>>;

#[derive(Serialize)]
struct AggRow {
    source: String,
    account: String,
    date: String,
    model: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
    turns: u64,
}

fn agg_totals_to_row(
    source: Source,
    account: &str,
    date: &str,
    model: &str,
    totals: &AggTotals,
) -> AggRow {
    AggRow {
        source: source.to_string(),
        account: account.to_string(),
        date: date.to_string(),
        model: model.to_string(),
        input_tokens: totals.input_tokens,
        cached_input_tokens: totals.cached_input_tokens,
        output_tokens: totals.output_tokens,
        reasoning_output_tokens: totals.reasoning_output_tokens,
        total_tokens: totals.total_tokens,
        turns: totals.count,
    }
}

#[derive(Serialize)]
struct AccountTotalRow {
    source: String,
    account: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
    turns: u64,
}

/// GET /api/usage - full source x account x date x model aggregation.
async fn get_usage(State(state): State<SharedState>) -> impl IntoResponse {
    let data = state.read().unwrap();
    let agg = aggregate(&data.records);
    let rows: Vec<AggRow> = agg
        .iter()
        .map(|((source, account, date, model), totals)| {
            agg_totals_to_row(*source, account, date, model, totals)
        })
        .collect();
    Json(rows)
}

/// GET /api/accounts - per (source, account) totals, summed across all dates/models.
async fn get_accounts(State(state): State<SharedState>) -> impl IntoResponse {
    let data = state.read().unwrap();
    let agg = aggregate(&data.records);

    let per_account = aggregate_per_account(&agg);

    let rows: Vec<AccountTotalRow> = per_account
        .iter()
        .map(|((source, account), totals)| AccountTotalRow {
            source: source.to_string(),
            account: account.clone(),
            input_tokens: totals.input_tokens,
            cached_input_tokens: totals.cached_input_tokens,
            output_tokens: totals.output_tokens,
            reasoning_output_tokens: totals.reasoning_output_tokens,
            total_tokens: totals.total_tokens,
            turns: totals.count,
        })
        .collect();
    Json(rows)
}

/// GET /api/rate_limits - latest Codex rate limit snapshot per account.
async fn get_rate_limits(State(state): State<SharedState>) -> impl IntoResponse {
    let data = state.read().unwrap();
    Json(data.rate_limits.clone())
}

#[derive(Serialize)]
struct RefreshResponse {
    status: &'static str,
    record_count: usize,
    rate_limit_count: usize,
}

/// POST /api/refresh - re-run the parsers and replace the in-memory state.
async fn post_refresh(State(state): State<SharedState>) -> impl IntoResponse {
    let (config, include_dormant_claude) = {
        let data = state.read().unwrap();
        (data.config.clone(), data.include_dormant_claude)
    };

    // parse_all is synchronous and I/O-heavy; run it off the async runtime.
    let (records, rate_limits, _summary) =
        tokio::task::spawn_blocking(move || parse_all(&config, include_dormant_claude))
            .await
            .expect("parse task panicked");
    let record_count = records.len();
    let rate_limit_count = rate_limits.len();

    let mut data = state.write().unwrap();
    data.records = records;
    data.rate_limits = rate_limits;

    (
        StatusCode::OK,
        Json(RefreshResponse {
            status: "ok",
            record_count,
            rate_limit_count,
        }),
    )
}

async fn get_index() -> impl IntoResponse {
    Html(DASHBOARD_HTML)
}

/// Build the axum router and block on serving it via a multi-thread tokio
/// runtime (so blocking parse work can run on the blocking pool).
pub fn run(config: Config, include_dormant_claude: bool) {
    println!("Parsing accounts before starting server...");
    let (records, rate_limits, summary) = parse_all(&config, include_dormant_claude);
    crate::print_parse_summary(&summary);

    let config_port = config.port();
    let state: SharedState = Arc::new(RwLock::new(AppData {
        records,
        rate_limits,
        config,
        include_dormant_claude,
    }));

    let app = Router::new()
        .route("/", get(get_index))
        .route("/api/usage", get(get_usage))
        .route("/api/accounts", get(get_accounts))
        .route("/api/rate_limits", get(get_rate_limits))
        .route("/api/refresh", post(post_refresh))
        .with_state(state);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    // PORT env var overrides config.toml `port`, which overrides the default.
    rt.block_on(async move {
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .or(config_port)
            .unwrap_or(DEFAULT_PORT);
        let addr = format!("127.0.0.1:{port}");
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
        println!("Dashboard listening on http://{addr}");
        axum::serve(listener, app)
            .await
            .expect("server error");
    });
}
