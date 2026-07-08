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

use crate::cache::Cache;
use crate::config::Config;
use crate::model::{RateLimitSnapshot, Source, UsageRecord};
use crate::{aggregate, aggregate_per_account, parse_all, AggTotals};

const DASHBOARD_HTML: &str = include_str!("frontend/dashboard.html");
const DASHBOARD_CSS: &str = include_str!("frontend/styles.css");
const JS_UTIL: &str = include_str!("frontend/util.js");
const JS_TABLES: &str = include_str!("frontend/tables.js");
const JS_CHARTS: &str = include_str!("frontend/charts.js");
const JS_RATE_LIMITS: &str = include_str!("frontend/rate-limits.js");
const JS_MAIN: &str = include_str!("frontend/main.js");
const DEFAULT_PORT: u16 = 3000;

const CSS_CONTENT_TYPE: &str = "text/css; charset=utf-8";
const JS_CONTENT_TYPE: &str = "text/javascript; charset=utf-8";

/// Serve a static asset with an explicit Content-Type header.
fn static_asset(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, content_type)], body)
}

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

/// GET /api/rate_limits - latest rate limit snapshot per (source, account).
/// Codex snapshots come from session logs; claude_code from the OAuth usage API.
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

/// POST /api/refresh - re-run the parsers, merge the result into the on-disk
/// cache, and replace the in-memory state with the merged history (so records
/// preserved from rotated-out session logs stay visible).
async fn post_refresh(State(state): State<SharedState>) -> impl IntoResponse {
    let (config, include_dormant_claude) = {
        let data = state.read().unwrap();
        (data.config.clone(), data.include_dormant_claude)
    };

    // parse_all is synchronous and I/O-heavy; run it off the async runtime.
    let config_dir = config.config_dir().map(|p| p.to_path_buf());
    let (records, rate_limits, _summary) = tokio::task::spawn_blocking(move || {
        let (records, rate_limits, summary) = parse_all(&config, include_dormant_claude);
        let merged = match Cache::open(config_dir.as_deref()) {
            Ok(mut cache) => match cache.save(&records, &rate_limits) {
                Ok(()) => cache.load().ok().flatten(),
                Err(e) => {
                    eprintln!("Warning: failed to write cache: {e}");
                    None
                }
            },
            Err(e) => {
                eprintln!("Warning: failed to open cache: {e}");
                None
            }
        };
        // If the cache is unavailable, fall back to just the fresh parse.
        let (records, rate_limits) = merged.unwrap_or((records, rate_limits));
        (records, rate_limits, summary)
    })
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
    let config_dir = config.config_dir().map(|p| p.to_path_buf());
    let cached = Cache::open(config_dir.as_deref())
        .ok()
        .and_then(|c| c.load().ok().flatten());

    let (records, rate_limits) = match cached {
        Some((records, rate_limits)) => {
            println!(
                "Loaded {} cached records from cache.sqlite3 (click 데이터 새로고침 to re-parse).",
                records.len()
            );
            (records, rate_limits)
        }
        None => {
            println!("No cache found; parsing accounts before starting server...");
            let (records, rate_limits, summary) = parse_all(&config, include_dormant_claude);
            crate::print_parse_summary(&summary);
            if let Ok(mut cache) = Cache::open(config_dir.as_deref()) {
                if let Err(e) = cache.save(&records, &rate_limits) {
                    eprintln!("Warning: failed to write cache: {e}");
                }
            }
            (records, rate_limits)
        }
    };

    let config_port = config.port();
    let state: SharedState = Arc::new(RwLock::new(AppData {
        records,
        rate_limits,
        config,
        include_dormant_claude,
    }));

    let app = Router::new()
        .route("/", get(get_index))
        .route(
            "/styles.css",
            get(move || async move { static_asset(CSS_CONTENT_TYPE, DASHBOARD_CSS) }),
        )
        .route(
            "/js/util.js",
            get(move || async move { static_asset(JS_CONTENT_TYPE, JS_UTIL) }),
        )
        .route(
            "/js/tables.js",
            get(move || async move { static_asset(JS_CONTENT_TYPE, JS_TABLES) }),
        )
        .route(
            "/js/charts.js",
            get(move || async move { static_asset(JS_CONTENT_TYPE, JS_CHARTS) }),
        )
        .route(
            "/js/rate-limits.js",
            get(move || async move { static_asset(JS_CONTENT_TYPE, JS_RATE_LIMITS) }),
        )
        .route(
            "/js/main.js",
            get(move || async move { static_asset(JS_CONTENT_TYPE, JS_MAIN) }),
        )
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
