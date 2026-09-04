use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::component::input::EditorState;
use gpui_kit::{App, Context, Pixels, Point, Window};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::paths::BeamPaths;
use crate::script::{EnvironmentChange, TestResult};

use super::super::{BeamView, HttpResponseSnapshot};
use super::history::{
    RequestHistoryExecution, RequestHistoryFile, RequestHistoryHeader, RequestHistoryMeta,
    RequestHistoryResponseSummary,
};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub(in crate::ui) struct PersistedScriptResult {
    pub(in crate::ui) request_id: String,
    pub(in crate::ui) success: bool,
    pub(in crate::ui) failed: bool,
    pub(in crate::ui) error_type: Option<String>,
    pub(in crate::ui) error_message: Option<String>,
    pub(in crate::ui) failure_message: Option<String>,
    #[serde(default)]
    pub(in crate::ui) console_output: Vec<ConsoleMessageView>,
    #[serde(default)]
    pub(in crate::ui) test_results: Vec<TestResult>,
    #[serde(default)]
    pub(in crate::ui) environment_diff: Vec<EnvironmentChange>,
    #[serde(default)]
    pub(in crate::ui) no_environment_selected_with_env_writes: bool,
    pub(in crate::ui) updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(in crate::ui) struct ConsoleMessageView {
    pub(in crate::ui) level: String,
    pub(in crate::ui) message: String,
    pub(in crate::ui) timestamp: String,
}

pub(in crate::ui) fn script_result_file_path(paths: &BeamPaths, request_id: Ulid) -> PathBuf {
    paths
        .local_dir
        .join("script_results")
        .join(format!("{request_id}.toml"))
}

pub(in crate::ui) fn load_script_result(
    paths: &BeamPaths,
    request_id: Ulid,
) -> Option<PersistedScriptResult> {
    let path = script_result_file_path(paths, request_id);
    let content = fs::read_to_string(path).ok()?;
    let parsed: PersistedScriptResult = toml::from_str(&content).ok()?;
    (parsed.request_id == request_id.to_string()).then_some(parsed)
}

pub(in crate::ui) fn persist_script_result(
    paths: &BeamPaths,
    request_id: Ulid,
    result: &PersistedScriptResult,
) -> Result<(), String> {
    let dir = paths.local_dir.join("script_results");
    fs::create_dir_all(&dir)
        .map_err(|error| format!("Failed to create script_results directory: {error}"))?;
    let path = dir.join(format!("{request_id}.toml"));
    let content = toml::to_string_pretty(result)
        .map_err(|error| format!("Failed to encode script result: {error}"))?;
    fs::write(path, content).map_err(|error| format!("Failed to write script result: {error}"))
}

pub(in crate::ui) fn clear_script_result_for_request(
    paths: &BeamPaths,
    request_id: Ulid,
) -> Result<(), String> {
    let path = script_result_file_path(paths, request_id);
    if path.exists() {
        fs::remove_file(path).map_err(|error| format!("Failed to clear script result: {error}"))?;
    }
    Ok(())
}

pub(in crate::ui) fn persist_response_snapshot(
    paths: &BeamPaths,
    request_id: Ulid,
    response: &HttpResponseSnapshot,
) -> Result<(), String> {
    let history_dir = paths.local_dir.join("history");
    let by_request_dir = history_dir.join("by-request");
    let responses_dir = history_dir.join("responses");
    fs::create_dir_all(&by_request_dir)
        .map_err(|error| format!("Failed to create history directory: {error}"))?;
    fs::create_dir_all(&responses_dir)
        .map_err(|error| format!("Failed to create responses directory: {error}"))?;

    let history_path = by_request_dir.join(format!("{request_id}.history.toml"));
    let mut history_file = fs::read_to_string(&history_path)
        .ok()
        .and_then(|content| toml::from_str::<RequestHistoryFile>(&content).ok())
        .unwrap_or_default();

    let execution_id = Ulid::new().to_string();
    let body_ref = format!("{execution_id}.response.bin");
    fs::write(responses_dir.join(&body_ref), response.body.as_bytes())
        .map_err(|error| format!("Failed to write response body: {error}"))?;

    history_file.meta = Some(RequestHistoryMeta {
        request_id: request_id.to_string(),
        schema_version: Some(1),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
    });
    history_file.executions.push(RequestHistoryExecution {
        timestamp: Some(response.timestamp.clone()),
        status: response.status_code,
        duration_ms: parse_response_duration_ms(&response.time),
        response_summary: Some(RequestHistoryResponseSummary {
            body_bytes: Some(response.body.len() as u64),
            body_ref: Some(body_ref),
            body_truncated: false,
            headers: parse_response_headers(&response.headers)
                .into_iter()
                .map(|(name, value)| RequestHistoryHeader { name, value })
                .collect(),
        }),
    });

    let content = toml::to_string_pretty(&history_file)
        .map_err(|error| format!("Failed to encode history file: {error}"))?;
    fs::write(history_path, content)
        .map_err(|error| format!("Failed to write history file: {error}"))
}

fn parse_response_duration_ms(time: &str) -> Option<u64> {
    time.strip_suffix(" ms")
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn parse_response_headers(headers: &str) -> Vec<(String, String)> {
    headers
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

impl BeamView {
    pub(in crate::ui) fn on_response_body_editor_updated(&mut self, cx: &mut Context<Self>) {
        if self.suppress_response_scroll_offset_persistence {
            return;
        }
        self.schedule_response_scroll_offset_persistence(cx);
    }

    fn schedule_response_scroll_offset_persistence(&mut self, cx: &mut Context<Self>) {
        if !self.response_scroll_offset_needs_persist(cx) {
            return;
        }
        self.pending_response_scroll_offset_persistence_due_at =
            Some(Instant::now() + Duration::from_millis(150));
        if self.response_scroll_offset_persistence_tick_scheduled {
            return;
        }
        self.response_scroll_offset_persistence_tick_scheduled = true;
        self.schedule_response_scroll_offset_persistence_tick(cx);
    }

    fn process_pending_response_scroll_offset_persistence(&mut self, cx: &mut Context<Self>) {
        let Some(due_at) = self.pending_response_scroll_offset_persistence_due_at else {
            self.response_scroll_offset_persistence_tick_scheduled = false;
            return;
        };
        if Instant::now() < due_at {
            self.schedule_response_scroll_offset_persistence_tick(cx);
            return;
        }
        self.pending_response_scroll_offset_persistence_due_at = None;
        self.response_scroll_offset_persistence_tick_scheduled = false;
        if self.response_scroll_offset_needs_persist(cx) {
            self.persist_current_response_scroll_offset(cx);
        }
    }

    fn schedule_response_scroll_offset_persistence_tick(&self, cx: &mut Context<Self>) {
        let view = cx.entity();

        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .spawn(async move {
                    std::thread::sleep(Duration::from_millis(25));
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.process_pending_response_scroll_offset_persistence(cx);
            });
        })
        .detach();
    }

    fn response_scroll_offset_needs_persist(&self, cx: &App) -> bool {
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            return false;
        };
        let Some(pane_data) = self.shell.request_pane_data.get(&request_id) else {
            return false;
        };
        self.current_response_scroll_offset(cx) != pane_data.response_scroll_offset
    }

    fn current_response_scroll_offset(&self, cx: &App) -> Point<Pixels> {
        self.response_body_editor.read(cx).scroll_offset()
    }

    pub(in crate::ui) fn persist_current_response_scroll_offset(&mut self, cx: &App) {
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            return;
        };
        self.persist_response_scroll_offset_for_request(request_id, cx);
    }

    pub(in crate::ui) fn update_response_body_editor_with_scroll_persistence_suppressed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut EditorState, &mut Window, &mut Context<EditorState>),
    ) {
        self.suppress_response_scroll_offset_persistence = true;
        self.response_body_editor.update(cx, |input, cx| {
            update(input, window, cx);
        });
        self.suppress_response_scroll_offset_persistence = false;
    }

    pub(in crate::ui) fn restore_selected_request_response_scroll_offset(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            return;
        };
        let Some(pane_data) = self.shell.request_pane_data.get(&request_id) else {
            return;
        };

        let response_scroll_offset = pane_data.response_scroll_offset;

        let previous_focus = window.focused(cx);
        self.response_body_editor.update(cx, |input, cx| {
            input.set_scroll_offset(response_scroll_offset, cx);
        });
        if let Some(previous_focus) = previous_focus {
            previous_focus.focus(window, cx);
        }
    }

    fn persist_response_scroll_offset_for_request(&mut self, request_id: Ulid, cx: &App) {
        let response_scroll_offset = self.current_response_scroll_offset(cx);
        if let Some(pane_data) = self.shell.request_pane_data.get_mut(&request_id) {
            pane_data.response_scroll_offset = response_scroll_offset;
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use ulid::Ulid;

    use crate::paths::BeamPaths;

    use super::{
        PersistedScriptResult, clear_script_result_for_request, load_script_result,
        persist_response_snapshot, persist_script_result,
    };
    use crate::ui::HttpResponseSnapshot;
    use crate::ui::response::history::{
        load_response_history_entries, load_response_snapshot_for_history_entry,
    };

    #[test]
    fn response_snapshot_round_trips_through_history_files() {
        let temp = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(temp.path().to_path_buf());
        let request_id = Ulid::new();
        let response = HttpResponseSnapshot {
            status: "Created".to_string(),
            status_code: Some(201),
            time: "37 ms".to_string(),
            size: "7 B".to_string(),
            timestamp: "2026-07-25T00:00:00Z".to_string(),
            body: "created".to_string(),
            headers: "Content-Type: text/plain\nX-Test: yes".to_string(),
            content_type: Some("text/plain".to_string()),
        };

        persist_response_snapshot(&paths, request_id, &response).expect("persist response");
        let entries = load_response_history_entries(&paths, request_id);
        let restored = load_response_snapshot_for_history_entry(&paths, &entries[0]);

        assert_eq!(entries.len(), 1);
        assert_eq!(restored.status_code, Some(201));
        assert_eq!(restored.time, "37 ms");
        assert_eq!(restored.body, "created");
        assert_eq!(
            restored.headers_raw,
            "Content-Type: text/plain\nX-Test: yes"
        );
        assert_eq!(restored.content_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn script_result_round_trips_and_can_be_cleared() {
        let temp = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(temp.path().to_path_buf());
        let request_id = Ulid::new();
        let result = PersistedScriptResult {
            request_id: request_id.to_string(),
            success: true,
            updated_at: "2026-07-25T00:00:00Z".to_string(),
            ..PersistedScriptResult::default()
        };

        persist_script_result(&paths, request_id, &result).expect("persist script result");
        let restored = load_script_result(&paths, request_id).expect("load script result");
        assert!(restored.success);
        assert_eq!(restored.request_id, request_id.to_string());

        clear_script_result_for_request(&paths, request_id).expect("clear script result");
        assert!(load_script_result(&paths, request_id).is_none());
    }
}
