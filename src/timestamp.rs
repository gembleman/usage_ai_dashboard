//! UTC timestamps shared by parsers, cache, and JSON responses.

use serde::{Deserialize, Deserializer, Serializer, de::Error};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

pub fn parse(value: &str) -> Result<OffsetDateTime, time::error::Parse> {
    OffsetDateTime::parse(value, &Rfc3339).map(|dt| dt.to_offset(UtcOffset::UTC))
}

pub fn format(value: OffsetDateTime) -> String {
    value
        .to_offset(UtcOffset::UTC)
        .format(&Rfc3339)
        .expect("RFC 3339 UTC timestamp")
}

pub fn hour(value: OffsetDateTime) -> String {
    let value = value.to_offset(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:00:00Z",
        value.year(),
        u8::from(value.month()),
        value.day(),
        value.hour()
    )
}

/// Matches SQLite's strftime('%Y-%m-%dT%H:%M:%fZ', ...) cache migration.
pub fn db_format(value: OffsetDateTime) -> String {
    let value = value.to_offset(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        value.year(),
        u8::from(value.month()),
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.millisecond()
    )
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<OffsetDateTime, D::Error> {
    let value = String::deserialize(deserializer)?;
    parse(&value).map_err(D::Error::custom)
}

pub fn deserialize_option<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<OffsetDateTime>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|value| parse(&value).map_err(D::Error::custom))
        .transpose()
}

pub fn serialize<S: Serializer>(value: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&format(*value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_offsets_and_preserves_cache_milliseconds() {
        let value = parse("2026-08-02T08:05:00.472088+08:00").unwrap();
        assert_eq!(format(value), "2026-08-02T00:05:00.472088Z");
        assert_eq!(hour(value), "2026-08-02T00:00:00Z");
        assert_eq!(db_format(value), "2026-08-02T00:05:00.472Z");
    }

    #[test]
    fn serializes_timestamp_as_rfc3339_string() {
        let value = parse("2026-08-02T00:05:00Z").unwrap();
        let record = crate::model::UsageRecord {
            source: crate::model::Source::Codex,
            account: "test".to_string(),
            timestamp: value,
            model: None,
            cost_usd: None,
            input_tokens: 0,
            cached_input_tokens: 0,
            cache_creation_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens: 0,
            is_subagent: false,
        };
        let json = serde_json::to_value(record).unwrap();
        assert_eq!(json["timestamp"], "2026-08-02T00:05:00Z");
    }
}
