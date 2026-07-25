use std::fs;

use chrono::Local;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::paths::BeamPaths;

use super::super::format_bytes;

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
            fs::read(body_path)
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                .unwrap_or_default()
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

fn format_human_timestamp(timestamp: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|parsed| {
            parsed
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
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
        RequestHistoryResponseSummary, load_response_history_entries,
        response_snapshot_from_history_execution,
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
}
