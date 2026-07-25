use std::collections::HashMap;
use std::fs;
use std::sync::OnceLock;
use std::time::Instant;

use chrono::Utc;
use gpui::{Context, Window};
use reqwest::{Client, Method};
use tokio::runtime::{Builder as TokioRuntimeBuilder, Runtime as TokioRuntime};
use tokio::sync::oneshot;
use ulid::Ulid;

use super::super::BeamView;
use crate::app_shell::{AppCommand, next_command_id};
use crate::models::{AuthConfig, BodyConfig, EnvironmentVariable, HttpMethod};
use crate::request_authoring::{RequestAuthoringState, SendButtonState, SendDisabledReason};
use crate::script::{
    ConsoleLevel, ScriptExecutionResult, ScriptRuntimeResponse, execute_post_request_script,
};

use super::super::dialogs::EnvironmentManagerDialogView;
use super::super::response::history::load_response_history_entries;
use super::super::response::persistence::{
    ConsoleMessageView, PersistedScriptResult, clear_script_result_for_request,
    persist_response_snapshot, persist_script_result,
};
use super::super::response_body_editor_language;

pub(in crate::ui) const DEFAULT_API_KEY_HEADER_NAME: &str = "X-API-Key";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum RequestExecutionStatus {
    Idle,
    Sending,
    Canceled,
    Failed,
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    fn ready_request() -> RequestAuthoringState {
        RequestAuthoringState {
            url: "https://example.com".to_string(),
            ..RequestAuthoringState::default()
        }
    }

    #[test]
    fn send_button_state_is_scoped_to_selected_request() {
        let request_a = Ulid::new();
        let request_b = Ulid::new();
        let mut execution_states = HashMap::new();
        execution_states.insert(
            request_a,
            RequestExecutionState {
                run_id: 10,
                status: RequestExecutionStatus::Sending,
                cancel_tx: None,
            },
        );
        let request = ready_request();

        assert_eq!(
            send_button_state_for_selected_request(
                Some(request_a),
                &execution_states,
                &request,
                None
            ),
            SendButtonState::Sending
        );
        assert_eq!(
            send_button_state_for_selected_request(
                Some(request_b),
                &execution_states,
                &request,
                None
            ),
            SendButtonState::Ready
        );

        execution_states.insert(
            request_b,
            RequestExecutionState {
                run_id: 20,
                status: RequestExecutionStatus::Sending,
                cancel_tx: None,
            },
        );
        assert_eq!(
            send_button_state_for_selected_request(
                Some(request_a),
                &execution_states,
                &request,
                None
            ),
            SendButtonState::Sending
        );
        assert_eq!(
            send_button_state_for_selected_request(
                Some(request_b),
                &execution_states,
                &request,
                None
            ),
            SendButtonState::Sending
        );
    }

    #[test]
    fn stale_completion_does_not_overwrite_newer_run_state() {
        let request_id = Ulid::new();
        let mut execution_states = HashMap::new();
        execution_states.insert(
            request_id,
            RequestExecutionState {
                run_id: 2,
                status: RequestExecutionStatus::Sending,
                cancel_tx: None,
            },
        );

        assert!(!request_run_completion_is_current(
            &execution_states,
            request_id,
            1
        ));
        apply_request_run_completion_status(&mut execution_states, request_id, 1, true);
        assert_eq!(
            execution_states
                .get(&request_id)
                .expect("request execution state should exist")
                .status,
            RequestExecutionStatus::Sending
        );

        assert!(request_run_completion_is_current(
            &execution_states,
            request_id,
            2
        ));
        apply_request_run_completion_status(&mut execution_states, request_id, 2, true);
        assert_eq!(
            execution_states
                .get(&request_id)
                .expect("request execution state should exist")
                .status,
            RequestExecutionStatus::Idle
        );
    }

    #[test]
    fn completion_for_non_selected_request_keeps_selected_send_state() {
        let request_a = Ulid::new();
        let request_b = Ulid::new();
        let mut execution_states = HashMap::new();
        execution_states.insert(
            request_a,
            RequestExecutionState {
                run_id: 7,
                status: RequestExecutionStatus::Sending,
                cancel_tx: None,
            },
        );
        let request = ready_request();

        let before = send_button_state_for_selected_request(
            Some(request_b),
            &execution_states,
            &request,
            None,
        );
        apply_request_run_completion_status(&mut execution_states, request_a, 7, true);
        let after = send_button_state_for_selected_request(
            Some(request_b),
            &execution_states,
            &request,
            None,
        );

        assert_eq!(before, SendButtonState::Ready);
        assert_eq!(after, SendButtonState::Ready);
    }

    #[test]
    fn completion_updates_only_selected_request_ui_flow() {
        let request_a = Ulid::new();
        let request_b = Ulid::new();
        let mut execution_states = HashMap::new();
        execution_states.insert(
            request_a,
            RequestExecutionState {
                run_id: 11,
                status: RequestExecutionStatus::Sending,
                cancel_tx: None,
            },
        );
        let request = ready_request();

        // Simulate "A sending -> switch to B -> A completes".
        let selected_request_id = Some(request_b);
        assert!(!completion_updates_selected_request_ui(
            selected_request_id,
            request_a
        ));
        let before = send_button_state_for_selected_request(
            selected_request_id,
            &execution_states,
            &request,
            None,
        );
        apply_request_run_completion_status(&mut execution_states, request_a, 11, true);
        let after = send_button_state_for_selected_request(
            selected_request_id,
            &execution_states,
            &request,
            None,
        );

        assert_eq!(before, SendButtonState::Ready);
        assert_eq!(after, SendButtonState::Ready);
        assert!(completion_updates_selected_request_ui(
            Some(request_a),
            request_a
        ));
    }

    #[test]
    fn selected_request_runtime_state_overrides_response_summary() {
        let request_a = Ulid::new();
        let request_b = Ulid::new();
        let mut execution_states = HashMap::new();
        execution_states.insert(
            request_a,
            RequestExecutionState {
                run_id: 42,
                status: RequestExecutionStatus::Sending,
                cancel_tx: None,
            },
        );

        let selected_a = response_summary_for_selected_request(
            Some(request_a),
            &execution_states,
            "200",
            Some(200),
            "120 ms",
            "1.2 KB",
        );
        let selected_b = response_summary_for_selected_request(
            Some(request_b),
            &execution_states,
            "200",
            Some(200),
            "120 ms",
            "1.2 KB",
        );

        assert_eq!(
            selected_a,
            (
                "Sending...".to_string(),
                None,
                "—".to_string(),
                "—".to_string()
            )
        );
        assert_eq!(
            selected_b,
            (
                "200".to_string(),
                Some(200),
                "120 ms".to_string(),
                "1.2 KB".to_string()
            )
        );
    }

    #[test]
    fn parse_environment_file_accepts_current_format() {
        let content = r#"
schema_version = 1
variables = []

[environment]
environment_id = "01KSM9Y8VJ1ZMWX9X0W7G5EM70"
scope = "global"
name = "Default"
file_name = "default.env.toml"
created_at = "2026-05-27T08:30:00.000000Z"
updated_at = "2026-05-27T08:30:00.000000Z"
"#;

        let parsed = EnvironmentManagerDialogView::parse_environment_file(content)
            .expect("current environment format should parse");

        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.environment.file_name, "default.env.toml");
        assert!(parsed.variables.is_empty());
    }

    #[test]
    fn parse_environment_file_rejects_nested_schema_version_format() {
        let content = r#"
variables = []

[environment]
schema_version = 1
environment_id = "01KSM9Y8VJ1ZMWX9X0W7G5EM70"
scope = "global"
name = "Default"
created_at = "2026-05-27T08:30:00.000000Z"
updated_at = "2026-05-27T08:30:00.000000Z"
"#;

        let error = EnvironmentManagerDialogView::parse_environment_file(content)
            .expect_err("environment files with nested schema_version should be rejected");

        assert!(error.contains("Failed to parse environment file"));
    }
}

pub(in crate::ui) struct RequestExecutionState {
    pub(in crate::ui) run_id: u64,
    pub(in crate::ui) status: RequestExecutionStatus,
    pub(in crate::ui) cancel_tx: Option<oneshot::Sender<()>>,
}

pub(in crate::ui) struct HttpResponseSnapshot {
    pub(in crate::ui) status: String,
    pub(in crate::ui) status_code: Option<u16>,
    pub(in crate::ui) time: String,
    pub(in crate::ui) size: String,
    pub(in crate::ui) timestamp: String,
    pub(in crate::ui) body: String,
    pub(in crate::ui) headers: String,
    pub(in crate::ui) content_type: Option<String>,
}

struct RequestExecutionOutcome {
    response: HttpResponseSnapshot,
    script_result: Option<PersistedScriptResult>,
    updated_environment_variables: Option<Vec<EnvironmentVariable>>,
}

struct RequestRunCompletion {
    request_id: Ulid,
    run_id: u64,
    outcome: Option<RequestExecutionOutcome>,
}

static HTTP_CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
static HTTP_RUNTIME: OnceLock<Result<TokioRuntime, String>> = OnceLock::new();

impl BeamView {
    pub(in crate::ui) fn begin_request_run_for(&mut self, request_id: Ulid) -> u64 {
        let run_id = self.next_request_run_id;
        self.next_request_run_id = self.next_request_run_id.saturating_add(1);
        self.request_execution_states.insert(
            request_id,
            RequestExecutionState {
                run_id,
                status: RequestExecutionStatus::Sending,
                cancel_tx: None,
            },
        );
        run_id
    }

    pub(in crate::ui) fn cancel_request_run_for(&mut self, request_id: Ulid) {
        let Some(state) = self.request_execution_states.get_mut(&request_id) else {
            return;
        };
        if let Some(cancel_tx) = state.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }
        state.status = RequestExecutionStatus::Canceled;
    }

    pub(in crate::ui) fn clear_request_execution_state(&mut self, request_id: Ulid) {
        let Some(mut state) = self.request_execution_states.remove(&request_id) else {
            return;
        };
        if let Some(cancel_tx) = state.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }
    }

    pub(in crate::ui) fn prune_request_execution_states(&mut self) {
        let request_pane_data = &self.shell.request_pane_data;
        self.request_execution_states.retain(|request_id, state| {
            let keep = request_pane_data.contains_key(request_id);
            if !keep {
                if let Some(cancel_tx) = state.cancel_tx.take() {
                    let _ = cancel_tx.send(());
                }
            }
            keep
        });
    }

    pub(in crate::ui) fn is_request_sending(&self, request_id: Ulid) -> bool {
        self.request_execution_states
            .get(&request_id)
            .is_some_and(|state| state.status == RequestExecutionStatus::Sending)
    }

    pub(in crate::ui) fn cancel_active_request_wait(&mut self) {
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            return;
        };

        self.cancel_request_run_for(request_id);
        self.response_status = "Canceled".to_string();
        self.response_status_code = None;
        self.response_time = "—".to_string();
        self.response_size = "—".to_string();
    }

    pub(in crate::ui) fn send_button_state_for_view(&self) -> SendButtonState {
        send_button_state_for_selected_request(
            self.shell.workspace_tree.selected_request_id(),
            &self.request_execution_states,
            &self.request,
            self.selected_environment_id_for_view(),
        )
    }

    fn send_button_state_without_runtime(request: &RequestAuthoringState) -> SendButtonState {
        let trimmed = request.url.trim();
        if trimmed.is_empty() {
            return SendButtonState::Disabled(SendDisabledReason::EmptyUrl);
        }
        if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
            return SendButtonState::Disabled(SendDisabledReason::InvalidUrl);
        }
        SendButtonState::Ready
    }

    pub(in crate::ui) fn handle_send_or_cancel_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.send_button_state_for_view() {
            SendButtonState::Sending => self.cancel_active_request_wait(),
            SendButtonState::Disabled(SendDisabledReason::EmptyUrl) => {}
            _ => self.send_request(window, cx),
        }
    }

    pub(in crate::ui) fn send_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let latest_script = self.post_script_editor.read(cx).value().to_string();
        self.request.post_script = (!latest_script.trim().is_empty()).then_some(latest_script);
        if matches!(
            self.send_button_state_for_view(),
            SendButtonState::Disabled(SendDisabledReason::InvalidUrl)
        ) {
            self.show_invalid_url_border = true;
            cx.notify();
            return;
        }
        self.show_invalid_url_border = false;
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            return;
        };

        let selected_environment_id = self.selected_environment_id_for_view();
        let response_persistence_paths = self.current_workspace_paths.clone();
        let no_environment_selected = selected_environment_id.is_none();
        let environment_variables = self.load_environment_for_script(selected_environment_id);
        let request_snapshot =
            resolve_request_with_environment(self.request.clone(), &environment_variables);
        if !matches!(
            Self::send_button_state_without_runtime(&request_snapshot),
            SendButtonState::Ready
        ) {
            if matches!(
                Self::send_button_state_without_runtime(&request_snapshot),
                SendButtonState::Disabled(SendDisabledReason::InvalidUrl)
            ) {
                self.show_invalid_url_border = true;
                cx.notify();
            }
            return;
        }

        let run_id = self.begin_request_run_for(request_id);
        self.response_status = "Sending...".to_string();
        self.response_status_code = None;
        self.response_time = "—".to_string();
        self.response_size = "—".to_string();
        let http_runtime = match shared_http_runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                if let Some(state) = self.request_execution_states.get_mut(&request_id) {
                    if state.run_id == run_id {
                        state.cancel_tx = None;
                        state.status = RequestExecutionStatus::Failed;
                    }
                }
                self.response_status = "Error".to_string();
                self.response_status_code = None;
                self.update_response_body_editor_with_scroll_persistence_suppressed(
                    window,
                    cx,
                    |input, window, cx| input.set_value(error, window, cx),
                );
                cx.notify();
                return;
            }
        };

        let view = cx.entity();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (result_tx, result_rx) = oneshot::channel::<RequestRunCompletion>();
        if let Some(state) = self.request_execution_states.get_mut(&request_id) {
            if state.run_id == run_id {
                state.cancel_tx = Some(cancel_tx);
            }
        }

        http_runtime.spawn(async move {
            let request_future = execute_request_with_script(
                request_snapshot,
                Some(request_id),
                no_environment_selected,
                environment_variables,
            );
            let outcome = tokio::select! {
                _ = async { let _ = cancel_rx.await; } => None,
                outcome = request_future => Some(outcome),
            };
            let _ = result_tx.send(RequestRunCompletion {
                request_id,
                run_id,
                outcome,
            });
        });

        cx.spawn_in(window, async move |_, cx| {
            let Some(completion) = result_rx.await.ok() else {
                return;
            };
            let _ = view.update_in(cx, |this, window, cx| {
                let request_id = completion.request_id;
                let run_id = completion.run_id;
                let maybe_outcome = completion.outcome;
                if !request_run_completion_is_current(
                    &this.request_execution_states,
                    request_id,
                    run_id,
                ) {
                    return;
                }
                apply_request_run_completion_status(
                    &mut this.request_execution_states,
                    request_id,
                    run_id,
                    maybe_outcome.is_some(),
                );
                let should_update_visible_response = completion_updates_selected_request_ui(
                    this.shell.workspace_tree.selected_request_id(),
                    request_id,
                );
                let Some(outcome) = maybe_outcome else {
                    if should_update_visible_response {
                        this.response_status = "Canceled".to_string();
                        this.response_status_code = None;
                        this.response_time = "—".to_string();
                        this.response_size = "—".to_string();
                    }
                    cx.notify();
                    return;
                };

                let response = outcome.response;
                let response_body = this
                    .response_body_for_display(&response.body, response.content_type.as_deref());
                let response_language =
                    response_body_editor_language(response.content_type.as_deref());
                if should_update_visible_response {
                    this.response_status = response.status.clone();
                    this.response_status_code = response.status_code;
                    this.response_time = response.time.clone();
                    this.response_size = response.size.clone();
                    this.update_response_body_editor_with_scroll_persistence_suppressed(
                        window,
                        cx,
                        |input, window, cx| {
                            input.set_highlighter(response_language, cx);
                            input.set_value(response_body.clone(), window, cx);
                        },
                    );
                    this.response_body_language = response_language;
                    this.response_headers_raw = response.headers.clone();
                    this.response_content_type = response.content_type.clone();
                    this.script_result = outcome.script_result.clone();
                }
                if let (Some(environment_id), Some(variables)) = (
                    selected_environment_id,
                    outcome.updated_environment_variables,
                ) {
                    let command = AppCommand::UpdateEnvironmentVariables {
                        environment_id,
                        variables,
                        command_id: next_command_id(),
                    };
                    if let Err(error) = this.publish_app_command(command) {
                        log::error!(
                            "Failed to queue script-driven environment update command: {error}"
                        );
                    }
                }
                if let Err(error) =
                    persist_response_snapshot(&response_persistence_paths, request_id, &response)
                {
                    log::error!("Failed to persist response snapshot: {error}");
                }
                if Some(request_id) == this.shell.workspace_tree.selected_request_id() {
                    this.response_history_entries =
                        load_response_history_entries(&this.current_workspace_paths, request_id);
                }
                match outcome.script_result.as_ref() {
                    Some(script_result) => {
                        if let Err(error) = persist_script_result(
                            &response_persistence_paths,
                            request_id,
                            script_result,
                        ) {
                            log::error!("Failed to persist script result: {error}");
                        }
                    }
                    None => {
                        if let Err(error) =
                            clear_script_result_for_request(&response_persistence_paths, request_id)
                        {
                            log::error!("Failed to clear script result: {error}");
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();

        cx.notify();
    }

    pub(in crate::ui) fn load_environment_for_script(
        &self,
        selected_environment_id: Option<Ulid>,
    ) -> Vec<EnvironmentVariable> {
        let Some(environment_id) = selected_environment_id else {
            return Vec::new();
        };
        let Some(path) = self.environment_file_path_from_shell(environment_id) else {
            return Vec::new();
        };
        let Ok(content) = fs::read_to_string(&path) else {
            return Vec::new();
        };
        let Ok(parsed) = EnvironmentManagerDialogView::parse_environment_file(&content) else {
            return Vec::new();
        };
        parsed.variables
    }
}

async fn execute_request_with_script(
    request: RequestAuthoringState,
    request_id: Option<Ulid>,
    no_environment_selected: bool,
    environment_variables: Vec<EnvironmentVariable>,
) -> RequestExecutionOutcome {
    let response = execute_http_request(request.clone()).await;
    let script_text = request.post_script.clone().unwrap_or_default();
    if script_text.trim().is_empty() {
        return RequestExecutionOutcome {
            response,
            script_result: None,
            updated_environment_variables: None,
        };
    }

    let runtime_response = ScriptRuntimeResponse {
        status: response.status_code.unwrap_or(0),
        status_text: response.status.clone(),
        headers: parse_response_headers(&response.headers),
        body: response.body.clone(),
        response_time_ms: parse_response_duration_ms(&response.time).unwrap_or(0),
        body_size_bytes: response.body.len(),
    };
    let script_exec_result = execute_post_request_script(
        &script_text,
        &runtime_response,
        &environment_variables,
        !no_environment_selected,
    );
    let no_environment_selected_with_env_writes =
        no_environment_selected && script_contains_environment_mutation(&script_text);
    let updated_environment_variables = if script_exec_result.environment_changes.is_empty()
        && script_exec_result.removed_env_keys.is_empty()
    {
        None
    } else {
        Some(apply_script_environment_changes_to_variables(
            &environment_variables,
            &script_exec_result,
        ))
    };
    let request_id_text = request_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "unknown-request".to_string());
    RequestExecutionOutcome {
        response,
        script_result: Some(to_persisted_script_result(
            &script_exec_result,
            request_id_text,
            no_environment_selected_with_env_writes,
        )),
        updated_environment_variables,
    }
}

fn script_contains_environment_mutation(script: &str) -> bool {
    let lowered = script.to_ascii_lowercase();
    [
        "pm.environment.set(",
        "pm.environment.unset(",
        "pm.environment.clear(",
        "pm.environment.setall(",
        "pm.environment.setifpresent(",
        "pm.extract(",
    ]
    .iter()
    .any(|pattern| lowered.contains(pattern))
}

fn apply_script_environment_changes_to_variables(
    current_variables: &[EnvironmentVariable],
    script_result: &ScriptExecutionResult,
) -> Vec<EnvironmentVariable> {
    let mut next_variables = current_variables.to_vec();
    next_variables.retain(|var| {
        !script_result
            .removed_env_keys
            .iter()
            .any(|removed| removed == &var.name)
    });
    for (key, value) in &script_result.environment_changes {
        if let Some(var) = next_variables.iter_mut().find(|var| var.name == *key) {
            var.value = value.clone();
            var.enabled = true;
        } else {
            next_variables.push(EnvironmentVariable {
                name: key.clone(),
                value: value.clone(),
                enabled: true,
                description: None,
            });
        }
    }
    next_variables
}

fn to_persisted_script_result(
    result: &ScriptExecutionResult,
    request_id: String,
    no_environment_selected_with_env_writes: bool,
) -> PersistedScriptResult {
    PersistedScriptResult {
        request_id,
        success: result.success,
        failed: result.failed,
        error_type: result.error_type.map(|kind| format!("{kind:?}")),
        error_message: result.error_message.clone(),
        failure_message: result.failure_message.clone(),
        console_output: result
            .console_output
            .iter()
            .map(|entry| ConsoleMessageView {
                level: match entry.level {
                    ConsoleLevel::Log => "log".to_string(),
                    ConsoleLevel::Info => "info".to_string(),
                    ConsoleLevel::Warn => "warn".to_string(),
                    ConsoleLevel::Error => "error".to_string(),
                    ConsoleLevel::Debug => "debug".to_string(),
                },
                message: entry.message.clone(),
                timestamp: entry.timestamp.to_rfc3339(),
            })
            .collect(),
        test_results: result.test_results.clone(),
        environment_diff: result.environment_diff.clone(),
        no_environment_selected_with_env_writes,
        updated_at: Utc::now().to_rfc3339(),
    }
}

fn default_user_agent() -> String {
    format!("Beam/{}", env!("CARGO_PKG_VERSION"))
}

fn shared_http_runtime() -> Result<&'static TokioRuntime, String> {
    HTTP_RUNTIME
        .get_or_init(|| {
            TokioRuntimeBuilder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
                .map_err(|error| format!("Failed to initialize HTTP runtime: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn shared_http_client() -> Result<&'static Client, String> {
    HTTP_CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent(default_user_agent())
                .danger_accept_invalid_certs(true)
                .build()
                .map_err(|error| format!("Failed to initialize HTTP client: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub(in crate::ui) fn request_run_completion_is_current(
    execution_states: &HashMap<Ulid, RequestExecutionState>,
    request_id: Ulid,
    run_id: u64,
) -> bool {
    execution_states
        .get(&request_id)
        .is_some_and(|state| state.run_id == run_id)
}

pub(in crate::ui) fn apply_request_run_completion_status(
    execution_states: &mut HashMap<Ulid, RequestExecutionState>,
    request_id: Ulid,
    run_id: u64,
    completed: bool,
) {
    let Some(state) = execution_states.get_mut(&request_id) else {
        return;
    };
    if state.run_id != run_id {
        return;
    }
    state.cancel_tx = None;
    state.status = if completed {
        RequestExecutionStatus::Idle
    } else {
        RequestExecutionStatus::Canceled
    };
}

pub(in crate::ui) fn completion_updates_selected_request_ui(
    selected_request_id: Option<Ulid>,
    completed_request_id: Ulid,
) -> bool {
    selected_request_id == Some(completed_request_id)
}

pub(in crate::ui) fn response_summary_for_selected_request(
    selected_request_id: Option<Ulid>,
    execution_states: &HashMap<Ulid, RequestExecutionState>,
    fallback_status: &str,
    fallback_status_code: Option<u16>,
    fallback_time: &str,
    fallback_size: &str,
) -> (String, Option<u16>, String, String) {
    if let Some(request_id) = selected_request_id {
        if execution_states
            .get(&request_id)
            .is_some_and(|state| state.status == RequestExecutionStatus::Sending)
        {
            return (
                "Sending...".to_string(),
                None,
                "—".to_string(),
                "—".to_string(),
            );
        }
    }
    (
        fallback_status.to_string(),
        fallback_status_code,
        fallback_time.to_string(),
        fallback_size.to_string(),
    )
}

pub(in crate::ui) fn send_button_state_for_selected_request(
    selected_request_id: Option<Ulid>,
    execution_states: &HashMap<Ulid, RequestExecutionState>,
    request: &RequestAuthoringState,
    selected_environment_id: Option<Ulid>,
) -> SendButtonState {
    if let Some(request_id) = selected_request_id {
        if execution_states
            .get(&request_id)
            .is_some_and(|state| state.status == RequestExecutionStatus::Sending)
        {
            return SendButtonState::Sending;
        }
    }
    let state = BeamView::send_button_state_without_runtime(request);
    if matches!(
        state,
        SendButtonState::Disabled(SendDisabledReason::InvalidUrl)
    ) && selected_environment_id.is_some()
        && request.url.contains("{{")
        && request.url.contains("}}")
    {
        return SendButtonState::Ready;
    }
    state
}

pub(in crate::ui) fn resolve_request_with_environment(
    mut request: RequestAuthoringState,
    environment_variables: &[EnvironmentVariable],
) -> RequestAuthoringState {
    let resolved_env = build_enabled_environment_lookup(environment_variables);
    request.url = resolve_template_variables(&request.url, &resolved_env);
    for header in &mut request.headers {
        header.name = resolve_template_variables(&header.name, &resolved_env);
        header.value = resolve_template_variables(&header.value, &resolved_env);
    }
    for param in &mut request.query_params {
        param.name = resolve_template_variables(&param.name, &resolved_env);
        param.value = resolve_template_variables(&param.value, &resolved_env);
    }
    match &mut request.auth {
        AuthConfig::None => {}
        AuthConfig::Bearer { token } => {
            if let Some(value) = token.as_mut() {
                *value = resolve_template_variables(value, &resolved_env);
            }
        }
        AuthConfig::Basic { username, password } => {
            if let Some(value) = username.as_mut() {
                *value = resolve_template_variables(value, &resolved_env);
            }
            if let Some(value) = password.as_mut() {
                *value = resolve_template_variables(value, &resolved_env);
            }
        }
        AuthConfig::ApiKey { key, value, .. } => {
            if let Some(key_value) = key.as_mut() {
                *key_value = resolve_template_variables(key_value, &resolved_env);
            }
            if let Some(auth_value) = value.as_mut() {
                *auth_value = resolve_template_variables(auth_value, &resolved_env);
            }
        }
    }
    match &mut request.body {
        BodyConfig::None => {}
        BodyConfig::Raw { media_type, text } => {
            if let Some(content_type) = media_type.as_mut() {
                *content_type = resolve_template_variables(content_type, &resolved_env);
            }
            *text = resolve_template_variables(text, &resolved_env);
        }
        BodyConfig::Json { text } | BodyConfig::Xml { text } => {
            *text = resolve_template_variables(text, &resolved_env);
        }
        BodyConfig::FormUrlEncoded { fields } | BodyConfig::Multipart { fields } => {
            for field in fields {
                field.name = resolve_template_variables(&field.name, &resolved_env);
                field.value = resolve_template_variables(&field.value, &resolved_env);
            }
        }
        BodyConfig::Graphql {
            query,
            variables_json,
        } => {
            *query = resolve_template_variables(query, &resolved_env);
            if let Some(variables_text) = variables_json.as_mut() {
                *variables_text = resolve_template_variables(variables_text, &resolved_env);
            }
        }
    }
    request
}

pub(in crate::ui) fn build_enabled_environment_lookup(
    environment_variables: &[EnvironmentVariable],
) -> HashMap<String, String> {
    environment_variables
        .iter()
        .filter(|entry| entry.enabled)
        .filter_map(|entry| {
            let name = entry.name.trim();
            (!name.is_empty()).then_some((name.to_string(), entry.value.clone()))
        })
        .collect()
}

fn resolve_template_variables(input: &str, resolved_env: &HashMap<String, String>) -> String {
    let mut output = String::new();
    let mut index = 0usize;
    while let Some(start_offset) = input[index..].find("{{") {
        let start = index + start_offset;
        output.push_str(&input[index..start]);
        let token_start = start + 2;
        let Some(end_offset) = input[token_start..].find("}}") else {
            output.push_str(&input[start..]);
            return output;
        };
        let end = token_start + end_offset;
        let variable_name = input[token_start..end].trim();
        if let Some(value) = resolved_env.get(variable_name) {
            output.push_str(value);
        } else {
            output.push_str(&input[start..end + 2]);
        }
        index = end + 2;
    }
    output.push_str(&input[index..]);
    output
}

pub(in crate::ui) fn parse_response_headers(headers: &str) -> Vec<(String, String)> {
    headers
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn parse_response_duration_ms(time: &str) -> Option<u64> {
    time.strip_suffix(" ms")
        .and_then(|value| value.trim().parse::<u64>().ok())
}

async fn execute_http_request(request: RequestAuthoringState) -> HttpResponseSnapshot {
    let start = Instant::now();
    let client = match shared_http_client() {
        Ok(client) => client,
        Err(error) => {
            return HttpResponseSnapshot {
                status: "Error".to_string(),
                status_code: None,
                time: "—".to_string(),
                size: "—".to_string(),
                timestamp: Utc::now().to_rfc3339(),
                body: error,
                headers: String::new(),
                content_type: None,
            };
        }
    };
    let mut query_pairs: Vec<(String, String)> = request
        .query_params
        .iter()
        .filter(|param| param.enabled && !param.name.trim().is_empty())
        .map(|param| (param.name.trim().to_string(), param.value.clone()))
        .collect();
    let mut builder = client
        .request(http_method_to_reqwest(request.method), request.url.trim())
        .query(&query_pairs);
    for header in request
        .headers
        .iter()
        .filter(|header| header.enabled && !header.name.trim().is_empty())
    {
        builder = builder.header(header.name.trim(), &header.value);
    }
    match &request.auth {
        AuthConfig::None => {}
        AuthConfig::Bearer { token } => {
            if let Some(token) = token.as_ref().filter(|token| !token.trim().is_empty()) {
                builder = builder.bearer_auth(token.trim());
            }
        }
        AuthConfig::Basic { username, password } => {
            let user = username.clone().unwrap_or_default();
            let pass = password.clone().unwrap_or_default();
            builder = builder.basic_auth(user, (!pass.is_empty()).then_some(pass));
        }
        AuthConfig::ApiKey {
            key,
            value,
            location,
        } => {
            if let (Some(key), Some(value)) = (
                Some(
                    key.as_ref()
                        .map(String::as_str)
                        .unwrap_or(DEFAULT_API_KEY_HEADER_NAME)
                        .trim(),
                )
                .filter(|key| !key.is_empty()),
                value.as_ref().filter(|value| !value.trim().is_empty()),
            ) {
                match location {
                    crate::models::ApiKeyLocation::Header => {
                        builder = builder.header(key.trim(), value);
                    }
                    crate::models::ApiKeyLocation::Query => {
                        query_pairs.push((key.trim().to_string(), value.clone()));
                        builder = builder.query(&query_pairs);
                    }
                }
            }
        }
    }
    match &request.body {
        BodyConfig::None => {}
        BodyConfig::Raw { media_type, text } => {
            if let Some(content_type) = media_type.as_ref().filter(|value| !value.trim().is_empty())
            {
                builder = builder.header("Content-Type", content_type);
            }
            builder = builder.body(text.clone());
        }
        BodyConfig::Json { text } => {
            builder = builder
                .header("Content-Type", "application/json")
                .body(text.clone());
        }
        BodyConfig::Xml { text } => {
            builder = builder
                .header("Content-Type", "application/xml")
                .body(text.clone());
        }
        BodyConfig::FormUrlEncoded { fields } => {
            let pairs: Vec<(String, String)> = fields
                .iter()
                .filter(|field| field.enabled && !field.name.trim().is_empty())
                .map(|field| (field.name.trim().to_string(), field.value.clone()))
                .collect();
            builder = builder.form(&pairs);
        }
        BodyConfig::Multipart { fields } => {
            let mut form = reqwest::multipart::Form::new();
            for field in fields
                .iter()
                .filter(|field| field.enabled && !field.name.trim().is_empty())
            {
                form = form.text(field.name.trim().to_string(), field.value.clone());
            }
            builder = builder.multipart(form);
        }
        BodyConfig::Graphql {
            query,
            variables_json,
        } => {
            let mut payload = String::from("{\"query\":");
            payload.push_str(&serde_json::to_string(query).unwrap_or_else(|_| "\"\"".to_string()));
            if let Some(variables_json) = variables_json
                .as_ref()
                .filter(|variables| !variables.trim().is_empty())
            {
                payload.push_str(",\"variables\":");
                payload.push_str(variables_json);
            }
            payload.push('}');
            builder = builder
                .header("Content-Type", "application/json")
                .body(payload);
        }
    }
    match builder.send().await {
        Ok(response) => {
            let status = response.status();
            let status_text = status
                .canonical_reason()
                .map(|reason| format!("{} {}", status.as_u16(), reason))
                .unwrap_or_else(|| status.as_u16().to_string());
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| {
                    let value = value.to_str().unwrap_or("<non-utf8>");
                    format!("{}: {value}", name.as_str())
                })
                .collect::<Vec<_>>()
                .join("\n");
            let completed_at = Utc::now().to_rfc3339();
            match response.bytes().await {
                Ok(bytes) => HttpResponseSnapshot {
                    status: status_text,
                    status_code: Some(status.as_u16()),
                    time: format!("{} ms", start.elapsed().as_millis()),
                    size: format_bytes(bytes.len()),
                    timestamp: completed_at,
                    body: String::from_utf8_lossy(&bytes).to_string(),
                    headers,
                    content_type,
                },
                Err(error) => HttpResponseSnapshot {
                    status: status_text,
                    status_code: Some(status.as_u16()),
                    time: format!("{} ms", start.elapsed().as_millis()),
                    size: "—".to_string(),
                    timestamp: completed_at,
                    body: format!("Failed to read response body: {error}"),
                    headers,
                    content_type,
                },
            }
        }
        Err(error) => HttpResponseSnapshot {
            status: "Error".to_string(),
            status_code: None,
            time: format!("{} ms", start.elapsed().as_millis()),
            size: "—".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            body: format!("Request failed: {error}"),
            headers: String::new(),
            content_type: None,
        },
    }
}

fn http_method_to_reqwest(method: HttpMethod) -> Method {
    match method {
        HttpMethod::Get => Method::GET,
        HttpMethod::Post => Method::POST,
        HttpMethod::Put => Method::PUT,
        HttpMethod::Delete => Method::DELETE,
        HttpMethod::Patch => Method::PATCH,
        HttpMethod::Head => Method::HEAD,
        HttpMethod::Options => Method::OPTIONS,
        HttpMethod::Query => {
            Method::from_bytes(b"QUERY").expect("QUERY is a valid HTTP method token")
        }
    }
}

pub(in crate::ui) fn format_bytes(byte_count: usize) -> String {
    if byte_count < 1024 {
        return format!("{byte_count} B");
    }
    let kib = byte_count as f64 / 1024.0;
    if kib < 1024.0 {
        return format!("{kib:.1} KiB");
    }
    format!("{:.1} MiB", kib / 1024.0)
}
