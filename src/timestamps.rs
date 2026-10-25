//! Timestamp formatting shared by persistence, dynamic variables, and local displays.

use time::{
    OffsetDateTime, UtcOffset,
    format_description::{BorrowedFormatItem, well_known::Rfc3339},
    macros::format_description,
};

pub(crate) const DATE_FORMAT: &[BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day]");
pub(crate) const MINUTE_FORMAT: &[BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]");
pub(crate) const DATE_TIME_FORMAT: &[BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
pub(crate) const TIME_FORMAT: &[BorrowedFormatItem<'_>] =
    format_description!("[hour]:[minute]:[second]");
pub(crate) const LOG_FORMAT: &[BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]");

pub(crate) fn format_timestamp(timestamp: OffsetDateTime) -> String {
    timestamp
        .format(&Rfc3339)
        .unwrap_or_else(|_| timestamp.to_string())
}

pub(crate) fn format_iso_millis(timestamp: OffsetDateTime) -> String {
    let utc = timestamp.to_offset(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        utc.year(),
        u8::from(utc.month()),
        utc.day(),
        utc.hour(),
        utc.minute(),
        utc.second(),
        utc.millisecond(),
    )
}

pub(crate) fn format_local_input(input: &str, format: &[BorrowedFormatItem<'_>]) -> String {
    OffsetDateTime::parse(input, &Rfc3339)
        .map(|timestamp| format_local(timestamp, format))
        .unwrap_or_else(|_| input.to_owned())
}

pub(crate) fn format_local(timestamp: OffsetDateTime, format: &[BorrowedFormatItem<'_>]) -> String {
    local_time(timestamp)
        .format(format)
        .unwrap_or_else(|_| format_timestamp(timestamp))
}

pub(crate) fn local_time(timestamp: OffsetDateTime) -> OffsetDateTime {
    let offset = UtcOffset::local_offset_at(timestamp).unwrap_or(UtcOffset::UTC);
    timestamp.to_offset(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::WorkspaceFile, script::ConsoleMessage};

    #[test]
    fn reads_existing_toml_timestamps_and_writes_rfc3339_strings() {
        for timestamp in [
            "2026-01-02T03:04:56+00:00",
            "2026-01-02T03:04:56.123Z",
            "2026-01-02T03:04:56.123456789+05:30",
        ] {
            let workspace = WorkspaceFile::new("Timestamp test");
            let mut value = toml::Value::try_from(&workspace).expect("serialize workspace");
            value["workspace"]["created_at"] = timestamp.into();
            value["workspace"]["updated_at"] = timestamp.into();
            let parsed: WorkspaceFile = value.try_into().expect("read existing timestamp");
            let expected = OffsetDateTime::parse(timestamp, &Rfc3339).expect("parse fixture");
            assert_eq!(parsed.workspace.created_at, expected);
            let encoded = toml::to_string(&parsed).expect("serialize timestamp");
            let decoded: WorkspaceFile =
                toml::from_str(&encoded).expect("read serialized timestamp");
            assert_eq!(decoded, parsed);
            let encoded: toml::Value = toml::from_str(&encoded).expect("parse serialized TOML");
            assert!(encoded["workspace"]["created_at"].is_str());
        }
    }

    #[test]
    fn reads_existing_console_json_with_nanoseconds() {
        let json =
            r#"{"level":"Log","message":"ok","timestamp":"2026-01-02T03:04:56.123456789+00:00"}"#;
        let message: ConsoleMessage = serde_json::from_str(json).expect("read console history");
        assert_eq!(message.timestamp.nanosecond(), 123456789);
        let json = serde_json::to_value(&message).expect("write console history");
        assert!(json["timestamp"].is_string());
        let decoded: ConsoleMessage = serde_json::from_value(json).expect("read console history");
        assert_eq!(decoded.timestamp, message.timestamp);
    }

    #[test]
    fn iso_timestamp_has_exactly_three_fractional_digits_in_utc() {
        for (input, expected) in [
            ("2020-06-09T21:10:36Z", "2020-06-09T21:10:36.000Z"),
            ("2020-06-09T21:10:36.1Z", "2020-06-09T21:10:36.100Z"),
            (
                "2020-06-10T02:40:36.123999+05:30",
                "2020-06-09T21:10:36.123Z",
            ),
        ] {
            let timestamp = OffsetDateTime::parse(input, &Rfc3339).expect("parse timestamp");
            assert_eq!(format_iso_millis(timestamp), expected);
        }
    }

    #[test]
    fn local_offset_lookup_works_on_worker_threads() {
        std::thread::spawn(|| {
            let now = OffsetDateTime::now_utc();
            let offset = UtcOffset::local_offset_at(now).expect("system timezone on worker thread");
            assert_eq!(local_time(now).offset(), offset);
            assert_eq!(
                local_time(now).unix_timestamp_nanos(),
                now.unix_timestamp_nanos()
            );
        })
        .join()
        .expect("worker thread");
    }
}
