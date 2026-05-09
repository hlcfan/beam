use std::sync::OnceLock;
use std::time::{Duration, Instant};
use std::{fs, path::PathBuf};

use chrono::{Local, Utc};
use gpui::*;
use gpui_component::{
    ActiveTheme, Disableable, Icon, Root, Selectable, Sizable, StyledExt, TitleBar, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{self, Input, InputEvent, InputState, Position, TabSize},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    resizable::{h_resizable, resizable_panel},
    scroll::ScrollableElement,
    text::html,
    v_flex,
};
use reqwest::{Method, blocking::Client};
use ulid::Ulid;

use crate::app_shell::{
    AppShellState, RequestPaneData, StartupLoad, StartupMessage, TreeNodeKind, startup_preload,
};
use crate::assets::Assets;
use crate::models::{
    AuthConfig, BodyConfig, EnvironmentFile, EnvironmentScope, EnvironmentVariable, HttpMethod,
    LocalStateFile,
};
use crate::paths::BeamPaths;
use crate::request_authoring::{
    RenameValidationError, RequestAuthoringState, RequestTab, SendButtonState, validate_rename,
};
use crate::script::{
    ConsoleLevel, EnvironmentChange, EnvironmentChangeKind, ScriptExecutionResult,
    ScriptRuntimeResponse, TestResult, execute_post_request_script,
};
use crate::storage::toml_backend::TomlWorkspaceStorage;
use crate::storage::{
    CreateFolderInput, CreateRequestInput, FolderParentRef, RequestParentRef, WorkspaceStorage,
};

pub fn run_app(state: AppShellState, startup_messages: Vec<StartupMessage>) {
    let app = gpui_platform::application().with_assets(Assets);
    app.run(move |cx| {
        gpui_component::init(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
            titlebar: Some(TitleBar::title_bar_options()),
            ..Default::default()
        };

        let state = state.clone();
        let startup_messages = startup_messages.clone();
        cx.open_window(window_options, |window, cx| {
            let view = cx.new(|cx| BeamView::new(state, startup_messages, window, cx));
            cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
        })
        .expect("Failed to open Beam window");
        cx.activate(true);
    });
}

struct BeamView {
    shell: AppShellState,
    request: RequestAuthoringState,
    startup_messages: Vec<StartupMessage>,
    url_input: Entity<InputState>,
    request_body_editor: Entity<InputState>,
    response_body_editor: Entity<InputState>,
    response_headers_raw: String,
    post_script_editor: Entity<InputState>,
    active_response_tab: ResponseTab,
    response_status: String,
    response_time: String,
    response_size: String,
    script_result: Option<PersistedScriptResult>,
    environment_manager_selected_id: Option<Ulid>,
    environment_manager_variables: Vec<EnvironmentVariable>,
    request_param_name_inputs: Vec<Entity<InputState>>,
    request_param_value_inputs: Vec<Entity<InputState>>,
    request_param_input_subscriptions: Vec<Subscription>,
    request_header_name_inputs: Vec<Entity<InputState>>,
    request_header_value_inputs: Vec<Entity<InputState>>,
    request_header_input_subscriptions: Vec<Subscription>,
    request_auth_bearer_token_input: Entity<InputState>,
    request_auth_basic_username_input: Entity<InputState>,
    request_auth_basic_password_input: Entity<InputState>,
    request_auth_api_key_name_input: Entity<InputState>,
    request_auth_api_key_value_input: Entity<InputState>,
    request_auth_input_subscriptions: Vec<Subscription>,
    suppress_request_auth_change_events: bool,
    environment_manager_name_input: Entity<InputState>,
    environment_manager_value_input: Entity<InputState>,
    environment_manager_error: Option<String>,
    pending_request_save_due_at: Option<Instant>,
    request_save_tick_scheduled: bool,
    request_save_in_flight: bool,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseTab {
    Body,
    Headers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestBodyFormat {
    None,
    Json,
    Xml,
    Graphql,
    Text,
    FormUrlEncoded,
    Multipart,
}

const DEFAULT_API_KEY_HEADER_NAME: &str = "X-API-Key";
const RESPONSE_BODY_PLACEHOLDER: &str = "// Send a request to view the response body.";
const RESPONSE_BODY_TRUNCATED_NOTE: &str =
    "[Response body omitted from local history (truncated).]";

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
struct PersistedScriptResult {
    request_id: String,
    success: bool,
    failed: bool,
    error_type: Option<String>,
    error_message: Option<String>,
    failure_message: Option<String>,
    #[serde(default)]
    console_output: Vec<ConsoleMessageView>,
    #[serde(default)]
    test_results: Vec<TestResult>,
    #[serde(default)]
    environment_diff: Vec<EnvironmentChange>,
    updated_at: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct ConsoleMessageView {
    level: String,
    message: String,
    timestamp: String,
}

struct EnvironmentManagerDialogView {
    options: Vec<(Ulid, String)>,
    selected_id: Option<Ulid>,
    variables: Vec<EnvironmentVariable>,
    environment_name_input: Entity<InputState>,
    variable_name_inputs: Vec<Entity<InputState>>,
    variable_value_inputs: Vec<Entity<InputState>>,
    variable_input_subscriptions: Vec<Subscription>,
    pending_variables_save_due_at: Option<Instant>,
    variables_save_tick_scheduled: bool,
    variables_save_in_flight: bool,
    suppress_environment_name_change_events: bool,
    environment_name_input_subscription: Option<Subscription>,
    error: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct EnvironmentTomlFile {
    environment: EnvironmentTomlMeta,
    #[serde(default)]
    variables: Vec<EnvironmentVariable>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct EnvironmentTomlMeta {
    schema_version: u32,
    environment_id: Ulid,
    collection_id: Option<Ulid>,
    scope: EnvironmentScope,
    name: String,
    description: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
struct RequestHistoryFile {
    #[serde(default)]
    meta: Option<RequestHistoryMeta>,
    #[serde(default)]
    executions: Vec<RequestHistoryExecution>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct RequestHistoryMeta {
    request_id: String,
    #[serde(default)]
    schema_version: Option<u32>,
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct RequestHistoryExecution {
    status: Option<u16>,
    duration_ms: Option<u64>,
    response_summary: Option<RequestHistoryResponseSummary>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct RequestHistoryResponseSummary {
    body_bytes: Option<u64>,
    body_ref: Option<String>,
    #[serde(default)]
    body_truncated: bool,
    #[serde(default)]
    headers: Vec<RequestHistoryHeader>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct RequestHistoryHeader {
    name: String,
    value: String,
}

struct StoredResponseSnapshot {
    status: String,
    time: String,
    size: String,
    body: String,
    headers_raw: String,
}

impl EnvironmentManagerDialogView {
    fn parse_environment_file(content: &str) -> Result<EnvironmentFile, String> {
        if let Ok(current) = toml::from_str::<EnvironmentFile>(content) {
            return Ok(current);
        }
        let legacy = toml::from_str::<EnvironmentTomlFile>(content)
            .map_err(|error| format!("Failed to parse environment file: {error}"))?;
        Ok(EnvironmentFile {
            schema_version: legacy.environment.schema_version,
            environment: crate::models::EnvironmentMeta {
                environment_id: legacy.environment.environment_id,
                collection_id: legacy.environment.collection_id,
                scope: legacy.environment.scope,
                name: legacy.environment.name,
                description: legacy.environment.description,
                created_at: legacy.environment.created_at,
                updated_at: legacy.environment.updated_at,
            },
            variables: legacy.variables,
        })
    }

    fn new(
        options: Vec<(Ulid, String)>,
        selected_id: Option<Ulid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let environment_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Environment name"));
        let mut view = Self {
            options,
            selected_id,
            variables: Vec::new(),
            environment_name_input,
            variable_name_inputs: Vec::new(),
            variable_value_inputs: Vec::new(),
            variable_input_subscriptions: Vec::new(),
            pending_variables_save_due_at: None,
            variables_save_tick_scheduled: false,
            variables_save_in_flight: false,
            suppress_environment_name_change_events: false,
            environment_name_input_subscription: None,
            error: None,
        };
        let environment_name_input_handle = view.environment_name_input.clone();
        view.environment_name_input_subscription = Some(cx.subscribe_in(
            &view.environment_name_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                if this.suppress_environment_name_change_events || this.selected_id.is_none() {
                    return;
                }
                let _ = environment_name_input_handle.read(cx);
                this.error = None;
                this.schedule_variables_save(cx);
            },
        ));
        if let Some(environment_id) = view.selected_id {
            view.load_variables(environment_id, window, cx);
        } else {
            view.error = Some("No environment available to manage.".to_string());
        }
        view
    }

    fn environment_option_label(name: &str, scope: EnvironmentScope) -> String {
        match scope {
            EnvironmentScope::Global => name.to_string(),
            EnvironmentScope::Collection => format!("Collection: {name}"),
        }
    }

    fn load_variables(
        &mut self,
        environment_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = BeamView::find_environment_file_path(environment_id) else {
            self.variables.clear();
            self.clear_variable_inputs();
            self.error = Some("Environment file not found.".to_string());
            return;
        };
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                self.variables.clear();
                self.clear_variable_inputs();
                self.error = Some(format!("Failed to read environment file: {error}"));
                return;
            }
        };
        let parsed = match Self::parse_environment_file(&content) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.variables.clear();
                self.clear_variable_inputs();
                self.error = Some(error);
                return;
            }
        };
        self.variables = parsed.variables;
        self.rebuild_variable_inputs(window, cx);
        self.suppress_environment_name_change_events = true;
        self.environment_name_input.update(cx, |input, cx| {
            input.set_value(parsed.environment.name.clone(), window, cx);
        });
        self.suppress_environment_name_change_events = false;
        if let Some((_, label)) = self
            .options
            .iter_mut()
            .find(|(option_id, _)| *option_id == environment_id)
        {
            *label =
                Self::environment_option_label(&parsed.environment.name, parsed.environment.scope);
        }
        self.error = None;
    }

    fn save_variables_to_disk(
        environment_id: Ulid,
        updated_name: String,
        variables: Vec<EnvironmentVariable>,
    ) -> Result<EnvironmentScope, String> {
        let Some(path) = BeamView::find_environment_file_path(environment_id) else {
            return Err("Environment file not found.".to_string());
        };
        let content =
            fs::read_to_string(&path).map_err(|error| format!("Failed to read file: {error}"))?;
        let mut parsed = Self::parse_environment_file(&content)
            .map_err(|error| format!("Failed to parse file: {error}"))?;
        if updated_name.is_empty() {
            return Err("Environment name cannot be empty.".to_string());
        }
        parsed.environment.name = updated_name;
        parsed.variables = variables;
        parsed.environment.updated_at = Utc::now();
        let updated_content = toml::to_string_pretty(&parsed)
            .map_err(|error| format!("Failed to serialize file: {error}"))?;
        fs::write(&path, updated_content)
            .map_err(|error| format!("Failed to write file: {error}"))?;
        Ok(parsed.environment.scope)
    }

    fn clear_variable_inputs(&mut self) {
        self.variable_name_inputs.clear();
        self.variable_value_inputs.clear();
        self.variable_input_subscriptions.clear();
    }

    fn rebuild_variable_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_variable_inputs();
        for index in 0..self.variables.len() {
            let key_value = self.variables[index].name.clone();
            let value_value = self.variables[index].value.clone();

            let key_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Key")
                    .default_value(key_value)
            });
            let value_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Value")
                    .default_value(value_value)
            });

            let key_input_handle = key_input.clone();
            let key_subscription = cx.subscribe_in(
                &key_input,
                window,
                move |this, _, ev: &InputEvent, _, cx| {
                    if !matches!(ev, InputEvent::Change) {
                        return;
                    }
                    if let Some(variable) = this.variables.get_mut(index) {
                        variable.name = key_input_handle.read(cx).value().to_string();
                        this.schedule_variables_save(cx);
                    }
                },
            );

            let value_input_handle = value_input.clone();
            let value_subscription = cx.subscribe_in(
                &value_input,
                window,
                move |this, _, ev: &InputEvent, _, cx| {
                    if !matches!(ev, InputEvent::Change) {
                        return;
                    }
                    if let Some(variable) = this.variables.get_mut(index) {
                        variable.value = value_input_handle.read(cx).value().to_string();
                        this.schedule_variables_save(cx);
                    }
                },
            );

            self.variable_name_inputs.push(key_input);
            self.variable_value_inputs.push(value_input);
            self.variable_input_subscriptions.push(key_subscription);
            self.variable_input_subscriptions.push(value_subscription);
        }
    }

    fn schedule_variables_save_with_delay(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if self.selected_id.is_none() {
            return;
        }
        self.pending_variables_save_due_at = Some(Instant::now() + delay);
        if self.variables_save_tick_scheduled {
            return;
        }
        self.variables_save_tick_scheduled = true;
        self.schedule_variables_save_tick(cx);
    }

    fn schedule_variables_save(&mut self, cx: &mut Context<Self>) {
        self.schedule_variables_save_with_delay(Duration::from_millis(350), cx);
    }

    fn schedule_variables_save_tick(&self, cx: &mut Context<Self>) {
        let view = cx.entity();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .spawn(async move {
                    std::thread::sleep(Duration::from_millis(25));
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.process_pending_variables_save(cx);
            });
        })
        .detach();
    }

    fn process_pending_variables_save(&mut self, cx: &mut Context<Self>) {
        if self.variables_save_in_flight {
            self.variables_save_tick_scheduled = false;
            return;
        }
        let Some(due_at) = self.pending_variables_save_due_at else {
            self.variables_save_tick_scheduled = false;
            return;
        };
        if Instant::now() < due_at {
            self.schedule_variables_save_tick(cx);
            return;
        }
        self.pending_variables_save_due_at = None;
        self.variables_save_tick_scheduled = false;
        let Some(environment_id) = self.selected_id else {
            return;
        };
        let updated_name = self
            .environment_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if updated_name.is_empty() {
            self.error = Some("Environment name cannot be empty.".to_string());
            cx.notify();
            return;
        }
        let variables = self.variables.clone();
        let updated_name_for_label = updated_name.clone();
        self.variables_save_in_flight = true;
        let view = cx.entity();
        cx.spawn(async move |_, cx| {
            let result = cx.background_executor().spawn(async move {
                Self::save_variables_to_disk(environment_id, updated_name, variables)
            });
            let result = result.await;
            let _ = view.update(cx, move |this, cx| {
                this.variables_save_in_flight = false;
                match result {
                    Ok(scope) => {
                        if let Some((_, label)) = this
                            .options
                            .iter_mut()
                            .find(|(option_id, _)| *option_id == environment_id)
                        {
                            *label = Self::environment_option_label(&updated_name_for_label, scope);
                        }
                        this.error = None;
                    }
                    Err(error) => {
                        this.error = Some(error);
                    }
                }
                if this.pending_variables_save_due_at.is_some()
                    && !this.variables_save_tick_scheduled
                {
                    this.variables_save_tick_scheduled = true;
                    this.schedule_variables_save_tick(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn add_variable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.variables.push(EnvironmentVariable {
            name: String::new(),
            value: String::new(),
            enabled: true,
            secret: false,
            description: None,
        });
        self.rebuild_variable_inputs(window, cx);
        self.schedule_variables_save(cx);
        cx.notify();
    }

    fn remove_variable(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.variables.len() {
            return;
        }
        self.variables.remove(index);
        self.rebuild_variable_inputs(window, cx);
        self.schedule_variables_save(cx);
        cx.notify();
    }
}

impl Render for EnvironmentManagerDialogView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_label = self.selected_id.and_then(|id| {
            self.options
                .iter()
                .find(|(environment_id, _)| *environment_id == id)
                .map(|(_, label)| label.clone())
        });

        let mut variables_panel = v_flex().h_full().w_full().gap_3();
        variables_panel = variables_panel.child(
            h_flex().w_full().items_center().child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(selected_label.unwrap_or_else(|| "No environment selected".to_string())),
            ),
        );
        variables_panel = variables_panel.child(
            h_flex().w_full().items_center().gap_2().child(
                div()
                    .flex_1()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(0xd1d5db))
                    .bg(rgb(0xf8fafc))
                    .px_2()
                    .py_1()
                    .child(
                        Input::new(&self.environment_name_input)
                            .small()
                            .w_full()
                            .appearance(false),
                    ),
            ),
        );
        if let Some(error) = &self.error {
            variables_panel = variables_panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(0xb91c1c))
                    .child(error.clone()),
            );
        }
        let mut variables_rows = v_flex().w_full().gap_1().child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .text_xs()
                .font_semibold()
                .text_color(rgb(0x6b7280))
                .child(div().w(px(28.0)).child("On"))
                .child(div().w(px(180.0)).child("Key"))
                .child(div().flex_1().child("Value"))
                .child(div().w(px(28.0))),
        );
        variables_rows = variables_rows.child(if self.variables.is_empty() {
            div()
                .text_xs()
                .text_color(rgb(0x6b7280))
                .px_2()
                .py_2()
                .child("No variables yet.")
                .into_any_element()
        } else {
            div().into_any_element()
        });
        variables_rows =
            variables_rows.children(self.variables.iter().enumerate().map(|(index, variable)| {
                let key_input = self.variable_name_inputs[index].clone();
                let value_input = self.variable_value_inputs[index].clone();
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(0xe5e7eb))
                    .child(
                        div().w(px(28.0)).child(
                            gpui_component::checkbox::Checkbox::new(format!(
                                "env-var-enabled-{index}"
                            ))
                            .small()
                            .checked(variable.enabled)
                            .on_click(cx.listener(
                                move |this, checked: &bool, _, cx| {
                                    if let Some(variable) = this.variables.get_mut(index) {
                                        variable.enabled = *checked;
                                        this.schedule_variables_save(cx);
                                        cx.notify();
                                    }
                                },
                            )),
                        ),
                    )
                    .child(
                        div()
                            .w(px(180.0))
                            .child(Input::new(&key_input).small().w_full().appearance(false)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&value_input).small().w_full().appearance(false)),
                    )
                    .child(
                        div().w(px(28.0)).child(
                            Button::new(format!("delete-environment-variable-{index}"))
                                .small()
                                .ghost()
                                .cursor_pointer()
                                .icon(
                                    Icon::default()
                                        .path("icons/delete.svg")
                                        .size(px(14.0))
                                        .text_color(rgb(0x6b7280)),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.remove_variable(index, window, cx);
                                })),
                        ),
                    )
            }));
        variables_rows = variables_rows.child(
            h_flex().w_full().justify_end().pt_2().child(
                Button::new("add-environment-variable")
                    .small()
                    .label("Add variable")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_variable(window, cx);
                    })),
            ),
        );
        variables_panel = variables_panel.child(div().w_full().child(variables_rows));

        v_flex().w_full().h(px(520.0)).p_3().gap_3().child(
            h_flex()
                .w_full()
                .h_full()
                .gap_3()
                .child(
                    v_flex()
                        .w(px(260.0))
                        .h_full()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(rgb(0xd1d5db))
                        .bg(rgb(0xffffff))
                        .p_3()
                        .gap_2()
                        .child(
                            v_flex()
                                .w_full()
                                .gap_1()
                                .child(div().text_xs().font_semibold().child("Environments")),
                        )
                        .child(
                            v_flex().w_full().gap_1().children(
                                self.options
                                    .clone()
                                    .into_iter()
                                    .map(|(environment_id, label)| {
                                        Button::new(format!(
                                            "environment-manager-select-{environment_id}"
                                        ))
                                        .small()
                                        .ghost()
                                        .selected(Some(environment_id) == self.selected_id)
                                        .w_full()
                                        .px_3()
                                        .py_1()
                                        .justify_start()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.selected_id = Some(environment_id);
                                            this.load_variables(environment_id, window, cx);
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .w_full()
                                                .text_sm()
                                                .line_height(relative(1.0))
                                                .child(label),
                                        )
                                    }),
                            ),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(rgb(0xd1d5db))
                        .bg(rgb(0xffffff))
                        .p_2()
                        .child(
                            div()
                                .w_full()
                                .h_full()
                                .overflow_y_scrollbar()
                                .child(variables_panel),
                        ),
                ),
        )
    }
}

struct TreeRenameDialogView {
    target_view: Entity<BeamView>,
    node_id: Ulid,
    node_kind: TreeNodeKind,
    name_input: Entity<InputState>,
}

impl TreeRenameDialogView {
    fn new(
        target_view: Entity<BeamView>,
        node_id: Ulid,
        node_kind: TreeNodeKind,
        current_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Name")
                .default_value(current_name)
        });
        Self {
            target_view,
            node_id,
            node_kind,
            name_input,
        }
    }

    fn focus_name_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.name_input.update(cx, |state, cx| {
            state.focus(window, cx);
            let cursor_end = state.value().encode_utf16().count() as u32;
            state.set_cursor_position(Position::new(0, cursor_end), window, cx);
        });
    }

    fn submit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next_name = self.name_input.read(cx).value().to_string();
        let _ = self.target_view.update(cx, |this, cx| {
            this.rename_tree_node_from_modal(self.node_id, self.node_kind, next_name, window, cx);
        });
    }

    fn title(&self) -> &'static str {
        match self.node_kind {
            TreeNodeKind::Collection => "Rename collection",
            TreeNodeKind::Folder => "Rename folder",
            TreeNodeKind::Request => "Rename request",
        }
    }
}

impl Render for TreeRenameDialogView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let target_view = self.target_view.clone();
        let node_id = self.node_id;
        let node_kind = self.node_kind;
        let name_input = self.name_input.clone();

        v_flex()
            .w(px(420.0))
            .p_3()
            .gap_3()
            .child(div().text_sm().font_semibold().child(self.title()))
            .child(
                div()
                    .w_full()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(0xd1d5db))
                    .bg(rgb(0xf8fafc))
                    .px_2()
                    .py_1()
                    .child(
                        Input::new(&self.name_input)
                            .small()
                            .w_full()
                            .appearance(false),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("rename-dialog-cancel")
                            .small()
                            .ghost()
                            .label("Cancel")
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("rename-dialog-submit")
                            .small()
                            .cursor_pointer()
                            .label("Rename")
                            .on_click(move |_, window, cx| {
                                let next_name = name_input.read(cx).value().to_string();
                                let _ = target_view.update(cx, |this, cx| {
                                    this.rename_tree_node_from_modal(
                                        node_id, node_kind, next_name, window, cx,
                                    );
                                });
                            }),
                    ),
            )
    }
}

impl BeamView {
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

    fn format_human_time(timestamp: &str) -> String {
        chrono::DateTime::parse_from_rfc3339(timestamp)
            .map(|parsed| parsed.with_timezone(&Local).format("%H:%M:%S").to_string())
            .unwrap_or_else(|_| timestamp.to_string())
    }

    fn selected_collection_id(&self) -> Option<Ulid> {
        let mut cursor = self.shell.collections.selected_request_id()?;
        loop {
            let node = self.shell.collections.node(cursor)?;
            if node.kind == TreeNodeKind::Collection {
                return Some(node.id);
            }
            cursor = node.parent_id?;
        }
    }

    fn active_environment_options(&self) -> Vec<(Ulid, String, EnvironmentScope)> {
        let selected_collection_id = self.selected_collection_id();
        self.shell
            .environments
            .iter()
            .filter(|environment| {
                environment.scope == EnvironmentScope::Global
                    || (environment.scope == EnvironmentScope::Collection
                        && environment.collection_id == selected_collection_id)
            })
            .map(|environment| {
                let label = match environment.scope {
                    EnvironmentScope::Global => environment.name.clone(),
                    EnvironmentScope::Collection => format!("Collection: {}", environment.name),
                };
                (environment.environment_id, label, environment.scope)
            })
            .collect()
    }

    fn selected_environment_id_for_view(&self) -> Option<Ulid> {
        if let Some(collection_id) = self.selected_collection_id() {
            if let Some(collection_environment_id) = self
                .shell
                .environment_selection
                .active_collection_environment_ids
                .get(&collection_id)
                .copied()
            {
                return Some(collection_environment_id);
            }
        }
        self.shell
            .environment_selection
            .active_global_environment_id
    }

    fn selected_environment_label(&self) -> String {
        let Some(selected_id) = self.selected_environment_id_for_view() else {
            return "No environment".to_string();
        };
        let Some((_, label, _)) = self
            .active_environment_options()
            .into_iter()
            .find(|(environment_id, _, _)| *environment_id == selected_id)
        else {
            return "No environment".to_string();
        };
        label
    }

    fn environment_manager_options(&self) -> Vec<(Ulid, String)> {
        self.shell
            .environments
            .iter()
            .map(|environment| {
                let label = match environment.scope {
                    EnvironmentScope::Global => environment.name.clone(),
                    EnvironmentScope::Collection => format!("Collection: {}", environment.name),
                };
                (environment.environment_id, label)
            })
            .collect()
    }

    fn open_environment_manager(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = self.environment_manager_options();
        let fallback_id = options.first().map(|(environment_id, _)| *environment_id);
        let selected = self.selected_environment_id_for_view().or(fallback_id);
        let manager_view =
            cx.new(|cx| EnvironmentManagerDialogView::new(options.clone(), selected, window, cx));
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    window.defer(cx, move |window, cx| {
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog
                                .title("Manage environment")
                                .w(px(920.0))
                                .max_w(px(1200.0))
                                .child(manager_view.clone())
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    fn find_environment_file_path(environment_id: Ulid) -> Option<PathBuf> {
        let paths = BeamPaths::default_user_config();
        let mut stack = vec![paths.environments_dir.clone(), paths.collections_dir];
        let explicit_environments_dir =
            dirs::home_dir().map(|home| home.join(".config").join("beam").join("environments"));
        if let Some(explicit_environments_dir) = explicit_environments_dir {
            if explicit_environments_dir != paths.environments_dir {
                stack.push(explicit_environments_dir);
            }
        }
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".env.toml"))
                {
                    continue;
                }
                let Ok(content) = fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(file) = EnvironmentManagerDialogView::parse_environment_file(&content)
                else {
                    continue;
                };
                if file.environment.environment_id == environment_id {
                    return Some(path);
                }
            }
        }
        None
    }

    fn load_environment_manager_variables(&mut self, environment_id: Ulid) {
        let Some(path) = Self::find_environment_file_path(environment_id) else {
            self.environment_manager_variables.clear();
            self.environment_manager_error = Some("Environment file not found.".to_string());
            return;
        };
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                self.environment_manager_variables.clear();
                self.environment_manager_error =
                    Some(format!("Failed to read environment file: {error}"));
                return;
            }
        };
        let parsed = match toml::from_str::<EnvironmentFile>(&content) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.environment_manager_variables.clear();
                self.environment_manager_error =
                    Some(format!("Failed to parse environment file: {error}"));
                return;
            }
        };
        self.environment_manager_variables = parsed.variables;
        self.environment_manager_error = None;
    }

    fn save_environment_manager_variables(&mut self, environment_id: Ulid) -> Result<(), String> {
        let Some(path) = Self::find_environment_file_path(environment_id) else {
            return Err("Environment file not found.".to_string());
        };
        let content =
            fs::read_to_string(&path).map_err(|error| format!("Failed to read file: {error}"))?;
        let mut parsed = toml::from_str::<EnvironmentFile>(&content)
            .map_err(|error| format!("Failed to parse file: {error}"))?;
        parsed.variables = self.environment_manager_variables.clone();
        parsed.environment.updated_at = Utc::now();
        let updated_content = toml::to_string_pretty(&parsed)
            .map_err(|error| format!("Failed to serialize file: {error}"))?;
        fs::write(&path, updated_content).map_err(|error| format!("Failed to write file: {error}"))
    }

    fn add_environment_variable_from_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self
            .environment_manager_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let value = self
            .environment_manager_value_input
            .read(cx)
            .value()
            .to_string();
        if name.is_empty() {
            self.environment_manager_error = Some("Variable name cannot be empty.".to_string());
            cx.notify();
            return;
        }
        if self
            .environment_manager_variables
            .iter()
            .any(|item| item.name == name)
        {
            self.environment_manager_error =
                Some("Variable name already exists in this environment.".to_string());
            cx.notify();
            return;
        }
        self.environment_manager_variables
            .push(EnvironmentVariable {
                name,
                value,
                enabled: true,
                secret: false,
                description: None,
            });
        if let Some(environment_id) = self.environment_manager_selected_id {
            if let Err(error) = self.save_environment_manager_variables(environment_id) {
                self.environment_manager_error = Some(error);
                cx.notify();
                return;
            }
        }
        self.environment_manager_name_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.environment_manager_value_input
            .update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
        self.environment_manager_error = None;
        cx.notify();
    }

    fn remove_environment_variable(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.environment_manager_variables.len() {
            return;
        }
        self.environment_manager_variables.remove(index);
        if let Some(environment_id) = self.environment_manager_selected_id {
            if let Err(error) = self.save_environment_manager_variables(environment_id) {
                self.environment_manager_error = Some(error);
                cx.notify();
                return;
            }
        }
        self.environment_manager_error = None;
        cx.notify();
    }

    fn hydrate_request_from_selection(request: &mut RequestAuthoringState, shell: &AppShellState) {
        let Some(request_id) = shell.collections.selected_request_id() else {
            return;
        };

        let Some(pane_data) = shell.request_pane_data.get(&request_id) else {
            let selected_node = shell.collections.node(request_id).cloned();
            let Some(node) = selected_node else {
                return;
            };
            request.method = node.request_method.unwrap_or(HttpMethod::Get);
            request.url = node.request_url.unwrap_or_default();
            return;
        };

        request.method = pane_data.method;
        request.url = pane_data.url.clone();
        request.headers = pane_data.headers.clone();
        request.query_params = pane_data.query_params.clone();
        request.auth = pane_data.auth.clone();
        request.body = pane_data.body.clone();
        request.post_script = pane_data.post_script.clone();
    }

    fn sync_request_editor_from_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        Self::hydrate_request_from_selection(&mut self.request, &self.shell);
        if self.request.query_params.is_empty() {
            self.request
                .query_params
                .push(crate::models::QueryParamField {
                    name: String::new(),
                    value: String::new(),
                    enabled: true,
                    description: None,
                });
        }
        if self.request.headers.is_empty() {
            self.request.headers.push(crate::models::HeaderField {
                name: String::new(),
                value: String::new(),
                enabled: true,
                description: None,
                secret: false,
            });
        }
        self.rebuild_request_param_inputs(window, cx);
        self.rebuild_request_header_inputs(window, cx);
        let next_url = self.request.url.clone();
        let next_body = Self::body_editor_text(&self.request.body);
        let next_script = self.request.post_script.clone().unwrap_or_default();
        self.url_input.update(cx, |input, cx| {
            input.set_value(next_url, window, cx);
        });
        let next_body_language = Self::body_editor_language(&self.request.body);
        self.request_body_editor.update(cx, |input, cx| {
            input.set_highlighter(next_body_language, cx);
            input.set_value(next_body, window, cx);
        });
        self.post_script_editor.update(cx, |input, cx| {
            input.set_value(next_script, window, cx);
        });
        self.sync_request_auth_inputs(window, cx);
        self.sync_response_pane_from_selection(window, cx);
    }

    fn clear_response_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.response_status = "—".to_string();
        self.response_time = "—".to_string();
        self.response_size = "—".to_string();
        self.response_headers_raw.clear();
        self.response_body_editor.update(cx, |input, cx| {
            input.set_value(RESPONSE_BODY_PLACEHOLDER.to_string(), window, cx);
        });
    }

    fn sync_response_pane_from_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request_id) = self.shell.collections.selected_request_id() else {
            self.clear_response_pane(window, cx);
            self.script_result = None;
            return;
        };

        let Some(snapshot) = Self::load_latest_response_snapshot(request_id) else {
            self.clear_response_pane(window, cx);
            self.script_result = Self::load_script_result(request_id);
            return;
        };

        self.response_status = snapshot.status;
        self.response_time = snapshot.time;
        self.response_size = snapshot.size;
        self.response_headers_raw = snapshot.headers_raw;
        self.response_body_editor.update(cx, |input, cx| {
            input.set_value(snapshot.body, window, cx);
        });
        self.script_result = Self::load_script_result(request_id);
    }

    fn load_latest_response_snapshot(request_id: Ulid) -> Option<StoredResponseSnapshot> {
        let paths = BeamPaths::default_user_config();
        let history_file_path = paths
            .local_dir
            .join("history/by-request")
            .join(format!("{request_id}.history.toml"));
        let content = fs::read_to_string(history_file_path).ok()?;
        let history_file: RequestHistoryFile = toml::from_str(&content).ok()?;
        if let Some(meta) = history_file.meta.as_ref() {
            let _ = (&meta.schema_version, &meta.updated_at);
            if meta.request_id != request_id.to_string() {
                return None;
            }
        }
        let latest_execution = history_file.executions.last()?;

        let status = latest_execution
            .status
            .map(|code| code.to_string())
            .unwrap_or_else(|| "—".to_string());
        let time = latest_execution
            .duration_ms
            .map(|ms| format!("{ms} ms"))
            .unwrap_or_else(|| "—".to_string());

        let mut size = "—".to_string();
        let mut body = RESPONSE_BODY_PLACEHOLDER.to_string();
        let mut headers_raw = String::new();

        if let Some(summary) = latest_execution.response_summary.as_ref() {
            if let Some(bytes) = summary.body_bytes.and_then(|n| usize::try_from(n).ok()) {
                size = format_bytes(bytes);
            }
            if !summary.headers.is_empty() {
                headers_raw = summary
                    .headers
                    .iter()
                    .map(|header| format!("{}: {}", header.name, header.value))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            body = if summary.body_truncated {
                RESPONSE_BODY_TRUNCATED_NOTE.to_string()
            } else if let Some(body_ref) = summary.body_ref.as_ref() {
                let body_path = paths.local_dir.join("history/responses").join(body_ref);
                fs::read(body_path)
                    .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                    .unwrap_or_else(|_| RESPONSE_BODY_PLACEHOLDER.to_string())
            } else {
                String::new()
            };
        }

        Some(StoredResponseSnapshot {
            status,
            time,
            size,
            body,
            headers_raw,
        })
    }

    fn script_result_file_path(request_id: Ulid) -> PathBuf {
        let paths = BeamPaths::default_user_config();
        paths
            .local_dir
            .join("script_results")
            .join(format!("{request_id}.toml"))
    }

    fn load_script_result(request_id: Ulid) -> Option<PersistedScriptResult> {
        let path = Self::script_result_file_path(request_id);
        let content = fs::read_to_string(path).ok()?;
        let parsed: PersistedScriptResult = toml::from_str(&content).ok()?;
        (parsed.request_id == request_id.to_string()).then_some(parsed)
    }

    fn persist_script_result(
        request_id: Ulid,
        result: &PersistedScriptResult,
    ) -> Result<(), String> {
        let paths = BeamPaths::default_user_config();
        let dir = paths.local_dir.join("script_results");
        fs::create_dir_all(&dir)
            .map_err(|error| format!("Failed to create script_results directory: {error}"))?;
        let path = dir.join(format!("{request_id}.toml"));
        let content = toml::to_string_pretty(result)
            .map_err(|error| format!("Failed to encode script result: {error}"))?;
        fs::write(path, content).map_err(|error| format!("Failed to write script result: {error}"))
    }

    fn clear_script_result_for_request(request_id: Ulid) -> Result<(), String> {
        let path = Self::script_result_file_path(request_id);
        if path.exists() {
            fs::remove_file(path)
                .map_err(|error| format!("Failed to clear script result: {error}"))?;
        }
        Ok(())
    }

    fn clear_request_param_inputs(&mut self) {
        self.request_param_name_inputs.clear();
        self.request_param_value_inputs.clear();
        self.request_param_input_subscriptions.clear();
    }

    fn clear_request_header_inputs(&mut self) {
        self.request_header_name_inputs.clear();
        self.request_header_value_inputs.clear();
        self.request_header_input_subscriptions.clear();
    }

    fn sync_request_auth_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.suppress_request_auth_change_events = true;

        let (bearer_token, basic_username, basic_password, api_key_name, api_key_value) =
            match &self.request.auth {
                AuthConfig::None => (
                    String::new(),
                    String::new(),
                    String::new(),
                    DEFAULT_API_KEY_HEADER_NAME.to_string(),
                    String::new(),
                ),
                AuthConfig::Bearer { token } => (
                    token.clone().unwrap_or_default(),
                    String::new(),
                    String::new(),
                    DEFAULT_API_KEY_HEADER_NAME.to_string(),
                    String::new(),
                ),
                AuthConfig::Basic { username, password } => (
                    String::new(),
                    username.clone().unwrap_or_default(),
                    password.clone().unwrap_or_default(),
                    DEFAULT_API_KEY_HEADER_NAME.to_string(),
                    String::new(),
                ),
                AuthConfig::ApiKey { key, value, .. } => (
                    String::new(),
                    String::new(),
                    String::new(),
                    key.clone()
                        .unwrap_or_else(|| DEFAULT_API_KEY_HEADER_NAME.to_string()),
                    value.clone().unwrap_or_default(),
                ),
            };

        self.request_auth_bearer_token_input
            .update(cx, |input, cx| {
                input.set_value(bearer_token, window, cx);
            });
        self.request_auth_basic_username_input
            .update(cx, |input, cx| {
                input.set_value(basic_username, window, cx);
            });
        self.request_auth_basic_password_input
            .update(cx, |input, cx| {
                input.set_value(basic_password, window, cx);
            });
        self.request_auth_api_key_name_input
            .update(cx, |input, cx| {
                input.set_value(api_key_name, window, cx);
            });
        self.request_auth_api_key_value_input
            .update(cx, |input, cx| {
                input.set_value(api_key_value, window, cx);
            });

        self.suppress_request_auth_change_events = false;
    }

    fn clear_request_auth_input_subscriptions(&mut self) {
        self.request_auth_input_subscriptions.clear();
    }

    fn rebuild_request_auth_input_subscriptions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_request_auth_input_subscriptions();

        let bearer_input = self.request_auth_bearer_token_input.clone();
        self.request_auth_input_subscriptions.push(cx.subscribe_in(
            &self.request_auth_bearer_token_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) || this.suppress_request_auth_change_events {
                    return;
                }
                if let AuthConfig::Bearer { token } = &mut this.request.auth {
                    let value = bearer_input.read(cx).value().to_string();
                    *token = (!value.trim().is_empty()).then_some(value);
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            },
        ));

        let username_input = self.request_auth_basic_username_input.clone();
        self.request_auth_input_subscriptions.push(cx.subscribe_in(
            &self.request_auth_basic_username_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) || this.suppress_request_auth_change_events {
                    return;
                }
                if let AuthConfig::Basic { username, .. } = &mut this.request.auth {
                    let value = username_input.read(cx).value().to_string();
                    *username = (!value.trim().is_empty()).then_some(value);
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            },
        ));

        let password_input = self.request_auth_basic_password_input.clone();
        self.request_auth_input_subscriptions.push(cx.subscribe_in(
            &self.request_auth_basic_password_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) || this.suppress_request_auth_change_events {
                    return;
                }
                if let AuthConfig::Basic { password, .. } = &mut this.request.auth {
                    let value = password_input.read(cx).value().to_string();
                    *password = (!value.trim().is_empty()).then_some(value);
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            },
        ));

        let api_key_name_input = self.request_auth_api_key_name_input.clone();
        self.request_auth_input_subscriptions.push(cx.subscribe_in(
            &self.request_auth_api_key_name_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) || this.suppress_request_auth_change_events {
                    return;
                }
                if let AuthConfig::ApiKey { key, .. } = &mut this.request.auth {
                    let value = api_key_name_input.read(cx).value().to_string();
                    *key = if value.trim().is_empty() {
                        None
                    } else {
                        Some(value)
                    };
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            },
        ));

        let api_key_value_input = self.request_auth_api_key_value_input.clone();
        self.request_auth_input_subscriptions.push(cx.subscribe_in(
            &self.request_auth_api_key_value_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) || this.suppress_request_auth_change_events {
                    return;
                }
                if let AuthConfig::ApiKey { value, .. } = &mut this.request.auth {
                    let next_value = api_key_value_input.read(cx).value().to_string();
                    *value = if next_value.trim().is_empty() {
                        None
                    } else {
                        Some(next_value)
                    };
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            },
        ));
    }

    fn sync_selected_request_pane_data(&mut self) -> Option<(Ulid, RequestPaneData)> {
        let Some(request_id) = self.shell.collections.selected_request_id() else {
            return None;
        };
        let pane_data = RequestPaneData {
            method: self.request.method,
            url: self.request.url.clone(),
            headers: self.request.headers.clone(),
            query_params: self.request.query_params.clone(),
            auth: self.request.auth.clone(),
            body: self.request.body.clone(),
            post_script: self.request.post_script.clone(),
        };
        self.shell
            .request_pane_data
            .insert(request_id, pane_data.clone());
        Some((request_id, pane_data))
    }

    fn schedule_request_save_with_delay(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if self.sync_selected_request_pane_data().is_none() {
            return;
        }
        self.pending_request_save_due_at = Some(Instant::now() + delay);
        if self.request_save_tick_scheduled {
            return;
        }
        self.request_save_tick_scheduled = true;
        self.schedule_request_save_tick(cx);
    }

    fn schedule_request_save(&mut self, cx: &mut Context<Self>) {
        self.schedule_request_save_with_delay(Duration::from_millis(350), cx);
    }

    fn schedule_request_save_tick(&self, cx: &mut Context<Self>) {
        let view = cx.entity();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .spawn(async move {
                    std::thread::sleep(Duration::from_millis(25));
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.process_pending_request_save(cx);
            });
        })
        .detach();
    }

    fn save_request_snapshot_to_disk(
        request_id: Ulid,
        pane_data: RequestPaneData,
    ) -> Result<(), String> {
        let paths = BeamPaths::default_user_config();
        let storage = TomlWorkspaceStorage::new(paths);
        let mut request_file = storage
            .load_request(request_id)
            .map_err(|error| format!("Failed to load request for save: {error}"))?;
        request_file.request.method = pane_data.method;
        request_file.request.url = pane_data.url;
        request_file.request.headers = pane_data.headers;
        request_file.request.query_params = pane_data.query_params;
        request_file.auth = pane_data.auth;
        request_file.body = pane_data.body;
        request_file.scripts.post_response = pane_data.post_script;
        request_file.meta.updated_at = Utc::now();
        storage
            .save_request(&request_file)
            .map_err(|error| format!("Failed to save request: {error}"))
    }

    fn process_pending_request_save(&mut self, cx: &mut Context<Self>) {
        if self.request_save_in_flight {
            self.request_save_tick_scheduled = false;
            return;
        }
        let Some(due_at) = self.pending_request_save_due_at else {
            self.request_save_tick_scheduled = false;
            return;
        };
        if Instant::now() < due_at {
            self.schedule_request_save_tick(cx);
            return;
        }
        self.pending_request_save_due_at = None;
        self.request_save_tick_scheduled = false;
        let Some((request_id, pane_data)) = self.sync_selected_request_pane_data() else {
            return;
        };
        self.request_save_in_flight = true;
        let view = cx.entity();
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { Self::save_request_snapshot_to_disk(request_id, pane_data) })
                .await;
            let _ = view.update(cx, move |this, cx| {
                this.request_save_in_flight = false;
                if let Err(error) = result {
                    eprintln!("{error}");
                }
                if this.pending_request_save_due_at.is_some() && !this.request_save_tick_scheduled {
                    this.request_save_tick_scheduled = true;
                    this.schedule_request_save_tick(cx);
                }
            });
        })
        .detach();
    }

    fn rebuild_request_param_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_request_param_inputs();
        for index in 0..self.request.query_params.len() {
            let param_name = self.request.query_params[index].name.clone();
            let param_value = self.request.query_params[index].value.clone();

            let key_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Param key")
                    .default_value(param_name)
            });
            let value_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Param value")
                    .default_value(param_value)
            });

            let key_input_handle = key_input.clone();
            let key_subscription = cx.subscribe_in(
                &key_input,
                window,
                move |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let name = key_input_handle.read(cx).value().to_string();
                        let value = this
                            .request
                            .query_params
                            .get(index)
                            .map(|param| param.value.clone())
                            .unwrap_or_default();
                        this.request.set_param_value(index, name, value);
                        if this.request_param_name_inputs.len() != this.request.query_params.len() {
                            this.rebuild_request_param_inputs(window, cx);
                        }
                        this.schedule_request_save(cx);
                        cx.notify();
                    }
                    InputEvent::Focus => {
                        if index + 1 == this.request.query_params.len() {
                            this.request
                                .query_params
                                .push(crate::models::QueryParamField {
                                    name: String::new(),
                                    value: String::new(),
                                    enabled: true,
                                    description: None,
                                });
                            this.rebuild_request_param_inputs(window, cx);
                            if let Some(input) = this.request_param_name_inputs.get(index).cloned()
                            {
                                input.update(cx, |state, cx| {
                                    state.focus(window, cx);
                                });
                            }
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            );

            let value_input_handle = value_input.clone();
            let value_subscription = cx.subscribe_in(
                &value_input,
                window,
                move |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let name = this
                            .request
                            .query_params
                            .get(index)
                            .map(|param| param.name.clone())
                            .unwrap_or_default();
                        let value = value_input_handle.read(cx).value().to_string();
                        this.request.set_param_value(index, name, value);
                        if this.request_param_name_inputs.len() != this.request.query_params.len() {
                            this.rebuild_request_param_inputs(window, cx);
                        }
                        this.schedule_request_save(cx);
                        cx.notify();
                    }
                    InputEvent::Focus => {
                        if index + 1 == this.request.query_params.len() {
                            this.request
                                .query_params
                                .push(crate::models::QueryParamField {
                                    name: String::new(),
                                    value: String::new(),
                                    enabled: true,
                                    description: None,
                                });
                            this.rebuild_request_param_inputs(window, cx);
                            if let Some(input) = this.request_param_value_inputs.get(index).cloned()
                            {
                                input.update(cx, |state, cx| {
                                    state.focus(window, cx);
                                });
                            }
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            );

            self.request_param_name_inputs.push(key_input);
            self.request_param_value_inputs.push(value_input);
            self.request_param_input_subscriptions
                .push(key_subscription);
            self.request_param_input_subscriptions
                .push(value_subscription);
        }
    }

    fn rebuild_request_header_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_request_header_inputs();
        for index in 0..self.request.headers.len() {
            let header_name = self.request.headers[index].name.clone();
            let header_value = self.request.headers[index].value.clone();

            let key_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Header key")
                    .default_value(header_name)
            });
            let value_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Header value")
                    .default_value(header_value)
            });

            let key_input_handle = key_input.clone();
            let key_subscription = cx.subscribe_in(
                &key_input,
                window,
                move |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let name = key_input_handle.read(cx).value().to_string();
                        let value = this
                            .request
                            .headers
                            .get(index)
                            .map(|header| header.value.clone())
                            .unwrap_or_default();
                        this.request.set_header_value(index, name, value);
                        if this.request_header_name_inputs.len() != this.request.headers.len() {
                            this.rebuild_request_header_inputs(window, cx);
                        }
                        this.schedule_request_save(cx);
                        cx.notify();
                    }
                    InputEvent::Focus => {
                        if index + 1 == this.request.headers.len() {
                            this.request.headers.push(crate::models::HeaderField {
                                name: String::new(),
                                value: String::new(),
                                enabled: true,
                                description: None,
                                secret: false,
                            });
                            this.rebuild_request_header_inputs(window, cx);
                            if let Some(input) = this.request_header_name_inputs.get(index).cloned()
                            {
                                input.update(cx, |state, cx| {
                                    state.focus(window, cx);
                                });
                            }
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            );

            let value_input_handle = value_input.clone();
            let value_subscription = cx.subscribe_in(
                &value_input,
                window,
                move |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let name = this
                            .request
                            .headers
                            .get(index)
                            .map(|header| header.name.clone())
                            .unwrap_or_default();
                        let value = value_input_handle.read(cx).value().to_string();
                        this.request.set_header_value(index, name, value);
                        if this.request_header_name_inputs.len() != this.request.headers.len() {
                            this.rebuild_request_header_inputs(window, cx);
                        }
                        this.schedule_request_save(cx);
                        cx.notify();
                    }
                    InputEvent::Focus => {
                        if index + 1 == this.request.headers.len() {
                            this.request.headers.push(crate::models::HeaderField {
                                name: String::new(),
                                value: String::new(),
                                enabled: true,
                                description: None,
                                secret: false,
                            });
                            this.rebuild_request_header_inputs(window, cx);
                            if let Some(input) =
                                this.request_header_value_inputs.get(index).cloned()
                            {
                                input.update(cx, |state, cx| {
                                    state.focus(window, cx);
                                });
                            }
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            );

            self.request_header_name_inputs.push(key_input);
            self.request_header_value_inputs.push(value_input);
            self.request_header_input_subscriptions
                .push(key_subscription);
            self.request_header_input_subscriptions
                .push(value_subscription);
        }
    }

    fn delete_request_param_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.request.query_params.len() {
            return;
        }
        self.request.query_params.remove(index);
        if self.request.query_params.is_empty() {
            self.request
                .query_params
                .push(crate::models::QueryParamField {
                    name: String::new(),
                    value: String::new(),
                    enabled: true,
                    description: None,
                });
        }
        self.rebuild_request_param_inputs(window, cx);
        self.schedule_request_save(cx);
        cx.notify();
    }

    fn delete_request_header_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.request.headers.len() {
            return;
        }
        self.request.headers.remove(index);
        if self.request.headers.is_empty() {
            self.request.headers.push(crate::models::HeaderField {
                name: String::new(),
                value: String::new(),
                enabled: true,
                description: None,
                secret: false,
            });
        }
        self.rebuild_request_header_inputs(window, cx);
        self.schedule_request_save(cx);
        cx.notify();
    }

    fn collection_ancestor_for_node(&self, mut node_id: Ulid) -> Option<Ulid> {
        loop {
            let node = self.shell.collections.node(node_id)?;
            if node.kind == TreeNodeKind::Collection {
                return Some(node.id);
            }
            node_id = node.parent_id?;
        }
    }

    fn parent_ref_for_add_request(&self, node_id: Ulid) -> Option<RequestParentRef> {
        let node = self.shell.collections.node(node_id)?;
        match node.kind {
            TreeNodeKind::Collection => Some(RequestParentRef {
                collection_id: node.id,
                folder_id: None,
            }),
            TreeNodeKind::Folder => Some(RequestParentRef {
                collection_id: self.collection_ancestor_for_node(node.id)?,
                folder_id: Some(node.id),
            }),
            TreeNodeKind::Request => {
                let parent_id = node.parent_id?;
                let parent_node = self.shell.collections.node(parent_id)?;
                match parent_node.kind {
                    TreeNodeKind::Collection => Some(RequestParentRef {
                        collection_id: parent_node.id,
                        folder_id: None,
                    }),
                    TreeNodeKind::Folder => Some(RequestParentRef {
                        collection_id: self.collection_ancestor_for_node(parent_node.id)?,
                        folder_id: Some(parent_node.id),
                    }),
                    TreeNodeKind::Request => None,
                }
            }
        }
    }

    fn parent_ref_for_add_folder(&self, node_id: Ulid) -> Option<FolderParentRef> {
        let node = self.shell.collections.node(node_id)?;
        match node.kind {
            TreeNodeKind::Collection => Some(FolderParentRef {
                collection_id: node.id,
                parent_folder_id: None,
            }),
            TreeNodeKind::Folder => Some(FolderParentRef {
                collection_id: self.collection_ancestor_for_node(node.id)?,
                parent_folder_id: Some(node.id),
            }),
            TreeNodeKind::Request => {
                let parent_id = node.parent_id?;
                let parent_node = self.shell.collections.node(parent_id)?;
                match parent_node.kind {
                    TreeNodeKind::Collection => Some(FolderParentRef {
                        collection_id: parent_node.id,
                        parent_folder_id: None,
                    }),
                    TreeNodeKind::Folder => Some(FolderParentRef {
                        collection_id: self.collection_ancestor_for_node(parent_node.id)?,
                        parent_folder_id: Some(parent_node.id),
                    }),
                    TreeNodeKind::Request => None,
                }
            }
        }
    }

    fn request_sibling_names_in_parent(&self, parent: RequestParentRef) -> Vec<String> {
        let parent_id = parent.folder_id.unwrap_or(parent.collection_id);
        let Some(parent_node) = self.shell.collections.node(parent_id) else {
            return Vec::new();
        };
        parent_node
            .children
            .iter()
            .filter_map(|child_id| self.shell.collections.node(*child_id))
            .filter(|child| child.kind == TreeNodeKind::Request)
            .map(|child| child.name.clone())
            .collect()
    }

    fn folder_sibling_names_in_parent(&self, parent: FolderParentRef) -> Vec<String> {
        let parent_id = parent.parent_folder_id.unwrap_or(parent.collection_id);
        let Some(parent_node) = self.shell.collections.node(parent_id) else {
            return Vec::new();
        };
        parent_node
            .children
            .iter()
            .filter_map(|child_id| self.shell.collections.node(*child_id))
            .filter(|child| child.kind == TreeNodeKind::Folder)
            .map(|child| child.name.clone())
            .collect()
    }

    fn next_new_request_name(&self, parent: RequestParentRef) -> String {
        let sibling_names = self.request_sibling_names_in_parent(parent);
        let mut idx = 1;
        loop {
            let candidate = format!("New Request {idx}");
            if !sibling_names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&candidate))
            {
                return candidate;
            }
            idx += 1;
        }
    }

    fn next_new_folder_name(&self, parent: FolderParentRef) -> String {
        let sibling_names = self.folder_sibling_names_in_parent(parent);
        let mut idx = 1;
        loop {
            let candidate = format!("New Folder {idx}");
            if !sibling_names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&candidate))
            {
                return candidate;
            }
            idx += 1;
        }
    }

    fn next_duplicate_request_name(&self, request_id: Ulid) -> Option<String> {
        let source = self.shell.collections.node(request_id)?;
        let parent = self.parent_ref_for_add_request(request_id)?;
        let siblings = self.request_sibling_names_in_parent(parent);
        let base = format!("{} (Copy)", source.name);
        if !siblings.iter().any(|name| name.eq_ignore_ascii_case(&base)) {
            return Some(base);
        }
        let mut idx = 2;
        loop {
            let candidate = format!("{} (Copy {idx})", source.name);
            if !siblings
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&candidate))
            {
                return Some(candidate);
            }
            idx += 1;
        }
    }

    fn quote_shell_arg(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    fn build_curl_for_request(&self, request_id: Ulid) -> Option<String> {
        let pane = self.shell.request_pane_data.get(&request_id)?;
        let mut url = pane.url.clone();
        let query_pairs: Vec<String> = pane
            .query_params
            .iter()
            .filter(|param| param.enabled && !param.name.trim().is_empty())
            .map(|param| format!("{}={}", param.name.trim(), param.value.trim()))
            .collect();
        if !query_pairs.is_empty() {
            let joiner = if url.contains('?') { "&" } else { "?" };
            url.push_str(joiner);
            url.push_str(&query_pairs.join("&"));
        }

        let mut parts = vec![
            "curl".to_string(),
            "-X".to_string(),
            format!("{:?}", pane.method).to_uppercase(),
            Self::quote_shell_arg(&url),
        ];

        for header in pane
            .headers
            .iter()
            .filter(|header| header.enabled && !header.name.trim().is_empty())
        {
            parts.push("-H".to_string());
            parts.push(Self::quote_shell_arg(&format!(
                "{}: {}",
                header.name.trim(),
                header.value
            )));
        }

        let body_payload = match &pane.body {
            BodyConfig::None => None,
            BodyConfig::Raw { text, .. } | BodyConfig::Json { text } | BodyConfig::Xml { text } => {
                (!text.is_empty()).then_some(text.clone())
            }
            BodyConfig::Graphql {
                query,
                variables_json,
            } => {
                let mut payload = String::new();
                payload.push_str("{\"query\":");
                payload.push_str(&serde_json::to_string(query).ok()?);
                if let Some(variables) = variables_json.as_ref().filter(|v| !v.trim().is_empty()) {
                    payload.push_str(",\"variables\":");
                    if serde_json::from_str::<serde_json::Value>(variables).is_ok() {
                        payload.push_str(variables);
                    } else {
                        payload.push_str(&serde_json::to_string(variables).ok()?);
                    }
                }
                payload.push('}');
                Some(payload)
            }
            BodyConfig::FormUrlEncoded { fields } | BodyConfig::Multipart { fields } => {
                let payload = fields
                    .iter()
                    .filter(|field| !field.name.trim().is_empty())
                    .map(|field| format!("{}={}", field.name.trim(), field.value))
                    .collect::<Vec<_>>()
                    .join("&");
                (!payload.is_empty()).then_some(payload)
            }
        };

        if let Some(payload) = body_payload {
            parts.push("--data-raw".to_string());
            parts.push(Self::quote_shell_arg(&payload));
        }

        Some(parts.join(" "))
    }

    fn refresh_shell_from_disk(
        &mut self,
        preferred_selected_request_id: Option<Ulid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = BeamPaths::default_user_config();
        let storage = TomlWorkspaceStorage::new(paths.clone());
        match startup_preload(&storage, &paths) {
            StartupLoad::Ready { state, messages } => {
                self.shell = state;
                self.startup_messages = messages;
                if let Some(request_id) = preferred_selected_request_id {
                    self.shell.collections.select_request(request_id);
                }
                self.sync_request_editor_from_selection(window, cx);
            }
            StartupLoad::Fatal { message } => {
                self.startup_messages = vec![message];
            }
        }
    }

    fn persist_last_opened_request_id(&self, request_id: Ulid) -> Result<(), String> {
        let paths = BeamPaths::default_user_config();
        let storage = TomlWorkspaceStorage::new(paths);
        let mut local_state = match storage.load_local_state() {
            Ok(state) => state,
            Err(_) => LocalStateFile::default(),
        };

        if local_state.local_state.last_opened_request_id == Some(request_id) {
            return Ok(());
        }

        local_state.local_state.last_opened_request_id = Some(request_id);
        local_state.local_state.updated_at = Utc::now();
        storage
            .save_local_state(&local_state)
            .map_err(|error| format!("Failed to save local state: {error}"))
    }

    fn persist_tree_expansion_state(&self) -> Result<(), String> {
        let paths = BeamPaths::default_user_config();
        let storage = TomlWorkspaceStorage::new(paths);
        let mut local_state = match storage.load_local_state() {
            Ok(state) => state,
            Err(_) => LocalStateFile::default(),
        };

        let expanded_item_ids: Vec<Ulid> =
            self.shell.collections.expanded().iter().copied().collect();
        if local_state.tree_state.expanded_item_ids == expanded_item_ids {
            return Ok(());
        }

        local_state.tree_state.expanded_item_ids = expanded_item_ids;
        local_state.local_state.updated_at = Utc::now();
        storage
            .save_local_state(&local_state)
            .map_err(|error| format!("Failed to save local state: {error}"))
    }

    fn add_request_from_tree_node(
        &mut self,
        node_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent) = self.parent_ref_for_add_request(node_id) else {
            window.push_notification("Unable to determine request parent.", cx);
            return;
        };
        let request_name = self.next_new_request_name(parent);
        let paths = BeamPaths::default_user_config();
        let storage = TomlWorkspaceStorage::new(paths);
        match storage.create_request(CreateRequestInput {
            parent,
            name: request_name,
            method: HttpMethod::Get,
            url: String::new(),
        }) {
            Ok(request_file) => {
                self.refresh_shell_from_disk(Some(request_file.meta.request_id), window, cx);
                window.push_notification("Request added.", cx);
                cx.notify();
            }
            Err(error) => {
                window.push_notification(format!("Failed to add request: {error}"), cx);
            }
        }
    }

    fn add_folder_from_tree_node(
        &mut self,
        node_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent) = self.parent_ref_for_add_folder(node_id) else {
            window.push_notification("Unable to determine folder parent.", cx);
            return;
        };
        let folder_name = self.next_new_folder_name(parent);
        let paths = BeamPaths::default_user_config();
        let storage = TomlWorkspaceStorage::new(paths);
        match storage.create_folder(CreateFolderInput {
            parent,
            name: folder_name,
        }) {
            Ok(folder_file) => {
                let preferred_request = self.shell.collections.selected_request_id();
                self.refresh_shell_from_disk(preferred_request, window, cx);
                self.shell
                    .collections
                    .toggle_expanded(folder_file.folder.folder_id);
                if let Err(error) = self.persist_tree_expansion_state() {
                    window.push_notification(error, cx);
                }
                window.push_notification("Folder added.", cx);
                cx.notify();
            }
            Err(error) => {
                window.push_notification(format!("Failed to add folder: {error}"), cx);
            }
        }
    }

    fn open_rename_dialog_for_tree_node(
        &mut self,
        node_id: Ulid,
        node_kind: TreeNodeKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.shell.collections.node(node_id).cloned() else {
            window.push_notification("Unable to rename: item not found.", cx);
            return;
        };
        let view = cx.entity();
        let dialog_view = cx.new(|cx| {
            TreeRenameDialogView::new(
                view.clone(),
                node_id,
                node_kind,
                node.name.clone(),
                window,
                cx,
            )
        });
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let focus_dialog_view = dialog_view.clone();
                let _ = root_window.update(cx, |_, window, cx| {
                    window.defer(cx, move |window, cx| {
                        let submit_dialog_view = dialog_view.clone();
                        window.open_dialog(cx, move |dialog, _, _| {
                            let submit_dialog_view_for_ok = submit_dialog_view.clone();
                            dialog
                                .title("Rename item")
                                .w(px(460.0))
                                .child(dialog_view.clone())
                                .on_ok(move |_, window, cx| {
                                    let submit_dialog_view = submit_dialog_view_for_ok.clone();
                                    let _ = submit_dialog_view.update(cx, |this, cx| {
                                        this.submit_rename(window, cx);
                                    });
                                    false
                                })
                        });
                        window.defer(cx, move |window, cx| {
                            let _ = focus_dialog_view.update(cx, |this, cx| {
                                this.focus_name_input(window, cx);
                            });
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    fn rename_tree_node_from_modal(
        &mut self,
        node_id: Ulid,
        node_kind: TreeNodeKind,
        requested_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        eprintln!(
            "rename_tree_node_from_modal invoked: id={}, kind={:?}, requested_name={}",
            node_id, node_kind, requested_name
        );
        let Some(node) = self.shell.collections.node(node_id).cloned() else {
            eprintln!("rename: node not found for id={node_id}");
            window.push_notification("Unable to rename: item not found.", cx);
            return;
        };
        let next_name = requested_name.trim();
        if next_name.is_empty() {
            eprintln!("rename: rejected empty name");
            window.push_notification("Name cannot be empty.", cx);
            return;
        }

        let selected_request = self.shell.collections.selected_request_id();
        let validated_name = match node_kind {
            TreeNodeKind::Collection => {
                let siblings: Vec<&str> = self
                    .shell
                    .collections
                    .visible_rows()
                    .into_iter()
                    .filter(|row| row.kind == TreeNodeKind::Collection && row.id != node_id)
                    .filter_map(|row| self.shell.collections.node(row.id))
                    .map(|n| n.name.as_str())
                    .collect();
                let validated = match validate_rename(&node.name, next_name, siblings) {
                    Ok(value) => value,
                    Err(RenameValidationError::EmptyName) => {
                        eprintln!("rename: collection empty name after validation");
                        window.push_notification("Collection name cannot be empty.", cx);
                        return;
                    }
                    Err(RenameValidationError::DuplicateName) => {
                        eprintln!("rename: collection duplicate name '{}'", next_name);
                        window.push_notification("A collection with this name already exists.", cx);
                        return;
                    }
                };
                validated
            }
            TreeNodeKind::Folder => {
                let Some(parent) = self.parent_ref_for_add_folder(node_id) else {
                    eprintln!("rename: unable to determine folder parent for id={node_id}");
                    window.push_notification("Unable to determine folder parent.", cx);
                    return;
                };
                let siblings = self.folder_sibling_names_in_parent(parent);
                let validated = match validate_rename(
                    &node.name,
                    next_name,
                    siblings.iter().map(String::as_str),
                ) {
                    Ok(value) => value,
                    Err(RenameValidationError::EmptyName) => {
                        eprintln!("rename: folder empty name after validation");
                        window.push_notification("Folder name cannot be empty.", cx);
                        return;
                    }
                    Err(RenameValidationError::DuplicateName) => {
                        eprintln!("rename: folder duplicate name '{}'", next_name);
                        window.push_notification("A folder with this name already exists.", cx);
                        return;
                    }
                };
                validated
            }
            TreeNodeKind::Request => {
                let Some(parent) = self.parent_ref_for_add_request(node_id) else {
                    eprintln!("rename: unable to determine request parent for id={node_id}");
                    window.push_notification("Unable to determine request parent.", cx);
                    return;
                };
                let siblings = self.request_sibling_names_in_parent(parent);
                let validated = match validate_rename(
                    &node.name,
                    next_name,
                    siblings.iter().map(String::as_str),
                ) {
                    Ok(value) => value,
                    Err(RenameValidationError::EmptyName) => {
                        eprintln!("rename: request empty name after validation");
                        window.push_notification("Request name cannot be empty.", cx);
                        return;
                    }
                    Err(RenameValidationError::DuplicateName) => {
                        eprintln!("rename: request duplicate name '{}'", next_name);
                        window.push_notification("A request with this name already exists.", cx);
                        return;
                    }
                };
                validated
            }
        };
        let preferred_selection = match node_kind {
            TreeNodeKind::Request => selected_request.or(Some(node_id)),
            TreeNodeKind::Collection | TreeNodeKind::Folder => selected_request,
        };
        let success_message = match node_kind {
            TreeNodeKind::Collection => "Collection renamed.",
            TreeNodeKind::Folder => "Folder renamed.",
            TreeNodeKind::Request => "Request renamed.",
        };
        let confirmed_name = validated_name.clone();
        let persisted_name = validated_name;
        window.close_dialog(cx);
        cx.notify();
        let paths = BeamPaths::default_user_config();
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            eprintln!(
                "rename: async start node_id={}, kind={:?}, validated_name='{}'",
                node_id, node_kind, persisted_name
            );
            let result = cx
                .background_executor()
                .spawn(async move {
                    let storage = TomlWorkspaceStorage::new(paths.clone());
                    let rename_result = match node_kind {
                        TreeNodeKind::Collection => storage
                            .rename_collection(node_id, &persisted_name)
                            .map(|_| ()),
                        TreeNodeKind::Folder => {
                            storage.rename_folder(node_id, &persisted_name).map(|_| ())
                        }
                        TreeNodeKind::Request => {
                            storage.rename_request(node_id, &persisted_name).map(|_| ())
                        }
                    };
                    if let Err(error) = rename_result {
                        return Err(format!("Failed to rename: {error}"));
                    }
                    Ok(())
                })
                .await;
            let _ = view.update_in(cx, move |this, window, cx| match result {
                Ok(()) => {
                    eprintln!("rename: async success for node_id={node_id}");
                    let _ = this
                        .shell
                        .collections
                        .rename_node(node_id, confirmed_name.clone());
                    if let Some(request_id) = preferred_selection {
                        this.shell.collections.select_request(request_id);
                    }
                    window.push_notification(success_message, cx);
                    cx.notify();
                }
                Err(error) => {
                    eprintln!("rename: async error for node_id={node_id}: {error}");
                    window.push_notification(error, cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn send_request_from_tree_node(
        &mut self,
        request_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shell.collections.select_request(request_id);
        self.sync_request_editor_from_selection(window, cx);
        self.send_request(window, cx);
    }

    fn copy_request_as_curl_from_tree_node(
        &mut self,
        request_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(curl) = self.build_curl_for_request(request_id) else {
            window.push_notification("Unable to build cURL command for this request.", cx);
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(curl));
        window.push_notification("Copied as cURL.", cx);
    }

    fn duplicate_request_from_tree_node(
        &mut self,
        request_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(duplicate_name) = self.next_duplicate_request_name(request_id) else {
            window.push_notification("Unable to duplicate this request.", cx);
            return;
        };
        let paths = BeamPaths::default_user_config();
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let storage = TomlWorkspaceStorage::new(paths.clone());
                    let request_file = storage
                        .duplicate_request(request_id, &duplicate_name)
                        .map_err(|error| format!("Failed to duplicate request: {error}"))?;
                    let reload_storage = TomlWorkspaceStorage::new(paths.clone());
                    let (state, messages) = match startup_preload(&reload_storage, &paths) {
                        StartupLoad::Ready { state, messages } => (state, messages),
                        StartupLoad::Fatal { message } => {
                            return Err(format!("Failed to reload workspace: {}", message.text));
                        }
                    };
                    Ok((request_file.meta.request_id, state, messages))
                })
                .await;
            let _ = view.update_in(cx, move |this, window, cx| match result {
                Ok((new_request_id, state, messages)) => {
                    this.shell = state;
                    this.startup_messages = messages;
                    this.shell.collections.select_request(new_request_id);
                    this.sync_request_editor_from_selection(window, cx);
                    window.push_notification("Request duplicated.", cx);
                    cx.notify();
                }
                Err(error) => {
                    window.push_notification(error, cx);
                }
            });
        })
        .detach();
    }

    fn delete_request_from_tree_node(
        &mut self,
        request_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = BeamPaths::default_user_config();
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let storage = TomlWorkspaceStorage::new(paths.clone());
                    storage
                        .delete_request(request_id)
                        .map_err(|error| format!("Failed to delete request: {error}"))?;
                    let reload_storage = TomlWorkspaceStorage::new(paths.clone());
                    match startup_preload(&reload_storage, &paths) {
                        StartupLoad::Ready { state, messages } => Ok((state, messages)),
                        StartupLoad::Fatal { message } => {
                            Err(format!("Failed to reload workspace: {}", message.text))
                        }
                    }
                })
                .await;
            let _ = view.update_in(cx, move |this, window, cx| match result {
                Ok((state, messages)) => {
                    this.shell = state;
                    this.startup_messages = messages;
                    this.sync_request_editor_from_selection(window, cx);
                    window.push_notification("Request deleted.", cx);
                    cx.notify();
                }
                Err(error) => {
                    window.push_notification(error, cx);
                }
            });
        })
        .detach();
    }

    fn delete_collection_from_tree_node(
        &mut self,
        collection_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = BeamPaths::default_user_config();
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let storage = TomlWorkspaceStorage::new(paths.clone());
                    storage
                        .delete_collection(collection_id)
                        .map_err(|error| format!("Failed to delete collection: {error}"))?;
                    let reload_storage = TomlWorkspaceStorage::new(paths.clone());
                    match startup_preload(&reload_storage, &paths) {
                        StartupLoad::Ready { state, messages } => Ok((state, messages)),
                        StartupLoad::Fatal { message } => {
                            Err(format!("Failed to reload workspace: {}", message.text))
                        }
                    }
                })
                .await;
            let _ = view.update_in(cx, move |this, window, cx| match result {
                Ok((state, messages)) => {
                    this.shell = state;
                    this.startup_messages = messages;
                    this.sync_request_editor_from_selection(window, cx);
                    window.push_notification("Collection deleted.", cx);
                    cx.notify();
                }
                Err(error) => {
                    window.push_notification(error, cx);
                }
            });
        })
        .detach();
    }

    fn delete_folder_from_tree_node(
        &mut self,
        folder_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preferred_request = self.shell.collections.selected_request_id();
        let paths = BeamPaths::default_user_config();
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let storage = TomlWorkspaceStorage::new(paths.clone());
                    storage
                        .delete_folder(folder_id)
                        .map_err(|error| format!("Failed to delete folder: {error}"))?;
                    let reload_storage = TomlWorkspaceStorage::new(paths.clone());
                    match startup_preload(&reload_storage, &paths) {
                        StartupLoad::Ready { state, messages } => Ok((state, messages)),
                        StartupLoad::Fatal { message } => {
                            Err(format!("Failed to reload workspace: {}", message.text))
                        }
                    }
                })
                .await;
            let _ = view.update_in(cx, move |this, window, cx| match result {
                Ok((state, messages)) => {
                    this.shell = state;
                    this.startup_messages = messages;
                    if let Some(request_id) = preferred_request {
                        this.shell.collections.select_request(request_id);
                    }
                    this.sync_request_editor_from_selection(window, cx);
                    window.push_notification("Folder deleted.", cx);
                    cx.notify();
                }
                Err(error) => {
                    window.push_notification(error, cx);
                }
            });
        })
        .detach();
    }

    fn render_key_value_lines(lines: Vec<String>) -> AnyElement {
        if lines.is_empty() {
            return div()
                .h_full()
                .w_full()
                .text_sm()
                .text_color(rgb(0x6b7280))
                .child("No configured entries.")
                .into_any_element();
        }

        v_flex()
            .h_full()
            .w_full()
            .gap_1()
            .items_start()
            .children(lines.into_iter().map(|line| {
                div()
                    .text_sm()
                    .font_family(".SystemUIFont")
                    .text_color(rgb(0x1f2937))
                    .child(line)
            }))
            .into_any_element()
    }

    fn method_label(method: HttpMethod) -> &'static str {
        match method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
        }
    }

    fn supported_http_methods() -> [HttpMethod; 7] {
        [
            HttpMethod::Get,
            HttpMethod::Post,
            HttpMethod::Put,
            HttpMethod::Delete,
            HttpMethod::Patch,
            HttpMethod::Head,
            HttpMethod::Options,
        ]
    }

    fn method_badge_colors(method: HttpMethod) -> (Rgba, Rgba) {
        match method {
            HttpMethod::Get => (rgb(0xdcfce7), rgb(0x166534)),
            HttpMethod::Post => (rgb(0xffedd5), rgb(0x9a3412)),
            HttpMethod::Put | HttpMethod::Patch => (rgb(0xdbeafe), rgb(0x1d4ed8)),
            HttpMethod::Delete => (rgb(0xfee2e2), rgb(0xb91c1c)),
            HttpMethod::Head | HttpMethod::Options => (rgb(0xe5e7eb), rgb(0x374151)),
        }
    }

    fn render_method_badge(method: HttpMethod) -> Div {
        let (badge_bg, badge_text) = Self::method_badge_colors(method);
        div()
            .px_1()
            .py(px(1.0))
            .rounded(px(4.0))
            .bg(badge_bg)
            .text_xs()
            .font_semibold()
            .text_color(badge_text)
            .child(format!("{method:?}").to_uppercase())
    }

    fn render_title_bar_content(&self) -> Div {
        h_flex()
            .items_center()
            .justify_between()
            .w_full()
            .h_full()
            .px_2()
            .text_sm()
            .text_color(rgb(0x1f2937))
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child("Beam")
                    .child("File")
                    .child("Edit")
                    .child("View")
                    .child("Run")
                    .child("Help"),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(rgb(0x6b7280))
                    .child("Workspace: default")
                    .child("Profile: local"),
            )
    }

    fn new(
        shell: AppShellState,
        startup_messages: Vec<StartupMessage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut request = RequestAuthoringState::default();
        Self::hydrate_request_from_selection(&mut request, &shell);
        if request.query_params.is_empty() {
            request.query_params.push(crate::models::QueryParamField {
                name: String::new(),
                value: String::new(),
                enabled: true,
                description: None,
            });
        }
        if request.headers.is_empty() {
            request.headers.push(crate::models::HeaderField {
                name: String::new(),
                value: String::new(),
                enabled: true,
                description: None,
                secret: false,
            });
        }
        let url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://api.example.com/resource")
                .default_value(request.url.clone())
        });
        let request_body_text = Self::body_editor_text(&request.body);
        let request_body_language = Self::body_editor_language(&request.body);
        let post_script_text = request.post_script.clone().unwrap_or_default();
        let environment_manager_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Variable name"));
        let environment_manager_value_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Variable value"));

        let request_body_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(request_body_language)
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .searchable(true)
                .placeholder("Enter request body...")
                .default_value(request_body_text)
        });

        let response_body_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("json")
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .searchable(true)
                .placeholder("Response body will appear here...")
                .default_value(RESPONSE_BODY_PLACEHOLDER)
        });

        let post_script_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("javascript")
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .searchable(true)
                .placeholder("Write post-request script...")
                .default_value(post_script_text)
        });
        let request_auth_bearer_token_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Token"));
        let request_auth_basic_username_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Username"));
        let request_auth_basic_password_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Password"));
        let request_auth_api_key_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Header / Query Name"));
        let request_auth_api_key_value_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("API key value"));

        let _subscriptions = vec![
            cx.subscribe_in(&url_input, window, {
                let url_input = url_input.clone();
                move |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        this.request.url = url_input.read(cx).value().to_string();
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        this.request.url = url_input.read(cx).value().to_string();
                        this.send_request(window, cx);
                        cx.notify();
                    }
                    _ => {}
                }
            }),
            cx.subscribe_in(&request_body_editor, window, {
                let request_body_editor = request_body_editor.clone();
                move |this, _, ev: &InputEvent, _, cx| {
                    if !matches!(ev, InputEvent::Change) {
                        return;
                    }
                    let next_body_text = request_body_editor.read(cx).value().to_string();
                    this.request.body =
                        Self::body_with_updated_text(&this.request.body, next_body_text);
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            }),
            cx.subscribe_in(&post_script_editor, window, {
                let post_script_editor = post_script_editor.clone();
                move |this, _, ev: &InputEvent, _, cx| {
                    if !matches!(ev, InputEvent::Change) {
                        return;
                    }
                    let next_script_text = post_script_editor.read(cx).value().to_string();
                    this.request.post_script =
                        (!next_script_text.trim().is_empty()).then_some(next_script_text);
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            }),
        ];

        let mut view = Self {
            shell,
            request,
            startup_messages,
            url_input,
            request_body_editor,
            response_body_editor,
            response_headers_raw: String::new(),
            post_script_editor,
            active_response_tab: ResponseTab::Body,
            response_status: "—".to_string(),
            response_time: "—".to_string(),
            response_size: "—".to_string(),
            script_result: None,
            environment_manager_selected_id: None,
            environment_manager_variables: Vec::new(),
            request_param_name_inputs: Vec::new(),
            request_param_value_inputs: Vec::new(),
            request_param_input_subscriptions: Vec::new(),
            request_header_name_inputs: Vec::new(),
            request_header_value_inputs: Vec::new(),
            request_header_input_subscriptions: Vec::new(),
            request_auth_bearer_token_input,
            request_auth_basic_username_input,
            request_auth_basic_password_input,
            request_auth_api_key_name_input,
            request_auth_api_key_value_input,
            request_auth_input_subscriptions: Vec::new(),
            suppress_request_auth_change_events: false,
            environment_manager_name_input,
            environment_manager_value_input,
            environment_manager_error: None,
            pending_request_save_due_at: None,
            request_save_tick_scheduled: false,
            request_save_in_flight: false,
            _subscriptions,
        };
        view.rebuild_request_param_inputs(window, cx);
        view.rebuild_request_header_inputs(window, cx);
        view.sync_request_auth_inputs(window, cx);
        view.rebuild_request_auth_input_subscriptions(window, cx);
        view.sync_response_pane_from_selection(window, cx);
        view
    }

    fn send_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.request.send_button_state(), SendButtonState::Ready) {
            return;
        }

        let latest_script = self.post_script_editor.read(cx).value().to_string();
        self.request.post_script = (!latest_script.trim().is_empty()).then_some(latest_script);
        self.request.is_sending = true;
        let request_id = self.shell.collections.selected_request_id();
        let selected_environment_id = self.selected_environment_id_for_view();
        self.response_status = "Sending...".to_string();
        self.response_time = "—".to_string();
        self.response_size = "—".to_string();
        let request_snapshot = self.request.clone();
        let view = cx.entity();

        cx.spawn_in(window, async move |_, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    Self::execute_request_with_script(
                        request_snapshot,
                        request_id,
                        selected_environment_id,
                    )
                })
                .await;

            let _ = view.update_in(cx, |this, window, cx| {
                this.request.is_sending = false;
                let response = outcome.response;
                let response_status = response.status.clone();
                let response_time = response.time.clone();
                let response_size = response.size.clone();
                let response_body = response.body.clone();
                let response_headers = response.headers.clone();
                this.response_status = response_status;
                this.response_time = response_time;
                this.response_size = response_size;
                this.response_body_editor.update(cx, |input, cx| {
                    input.set_value(response_body.clone(), window, cx);
                });
                this.response_headers_raw = response_headers;
                this.script_result = outcome.script_result.clone();
                if let Some(request_id) = request_id {
                    if let Err(error) = Self::persist_response_snapshot(request_id, &response) {
                        eprintln!("Failed to persist response snapshot: {error}");
                    }
                    match outcome.script_result.as_ref() {
                        Some(script_result) => {
                            if let Err(error) =
                                Self::persist_script_result(request_id, script_result)
                            {
                                eprintln!("Failed to persist script result: {error}");
                            }
                        }
                        None => {
                            if let Err(error) = Self::clear_script_result_for_request(request_id) {
                                eprintln!("Failed to clear script result: {error}");
                            }
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();

        cx.notify();
    }

    fn execute_request_with_script(
        request: RequestAuthoringState,
        request_id: Option<Ulid>,
        selected_environment_id: Option<Ulid>,
    ) -> SendRequestOutcome {
        let response = execute_http_request(request.clone());
        let script_text = request.post_script.clone().unwrap_or_default();
        if script_text.trim().is_empty() {
            return SendRequestOutcome {
                response,
                script_result: None,
            };
        }

        let (environment_path, environment_variables) =
            Self::load_environment_for_script(selected_environment_id);
        let runtime_response = ScriptRuntimeResponse {
            status: Self::parse_response_status_code(&response.status).unwrap_or(0),
            status_text: response.status.clone(),
            headers: Self::parse_response_headers(&response.headers),
            body: response.body.clone(),
            response_time_ms: Self::parse_response_duration_ms(&response.time).unwrap_or(0),
            body_size_bytes: response.body.len(),
        };
        let script_exec_result =
            execute_post_request_script(&script_text, &runtime_response, &environment_variables);

        if let Some(path) = environment_path.as_ref() {
            if let Err(error) = Self::apply_script_environment_changes(path, &script_exec_result) {
                eprintln!("Failed to apply script environment changes: {error}");
            }
        }

        let request_id_text = request_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "unknown-request".to_string());
        SendRequestOutcome {
            response,
            script_result: Some(Self::to_persisted_script_result(
                &script_exec_result,
                request_id_text,
            )),
        }
    }

    fn load_environment_for_script(
        selected_environment_id: Option<Ulid>,
    ) -> (Option<PathBuf>, Vec<EnvironmentVariable>) {
        let Some(environment_id) = selected_environment_id else {
            return (None, Vec::new());
        };
        let Some(path) = Self::find_environment_file_path(environment_id) else {
            return (None, Vec::new());
        };
        let Ok(content) = fs::read_to_string(&path) else {
            return (None, Vec::new());
        };
        let Ok(parsed) = EnvironmentManagerDialogView::parse_environment_file(&content) else {
            return (None, Vec::new());
        };
        (Some(path), parsed.variables)
    }

    fn apply_script_environment_changes(
        environment_file_path: &PathBuf,
        script_result: &ScriptExecutionResult,
    ) -> Result<(), String> {
        let content = fs::read_to_string(environment_file_path)
            .map_err(|error| format!("Failed to read environment file: {error}"))?;
        let mut parsed = EnvironmentManagerDialogView::parse_environment_file(&content)?;

        parsed.variables.retain(|var| {
            !script_result
                .removed_env_keys
                .iter()
                .any(|removed| removed == &var.name)
        });

        for (key, value) in &script_result.environment_changes {
            if let Some(var) = parsed.variables.iter_mut().find(|var| var.name == *key) {
                var.value = value.clone();
                var.enabled = true;
            } else {
                parsed.variables.push(EnvironmentVariable {
                    name: key.clone(),
                    value: value.clone(),
                    enabled: true,
                    secret: false,
                    description: None,
                });
            }
        }

        parsed.environment.updated_at = Utc::now();
        let updated_content = toml::to_string_pretty(&parsed)
            .map_err(|error| format!("Failed to encode environment file: {error}"))?;
        fs::write(environment_file_path, updated_content)
            .map_err(|error| format!("Failed to write environment file: {error}"))
    }

    fn to_persisted_script_result(
        result: &ScriptExecutionResult,
        request_id: String,
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
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    fn persist_response_snapshot(
        request_id: Ulid,
        response: &HttpResponseView,
    ) -> Result<(), String> {
        let paths = BeamPaths::default_user_config();
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
            updated_at: Some(Utc::now().to_rfc3339()),
        });
        history_file.executions.push(RequestHistoryExecution {
            status: Self::parse_response_status_code(&response.status),
            duration_ms: Self::parse_response_duration_ms(&response.time),
            response_summary: Some(RequestHistoryResponseSummary {
                body_bytes: Some(response.body.len() as u64),
                body_ref: Some(body_ref),
                body_truncated: false,
                headers: Self::parse_response_headers(&response.headers)
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

    fn parse_response_status_code(status: &str) -> Option<u16> {
        status
            .split_whitespace()
            .next()
            .and_then(|token| token.parse::<u16>().ok())
    }

    fn parse_response_duration_ms(time: &str) -> Option<u64> {
        time.strip_suffix(" ms")
            .and_then(|value| value.trim().parse::<u64>().ok())
    }

    fn body_editor_text(body: &BodyConfig) -> String {
        match body {
            BodyConfig::None => String::new(),
            BodyConfig::Raw { text, .. } => text.clone(),
            BodyConfig::Json { text } => text.clone(),
            BodyConfig::Xml { text } => text.clone(),
            BodyConfig::FormUrlEncoded { fields } | BodyConfig::Multipart { fields } => fields
                .iter()
                .map(|field| format!("{}={}", field.name, field.value))
                .collect::<Vec<_>>()
                .join("\n"),
            BodyConfig::Graphql {
                query,
                variables_json,
            } => match variables_json {
                Some(variables) if !variables.is_empty() => {
                    format!("query:\n{query}\n\nvariables:\n{variables}")
                }
                _ => query.clone(),
            },
        }
    }

    fn body_format_label(format: RequestBodyFormat) -> &'static str {
        match format {
            RequestBodyFormat::None => "None",
            RequestBodyFormat::Json => "JSON",
            RequestBodyFormat::Xml => "XML",
            RequestBodyFormat::Graphql => "GraphQL",
            RequestBodyFormat::Text => "Text",
            RequestBodyFormat::FormUrlEncoded => "Form URL Encoded",
            RequestBodyFormat::Multipart => "Multipart",
        }
    }

    fn supported_body_formats() -> [RequestBodyFormat; 7] {
        [
            RequestBodyFormat::None,
            RequestBodyFormat::Json,
            RequestBodyFormat::Xml,
            RequestBodyFormat::Graphql,
            RequestBodyFormat::Text,
            RequestBodyFormat::FormUrlEncoded,
            RequestBodyFormat::Multipart,
        ]
    }

    fn body_format_from_config(body: &BodyConfig) -> RequestBodyFormat {
        match body {
            BodyConfig::None => RequestBodyFormat::None,
            BodyConfig::Raw { .. } => RequestBodyFormat::Text,
            BodyConfig::Json { .. } => RequestBodyFormat::Json,
            BodyConfig::Xml { .. } => RequestBodyFormat::Xml,
            BodyConfig::FormUrlEncoded { .. } => RequestBodyFormat::FormUrlEncoded,
            BodyConfig::Multipart { .. } => RequestBodyFormat::Multipart,
            BodyConfig::Graphql { .. } => RequestBodyFormat::Graphql,
        }
    }

    fn parse_form_body_fields(text: &str) -> Vec<crate::models::QueryParamField> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                let (name, value) = line
                    .split_once('=')
                    .map(|(name, value)| (name.trim().to_string(), value.to_string()))
                    .unwrap_or_else(|| (line.to_string(), String::new()));
                crate::models::QueryParamField {
                    name,
                    value,
                    enabled: true,
                    description: None,
                }
            })
            .collect()
    }

    fn parse_graphql_editor_text(text: &str) -> (String, Option<String>) {
        if let Some(rest) = text.strip_prefix("query:\n") {
            if let Some((query, variables)) = rest.split_once("\n\nvariables:\n") {
                let variables = variables.trim().to_string();
                let variables_json = (!variables.is_empty()).then_some(variables);
                return (query.to_string(), variables_json);
            }
            return (rest.to_string(), None);
        }
        (text.to_string(), None)
    }

    fn body_with_updated_text(current: &BodyConfig, text: String) -> BodyConfig {
        match current {
            BodyConfig::None => BodyConfig::None,
            BodyConfig::Raw { media_type, .. } => BodyConfig::Raw {
                media_type: media_type.clone(),
                text,
            },
            BodyConfig::Json { .. } => BodyConfig::Json { text },
            BodyConfig::Xml { .. } => BodyConfig::Xml { text },
            BodyConfig::FormUrlEncoded { .. } => BodyConfig::FormUrlEncoded {
                fields: Self::parse_form_body_fields(&text),
            },
            BodyConfig::Multipart { .. } => BodyConfig::Multipart {
                fields: Self::parse_form_body_fields(&text),
            },
            BodyConfig::Graphql { .. } => {
                let (query, variables_json) = Self::parse_graphql_editor_text(&text);
                BodyConfig::Graphql {
                    query,
                    variables_json,
                }
            }
        }
    }

    fn body_from_format(format: RequestBodyFormat, text: String) -> BodyConfig {
        match format {
            RequestBodyFormat::None => BodyConfig::None,
            RequestBodyFormat::Json => BodyConfig::Json { text },
            RequestBodyFormat::Xml => BodyConfig::Xml { text },
            RequestBodyFormat::Graphql => {
                let (query, variables_json) = Self::parse_graphql_editor_text(&text);
                BodyConfig::Graphql {
                    query,
                    variables_json,
                }
            }
            RequestBodyFormat::Text => BodyConfig::Raw {
                media_type: Some("text/plain".to_string()),
                text,
            },
            RequestBodyFormat::FormUrlEncoded => BodyConfig::FormUrlEncoded {
                fields: Self::parse_form_body_fields(&text),
            },
            RequestBodyFormat::Multipart => BodyConfig::Multipart {
                fields: Self::parse_form_body_fields(&text),
            },
        }
    }

    fn set_request_body_format(
        &mut self,
        format: RequestBodyFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current_text = self.request_body_editor.read(cx).value().to_string();
        self.request.body = Self::body_from_format(format, current_text);
        self.request.active_tab = RequestTab::Body;

        let editor_text = Self::body_editor_text(&self.request.body);
        let language = Self::body_editor_language(&self.request.body);
        self.request_body_editor.update(cx, |input, cx| {
            input.set_highlighter(language, cx);
            input.set_value(editor_text, window, cx);
            input.focus(window, cx);
        });

        self.schedule_request_save(cx);
        cx.notify();
    }

    fn format_request_body_text(body: &BodyConfig, text: &str) -> Result<String, String> {
        match body {
            BodyConfig::Json { .. } => {
                let value = serde_json::from_str::<serde_json::Value>(text)
                    .map_err(|err| format!("Unable to format JSON body: {err}"))?;
                serde_json::to_string_pretty(&value)
                    .map_err(|err| format!("Unable to format JSON body: {err}"))
            }
            BodyConfig::Graphql { .. } => {
                let (query, variables_json) = Self::parse_graphql_editor_text(text);
                let query = query.trim().to_string();
                let formatted_variables = if let Some(variables) = variables_json {
                    let trimmed = variables.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        let value =
                            serde_json::from_str::<serde_json::Value>(trimmed).map_err(|err| {
                                format!("Unable to format GraphQL variables JSON: {err}")
                            })?;
                        Some(serde_json::to_string_pretty(&value).map_err(|err| {
                            format!("Unable to format GraphQL variables JSON: {err}")
                        })?)
                    }
                } else {
                    None
                };

                if let Some(variables) = formatted_variables {
                    Ok(format!("query:\n{query}\n\nvariables:\n{variables}"))
                } else {
                    Ok(query)
                }
            }
            BodyConfig::FormUrlEncoded { .. } | BodyConfig::Multipart { .. } => {
                let formatted = Self::parse_form_body_fields(text)
                    .into_iter()
                    .map(|field| format!("{}={}", field.name.trim(), field.value.trim()))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(formatted)
            }
            _ => Err("Formatting is only supported for JSON, GraphQL, and form bodies.".into()),
        }
    }

    fn format_request_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let body = self.request.body.clone();
        let current_text = self.request_body_editor.read(cx).value().to_string();
        let source_text = current_text.clone();

        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { Self::format_request_body_text(&body, &current_text) })
                .await;

            let _ = view.update_in(cx, |this, window, cx| {
                let latest_text = this.request_body_editor.read(cx).value().to_string();
                if latest_text != source_text {
                    window.push_notification(
                        (
                            gpui_component::notification::NotificationType::Warning,
                            "Body changed while formatting. Please run Format again.",
                        ),
                        cx,
                    );
                    cx.notify();
                    return;
                }

                let formatted = match result {
                    Ok(formatted) => formatted,
                    Err(error) => {
                        window.push_notification(
                            (
                                gpui_component::notification::NotificationType::Error,
                                SharedString::from(format!(
                                    "Failed to format request body: {error}"
                                )),
                            ),
                            cx,
                        );
                        cx.notify();
                        return;
                    }
                };

                if formatted == latest_text {
                    window.push_notification("Body is already formatted.".to_string(), cx);
                    cx.notify();
                    return;
                }

                this.request.body =
                    Self::body_with_updated_text(&this.request.body, formatted.clone());
                this.request.active_tab = RequestTab::Body;
                this.request_body_editor.update(cx, |input, cx| {
                    input.set_value(formatted, window, cx);
                    input.focus(window, cx);
                });
                this.schedule_request_save(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn format_response_body_text(text: &str) -> Result<String, String> {
        let value = serde_json::from_str::<serde_json::Value>(text)
            .map_err(|err| format!("Unable to format response body as JSON: {err}"))?;
        serde_json::to_string_pretty(&value)
            .map_err(|err| format!("Unable to format response body as JSON: {err}"))
    }

    fn format_response_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current_text = self.response_body_editor.read(cx).value().to_string();
        let trimmed = current_text.trim();
        if trimmed.is_empty() || trimmed == RESPONSE_BODY_PLACEHOLDER {
            window.push_notification("No response body to format.".to_string(), cx);
            cx.notify();
            return;
        }

        let formatted = match Self::format_response_body_text(&current_text) {
            Ok(formatted) => formatted,
            Err(error) => {
                window.push_notification(
                    (
                        gpui_component::notification::NotificationType::Error,
                        SharedString::from(format!("Failed to format response body: {error}")),
                    ),
                    cx,
                );
                cx.notify();
                return;
            }
        };

        if formatted == current_text {
            window.push_notification("Body is already formatted.".to_string(), cx);
            cx.notify();
            return;
        }

        self.response_body_editor.update(cx, |input, cx| {
            input.set_value(formatted, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn body_editor_language(body: &BodyConfig) -> &'static str {
        match body {
            BodyConfig::Json { .. } => "json",
            BodyConfig::Xml { .. } => "xml",
            BodyConfig::Graphql { .. } => "graphql",
            _ => "text",
        }
    }

    fn render_collections_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut panel = v_flex()
            .h_full()
            .w_full()
            .gap(px(2.0))
            .p_2()
            .bg(rgb(0xf5f7fb))
            .text_color(rgb(0x1f2937));

        if !self.startup_messages.is_empty() {
            for msg in &self.startup_messages {
                panel = panel.child(
                    div()
                        .text_xs()
                        .text_color(rgb(0xb7791f))
                        .child(msg.text.clone()),
                );
            }
        }

        let rows = self.shell.collections.visible_rows();
        if rows.is_empty() {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(0x6b7280))
                    .child("No collections yet"),
            );
        } else {
            for row in rows {
                let node = self.shell.collections.node(row.id).cloned();

                let label = node
                    .as_ref()
                    .map(|n| n.name.clone())
                    .unwrap_or_else(|| "Unknown".to_string());

                let chevron_icon = match row.kind {
                    TreeNodeKind::Collection | TreeNodeKind::Folder => {
                        if self.shell.collections.is_expanded(row.id) {
                            Some("icons/chevron-down.svg")
                        } else {
                            Some("icons/chevron-right.svg")
                        }
                    }
                    TreeNodeKind::Request => None,
                };
                let indent = px((row.depth as f32) * 14.0);

                let mut row_content = h_flex().w_full().items_center().justify_start().gap_2();
                if let Some(icon_path) = chevron_icon {
                    row_content = row_content.child(
                        Icon::default()
                            .path(icon_path)
                            .size(px(14.0))
                            .text_color(rgb(0x6b7280)),
                    );
                }
                if let Some(method) = node.as_ref().and_then(|n| n.request_method) {
                    row_content = row_content.child(Self::render_method_badge(method));
                }
                row_content = row_content.child(label);

                let row_id = row.id;
                let row_kind = row.kind;
                let view = cx.entity();

                panel =
                    panel.child(
                        Button::new(format!("tree-row-{}", row_id))
                            .ghost()
                            .selected(row.selected)
                            .cursor_pointer()
                            .w_full()
                            .rounded(px(8.0))
                            .pl(indent)
                            .px_1()
                            .py(px(1.0))
                            .on_click(cx.listener(move |this, _, window, cx| match row_kind {
                                TreeNodeKind::Collection | TreeNodeKind::Folder => {
                                    this.shell.collections.toggle_expanded(row_id);
                                    if let Err(error) = this.persist_tree_expansion_state() {
                                        window.push_notification(error, cx);
                                    }
                                }
                                TreeNodeKind::Request => {
                                    this.shell.collections.select_request(row_id);
                                    if let Err(error) = this.persist_last_opened_request_id(row_id)
                                    {
                                        window.push_notification(error, cx);
                                    }
                                    this.sync_request_editor_from_selection(window, cx);
                                }
                            }))
                            .child(row_content)
                            .context_menu(move |menu, window, _| {
                                let mut menu = menu.min_w(px(180.0));
                                match row_kind {
                                    TreeNodeKind::Collection => {
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/add.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Add Request")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.add_request_from_tree_node(
                                                        row_id, window, cx,
                                                    );
                                                },
                                            )),
                                        );
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/folder-add.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Add Folder")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.add_folder_from_tree_node(
                                                        row_id, window, cx,
                                                    );
                                                },
                                            )),
                                        );
                                        menu = menu.separator();
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/edit.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Rename")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.open_rename_dialog_for_tree_node(
                                                        row_id,
                                                        TreeNodeKind::Collection,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            )),
                                        );
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/trash.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Delete")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.delete_collection_from_tree_node(
                                                        row_id, window, cx,
                                                    );
                                                },
                                            )),
                                        );
                                    }
                                    TreeNodeKind::Folder => {
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/add.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Add Request")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.add_request_from_tree_node(
                                                        row_id, window, cx,
                                                    );
                                                },
                                            )),
                                        );
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/folder-add.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Add Folder")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.add_folder_from_tree_node(
                                                        row_id, window, cx,
                                                    );
                                                },
                                            )),
                                        );
                                        menu = menu.separator();
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/edit.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Rename")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.open_rename_dialog_for_tree_node(
                                                        row_id,
                                                        TreeNodeKind::Folder,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            )),
                                        );
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/trash.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Delete")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.delete_folder_from_tree_node(
                                                        row_id, window, cx,
                                                    );
                                                },
                                            )),
                                        );
                                    }
                                    TreeNodeKind::Request => {
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/send.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Send Request")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.send_request_from_tree_node(
                                                        row_id, window, cx,
                                                    );
                                                },
                                            )),
                                        );
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/copy.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Copy as cURL")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.copy_request_as_curl_from_tree_node(
                                                        row_id, window, cx,
                                                    );
                                                },
                                            )),
                                        );
                                        menu = menu.separator();
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/edit.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Rename")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.open_rename_dialog_for_tree_node(
                                                        row_id,
                                                        TreeNodeKind::Request,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            )),
                                        );
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/duplicate.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Duplicate")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.duplicate_request_from_tree_node(
                                                        row_id, window, cx,
                                                    );
                                                },
                                            )),
                                        );
                                        menu = menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                h_flex()
                                                    .w_full()
                                                    .cursor_pointer()
                                                    .items_center()
                                                    .gap_2()
                                                    .px_2()
                                                    .py_1()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/trash.svg")
                                                            .size(px(14.0))
                                                            .text_color(rgb(0x6b7280)),
                                                    )
                                                    .child("Delete")
                                            })
                                            .on_click(window.listener_for(
                                                &view,
                                                move |this, _, window, cx| {
                                                    this.delete_request_from_tree_node(
                                                        row_id, window, cx,
                                                    );
                                                },
                                            )),
                                        );
                                    }
                                }
                                menu
                            }),
                    );
            }
        }

        panel
    }

    fn render_url_bar(&self, cx: &mut Context<Self>) -> Div {
        let send_state = self.request.send_button_state();
        let send_disabled = !matches!(send_state, SendButtonState::Ready);
        let current_method = self.request.method;
        let url_has_selection = !self.url_input.read(cx).selected_range().is_empty();
        let selected_environment_id = self.selected_environment_id_for_view();
        let selected_collection_id = self.selected_collection_id();
        let environment_options = self.active_environment_options();
        let env_label = self.selected_environment_label();
        let method_view = cx.entity();
        let environment_view = method_view.clone();

        h_flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(
                div()
                    .flex_1()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(rgb(0xd1d5db))
                    .bg(rgb(0xf8fafc))
                    .p_1()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .w_full()
                            .child(
                                Button::new("request-method-dropdown")
                                    .ghost()
                                    .small()
                                    .px_1()
                                    .cursor_pointer()
                                    .label(Self::method_label(self.request.method))
                                    .dropdown_menu(move |menu, window, _| {
                                        let mut menu = menu.min_w(px(120.));
                                        for method in Self::supported_http_methods() {
                                            let checked = method == current_method;
                                            menu = menu.item(
                                                PopupMenuItem::element(move |_, _| {
                                                    div()
                                                        .w_full()
                                                        .cursor_pointer()
                                                        .child(Self::method_label(method))
                                                })
                                                .checked(checked)
                                                .on_click(window.listener_for(
                                                    &method_view,
                                                    move |this, _, _, cx| {
                                                        this.request.method = method;
                                                        cx.notify();
                                                    },
                                                )),
                                            );
                                        }
                                        menu
                                    }),
                            )
                            .child(
                                Input::new(&self.url_input)
                                    .flex_1()
                                    .small()
                                    .appearance(false)
                                    .context_menu({
                                        move |menu, _, _| {
                                            Self::build_text_edit_context_menu(
                                                menu,
                                                url_has_selection,
                                            )
                                        }
                                    }),
                            )
                            .child(
                                Button::new("send-request")
                                    .ghost()
                                    .small()
                                    .h(px(28.0))
                                    .min_w(px(36.0))
                                    .rounded(px(6.0))
                                    .cursor_pointer()
                                    .disabled(send_disabled)
                                    .icon(
                                        Icon::default()
                                            .path("icons/send.svg")
                                            .size(px(16.0))
                                            .text_color(rgb(0x6b7280)),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_request(window, cx);
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                Button::new("environment-dropdown")
                    .ghost()
                    .small()
                    .h(px(36.0))
                    .min_w(px(180.0))
                    .px_2()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(rgb(0xd1d5db))
                    .bg(rgb(0xf8fafc))
                    .cursor_pointer()
                    .justify_start()
                    .child(div().w_full().child(env_label))
                    .dropdown_menu(move |menu, window, _| {
                        let mut menu = menu.min_w(px(220.));
                        let environment_view = environment_view.clone();

                        let no_env_selected = selected_environment_id.is_none();
                        menu = menu.item(
                            PopupMenuItem::element(move |_, _| {
                                div().w_full().cursor_pointer().child("No environment")
                            })
                            .checked(no_env_selected)
                            .on_click(window.listener_for(
                                &environment_view,
                                move |this, _, _, cx| {
                                    if let Some(collection_id) = selected_collection_id {
                                        this.shell
                                            .environment_selection
                                            .active_collection_environment_ids
                                            .remove(&collection_id);
                                    } else {
                                        this.shell
                                            .environment_selection
                                            .active_global_environment_id = None;
                                    }
                                    cx.notify();
                                },
                            )),
                        );

                        for (environment_id, label, scope) in environment_options.clone() {
                            let checked = Some(environment_id) == selected_environment_id;
                            let item_view = environment_view.clone();
                            menu = menu.item(
                                PopupMenuItem::element(move |_, _| {
                                    div().w_full().cursor_pointer().child(label.clone())
                                })
                                .checked(checked)
                                .on_click(window.listener_for(
                                    &item_view,
                                    move |this, _, _, cx| {
                                        match scope {
                                            EnvironmentScope::Global => {
                                                this.shell
                                                    .environment_selection
                                                    .active_global_environment_id =
                                                    Some(environment_id);
                                            }
                                            EnvironmentScope::Collection => {
                                                if let Some(collection_id) = selected_collection_id
                                                {
                                                    this.shell
                                                        .environment_selection
                                                        .active_collection_environment_ids
                                                        .insert(collection_id, environment_id);
                                                }
                                            }
                                        }
                                        cx.notify();
                                    },
                                )),
                            );
                        }
                        menu = menu.separator().item(
                            PopupMenuItem::new("Manage environment").on_click(window.listener_for(
                                &environment_view,
                                move |this, _, window, cx| {
                                    this.open_environment_manager(window, cx);
                                },
                            )),
                        );
                        menu
                    }),
            )
    }

    fn render_environment_manager_panel(
        &self,
        environment_view: Entity<Self>,
        window: &mut Window,
    ) -> Div {
        let environment_options = self.environment_manager_options();
        let selected_id = self.environment_manager_selected_id;
        let selected_label = selected_id.and_then(|id| {
            environment_options
                .iter()
                .find(|(environment_id, _)| *environment_id == id)
                .map(|(_, label)| label.clone())
        });

        let mut variables_panel = v_flex().h_full().w_full().gap_3();
        variables_panel =
            variables_panel.child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().font_semibold().child(
                        selected_label.unwrap_or_else(|| "No environment selected".to_string()),
                    ))
                    .child(
                        Button::new("close-environment-manager")
                            .small()
                            .ghost()
                            .label("Close")
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                            }),
                    ),
            );
        variables_panel = variables_panel.child(div().text_xs().text_color(rgb(0x6b7280)).child(
            format!("Variables: {}", self.environment_manager_variables.len()),
        ));
        if let Some(error) = &self.environment_manager_error {
            variables_panel = variables_panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(0xb91c1c))
                    .child(error.clone()),
            );
        }
        variables_panel = variables_panel.child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(
                    Input::new(&self.environment_manager_name_input)
                        .small()
                        .flex_1()
                        .appearance(false),
                )
                .child(
                    Input::new(&self.environment_manager_value_input)
                        .small()
                        .flex_1()
                        .appearance(false),
                )
                .child(
                    Button::new("add-environment-variable")
                        .small()
                        .label("Add variable")
                        .on_click(window.listener_for(
                            &environment_view,
                            move |this, _, window, cx| {
                                this.add_environment_variable_from_inputs(window, cx);
                            },
                        )),
                ),
        );

        if self.environment_manager_variables.is_empty() {
            variables_panel = variables_panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(0x6b7280))
                    .child("No variables yet."),
            );
        } else {
            for (index, variable) in self.environment_manager_variables.iter().enumerate() {
                variables_panel = variables_panel.child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .p_1()
                        .rounded(px(6.0))
                        .bg(rgb(0xf8fafc))
                        .border_1()
                        .border_color(rgb(0xe5e7eb))
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(div().text_xs().text_color(rgb(0x6b7280)).child(
                                    if variable.enabled {
                                        "Enabled"
                                    } else {
                                        "Disabled"
                                    },
                                ))
                                .child(
                                    div()
                                        .text_sm()
                                        .child(format!("{} = {}", variable.name, variable.value)),
                                ),
                        )
                        .child(
                            Button::new(format!("delete-environment-variable-{index}"))
                                .small()
                                .ghost()
                                .label("Delete")
                                .on_click(window.listener_for(
                                    &environment_view,
                                    move |this, _, _, cx| {
                                        this.remove_environment_variable(index, cx);
                                    },
                                )),
                        ),
                );
            }
        }

        v_flex()
            .w_full()
            .h(px(420.0))
            .p_3()
            .gap_3()
            .bg(rgb(0xf3f4f6))
            .border_b_1()
            .border_color(rgb(0xd1d5db))
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .gap_3()
                    .child(
                        v_flex()
                            .w(px(260.0))
                            .h_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(0xd1d5db))
                            .bg(rgb(0xffffff))
                            .p_2()
                            .gap_2()
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(div().text_xs().font_semibold().child("Environments"))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x6b7280))
                                            .child(format!("{} total", environment_options.len())),
                                    ),
                            )
                            .child(
                                v_flex().w_full().gap_1().children(
                                    environment_options.into_iter().map(
                                        |(environment_id, label)| {
                                            Button::new(format!(
                                                "environment-manager-select-{environment_id}"
                                            ))
                                            .small()
                                            .ghost()
                                            .selected(Some(environment_id) == selected_id)
                                            .w_full()
                                            .px_2()
                                            .justify_start()
                                            .on_click(window.listener_for(
                                                &environment_view,
                                                move |this, _, _, cx| {
                                                    this.environment_manager_selected_id =
                                                        Some(environment_id);
                                                    this.load_environment_manager_variables(
                                                        environment_id,
                                                    );
                                                    cx.notify();
                                                },
                                            ))
                                            .child(
                                                div()
                                                    .w_full()
                                                    .text_sm()
                                                    .line_height(relative(1.0))
                                                    .child(label),
                                            )
                                        },
                                    ),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(0xd1d5db))
                            .bg(rgb(0xffffff))
                            .p_2()
                            .child(variables_panel),
                    ),
            )
    }

    fn render_request_tabs(&self, cx: &mut Context<Self>) -> Div {
        let mut tabs = h_flex().items_center().gap_1().w_full();
        let body_tab_view = cx.entity();
        let current_body_format = Self::body_format_from_config(&self.request.body);
        let body_tab_button = Button::new("tab-Body")
            .small()
            .ghost()
            .cursor_pointer()
            .selected(self.request.active_tab == RequestTab::Body)
            .child(
                h_flex().items_center().gap_1().child("Body").child(
                    Icon::default()
                        .path("icons/chevron-down.svg")
                        .size(px(12.0))
                        .text_color(rgb(0x6b7280)),
                ),
            );
        if self.request.active_tab == RequestTab::Body {
            tabs = tabs.child(body_tab_button.dropdown_menu(move |menu, window, _| {
                let mut menu = menu.min_w(px(180.0));
                for format in Self::supported_body_formats() {
                    let item_label = Self::body_format_label(format);
                    let checked = format == current_body_format;
                    let format_view = body_tab_view.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            div().w_full().cursor_pointer().child(item_label)
                        })
                        .checked(checked)
                        .on_click(window.listener_for(
                            &format_view,
                            move |this, _, window, cx| {
                                this.set_request_body_format(format, window, cx);
                            },
                        )),
                    );
                }
                menu
            }));
        } else {
            tabs = tabs.child(body_tab_button.on_click(cx.listener(|this, _, window, cx| {
                this.request.active_tab = RequestTab::Body;
                this.request_body_editor.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
            })));
        }

        let tab_specs = [
            (RequestTab::Params, "Params"),
            (RequestTab::Headers, "Headers"),
            (RequestTab::Auth, "Auth"),
        ];

        for (tab, label) in tab_specs {
            tabs = tabs.child(
                Button::new(format!("tab-{label}"))
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .selected(self.request.active_tab == tab)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.request.active_tab = tab;
                        if tab == RequestTab::Body {
                            this.request_body_editor.update(cx, |state, cx| {
                                state.focus(window, cx);
                            });
                        } else if tab == RequestTab::PostScript {
                            this.post_script_editor.update(cx, |state, cx| {
                                state.focus(window, cx);
                            });
                        }
                    }))
                    .label(label),
            );
        }

        let has_script = self
            .request
            .post_script
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty());
        let indicator_color = if !has_script {
            None
        } else if let Some(result) = self.script_result.as_ref() {
            Some(if result.success {
                rgb(0x10b981)
            } else {
                rgb(0xef4444)
            })
        } else {
            Some(rgb(0x9ca3af))
        };
        let post_script_label = if let Some(color) = indicator_color {
            h_flex()
                .items_center()
                .gap_1()
                .child("Post Script")
                .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(color))
        } else {
            h_flex().items_center().gap_1().child("Post Script")
        };
        tabs = tabs.child(
            Button::new("tab-Post Script")
                .small()
                .ghost()
                .cursor_pointer()
                .selected(self.request.active_tab == RequestTab::PostScript)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.request.active_tab = RequestTab::PostScript;
                    this.post_script_editor.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                }))
                .child(post_script_label),
        );

        tabs
    }

    fn context_menu_item_row(
        label: &'static str,
        icon_path: &'static str,
        shortcut: &'static str,
    ) -> Div {
        h_flex()
            .w_full()
            .cursor_pointer()
            .items_center()
            .justify_between()
            .gap_3()
            .px_2()
            .py_1()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::default()
                            .path(icon_path)
                            .size(px(14.0))
                            .text_color(rgb(0x6b7280)),
                    )
                    .child(label),
            )
            .child(div().text_xs().text_color(rgb(0x9ca3af)).child(shortcut))
    }

    fn context_menu_action_item(
        label: &'static str,
        icon_path: &'static str,
        shortcut: &'static str,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> PopupMenuItem {
        PopupMenuItem::element(move |_, _| Self::context_menu_item_row(label, icon_path, shortcut))
            .action(action)
            .disabled(disabled)
    }

    fn build_text_edit_context_menu(menu: PopupMenu, has_selection: bool) -> PopupMenu {
        menu.min_w(px(180.0))
            .item(Self::context_menu_action_item(
                "Cut",
                "icons/cut.svg",
                "Cmd+X",
                Box::new(input::Cut),
                !has_selection,
            ))
            .item(Self::context_menu_action_item(
                "Copy",
                "icons/copy.svg",
                "Cmd+C",
                Box::new(input::Copy),
                !has_selection,
            ))
            .item(Self::context_menu_action_item(
                "Paste",
                "icons/clipboard-paste.svg",
                "Cmd+V",
                Box::new(input::Paste),
                false,
            ))
            .separator()
            .item(Self::context_menu_action_item(
                "Select All",
                "icons/square-dashed-text.svg",
                "Cmd+A",
                Box::new(input::SelectAll),
                false,
            ))
    }

    fn build_text_edit_context_menu_with_find(menu: PopupMenu, has_selection: bool) -> PopupMenu {
        let menu = menu
            .min_w(px(180.0))
            .item(Self::context_menu_action_item(
                "Find",
                "icons/search.svg",
                "Cmd+F",
                Box::new(input::Search),
                false,
            ))
            .separator();
        Self::build_text_edit_context_menu(menu, has_selection)
    }

    fn render_request_editor_surface(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.request.active_tab {
            RequestTab::Body => {
                let request_body_has_selection = !self
                    .request_body_editor
                    .read(cx)
                    .selected_range()
                    .is_empty();
                Input::new(&self.request_body_editor)
                    .h_full()
                    .p_0()
                    .border_0()
                    .focus_bordered(false)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .context_menu({
                        let view = cx.entity();
                        move |menu, window, _| {
                            let menu = menu.min_w(px(180.0)).item(
                                PopupMenuItem::element(move |_, _| {
                                    h_flex()
                                        .w_full()
                                        .cursor_pointer()
                                        .items_center()
                                        .gap_2()
                                        .px_2()
                                        .py_1()
                                        .child(
                                            Icon::default()
                                                .path("icons/indent.svg")
                                                .size(px(14.0))
                                                .text_color(rgb(0x6b7280)),
                                        )
                                        .child("Format")
                                })
                                .on_click(window.listener_for(
                                    &view,
                                    |this, _, window, cx| {
                                        this.format_request_body(window, cx);
                                    },
                                )),
                            );
                            Self::build_text_edit_context_menu_with_find(
                                menu,
                                request_body_has_selection,
                            )
                        }
                    })
                    .into_any_element()
            }
            RequestTab::PostScript => self.render_post_script_editor_and_results(window, cx),
            RequestTab::Params => {
                let mut table = v_flex().h_full().w_full().gap_2();

                for (index, param) in self.request.query_params.iter().enumerate() {
                    let key_input = self.request_param_name_inputs[index].clone();
                    let value_input = self.request_param_value_inputs[index].clone();
                    let key_has_selection = !key_input.read(cx).selected_range().is_empty();
                    let value_has_selection = !value_input.read(cx).selected_range().is_empty();
                    table = table.child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(0xe5e7eb))
                            .child(
                                div().w(px(28.0)).child(
                                    gpui_component::checkbox::Checkbox::new(format!(
                                        "request-param-enabled-{index}"
                                    ))
                                    .small()
                                    .checked(param.enabled)
                                    .on_click(cx.listener(
                                        move |this, checked: &bool, _, cx| {
                                            if let Some(item) =
                                                this.request.query_params.get_mut(index)
                                            {
                                                item.enabled = *checked;
                                                this.schedule_request_save(cx);
                                                cx.notify();
                                            }
                                        },
                                    )),
                                ),
                            )
                            .child(
                                div().flex_1().child(
                                    Input::new(&key_input)
                                        .small()
                                        .w_full()
                                        .appearance(false)
                                        .context_menu({
                                            move |menu, _, _| {
                                                Self::build_text_edit_context_menu(
                                                    menu,
                                                    key_has_selection,
                                                )
                                            }
                                        }),
                                ),
                            )
                            .child(
                                div().flex_1().child(
                                    Input::new(&value_input)
                                        .small()
                                        .w_full()
                                        .appearance(false)
                                        .context_menu({
                                            move |menu, _, _| {
                                                Self::build_text_edit_context_menu(
                                                    menu,
                                                    value_has_selection,
                                                )
                                            }
                                        }),
                                ),
                            )
                            .child(if self.request.query_params.len() > 1 {
                                div().w(px(28.0)).child(
                                    Button::new(format!("delete-request-param-{index}"))
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .icon(
                                            Icon::default()
                                                .path("icons/delete.svg")
                                                .size(px(14.0))
                                                .text_color(rgb(0x6b7280)),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.delete_request_param_row(index, window, cx);
                                        })),
                                )
                            } else {
                                div().w(px(28.0))
                            }),
                    );
                }

                table.into_any_element()
            }
            RequestTab::Headers => {
                let mut table = v_flex().h_full().w_full().gap_2();

                for (index, header) in self.request.headers.iter().enumerate() {
                    let key_input = self.request_header_name_inputs[index].clone();
                    let value_input = self.request_header_value_inputs[index].clone();
                    let key_has_selection = !key_input.read(cx).selected_range().is_empty();
                    let value_has_selection = !value_input.read(cx).selected_range().is_empty();
                    table = table.child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(0xe5e7eb))
                            .child(
                                div().w(px(28.0)).child(
                                    gpui_component::checkbox::Checkbox::new(format!(
                                        "request-header-enabled-{index}"
                                    ))
                                    .small()
                                    .checked(header.enabled)
                                    .on_click(cx.listener(
                                        move |this, checked: &bool, _, cx| {
                                            if let Some(item) = this.request.headers.get_mut(index)
                                            {
                                                item.enabled = *checked;
                                                this.schedule_request_save(cx);
                                                cx.notify();
                                            }
                                        },
                                    )),
                                ),
                            )
                            .child(
                                div().flex_1().child(
                                    Input::new(&key_input)
                                        .small()
                                        .w_full()
                                        .appearance(false)
                                        .context_menu({
                                            move |menu, _, _| {
                                                Self::build_text_edit_context_menu(
                                                    menu,
                                                    key_has_selection,
                                                )
                                            }
                                        }),
                                ),
                            )
                            .child(
                                div().flex_1().child(
                                    Input::new(&value_input)
                                        .small()
                                        .w_full()
                                        .appearance(false)
                                        .context_menu({
                                            move |menu, _, _| {
                                                Self::build_text_edit_context_menu(
                                                    menu,
                                                    value_has_selection,
                                                )
                                            }
                                        }),
                                ),
                            )
                            .child(if self.request.headers.len() > 1 {
                                div().w(px(28.0)).child(
                                    Button::new(format!("delete-request-header-{index}"))
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .icon(
                                            Icon::default()
                                                .path("icons/delete.svg")
                                                .size(px(14.0))
                                                .text_color(rgb(0x6b7280)),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.delete_request_header_row(index, window, cx);
                                        })),
                                )
                            } else {
                                div().w(px(28.0))
                            }),
                    );
                }

                table.into_any_element()
            }
            RequestTab::Auth => {
                div()
                    .h_full()
                    .w_full()
                    .gap_3()
                    .child({
                        let bearer_input = self.request_auth_bearer_token_input.clone();
                        let basic_username_input = self.request_auth_basic_username_input.clone();
                        let basic_password_input = self.request_auth_basic_password_input.clone();
                        let api_key_name_input = self.request_auth_api_key_name_input.clone();
                        let api_key_value_input = self.request_auth_api_key_value_input.clone();
                        let is_none = matches!(self.request.auth, AuthConfig::None);
                        let is_bearer = matches!(self.request.auth, AuthConfig::Bearer { .. });
                        let is_basic = matches!(self.request.auth, AuthConfig::Basic { .. });
                        let is_api_key = matches!(self.request.auth, AuthConfig::ApiKey { .. });

                        h_flex()
                            .items_center()
                            .gap_1()
                            .w_full()
                            .child(
                                Button::new("auth-mode-none")
                                    .small()
                                    .ghost()
                                    .cursor_pointer()
                                    .selected(is_none)
                                    .label("None")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.request.auth = AuthConfig::None;
                                        this.schedule_request_save(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("auth-mode-bearer")
                                    .small()
                                    .ghost()
                                    .cursor_pointer()
                                    .selected(is_bearer)
                                    .label("Bearer Token")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let token = bearer_input.read(cx).value().to_string();
                                        this.request.auth = AuthConfig::Bearer {
                                            token: (!token.trim().is_empty()).then_some(token),
                                        };
                                        this.schedule_request_save(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("auth-mode-basic")
                                    .small()
                                    .ghost()
                                    .cursor_pointer()
                                    .selected(is_basic)
                                    .label("Basic Auth")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let username =
                                            basic_username_input.read(cx).value().to_string();
                                        let password =
                                            basic_password_input.read(cx).value().to_string();
                                        this.request.auth = AuthConfig::Basic {
                                            username: (!username.trim().is_empty())
                                                .then_some(username),
                                            password: (!password.trim().is_empty())
                                                .then_some(password),
                                        };
                                        this.schedule_request_save(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("auth-mode-apikey")
                                    .small()
                                    .ghost()
                                    .cursor_pointer()
                                    .selected(is_api_key)
                                    .label("API Key")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let key = api_key_name_input.read(cx).value().to_string();
                                        let value =
                                            api_key_value_input.read(cx).value().to_string();
                                        this.request.auth = AuthConfig::ApiKey {
                                            key: if key.trim().is_empty() {
                                                Some(DEFAULT_API_KEY_HEADER_NAME.to_string())
                                            } else {
                                                Some(key)
                                            },
                                            value: (!value.trim().is_empty()).then_some(value),
                                            location: crate::models::ApiKeyLocation::Header,
                                        };
                                        this.schedule_request_save(cx);
                                        cx.notify();
                                    })),
                            )
                    })
                    .child({
                        let bearer_has_selection = !self
                            .request_auth_bearer_token_input
                            .read(cx)
                            .selected_range()
                            .is_empty();
                        let basic_username_has_selection = !self
                            .request_auth_basic_username_input
                            .read(cx)
                            .selected_range()
                            .is_empty();
                        let basic_password_has_selection = !self
                            .request_auth_basic_password_input
                            .read(cx)
                            .selected_range()
                            .is_empty();
                        let api_key_value_has_selection = !self
                            .request_auth_api_key_value_input
                            .read(cx)
                            .selected_range()
                            .is_empty();
                        let api_key_name_has_selection = !self
                            .request_auth_api_key_name_input
                            .read(cx)
                            .selected_range()
                            .is_empty();
                        match &self.request.auth {
                            AuthConfig::None => div()
                                .text_sm()
                                .text_color(rgb(0x6b7280))
                                .child("No auth header will be added.")
                                .into_any_element(),
                            AuthConfig::Bearer { .. } => v_flex()
                                .w_full()
                                .gap_2()
                                .child(div().text_xs().text_color(rgb(0x6b7280)).child("Token"))
                                .child(
                                    Input::new(&self.request_auth_bearer_token_input)
                                        .small()
                                        .w_full()
                                        .context_menu({
                                            move |menu, _, _| {
                                                Self::build_text_edit_context_menu(
                                                    menu,
                                                    bearer_has_selection,
                                                )
                                            }
                                        }),
                                )
                                .into_any_element(),
                            AuthConfig::Basic { .. } => v_flex()
                                .w_full()
                                .gap_2()
                                .child(div().text_xs().text_color(rgb(0x6b7280)).child("Username"))
                                .child(
                                    Input::new(&self.request_auth_basic_username_input)
                                        .small()
                                        .w_full()
                                        .context_menu({
                                            move |menu, _, _| {
                                                Self::build_text_edit_context_menu(
                                                    menu,
                                                    basic_username_has_selection,
                                                )
                                            }
                                        }),
                                )
                                .child(div().text_xs().text_color(rgb(0x6b7280)).child("Password"))
                                .child(
                                    Input::new(&self.request_auth_basic_password_input)
                                        .small()
                                        .w_full()
                                        .context_menu({
                                            move |menu, _, _| {
                                                Self::build_text_edit_context_menu(
                                                    menu,
                                                    basic_password_has_selection,
                                                )
                                            }
                                        }),
                                )
                                .into_any_element(),
                            AuthConfig::ApiKey { location, .. } => {
                                let using_header =
                                    matches!(location, crate::models::ApiKeyLocation::Header);
                                let using_query =
                                    matches!(location, crate::models::ApiKeyLocation::Query);
                                v_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        Button::new("auth-apikey-location-header")
                                            .small()
                                            .ghost()
                                            .cursor_pointer()
                                            .selected(using_header)
                                            .label("Header")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let AuthConfig::ApiKey { key, value, .. } =
                                                    &this.request.auth
                                                {
                                                    this.request.auth = AuthConfig::ApiKey {
                                                        key: key.clone(),
                                                        value: value.clone(),
                                                        location:
                                                            crate::models::ApiKeyLocation::Header,
                                                    };
                                                    this.schedule_request_save(cx);
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                    .child(
                                        Button::new("auth-apikey-location-query")
                                            .small()
                                            .ghost()
                                            .cursor_pointer()
                                            .selected(using_query)
                                            .label("Query")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let AuthConfig::ApiKey { key, value, .. } =
                                                    &this.request.auth
                                                {
                                                    this.request.auth = AuthConfig::ApiKey {
                                                        key: key.clone(),
                                                        value: value.clone(),
                                                        location:
                                                            crate::models::ApiKeyLocation::Query,
                                                    };
                                                    this.schedule_request_save(cx);
                                                    cx.notify();
                                                }
                                            })),
                                    ),
                            )
                            .child(div().text_xs().text_color(rgb(0x6b7280)).child("Key Value"))
                            .child(
                                Input::new(&self.request_auth_api_key_value_input)
                                    .small()
                                    .w_full()
                                    .context_menu({
                                        move |menu, _, _| {
                                            Self::build_text_edit_context_menu(
                                                menu,
                                                api_key_value_has_selection,
                                            )
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x6b7280))
                                    .child("Header / Query Name"),
                            )
                            .child(
                                Input::new(&self.request_auth_api_key_name_input)
                                    .small()
                                    .w_full()
                                    .context_menu({
                                        move |menu, _, _| {
                                            Self::build_text_edit_context_menu(
                                                menu,
                                                api_key_name_has_selection,
                                            )
                                        }
                                    }),
                            )
                            .into_any_element()
                            }
                        }
                    })
                    .into_any_element()
            }
        }
    }

    fn render_script_tests_section(&self, result: &PersistedScriptResult) -> AnyElement {
        if result.test_results.is_empty() {
            return div()
                .text_xs()
                .text_color(rgb(0x6b7280))
                .child("No tests recorded.")
                .into_any_element();
        }
        let mut column = v_flex().w_full().gap_1();
        for test in &result.test_results {
            let status = if test.passed { "PASS" } else { "FAIL" };
            let color = if test.passed {
                rgb(0x15803d)
            } else {
                rgb(0xb91c1c)
            };
            let summary = match (&test.expected, &test.actual) {
                (Some(expected), Some(actual)) if expected != actual => {
                    format!("expected={expected}, actual={actual}")
                }
                _ => String::new(),
            };
            let line = if summary.is_empty() {
                format!("[{status}] {}", test.name)
            } else {
                format!("[{status}] {} ({summary})", test.name)
            };
            column = column.child(div().text_xs().text_color(color).child(line));
            if let Some(error) = &test.error_message {
                if !error.is_empty() {
                    column = column.child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x7f1d1d))
                            .child(format!("  {error}")),
                    );
                }
            }
        }
        column.into_any_element()
    }

    fn render_script_env_changes_section(&self, result: &PersistedScriptResult) -> AnyElement {
        if result.environment_diff.is_empty() {
            return div()
                .text_xs()
                .text_color(rgb(0x6b7280))
                .child("No environment changes.")
                .into_any_element();
        }
        let mut column = v_flex().w_full().gap_1();
        for change in &result.environment_diff {
            let line = match change.kind {
                EnvironmentChangeKind::Added => {
                    format!(
                        "[added] {} = {}",
                        change.key,
                        change.new_value.clone().unwrap_or_default()
                    )
                }
                EnvironmentChangeKind::Updated => format!(
                    "[updated] {}: {} -> {}",
                    change.key,
                    change.old_value.clone().unwrap_or_default(),
                    change.new_value.clone().unwrap_or_default()
                ),
                EnvironmentChangeKind::Removed => format!(
                    "[removed] {} (was {})",
                    change.key,
                    change.old_value.clone().unwrap_or_default()
                ),
            };
            column = column.child(div().text_xs().child(line));
        }
        column.into_any_element()
    }

    fn render_script_console_section(&self, result: &PersistedScriptResult) -> AnyElement {
        if result.console_output.is_empty() {
            return div()
                .text_xs()
                .text_color(rgb(0x6b7280))
                .child("No console output.")
                .into_any_element();
        }
        let mut column = v_flex().w_full().gap_1();
        for line in &result.console_output {
            let prefix = line.level.to_uppercase();
            column = column.child(div().text_xs().child(format!(
                "{} [{}] {}",
                Self::format_human_time(&line.timestamp),
                prefix,
                line.message
            )));
        }
        column.into_any_element()
    }

    fn render_post_script_results(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut panel = v_flex()
            .h_full()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .p_2()
            .gap_2();

        if let Some(result) = self.script_result.as_ref() {
            let mut content = v_flex().w_full().gap_2();
            content = content
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .child(div().text_sm().font_semibold().child(if result.success {
                            "Script Succeeded"
                        } else {
                            "Script Failed"
                        }))
                        .child(
                            Button::new("clear-script-results")
                                .small()
                                .ghost()
                                .label("Clear")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.script_result = None;
                                    if let Some(request_id) =
                                        this.shell.collections.selected_request_id()
                                    {
                                        if let Err(error) =
                                            Self::clear_script_result_for_request(request_id)
                                        {
                                            eprintln!("Failed to clear script result: {error}");
                                        }
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().text_xs().text_color(rgb(0x6b7280)).child(format!(
                    "Updated {}",
                    Self::format_human_timestamp(&result.updated_at)
                )));

            if let Some(error_message) = &result.error_message {
                if !error_message.is_empty() {
                    content = content.child(
                        div()
                            .text_xs()
                            .text_color(rgb(0xb91c1c))
                            .child(format!("Error: {error_message}")),
                    );
                }
            }

            content = content.child(div().text_xs().font_semibold().child("Tests"));
            content = content.child(self.render_script_tests_section(result));
            content = content.child(div().text_xs().font_semibold().child("Environment Changes"));
            content = content.child(self.render_script_env_changes_section(result));
            content = content.child(div().text_xs().font_semibold().child("Console"));
            content = content.child(self.render_script_console_section(result));

            panel = panel.child(
                div()
                    .w_full()
                    .h_full()
                    .overflow_y_scrollbar()
                    .child(content),
            );
        } else {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(0x6b7280))
                    .child("Send a request to see script results"),
            );
        }

        panel.into_any_element()
    }

    fn render_post_script_editor_and_results(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let post_script_has_selection =
            !self.post_script_editor.read(cx).selected_range().is_empty();
        // TODO: Keep this as a single parent card with one divider between editor/results.
        // It avoids double-border overlap and is less error-prone than separate bordered panes.
        v_flex()
            .h_full()
            .w_full()
            .gap_0()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(0xd1d5db))
            .bg(rgb(0xffffff))
            .child(
                div()
                    .h_1_2()
                    .min_h_0()
                    .w_full()
                    .border_b_1()
                    .border_color(rgb(0xd1d5db))
                    .p_0()
                    .child(
                        div().w_full().h_full().overflow_y_scrollbar().child(
                            Input::new(&self.post_script_editor)
                                .h_full()
                                .p_0()
                                .border_0()
                                .focus_bordered(false)
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(cx.theme().mono_font_size)
                                .context_menu({
                                    move |menu, _, _| {
                                        Self::build_text_edit_context_menu_with_find(
                                            menu,
                                            post_script_has_selection,
                                        )
                                    }
                                }),
                        ),
                    ),
            )
            .child(
                div()
                    .h_1_2()
                    .min_h_0()
                    .w_full()
                    .child(self.render_post_script_results(window, cx)),
            )
            .into_any_element()
    }

    fn render_request_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let editor_container = match self.request.active_tab {
            RequestTab::Body => div()
                .flex_1()
                .w_full()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(0xd1d5db))
                .p_0()
                .child(self.render_request_editor_surface(window, cx)),
            RequestTab::PostScript => div()
                .flex_1()
                .w_full()
                .child(self.render_request_editor_surface(window, cx)),
            _ => div()
                .flex_1()
                .w_full()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(0xd1d5db))
                .p_3()
                .child(self.render_request_editor_surface(window, cx)),
        };

        v_flex()
            .h_full()
            .w_full()
            .gap_2()
            .p_3()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x1f2937))
            .child(self.render_request_tabs(cx))
            .child(editor_container)
    }

    fn render_response_tabs(&self, cx: &mut Context<Self>) -> Div {
        let mut tabs = h_flex().items_center().gap_1().w_full();
        let tab_specs = [
            (ResponseTab::Body, "Body"),
            (ResponseTab::Headers, "Headers"),
        ];

        for (tab, label) in tab_specs {
            tabs = tabs.child(
                Button::new(format!("response-tab-{label}"))
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .selected(self.active_response_tab == tab)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active_response_tab = tab;
                        cx.notify();
                    }))
                    .label(label),
            );
        }

        tabs
    }

    fn render_response_editor_surface(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.active_response_tab {
            ResponseTab::Body => {
                let response_body_has_selection = !self
                    .response_body_editor
                    .read(cx)
                    .selected_range()
                    .is_empty();
                Input::new(&self.response_body_editor)
                    .h_full()
                    .p_0()
                    .border_0()
                    .focus_bordered(false)
                    .disabled(true)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .context_menu({
                        let view = cx.entity();
                        move |menu, window, _| {
                            let menu = menu
                                .min_w(px(180.0))
                                .item(
                                    PopupMenuItem::element(move |_, _| {
                                        h_flex()
                                            .w_full()
                                            .cursor_pointer()
                                            .items_center()
                                            .gap_2()
                                            .px_2()
                                            .py_1()
                                            .child(
                                                Icon::default()
                                                    .path("icons/indent.svg")
                                                    .size(px(14.0))
                                                    .text_color(rgb(0x6b7280)),
                                            )
                                            .child("Format")
                                    })
                                    .on_click(
                                        window.listener_for(&view, |this, _, window, cx| {
                                            this.format_response_body(window, cx);
                                        }),
                                    ),
                                )
                                .separator();
                            Self::build_text_edit_context_menu(menu, response_body_has_selection)
                        }
                    })
                    .into_any_element()
            }
            ResponseTab::Headers => self.render_response_headers_table(),
        }
    }

    fn parse_response_headers(headers: &str) -> Vec<(String, String)> {
        headers
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                if let Some((key, value)) = line.split_once(':') {
                    (key.trim().to_string(), value.trim_start().to_string())
                } else {
                    (line.to_string(), String::new())
                }
            })
            .collect()
    }

    fn render_response_headers_table(&self) -> AnyElement {
        let rows = Self::parse_response_headers(&self.response_headers_raw);
        if rows.is_empty() {
            return div()
                .h_full()
                .w_full()
                .text_sm()
                .text_color(rgb(0x6b7280))
                .child("No response headers. Send a request to view headers.")
                .into_any_element();
        }

        fn escape_html(text: &str) -> String {
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#39;")
        }

        let mut table = String::from(
            "<table style=\"width:100%; border-collapse:collapse; font-size:12px;\">\
             <thead>\
             <tr style=\"background:#f3f4f6;\">\
             <th style=\"text-align:left; padding:6px 8px; border-bottom:1px solid #e5e7eb; width:220px;\">Header</th>\
             <th style=\"text-align:left; padding:6px 8px; border-bottom:1px solid #e5e7eb;\">Value</th>\
             </tr>\
             </thead><tbody>",
        );

        for (key, value) in rows {
            table.push_str(&format!(
                "<tr>\
                 <td style=\"padding:6px 8px; border-bottom:1px solid #f3f4f6; vertical-align:top; white-space:pre-wrap;\">{}</td>\
                 <td style=\"padding:6px 8px; border-bottom:1px solid #f3f4f6; vertical-align:top; white-space:pre-wrap;\">{}</td>\
                 </tr>",
                escape_html(&key),
                escape_html(&value)
            ));
        }
        table.push_str("</tbody></table>");

        html(table)
            .w_full()
            .h_full()
            .scrollable(true)
            .selectable(true)
            .into_any_element()
    }

    fn render_response_panel(&self, _: &mut Window, cx: &mut Context<Self>) -> Div {
        let response_container = match self.active_response_tab {
            ResponseTab::Body => div()
                .flex_1()
                .w_full()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(0xd1d5db))
                .p_0()
                .child(self.render_response_editor_surface(cx)),
            ResponseTab::Headers => div()
                .flex_1()
                .w_full()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(0xd1d5db))
                .p_3()
                .child(self.render_response_editor_surface(cx)),
        };

        v_flex()
            .h_full()
            .w_full()
            .gap_2()
            .p_3()
            .bg(rgb(0xfafafa))
            .text_color(rgb(0x1f2937))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .gap_2()
                    .child(self.render_response_tabs(cx))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(rgb(0x6b7280))
                            .child(format!("Status: {}", self.response_status))
                            .child(format!("Time: {}", self.response_time))
                            .child(format!("Size: {}", self.response_size)),
                    ),
            )
            .child(response_container)
    }

    fn render_status_bar(&self) -> Div {
        h_flex()
            .items_center()
            .justify_between()
            .w_full()
            .h(px(28.0))
            .px_3()
            .bg(rgb(0xf3f4f6))
            .border_t_1()
            .border_color(rgb(0xd1d5db))
            .text_xs()
            .text_color(rgb(0x6b7280))
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child("Ready")
                    .child("Env: Local")
                    .child("Collection: Sample"),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child("UTF-8")
                    .child("Ln 1, Col 1")
                    .child("Spaces: 2"),
            )
    }
}

struct HttpResponseView {
    status: String,
    time: String,
    size: String,
    body: String,
    headers: String,
}

struct SendRequestOutcome {
    response: HttpResponseView,
    script_result: Option<PersistedScriptResult>,
}

static HTTP_CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();

fn default_user_agent() -> String {
    format!("Beam/{}", env!("CARGO_PKG_VERSION"))
}

fn shared_http_client() -> Result<&'static Client, String> {
    HTTP_CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent(default_user_agent())
                .build()
                .map_err(|error| format!("Failed to initialize HTTP client: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn execute_http_request(request: RequestAuthoringState) -> HttpResponseView {
    let start = Instant::now();
    let client = match shared_http_client() {
        Ok(client) => client,
        Err(error) => {
            return HttpResponseView {
                status: "Error".to_string(),
                time: "—".to_string(),
                size: "—".to_string(),
                body: error,
                headers: String::new(),
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
            let password_for_header = (!pass.is_empty()).then_some(pass);
            builder = builder.basic_auth(user, password_for_header);
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
            let mut form = reqwest::blocking::multipart::Form::new();
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

    match builder.send() {
        Ok(response) => {
            let status = response.status();
            let status_text = status
                .canonical_reason()
                .map(|reason| format!("{} {}", status.as_u16(), reason))
                .unwrap_or_else(|| status.as_u16().to_string());
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| {
                    let value = value.to_str().unwrap_or("<non-utf8>");
                    format!("{}: {value}", name.as_str())
                })
                .collect::<Vec<_>>()
                .join("\n");
            match response.bytes() {
                Ok(bytes) => {
                    let body = String::from_utf8_lossy(&bytes).to_string();
                    HttpResponseView {
                        status: status_text,
                        time: format!("{} ms", start.elapsed().as_millis()),
                        size: format_bytes(bytes.len()),
                        body,
                        headers,
                    }
                }
                Err(error) => HttpResponseView {
                    status: status_text,
                    time: format!("{} ms", start.elapsed().as_millis()),
                    size: "—".to_string(),
                    body: format!("Failed to read response body: {error}"),
                    headers,
                },
            }
        }
        Err(error) => HttpResponseView {
            status: "Error".to_string(),
            time: format!("{} ms", start.elapsed().as_millis()),
            size: "—".to_string(),
            body: format!("Request failed: {error}"),
            headers: String::new(),
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
    }
}

fn format_bytes(byte_count: usize) -> String {
    if byte_count < 1024 {
        return format!("{byte_count} B");
    }
    let kib = byte_count as f64 / 1024.0;
    if kib < 1024.0 {
        return format!("{kib:.1} KiB");
    }
    let mib = kib / 1024.0;
    format!("{mib:.1} MiB")
}

impl Render for BeamView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let left_size = 1280.0 * self.shell.layout.collections_workspace.ratio();
        let request_size = (1280.0 - left_size) * 0.5;

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(TitleBar::new().child(self.render_title_bar_content()))
            .child(
                h_flex().flex_1().w_full().child(
                    h_resizable("beam-main-shell")
                        .child(
                            resizable_panel()
                                .size(px(left_size))
                                .child(self.render_collections_panel(cx)),
                        )
                        .child(resizable_panel().child({
                            let workspace = v_flex().h_full().w_full().bg(rgb(0xffffff)).child(
                                div()
                                    .w_full()
                                    .p_3()
                                    .border_b_1()
                                    .border_color(rgb(0xd1d5db))
                                    .child(self.render_url_bar(cx)),
                            );
                            workspace
                                .child(
                                    div().flex_1().child(
                                        h_resizable("beam-workspace-split")
                                            .child(
                                                resizable_panel()
                                                    .size(px(request_size))
                                                    .child(self.render_request_panel(window, cx)),
                                            )
                                            .child(
                                                resizable_panel()
                                                    .child(self.render_response_panel(window, cx)),
                                            )
                                            .into_any_element(),
                                    ),
                                )
                                .into_any_element()
                        })),
                ),
            )
            .child(self.render_status_bar())
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
