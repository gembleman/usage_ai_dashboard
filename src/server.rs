//! Local web dashboard for the usage data.
//!
//! Parses all accounts once at startup, serves frontend files from disk plus
//! a small JSON API, and exposes POST /api/refresh to re-parse on
//! demand (e.g. after new session logs have been written).

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::cache::Cache;
use crate::config::Config;
use crate::model::{RateLimitSnapshot, Source, UsageRecord};
use crate::{AggKey, AggTotals, aggregate, parse_all};

const CSS_CONTENT_TYPE: &str = "text/css; charset=utf-8";
const JS_CONTENT_TYPE: &str = "text/javascript; charset=utf-8";

/// Read a frontend file at request time so frontend changes do not require a
/// Rust rebuild (and the files are not embedded in the executable).
async fn frontend_asset(
    state: &SharedState,
    relative_path: &'static str,
    content_type: &'static str,
) -> axum::response::Response {
    let path = {
        let data = state.read().unwrap();
        data.frontend_dir.join(relative_path)
    };

    match tokio::fs::read(&path).await {
        Ok(body) => ([(axum::http::header::CONTENT_TYPE, content_type)], body).into_response(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Frontend asset not found: {}", path.display());
            (StatusCode::NOT_FOUND, "frontend asset not found").into_response()
        }
        Err(error) => {
            eprintln!("Failed to read frontend asset {}: {error}", path.display());
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read frontend asset",
            )
                .into_response()
        }
    }
}

struct AppData {
    /// Pre-serialized /api/usage body. The aggregation only changes when the
    /// records change (startup / refresh), so it is computed once there
    /// instead of on every request; `Bytes` clones are refcounted, so request
    /// handlers hold the read lock only long enough to bump a refcount.
    usage_json: Bytes,
    /// Pre-serialized /api/rate_limits body, same lifecycle as `usage_json`.
    rate_limits_json: Bytes,
    /// Model prices loaded from config.toml.
    pricing_json: Bytes,
    settings_json: Bytes,
    config: Config,
    include_dormant_claude: bool,
    frontend_dir: PathBuf,
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
    cache_creation_input_tokens: u64,
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
        cache_creation_input_tokens: totals.cache_creation_input_tokens,
        output_tokens: totals.output_tokens,
        reasoning_output_tokens: totals.reasoning_output_tokens,
        total_tokens: totals.total_tokens,
        turns: totals.count,
    }
}

/// Serialize a value into a pre-built JSON response body.
fn to_json_bytes<T: Serialize + ?Sized>(value: &T) -> Bytes {
    Bytes::from(serde_json::to_vec(value).expect("JSON serialization failed"))
}

/// Build the /api/usage body: full source x account x date x model aggregation.
fn build_usage_json(records: &[UsageRecord]) -> Bytes {
    let agg = aggregate(records);
    build_aggregate_json(&agg)
}

fn build_aggregate_json(agg: &std::collections::BTreeMap<AggKey, AggTotals>) -> Bytes {
    let rows: Vec<AggRow> = agg
        .iter()
        .map(|((source, account, date, model), totals)| {
            agg_totals_to_row(*source, account, date, model, totals)
        })
        .collect();
    to_json_bytes(&rows)
}

const JSON_CONTENT_TYPE: &str = "application/json";

/// GET /api/usage - full source x account x date x model aggregation,
/// precomputed at startup / refresh.
async fn get_usage(State(state): State<SharedState>) -> impl IntoResponse {
    let body = state.read().unwrap().usage_json.clone();
    (
        [(axum::http::header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        body,
    )
}

/// GET /api/rate_limits - latest rate limit snapshot per (source, account).
/// Codex snapshots come from its app-server; claude_code from the OAuth usage API.
async fn get_rate_limits(State(state): State<SharedState>) -> impl IntoResponse {
    let body = state.read().unwrap().rate_limits_json.clone();
    (
        [(axum::http::header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        body,
    )
}

/// GET /api/pricing - model prices from config.toml.
async fn get_pricing(State(state): State<SharedState>) -> impl IntoResponse {
    let body = state.read().unwrap().pricing_json.clone();
    (
        [(axum::http::header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        body,
    )
}

#[derive(Serialize)]
struct BrowserSettings<'a> {
    dashboard: &'a crate::config::DashboardConfig,
    timeouts: &'a crate::config::TimeoutConfig,
}

async fn get_settings(State(state): State<SharedState>) -> impl IntoResponse {
    let body = state.read().unwrap().settings_json.clone();
    (
        [(axum::http::header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        body,
    )
}

#[derive(Serialize)]
struct RefreshResponse {
    status: &'static str,
    record_count: usize,
    rate_limit_count: usize,
}

type RefreshPayload = (Bytes, Bytes, usize, usize);
type CachedAggregate = (
    std::collections::BTreeMap<AggKey, AggTotals>,
    Vec<RateLimitSnapshot>,
    usize,
);

fn build_refresh_payload(
    records: &[UsageRecord],
    fresh_rate_limits: &[RateLimitSnapshot],
    cached: Option<CachedAggregate>,
) -> RefreshPayload {
    match cached {
        Some((agg, rate_limits, record_count)) => (
            build_aggregate_json(&agg),
            to_json_bytes(&rate_limits),
            record_count,
            rate_limits.len(),
        ),
        None => (
            build_usage_json(records),
            to_json_bytes(fresh_rate_limits),
            records.len(),
            fresh_rate_limits.len(),
        ),
    }
}

/// POST /api/refresh - re-run the parsers, merge the result into the on-disk
/// cache, and replace the in-memory state with the merged history (so records
/// preserved from rotated-out session logs stay visible).
async fn post_refresh(State(state): State<SharedState>) -> impl IntoResponse {
    let (config, include_dormant_claude) = {
        let data = state.read().unwrap();
        (data.config.clone(), data.include_dormant_claude)
    };

    // parse_all is synchronous and I/O-heavy; run it (and the JSON
    // re-serialization) off the async runtime.
    let cache_path = config.cache_path().to_path_buf();
    let (usage_json, rate_limits_json, record_count, rate_limit_count) =
        tokio::task::spawn_blocking(move || {
            let (records, rate_limits, _summary) = parse_all(&config, include_dormant_claude);
            let merged = match Cache::open(&cache_path) {
                Ok(mut cache) => match cache.save(&records, &rate_limits) {
                    Ok(()) => cache.load_aggregate().ok().flatten(),
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
            // An empty cache has no aggregate, so retain the fresh parse until
            // this fallback has been built instead of taking and unwrapping it.
            build_refresh_payload(&records, &rate_limits, merged)
        })
        .await
        .expect("parse task panicked");

    let mut data = state.write().unwrap();
    data.usage_json = usage_json;
    data.rate_limits_json = rate_limits_json;

    (
        StatusCode::OK,
        Json(RefreshResponse {
            status: "ok",
            record_count,
            rate_limit_count,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_refresh_without_cached_aggregate_returns_empty_json() {
        let (usage, rate_limits, record_count, rate_limit_count) =
            build_refresh_payload(&[], &[], None);

        assert_eq!(usage.as_ref(), b"[]");
        assert_eq!(rate_limits.as_ref(), b"[]");
        assert_eq!(record_count, 0);
        assert_eq!(rate_limit_count, 0);
    }
}

async fn get_index(State(state): State<SharedState>) -> impl IntoResponse {
    frontend_asset(&state, "dashboard.html", "text/html; charset=utf-8").await
}

async fn get_styles(State(state): State<SharedState>) -> impl IntoResponse {
    frontend_asset(&state, "styles.css", CSS_CONTENT_TYPE).await
}

async fn get_util_js(State(state): State<SharedState>) -> impl IntoResponse {
    frontend_asset(&state, "util.js", JS_CONTENT_TYPE).await
}

async fn get_tables_js(State(state): State<SharedState>) -> impl IntoResponse {
    frontend_asset(&state, "tables.js", JS_CONTENT_TYPE).await
}

async fn get_charts_js(State(state): State<SharedState>) -> impl IntoResponse {
    frontend_asset(&state, "charts.js", JS_CONTENT_TYPE).await
}

async fn get_rate_limits_js(State(state): State<SharedState>) -> impl IntoResponse {
    frontend_asset(&state, "rate-limits.js", JS_CONTENT_TYPE).await
}

async fn get_main_js(State(state): State<SharedState>) -> impl IntoResponse {
    frontend_asset(&state, "main.js", JS_CONTENT_TYPE).await
}

/// Build the axum router and block on a single-thread Tokio runtime. The
/// dashboard has tiny request handlers; extra permanent worker threads only
/// add stack/runtime memory. Refresh parsing still uses the blocking pool.
pub fn run(config: Config, include_dormant_claude: bool) {
    let cache_path = config.cache_path().to_path_buf();
    let cached = Cache::open(&cache_path)
        .ok()
        .and_then(|c| c.load_aggregate().ok().flatten());

    let (usage_json, rate_limits) = match cached {
        Some((agg, rate_limits, record_count)) => {
            println!(
                "Loaded {} cached records from {} (click 데이터 새로고침 to re-parse).",
                record_count,
                cache_path.display()
            );
            (build_aggregate_json(&agg), rate_limits)
        }
        None => {
            println!("No cache found; parsing accounts before starting server...");
            let (records, rate_limits, summary) = parse_all(&config, include_dormant_claude);
            crate::print_parse_summary(&summary);
            if let Ok(mut cache) = Cache::open(&cache_path) {
                if let Err(e) = cache.save(&records, &rate_limits) {
                    eprintln!("Warning: failed to write cache: {e}");
                }
            }
            (build_usage_json(&records), rate_limits)
        }
    };

    let server_config = config.server().clone();
    let frontend_dir = config.frontend_dir().to_path_buf();
    let settings_json = to_json_bytes(&BrowserSettings {
        dashboard: config.dashboard(),
        timeouts: config.timeouts(),
    });
    let state: SharedState = Arc::new(RwLock::new(AppData {
        usage_json,
        rate_limits_json: to_json_bytes(&rate_limits),
        pricing_json: to_json_bytes(config.model_pricing()),
        settings_json,
        config,
        include_dormant_claude,
        frontend_dir,
    }));

    let app = Router::new()
        .route("/", get(get_index))
        .route("/styles.css", get(get_styles))
        .route("/js/util.js", get(get_util_js))
        .route("/js/tables.js", get(get_tables_js))
        .route("/js/charts.js", get(get_charts_js))
        .route("/js/rate-limits.js", get(get_rate_limits_js))
        .route("/js/main.js", get(get_main_js))
        .route("/api/usage", get(get_usage))
        .route("/api/rate_limits", get(get_rate_limits))
        .route("/api/pricing", get(get_pricing))
        .route("/api/settings", get(get_settings))
        .route("/api/refresh", post(post_refresh))
        .with_state(state);

    let rt = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    // PORT env var overrides config.toml server.port.
    rt.block_on(async move {
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(server_config.port);
        let addr = format!("{}:{port}", server_config.host);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
        println!("Dashboard listening on http://{addr}");
        axum::serve(listener, app).await.expect("server error");
    });
}
