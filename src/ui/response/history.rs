use std::fs;

use chrono::Local;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::paths::BeamPaths;

use std::sync::Arc;

use gpui_kit::gpui;

use super::super::format_bytes;
use super::{
    ImagePreview, classify_image_preview, image_placeholder_body, response_image_from_bytes,
    unavailable_image_body,
};

const RESPONSE_BODY_TRUNCATED_NOTE: &str =
    "[Response body omitted from local history (truncated).]";

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub(in crate::ui) struct RequestHistoryFile {
    #[serde(default)]
    pub(in crate::ui) meta: Option<RequestHistoryMeta>,
    #[serde(default)]
    pub(in crate::ui) executions: Vec<RequestHistoryExecution>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(in crate::ui) struct RequestHistoryMeta {
    pub(in crate::ui) request_id: String,
    #[serde(default)]
    pub(in crate::ui) schema_version: Option<u32>,
    #[serde(default)]
    pub(in crate::ui) updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(in crate::ui) struct RequestHistoryExecution {
    #[serde(default)]
    pub(in crate::ui) timestamp: Option<String>,
    pub(in crate::ui) status: Option<u16>,
    pub(in crate::ui) duration_ms: Option<u64>,
    pub(in crate::ui) response_summary: Option<RequestHistoryResponseSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(in crate::ui) struct RequestHistoryResponseSummary {
    pub(in crate::ui) body_bytes: Option<u64>,
    pub(in crate::ui) body_ref: Option<String>,
    #[serde(default)]
    pub(in crate::ui) body_truncated: bool,
    #[serde(default)]
    pub(in crate::ui) headers: Vec<RequestHistoryHeader>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(in crate::ui) struct RequestHistoryHeader {
    pub(in crate::ui) name: String,
    pub(in crate::ui) value: String,
}

#[derive(Clone)]
pub(in crate::ui) struct ResponseHistoryEntry {
    pub(in crate::ui) timestamp_text: String,
    pub(in crate::ui) status_text: String,
    pub(in crate::ui) execution: RequestHistoryExecution,
}

#[derive(Clone)]
pub(in crate::ui) struct StoredResponseSnapshot {
    pub(in crate::ui) status: String,
    pub(in crate::ui) status_code: Option<u16>,
    pub(in crate::ui) time: String,
    pub(in crate::ui) size: String,
    pub(in crate::ui) body: String,
    pub(in crate::ui) headers_raw: String,
    pub(in crate::ui) content_type: Option<String>,
    pub(in crate::ui) image: Option<Arc<gpui::Image>>,
}

pub(in crate::ui) fn load_request_history_file(
    paths: &BeamPaths,
    request_id: Ulid,
) -> Option<RequestHistoryFile> {
    let history_file_path = paths
        .local_dir
        .join("history/by-request")
        .join(format!("{request_id}.history.toml"));
    let content = fs::read_to_string(history_file_path).ok()?;
    toml::from_str(&content).ok()
}

pub(in crate::ui) fn response_snapshot_from_history_execution(
    paths: &BeamPaths,
    execution: &RequestHistoryExecution,
) -> StoredResponseSnapshot {
    let (status, status_code, time, size) = response_history_summary_parts(execution);
    let mut body = String::new();
    let mut headers_raw = String::new();
    let mut content_type = None;
    let mut image = None;

    if let Some(summary) = execution.response_summary.as_ref() {
        if !summary.headers.is_empty() {
            headers_raw = summary
                .headers
                .iter()
                .map(|header| format!("{}: {}", header.name, header.value))
                .collect::<Vec<_>>()
                .join("\n");
            content_type = summary
                .headers
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case("content-type"))
                .map(|header| header.value.clone());
        }
        body = if summary.body_truncated {
            RESPONSE_BODY_TRUNCATED_NOTE.to_string()
        } else if let Some(body_ref) = summary.body_ref.as_ref() {
            let body_path = paths.local_dir.join("history/responses").join(body_ref);
            let bytes = fs::read(body_path).unwrap_or_default();
            // Bodies saved under an earlier, looser limit are raw image bytes
            // that no longer qualify, so they get the note, not lossy text.
            match classify_image_preview(content_type.as_deref(), &bytes) {
                ImagePreview::NotImage => String::from_utf8_lossy(&bytes).to_string(),
                ImagePreview::Unavailable(note) => unavailable_image_body(&bytes, note),
                ImagePreview::Ready => {
                    let placeholder = image_placeholder_body(bytes.len());
                    image = response_image_from_bytes(content_type.as_deref(), bytes);
                    placeholder
                }
            }
        } else {
            String::new()
        };
    }

    StoredResponseSnapshot {
        status,
        status_code,
        time,
        size,
        body,
        headers_raw,
        content_type,
        image,
    }
}

pub(in crate::ui) fn response_history_summary_parts(
    execution: &RequestHistoryExecution,
) -> (String, Option<u16>, String, String) {
    let status = execution
        .status
        .map(|code| code.to_string())
        .unwrap_or_else(|| "—".to_string());
    let time = execution
        .duration_ms
        .map(|ms| format!("{ms} ms"))
        .unwrap_or_else(|| "—".to_string());
    let size = execution
        .response_summary
        .as_ref()
        .and_then(|summary| summary.body_bytes)
        .and_then(|n| usize::try_from(n).ok())
        .map(format_bytes)
        .unwrap_or_else(|| "—".to_string());

    (status, execution.status, time, size)
}

pub(in crate::ui) fn load_response_snapshot_for_history_entry(
    paths: &BeamPaths,
    entry: &ResponseHistoryEntry,
) -> StoredResponseSnapshot {
    response_snapshot_from_history_execution(paths, &entry.execution)
}

pub(in crate::ui) fn load_response_history_entries(
    paths: &BeamPaths,
    request_id: Ulid,
) -> Vec<ResponseHistoryEntry> {
    let Some(history_file) = load_request_history_file(paths, request_id) else {
        return Vec::new();
    };

    history_file
        .executions
        .iter()
        .rev()
        .map(|execution| {
            let timestamp_text = execution
                .timestamp
                .as_deref()
                .map(format_human_timestamp)
                .unwrap_or_else(|| "Unknown time".to_string());
            let status_text = execution
                .status
                .map(|code| code.to_string())
                .unwrap_or_else(|| "—".to_string());

            ResponseHistoryEntry {
                timestamp_text,
                status_text,
                execution: execution.clone(),
            }
        })
        .collect()
}

/// Compact relative text ("just now", "5m ago", "3h ago", "2d ago"), falling
/// back to the absolute date for entries older than a week.
pub(in crate::ui) fn format_relative_timestamp(timestamp: &str) -> String {
    format_relative_timestamp_at(timestamp, Local::now())
}

fn format_relative_timestamp_at(timestamp: &str, now: chrono::DateTime<Local>) -> String {
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(timestamp) else {
        return timestamp.to_string();
    };
    let elapsed = now.signed_duration_since(parsed);
    let seconds = elapsed.num_seconds();
    if seconds < 60 {
        "just now".to_string()
    } else if seconds < 3600 {
        format!("{}m ago", elapsed.num_minutes())
    } else if seconds < 86_400 {
        format!("{}h ago", elapsed.num_hours())
    } else if elapsed.num_days() < 7 {
        format!("{}d ago", elapsed.num_days())
    } else {
        parsed.with_timezone(&Local).format("%Y-%m-%d").to_string()
    }
}

fn format_human_timestamp(timestamp: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|parsed| {
            parsed
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| timestamp.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use ulid::Ulid;

    use crate::paths::BeamPaths;

    use super::{
        RequestHistoryExecution, RequestHistoryFile, RequestHistoryHeader,
        RequestHistoryResponseSummary, format_human_timestamp, format_relative_timestamp_at,
        load_response_history_entries, response_snapshot_from_history_execution,
    };

    #[test]
    fn response_history_loads_newest_execution_first() {
        let temp = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(temp.path().to_path_buf());
        let request_id = Ulid::new();
        let history_dir = paths.local_dir.join("history/by-request");
        fs::create_dir_all(&history_dir).expect("create history directory");
        let history = RequestHistoryFile {
            meta: None,
            executions: vec![
                RequestHistoryExecution {
                    timestamp: Some("2026-01-01T00:00:00Z".to_string()),
                    status: Some(200),
                    duration_ms: Some(10),
                    response_summary: None,
                },
                RequestHistoryExecution {
                    timestamp: Some("2026-01-02T00:00:00Z".to_string()),
                    status: Some(201),
                    duration_ms: Some(20),
                    response_summary: None,
                },
            ],
        };
        fs::write(
            history_dir.join(format!("{request_id}.history.toml")),
            toml::to_string(&history).expect("encode history"),
        )
        .expect("write history");

        let entries = load_response_history_entries(&paths, request_id);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status_text, "201");
        assert_eq!(entries[1].status_text, "200");
    }

    #[test]
    fn missing_or_corrupted_history_degrades_to_empty() {
        let temp = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(temp.path().to_path_buf());
        let request_id = Ulid::new();

        assert!(load_response_history_entries(&paths, request_id).is_empty());

        let history_dir = paths.local_dir.join("history/by-request");
        fs::create_dir_all(&history_dir).expect("create history directory");
        fs::write(
            history_dir.join(format!("{request_id}.history.toml")),
            "not valid toml =",
        )
        .expect("write corrupted history");

        assert!(load_response_history_entries(&paths, request_id).is_empty());
    }

    #[test]
    fn response_snapshot_restores_body_headers_and_content_type() {
        let temp = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(temp.path().to_path_buf());
        let responses_dir = paths.local_dir.join("history/responses");
        fs::create_dir_all(&responses_dir).expect("create responses directory");
        fs::write(responses_dir.join("body.response.bin"), b"{\"ok\":true}").expect("write body");
        let execution = RequestHistoryExecution {
            timestamp: None,
            status: Some(200),
            duration_ms: Some(42),
            response_summary: Some(RequestHistoryResponseSummary {
                body_bytes: Some(11),
                body_ref: Some("body.response.bin".to_string()),
                body_truncated: false,
                headers: vec![RequestHistoryHeader {
                    name: "Content-Type".to_string(),
                    value: "application/json".to_string(),
                }],
            }),
        };

        let snapshot = response_snapshot_from_history_execution(&paths, &execution);

        assert_eq!(snapshot.status, "200");
        assert_eq!(snapshot.time, "42 ms");
        assert_eq!(snapshot.body, "{\"ok\":true}");
        assert_eq!(snapshot.headers_raw, "Content-Type: application/json");
        assert_eq!(snapshot.content_type.as_deref(), Some("application/json"));
    }

    #[test]
    fn human_timestamp_includes_seconds_in_local_time() {
        let timestamp = "2026-01-02T03:04:56+05:30";
        let local = chrono::DateTime::parse_from_rfc3339(timestamp)
            .expect("valid timestamp")
            .with_timezone(&chrono::Local);

        assert_eq!(
            format_human_timestamp(timestamp),
            format!("{} {}", local.date_naive(), local.time())
        );
    }

    #[test]
    fn human_timestamp_returns_invalid_input_unchanged() {
        assert_eq!(format_human_timestamp("not a timestamp"), "not a timestamp");
    }

    #[test]
    fn relative_timestamp_buckets_elapsed_time() {
        use chrono::{Duration, Local};

        let now = Local::now();
        let at = |delta: Duration| (now - delta).to_rfc3339();

        assert_eq!(
            format_relative_timestamp_at(&at(Duration::seconds(30)), now),
            "just now"
        );
        assert_eq!(
            format_relative_timestamp_at(&at(Duration::minutes(5)), now),
            "5m ago"
        );
        assert_eq!(
            format_relative_timestamp_at(&at(Duration::hours(3)), now),
            "3h ago"
        );
        assert_eq!(
            format_relative_timestamp_at(&at(Duration::days(2)), now),
            "2d ago"
        );
    }

    #[test]
    fn relative_timestamp_falls_back_to_date_after_a_week() {
        use chrono::{Duration, Local};

        let now = Local::now();
        let old = now - Duration::days(10);
        assert_eq!(
            format_relative_timestamp_at(&old.to_rfc3339(), now),
            old.format("%Y-%m-%d").to_string()
        );
    }

    #[test]
    fn relative_timestamp_returns_invalid_input_unchanged() {
        assert_eq!(
            format_relative_timestamp_at("not a timestamp", chrono::Local::now()),
            "not a timestamp"
        );
    }

    #[test]
    fn image_format_maps_supported_content_types_only() {
        use crate::ui::response::response_image_format;
        use gpui_kit::gpui::ImageFormat;

        assert_eq!(
            response_image_format(Some("image/PNG; charset=binary")),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            response_image_format(Some("image/jpeg")),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(response_image_format(Some("image/svg+xml")), None);
        assert_eq!(response_image_format(Some("text/html")), None);
        assert_eq!(response_image_format(None), None);
    }

    #[test]
    fn image_bytes_survive_history_round_trip() {
        use crate::ui::HttpResponseSnapshot;
        use crate::ui::response::persistence::persist_response_snapshot;

        let temp = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(temp.path().to_path_buf());
        let request_id = Ulid::new();
        // PNG bytes are not valid UTF-8.
        let bytes = png_bytes(2, 2);
        let response = HttpResponseSnapshot {
            status: "OK".to_string(),
            status_code: Some(200),
            time: "1 ms".to_string(),
            size: "8 B".to_string(),
            timestamp: "2026-11-07T00:00:00Z".to_string(),
            body: crate::ui::response::image_placeholder_body(bytes.len()),
            headers: "Content-Type: image/png".to_string(),
            content_type: Some("image/png".to_string()),
            image: crate::ui::response::response_image_from_bytes(Some("image/png"), bytes.clone()),
        };

        persist_response_snapshot(&paths, request_id, &response).expect("persist");
        let entries = load_response_history_entries(&paths, request_id);
        let restored = response_snapshot_from_history_execution(&paths, &entries[0].execution);

        assert_eq!(
            restored.image.as_ref().map(|image| image.bytes()),
            Some(bytes.as_slice())
        );
        assert_eq!(restored.body, response.body);
    }

    #[test]
    fn classify_rejects_oversized_and_unreadable_images() {
        use crate::ui::response::{ImagePreview, classify_image_preview};

        let png = Some("image/png");
        assert!(matches!(
            classify_image_preview(png, &png_bytes(2, 2)),
            ImagePreview::Ready
        ));
        assert!(matches!(
            classify_image_preview(png, &png_bytes(9000, 1)),
            ImagePreview::Unavailable(message) if message.contains("9000x1")
        ));
        assert!(matches!(
            classify_image_preview(png, &vec![0; 21 * 1024 * 1024]),
            ImagePreview::Unavailable(message) if message.contains("too large")
        ));
        assert!(matches!(
            classify_image_preview(png, b"not an image"),
            ImagePreview::Unavailable(_)
        ));
        assert!(matches!(
            classify_image_preview(Some("text/plain"), b"hello"),
            ImagePreview::NotImage
        ));
    }

    #[test]
    fn gif_frame_budget_counts_width_height_and_frames() {
        use crate::ui::response::gif_frame_budget_exceeded;
        use gpui_kit::gpui::ImageFormat;

        let gif = gif_bytes(10, 10, 5);
        assert_eq!(
            gif_frame_budget_exceeded(ImageFormat::Gif, &gif, 10, 10, 500),
            None
        );
        assert_eq!(
            gif_frame_budget_exceeded(ImageFormat::Gif, &gif, 10, 10, 400),
            Some(5)
        );
        assert_eq!(
            gif_frame_budget_exceeded(ImageFormat::Png, &gif, 10, 10, 1),
            None
        );
    }

    fn gif_bytes(width: u16, height: u16, frames: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder =
                gif::Encoder::new(&mut bytes, width, height, &[0, 0, 0, 255, 255, 255])
                    .expect("gif encoder");
            for _ in 0..frames {
                let frame = gif::Frame {
                    width,
                    height,
                    buffer: std::borrow::Cow::Owned(vec![
                        0;
                        usize::from(width) * usize::from(height)
                    ]),
                    ..gif::Frame::default()
                };
                encoder.write_frame(&frame).expect("write frame");
            }
        }
        bytes
    }

    #[test]
    fn stored_image_over_current_limits_reads_back_as_note() {
        use crate::ui::HttpResponseSnapshot;
        use crate::ui::response::persistence::persist_response_snapshot;

        let temp = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(temp.path().to_path_buf());
        let request_id = Ulid::new();
        let bytes = png_bytes(9000, 1);
        let response = HttpResponseSnapshot {
            status: "OK".to_string(),
            status_code: Some(200),
            time: "1 ms".to_string(),
            size: String::new(),
            timestamp: "2026-11-07T00:00:00Z".to_string(),
            body: String::new(),
            headers: "Content-Type: image/png".to_string(),
            content_type: Some("image/png".to_string()),
            image: crate::ui::response::response_image_from_bytes(Some("image/png"), bytes),
        };

        persist_response_snapshot(&paths, request_id, &response).expect("persist");
        let entries = load_response_history_entries(&paths, request_id);
        let restored = response_snapshot_from_history_execution(&paths, &entries[0].execution);

        assert!(restored.image.is_none());
        assert!(
            restored.body.contains("dimensions too large"),
            "{}",
            restored.body
        );
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::new(width, height)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encode png");
        bytes
    }
}
