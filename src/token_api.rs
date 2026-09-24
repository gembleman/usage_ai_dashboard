//! Client for the standalone `token-usage-api` service.

use std::time::Duration;

use serde::Deserialize;
use time::OffsetDateTime;

use crate::model::{Source, UsageRecord};

#[derive(Debug, Clone)]
pub struct TokenApiServer {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
struct RecordsResponse {
    records: Vec<RemoteRecord>,
}

#[derive(Debug, Deserialize)]
struct RemoteRecord {
    source: String,
    #[serde(deserialize_with = "crate::timestamp::deserialize")]
    timestamp: OffsetDateTime,
    model: Option<String>,
    cost_usd: Option<f64>,
    input: u64,
    output: u64,
    #[serde(default)]
    reasoning: u64,
    #[serde(default)]
    cache_read: u64,
    #[serde(default)]
    cache_write: u64,
}

fn normalize_records(
    server_name: &str,
    response: RecordsResponse,
) -> Result<Vec<UsageRecord>, String> {
    response
        .records
        .into_iter()
        .map(|record| {
            let source = match record.source.as_str() {
                "codex" => Source::Codex,
                "claude" | "claude_code" => Source::ClaudeCode,
                "pi" => Source::Pi,
                "opencode" => Source::OpenCode,
                other => return Err(format!("unknown source in response: {other}")),
            };
            let total_tokens = record
                .input
                .saturating_add(record.output)
                .saturating_add(record.cache_read)
                .saturating_add(record.cache_write);
            Ok(UsageRecord {
                source,
                account: server_name.to_string(),
                timestamp: record.timestamp,
                model: record.model,
                cost_usd: record.cost_usd,
                input_tokens: record.input,
                cached_input_tokens: record.cache_read,
                cache_creation_input_tokens: record.cache_write,
                output_tokens: record.output,
                reasoning_output_tokens: record.reasoning,
                total_tokens,
                is_subagent: false,
            })
        })
        .collect()
}

/// Fetch every record exposed by one remote server. Failures are returned to
/// the caller so the cache can retain the last successful collection.
pub fn fetch_records(
    server: &TokenApiServer,
    timeout_seconds: u64,
) -> Result<Vec<UsageRecord>, String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|error| format!("failed to create HTTP client: {error}"))?;
    let url = format!("{}/records?limit=1000000", server.url);
    let response = client
        .get(&url)
        .send()
        .map_err(|error| format!("GET {url} failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GET {url} failed: {error}"))?
        .json::<RecordsResponse>()
        .map_err(|error| format!("invalid response from {url}: {error}"))?;
    normalize_records(&server.name, response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_remote_fields_and_claude_source() {
        let response: RecordsResponse = serde_json::from_str(
            r#"{"records":[{"source":"claude","timestamp":"2026-09-01T14:16:00.539Z","model":"claude-opus-5","cost_usd":null,"input":2,"output":790,"reasoning":12,"cache_read":86781,"cache_write":754}]}"#,
        )
        .unwrap();
        let records = normalize_records("server-169", response).unwrap();
        let record = &records[0];
        assert_eq!(record.source, Source::ClaudeCode);
        assert_eq!(record.account, "server-169");
        assert_eq!(record.total_tokens, 88_327);
        assert_eq!(record.reasoning_output_tokens, 12);
    }

    #[test]
    fn rejects_unknown_sources() {
        let response: RecordsResponse = serde_json::from_str(
            r#"{"records":[{"source":"other","timestamp":"2026-09-01T00:00:00Z","model":null,"cost_usd":null,"input":0,"output":0}]}"#,
        )
        .unwrap();
        assert!(normalize_records("remote", response).is_err());
    }
}
