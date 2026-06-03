use std::any::Any;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use std::{fs, path::PathBuf};

use chrono::{Local, Utc};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, Disableable, Icon, Placement, Root, Selectable, Sizable, StyledExt, Theme,
    ThemeMode, ThemeRegistry, TitleBar, WindowExt as _,
    button::{Button, ButtonVariants as _, DropdownButton},
    h_flex,
    hover_card::HoverCard,
    input::{self, Input, InputEvent, InputState, Position, TabSize},
    list::ListItem,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    resizable::{h_resizable, resizable_panel},
    scroll::ScrollableElement,
    tag::Tag,
    text::{html, markdown},
    v_flex,
};
use reqwest::{Client, Method};
use tokio::runtime::{Builder as TokioRuntimeBuilder, Runtime as TokioRuntime};
use tokio::sync::oneshot;
use ulid::Ulid;

use crate::app_shell::next_command_id;
use crate::app_shell::{
    AppCommand, AppEvent, AppShellState, DataSyncRuntime, RequestPaneData, StartupMessage,
    TreeNodeKind,
};
use crate::assets::{Assets, embedded_theme_contents};
use crate::models::{
    AuthConfig, BodyConfig, EnvironmentFile, EnvironmentScope, EnvironmentVariable, HttpMethod,
    LocalStateFile, RequestFile,
};
use crate::paths::{BeamPaths, DataRootPaths};
use crate::post_script_help::POST_SCRIPT_API_HELP_MARKDOWN;
use crate::request_authoring::{
    RenameValidationError, RequestAuthoringState, RequestTab, SendButtonState, SendDisabledReason,
    validate_rename,
};
use crate::script::{
    ConsoleLevel, EnvironmentChange, EnvironmentChangeKind, ScriptExecutionResult,
    ScriptRuntimeResponse, TestResult, execute_post_request_script,
};
use crate::storage::fs_backend::FileSystemStorage;
use crate::storage::workspace_repo::WorkspaceRepository;
use crate::storage::{
    CreateFolderInput, CreateRequestInput, DeleteRequestInput, DuplicateRequestInput,
    FolderParentRef, KnownParentManifestPath, MoveFolderInput, MoveRequestInput,
    RenameRequestInput, RequestParentRef,
};

actions!(
    beam,
    [
        QuitApp,
        SendActiveRequest,
        CreateRequestBelowActive,
        FocusUrlInput,
        OpenSettings
    ]
);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
struct SwitchTheme(pub SharedString);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
struct SwitchThemeMode(pub ThemeMode);

#[cfg(target_os = "macos")]
fn build_macos_theme_menu(cx: &App) -> MenuItem {
    let themes = ThemeRegistry::global(cx).sorted_themes();
    let active_theme_name = cx.theme().theme_name().clone();
    MenuItem::Submenu(Menu {
        name: "Theme".into(),
        items: themes
            .iter()
            .map(|theme| {
                MenuItem::action(theme.name.clone(), SwitchTheme(theme.name.clone()))
                    .checked(theme.name == active_theme_name)
            })
            .collect(),
        disabled: false,
    })
}

#[cfg(target_os = "macos")]
fn build_macos_system_menus(cx: &App) -> Vec<Menu> {
    vec![
        Menu {
            name: "Beam".into(),
            items: vec![
                MenuItem::action("Settings", OpenSettings),
                MenuItem::separator(),
                MenuItem::Submenu(Menu {
                    name: "Appearance".into(),
                    items: vec![
                        MenuItem::action("Light", SwitchThemeMode(ThemeMode::Light))
                            .checked(!cx.theme().mode.is_dark()),
                        MenuItem::action("Dark", SwitchThemeMode(ThemeMode::Dark))
                            .checked(cx.theme().mode.is_dark()),
                    ],
                    disabled: false,
                }),
                build_macos_theme_menu(cx),
                MenuItem::separator(),
                MenuItem::action("Quit Beam", QuitApp),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Request", CreateRequestBelowActive),
                MenuItem::separator(),
                MenuItem::action("Focus URL", FocusUrlInput),
            ],
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo", gpui_component::input::Undo),
                MenuItem::action("Redo", gpui_component::input::Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", gpui_component::input::Cut),
                MenuItem::action("Copy", gpui_component::input::Copy),
                MenuItem::action("Paste", gpui_component::input::Paste),
                MenuItem::separator(),
                MenuItem::action("Select All", gpui_component::input::SelectAll),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![MenuItem::action("Focus URL", FocusUrlInput)],
            disabled: false,
        },
    ]
}

#[cfg(not(target_family = "wasm"))]
fn init_theme_registry(preferred_theme_name: Option<SharedString>, cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for (theme_path, content) in embedded_theme_contents() {
        if let Err(error) = registry.load_themes_from_str(&content) {
            log::error!("Failed to preload theme file {theme_path}: {error}");
        }
    }

    if let Some(theme_name) = preferred_theme_name.as_ref() {
        let _ = BeamView::apply_named_theme_by_name(theme_name.as_ref(), cx, false);
    }
}

pub fn run_app(
    state: AppShellState,
    startup_messages: Vec<StartupMessage>,
    sync_runtime: DataSyncRuntime,
    workspace_paths: BeamPaths,
) {
    let app = gpui_platform::application().with_assets(Assets);
    app.run(move |cx| {
        gpui_component::init(cx);
        #[cfg(not(target_family = "wasm"))]
        init_theme_registry(state.theme.theme_name.clone().map(Into::into), cx);
        cx.bind_keys([
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-q", QuitApp, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-enter", SendActiveRequest, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-n", CreateRequestBelowActive, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-l", FocusUrlInput, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-,", OpenSettings, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("alt-f4", QuitApp, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-enter", SendActiveRequest, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-n", CreateRequestBelowActive, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-l", FocusUrlInput, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-,", OpenSettings, None),
        ]);
        cx.on_action(|_: &QuitApp, cx: &mut App| {
            cx.quit();
        });
        cx.on_action(|_: &SendActiveRequest, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.handle_send_or_cancel_action(window, cx);
                                });
                            });
                        }
                    }
                }
            });
        });
        cx.on_action(|_: &CreateRequestBelowActive, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.create_request_below_active(window, cx);
                                });
                            });
                        }
                    }
                }
            });
        });
        cx.on_action(|_: &FocusUrlInput, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.focus_url_input(window, cx);
                                });
                            });
                        }
                    }
                }
            });
        });
        cx.on_action(|_: &OpenSettings, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.open_settings_dialog(window, cx);
                                });
                            });
                        }
                    }
                }
            });
        });
        cx.on_action(|switch: &SwitchThemeMode, cx: &mut App| {
            BeamView::apply_theme_mode(switch.0, cx);
        });
        cx.on_action(|switch: &SwitchTheme, cx: &mut App| {
            BeamView::apply_named_theme(switch.0.clone(), cx);
        });
        #[cfg(target_os = "macos")]
        {
            cx.set_menus(build_macos_system_menus(cx));
            cx.observe_global::<Theme>(|cx| {
                cx.set_menus(build_macos_system_menus(cx));
            })
            .detach();
            cx.observe_global::<ThemeRegistry>(|cx| {
                cx.set_menus(build_macos_system_menus(cx));
            })
            .detach();
        }

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
            titlebar: Some(TitleBar::title_bar_options()),
            ..Default::default()
        };

        let state = state.clone();
        let startup_messages = startup_messages.clone();
        let workspace_paths = workspace_paths.clone();
        cx.open_window(window_options, |window, cx| {
            let view = cx.new(|cx| {
                BeamView::new(
                    state,
                    startup_messages,
                    sync_runtime,
                    workspace_paths,
                    window,
                    cx,
                )
            });
            cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
        })
        .expect("Failed to open Beam window");
        cx.activate(true);
    });
}

struct BeamView {
    shell: AppShellState,
    current_workspace_paths: BeamPaths,
    request: RequestAuthoringState,
    startup_messages: Vec<StartupMessage>,
    url_input: Entity<InputState>,
    request_body_editor: Entity<InputState>,
    response_body_editor: Entity<InputState>,
    response_headers_raw: String,
    response_content_type: Option<String>,
    response_history_entries: Vec<ResponseHistoryEntry>,
    post_script_editor: Entity<InputState>,
    active_response_tab: ResponseTab,
    response_status: String,
    response_time: String,
    response_size: String,
    script_result: Option<PersistedScriptResult>,
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
    pending_request_save_due_at: Option<Instant>,
    request_save_tick_scheduled: bool,
    request_save_in_flight: bool,
    show_invalid_url_border: bool,
    active_request_cache: Option<RequestFile>,
    request_file_index: HashMap<Ulid, PathBuf>,
    environment_manager_dialog_view: Option<Entity<EnvironmentManagerDialogView>>,
    settings_dialog_view: Option<Entity<SettingsDialogView>>,
    request_execution_states: HashMap<Ulid, RequestExecutionState>,
    next_request_run_id: u64,
    app_command_tx: std::sync::mpsc::SyncSender<AppCommand>,
    app_event_rx: std::sync::mpsc::Receiver<AppEvent>,
    app_event_poll_scheduled: bool,
    pending_request_placements: HashMap<String, PendingRequestPlacement>,
    _subscriptions: Vec<Subscription>,
    collection_scroll_handle: UniformListScrollHandle,
    collection_context_menu_row: Option<crate::app_shell::TreeRow>,
    tree_drag_hover: Option<(Ulid, TreeDropPlacement)>,
    env_var_hover: Option<EnvVarHoverInfo>,
    /// Cached resolved env variables for the overlay: (active_env_id, resolved_map).
    /// Invalidated when the effective environment changes or environment data updates.
    env_var_resolved_cache: Option<(Option<Ulid>, HashMap<String, String>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseTab {
    Body,
    Headers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingRequestPlacement {
    Append {
        parent: RequestParentRef,
    },
    After {
        parent: RequestParentRef,
        after_request_id: Ulid,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreeDropPlacement {
    Before,
    Into,
    After,
}

#[derive(Clone, Debug)]
struct DraggedFolder {
    folder_id: Ulid,
    label: String,
}

#[derive(Clone, Debug)]
struct DraggedRequest {
    request_id: Ulid,
    label: String,
}

#[derive(Clone, Debug)]
enum TreeMoveAction {
    MoveRequest(MoveRequestInput),
    MoveFolder(MoveFolderInput),
}

fn trailing_tree_drop_slot_target(
    rows: &[crate::app_shell::TreeRow],
    row_index: usize,
    mut parent_id_for: impl FnMut(Ulid) -> Option<Ulid>,
) -> Option<Ulid> {
    let row = rows.get(row_index)?;
    if row_index + 1 != rows.len() || row.depth == 0 {
        return None;
    }

    let mut current_id = row.id;
    loop {
        match parent_id_for(current_id) {
            Some(parent_id) => current_id = parent_id,
            None => return Some(current_id),
        }
    }
}

fn tree_row_shows_before_drop_slot(rows: &[crate::app_shell::TreeRow], row_index: usize) -> bool {
    if row_index == 0 {
        return true;
    }

    rows.get(row_index - 1)
        .zip(rows.get(row_index))
        .is_some_and(|(previous, current)| previous.depth != current.depth)
}

fn tree_row_shows_after_drop_slot(rows: &[crate::app_shell::TreeRow], row_index: usize) -> bool {
    let Some(current) = rows.get(row_index) else {
        return false;
    };

    match rows.get(row_index + 1) {
        Some(next) => next.depth <= current.depth,
        None => true,
    }
}

struct TreeDragPreview {
    label: String,
    kind: TreeNodeKind,
    position: Point<Pixels>,
}

#[derive(Clone, Debug)]
struct EnvVarHoverInfo {
    var_name: String,
    resolved_value: Option<String>,
    token_bounds: Bounds<Pixels>,
}

impl TreeDragPreview {
    fn new(label: String, kind: TreeNodeKind, position: Point<Pixels>) -> Self {
        Self {
            label,
            kind,
            position,
        }
    }
}

impl Render for TreeDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let icon_path = match self.kind {
            TreeNodeKind::Folder => "icons/folder.svg",
            TreeNodeKind::Request => "icons/file.svg",
        };

        div()
            .pl(self.position.x - px(72.0))
            .pt(self.position.y - px(18.0))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .rounded(px(8.0))
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .shadow_md()
                    .child(
                        Icon::default()
                            .path(icon_path)
                            .size(px(14.0))
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .max_w(px(240.0))
                            .truncate()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(self.label.clone()),
                    ),
            )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestExecutionStatus {
    Idle,
    Sending,
    Canceled,
    Failed,
}

struct RequestExecutionState {
    run_id: u64,
    status: RequestExecutionStatus,
    cancel_tx: Option<oneshot::Sender<()>>,
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
const RESPONSE_BODY_TRUNCATED_NOTE: &str =
    "[Response body omitted from local history (truncated).]";
const MACOS_COMMAND_ICON_PATH: &str = "icons/command.svg";
const NON_MACOS_COMMAND_ICON_PATH: &str = "icons/chevron-up.svg";

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
    #[serde(default)]
    no_environment_selected_with_env_writes: bool,
    updated_at: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct ConsoleMessageView {
    level: String,
    message: String,
    timestamp: String,
}

struct EnvironmentManagerDialogView {
    beam_view: Entity<BeamView>,
    options: Vec<(Ulid, String)>,
    environment_file_names: HashMap<Ulid, String>,
    selected_id: Option<Ulid>,
    active_environment_id: Option<Ulid>,
    show_environment_selector: bool,
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
    loaded_environment_name: Option<String>,
    pending_new_environment_command_id: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsSection {
    Theme,
}

struct SettingsDialogView {
    beam_view: Entity<BeamView>,
    selected_section: SettingsSection,
}

impl SettingsDialogView {
    fn new(beam_view: Entity<BeamView>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            beam_view,
            selected_section: SettingsSection::Theme,
        }
    }
}

impl Render for SettingsDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_theme_name = cx.theme().theme_name().clone();
        let theme_options: Vec<SharedString> = ThemeRegistry::global(cx)
            .sorted_themes()
            .into_iter()
            .map(|theme| theme.name.clone())
            .collect();

        let mut right_panel = v_flex().w_full().h_full().gap_3();
        match self.selected_section {
            SettingsSection::Theme => {
                let beam_view = self.beam_view.clone();
                let active_theme_name_for_menu = active_theme_name.clone();
                let theme_options_for_menu = theme_options.clone();
                right_panel = right_panel
                    .child(div().text_sm().font_semibold().child("Theme"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Choose a theme. The selected theme is also available from the system menu."),
                    )
                    .child(
                        DropdownButton::new("settings-theme-dropdown")
                            .w(px(320.0))
                            .button(
                                Button::new("settings-theme-dropdown-button")
                                    .w(px(290.0))
                                    .justify_start()
                                    .label(active_theme_name.to_string()),
                            )
                            .dropdown_menu(move |menu, window, _| {
                                theme_options_for_menu.clone().into_iter().fold(
                                    menu.scrollable(true).max_h(px(220.0)),
                                    |menu, theme_name| {
                                        let selected_theme = theme_name.clone();
                                    let target_view = beam_view.clone();
                                    let checked = theme_name == active_theme_name_for_menu;
                                    menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                div()
                                                    .w_full()
                                                    .px_2()
                                                    .py_1()
                                                    .cursor_pointer()
                                                    .child(theme_name.clone())
                                            })
                                        .checked(checked)
                                        .on_click(window.listener_for(
                                            &target_view,
                                            move |_: &mut BeamView, _, _, cx| {
                                                BeamView::apply_named_theme(
                                                    selected_theme.clone(),
                                                    cx,
                                                );
                                                cx.notify();
                                            },
                                        )),
                                    )
                                })
                            }),
                    );
            }
        }

        v_flex()
            .w_full()
            .h(px(520.0))
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .gap_3()
                    .child(
                        v_flex()
                            .w(px(220.0))
                            .h_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .p_2()
                            .gap_1()
                            .child(
                                ListItem::new("settings-section-theme")
                                    .w_full()
                                    .cursor_pointer()
                                    .rounded(px(8.0))
                                    .px_2()
                                    .py_1()
                                    .selected(self.selected_section == SettingsSection::Theme)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.selected_section = SettingsSection::Theme;
                                        cx.notify();
                                    }))
                                    .child("Theme"),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .p_3()
                            .child(right_panel),
                    ),
            )
            .into_any_element()
    }
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

#[derive(Clone)]
struct ResponseHistoryEntry {
    title: String,
    summary: String,
    execution: RequestHistoryExecution,
}

#[derive(Clone)]
struct StoredResponseSnapshot {
    status: String,
    time: String,
    size: String,
    body: String,
    headers_raw: String,
    content_type: Option<String>,
}

impl EnvironmentManagerDialogView {
    fn sync_environment_options(
        &mut self,
        next_options: Vec<(Ulid, String)>,
        next_file_names: HashMap<Ulid, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let options_changed = self.options != next_options;
        let file_names_changed = self.environment_file_names != next_file_names;
        if options_changed {
            self.options = next_options;
        }
        if file_names_changed {
            self.environment_file_names = next_file_names;
        }

        let selected_exists = self
            .selected_id
            .is_some_and(|id| self.options.iter().any(|(option_id, _)| *option_id == id));
        if selected_exists {
            return options_changed || file_names_changed;
        }

        let previous_selection = self.selected_id;
        self.selected_id = self.options.first().map(|(id, _)| *id);
        if let Some(environment_id) = self.selected_id {
            self.load_variables(environment_id, window, cx);
            return true;
        }

        if previous_selection.is_none() && !options_changed && !file_names_changed {
            return false;
        }

        self.variables.clear();
        self.clear_variable_inputs();
        self.loaded_environment_name = None;
        self.suppress_environment_name_change_events = true;
        self.environment_name_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.suppress_environment_name_change_events = false;
        self.error = Some("No environment available to manage.".to_string());
        true
    }

    fn parse_environment_file(content: &str) -> Result<EnvironmentFile, String> {
        toml::from_str::<EnvironmentFile>(content)
            .map_err(|error| format!("Failed to parse environment file: {error}"))
    }

    fn new(
        beam_view: Entity<BeamView>,
        options: Vec<(Ulid, String)>,
        environment_file_names: HashMap<Ulid, String>,
        selected_id: Option<Ulid>,
        active_environment_id: Option<Ulid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let environment_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Environment name"));
        let mut view = Self {
            beam_view,
            options,
            environment_file_names,
            selected_id,
            active_environment_id,
            show_environment_selector: true,
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
            loaded_environment_name: None,
            pending_new_environment_command_id: None,
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

    fn new_for_sheet(
        beam_view: Entity<BeamView>,
        selected_option: Option<(Ulid, String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (options, environment_file_names, selected_id) =
            if let Some((environment_id, label, file_name)) = selected_option {
                (
                    vec![(environment_id, label)],
                    HashMap::from([(environment_id, file_name)]),
                    Some(environment_id),
                )
            } else {
                (Vec::new(), HashMap::new(), None)
            };
        let mut view = Self::new(
            beam_view,
            options,
            environment_file_names,
            selected_id,
            selected_id,
            window,
            cx,
        );
        view.show_environment_selector = false;
        view
    }

    fn environment_file_path(&self, environment_id: Ulid) -> Option<PathBuf> {
        let file_name = self.environment_file_names.get(&environment_id)?;
        let trimmed = file_name.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(
            BeamPaths::default_user_config()
                .environments_dir
                .join(trimmed),
        )
    }

    fn load_variables(
        &mut self,
        environment_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.environment_file_path(environment_id) else {
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
        let environment_name = parsed.environment.name.clone();
        self.variables = parsed.variables;
        self.loaded_environment_name = Some(environment_name.clone());
        self.rebuild_variable_inputs(window, cx);
        self.suppress_environment_name_change_events = true;
        self.environment_name_input.update(cx, |input, cx| {
            input.set_value(environment_name.clone(), window, cx);
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

    fn next_default_environment_name(&self) -> String {
        let base_name = "New Environment";
        if !self
            .options
            .iter()
            .any(|(_, label)| label.eq_ignore_ascii_case(base_name))
        {
            return base_name.to_string();
        }
        let mut suffix = 2_u32;
        loop {
            let candidate = format!("{base_name} {suffix}");
            if !self
                .options
                .iter()
                .any(|(_, label)| label.eq_ignore_ascii_case(&candidate))
            {
                return candidate;
            }
            suffix += 1;
        }
    }

    fn add_environment(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let environment_name = self.next_default_environment_name();
        let command_id = next_command_id();
        let command = AppCommand::CreateEnvironment {
            name: environment_name,
            command_id: command_id.clone(),
        };
        let send_result = self
            .beam_view
            .update(cx, move |this, _| this.publish_app_command(command));
        match send_result {
            Ok(()) => {
                self.pending_new_environment_command_id = Some(command_id);
                self.error = None;
            }
            Err(error) => {
                self.pending_new_environment_command_id = None;
                self.error = Some(format!("Failed to queue environment creation: {error}"));
            }
        }
        cx.notify();
    }

    fn focus_environment_name_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.environment_name_input.update(cx, |input, cx| {
            input.focus(window, cx);
            let cursor_end = input.value().encode_utf16().count() as u32;
            input.set_cursor_position(Position::new(0, cursor_end), window, cx);
        });
    }

    fn delete_selected_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(environment_id) = self.selected_id else {
            self.error = Some("No environment selected.".to_string());
            cx.notify();
            return;
        };
        let command = AppCommand::DeleteEnvironment {
            environment_id,
            command_id: next_command_id(),
        };
        let send_result = self
            .beam_view
            .update(cx, move |this, _| this.publish_app_command(command));
        if let Err(error) = send_result {
            self.error = Some(format!("Failed to queue environment deletion: {error}"));
            cx.notify();
            return;
        }

        self.options
            .retain(|(option_environment_id, _)| *option_environment_id != environment_id);
        self.environment_file_names.remove(&environment_id);
        self.pending_variables_save_due_at = None;
        self.variables_save_tick_scheduled = false;
        self.variables_save_in_flight = false;
        self.selected_id = self.options.first().map(|(id, _)| *id);

        if let Some(next_environment_id) = self.selected_id {
            self.load_variables(next_environment_id, window, cx);
        } else {
            self.variables.clear();
            self.clear_variable_inputs();
            self.loaded_environment_name = None;
            self.suppress_environment_name_change_events = true;
            self.environment_name_input.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
            self.suppress_environment_name_change_events = false;
            self.error = Some("No environment available to manage.".to_string());
        }
        cx.notify();
    }

    fn environment_option_label(name: &str, _scope: EnvironmentScope) -> String {
        name.to_string()
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
        let variables: Vec<EnvironmentVariable> = self
            .variables
            .iter()
            .filter(|variable| !variable.name.trim().is_empty())
            .cloned()
            .collect();
        if self
            .loaded_environment_name
            .as_deref()
            .is_some_and(|name| name != updated_name.as_str())
        {
            let rename_command = AppCommand::RenameEnvironment {
                environment_id,
                new_name: updated_name.clone(),
                command_id: next_command_id(),
            };
            let rename_result = self
                .beam_view
                .update(cx, move |this, _| this.publish_app_command(rename_command));
            if let Err(error) = rename_result {
                self.variables_save_in_flight = false;
                self.error = Some(format!("Failed to queue environment rename: {error}"));
                cx.notify();
                return;
            }
            self.loaded_environment_name = Some(updated_name.clone());
        }
        let command = AppCommand::UpdateEnvironmentVariables {
            environment_id,
            variables,
            command_id: next_command_id(),
        };
        let send_result = self
            .beam_view
            .update(cx, move |this, _| this.publish_app_command(command));
        self.variables_save_in_flight = false;
        self.error = send_result.err().map(|error| {
            if error.starts_with("Backpressure:") {
                self.pending_variables_save_due_at =
                    Some(Instant::now() + Duration::from_millis(100));
            }
            format!("Failed to queue environment save: {error}")
        });
        if self.pending_variables_save_due_at.is_some() && !self.variables_save_tick_scheduled {
            self.variables_save_tick_scheduled = true;
            self.schedule_variables_save_tick(cx);
        }
        cx.notify();
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

    fn refresh_from_snapshot(
        &mut self,
        options: Vec<(Ulid, String)>,
        environment_file_names: HashMap<Ulid, String>,
        active_environment_id: Option<Ulid>,
        latest_upsert: Option<(Ulid, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_environment_changed = self.active_environment_id != active_environment_id;
        self.active_environment_id = active_environment_id;
        let mut should_notify =
            self.sync_environment_options(options, environment_file_names, window, cx);
        if let Some((environment_id, command_id)) = latest_upsert {
            if self.pending_new_environment_command_id.as_deref() == Some(command_id.as_str())
                && self
                    .options
                    .iter()
                    .any(|(option_id, _)| *option_id == environment_id)
            {
                self.pending_new_environment_command_id = None;
                self.selected_id = Some(environment_id);
                self.load_variables(environment_id, window, cx);
                self.focus_environment_name_input(window, cx);
                should_notify = true;
            }
        }
        if should_notify {
            cx.notify();
        } else if active_environment_changed {
            cx.notify();
        }
    }
}

impl Render for EnvironmentManagerDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let environment_name_has_selection = !self
            .environment_name_input
            .read(cx)
            .selected_range()
            .is_empty();
        let has_selected_environment = self.selected_id.is_some();
        let selected_label = self.selected_id.and_then(|id| {
            self.options
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
                        Button::new("delete-selected-environment")
                            .small()
                            .ghost()
                            .label("Delete")
                            .disabled(self.selected_id.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.delete_selected_environment(window, cx);
                            })),
                    ),
            );
        variables_panel = variables_panel.child(
            h_flex().w_full().items_center().gap_2().child(
                Input::new(&self.environment_name_input)
                    .w_full()
                    .context_menu({
                        move |menu, _, cx| {
                            BeamView::build_text_edit_context_menu(
                                menu,
                                environment_name_has_selection,
                                cx.theme().muted_foreground,
                            )
                        }
                    }),
            ),
        );
        if let Some(error) = &self.error {
            variables_panel = variables_panel.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger_foreground)
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
                .text_color(cx.theme().muted_foreground)
                .child(div().w(px(28.0)).child("On"))
                .child(div().w(px(180.0)).child("Key"))
                .child(div().flex_1().child("Value"))
                .child(div().w(px(28.0))),
        );
        variables_rows = variables_rows.child(if self.variables.is_empty() {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
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
                let key_has_selection = !key_input.read(cx).selected_range().is_empty();
                let value_has_selection = !value_input.read(cx).selected_range().is_empty();
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(cx.theme().border)
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
                        div().w(px(180.0)).child(
                            Input::new(&key_input)
                                .small()
                                .w_full()
                                .appearance(false)
                                .context_menu({
                                    move |menu, _, cx| {
                                        BeamView::build_text_edit_context_menu(
                                            menu,
                                            key_has_selection,
                                            cx.theme().muted_foreground,
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
                                    move |menu, _, cx| {
                                        BeamView::build_text_edit_context_menu(
                                            menu,
                                            value_has_selection,
                                            cx.theme().muted_foreground,
                                        )
                                    }
                                }),
                        ),
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
                                        .text_color(cx.theme().muted_foreground),
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

        if !self.show_environment_selector {
            if !has_selected_environment {
                return v_flex()
                    .w_full()
                    .h_full()
                    .p_4()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(px(360.0))
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_base()
                                    .font_semibold()
                                    .child("No environment selected"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_center()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        "Create an environment or select one to manage variables.",
                                    ),
                            ),
                    )
                    .into_any_element();
            }
            return v_flex()
                .w_full()
                .h_full()
                .p_2()
                .child(
                    div()
                        .w_full()
                        .h_full()
                        .overflow_y_scrollbar()
                        .child(variables_panel),
                )
                .into_any_element();
        }

        v_flex()
            .w_full()
            .h(px(520.0))
            .p_3()
            .gap_3()
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
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .overflow_hidden()
                            .p_3()
                            .gap_2()
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(div().text_xs().font_semibold().child("Environments")),
                            )
                            .child(v_flex().w_full().flex_1().min_h_0().child(
                                div().w_full().h_full().overflow_y_scrollbar().child(
                                    v_flex().w_full().gap_1().children(
                                        self.options.clone().into_iter().map(
                                            |(environment_id, label)| {
                                                let is_current = Some(environment_id)
                                                    == self.active_environment_id;
                                                let mut row_content = h_flex()
                                                    .w_full()
                                                    .items_center()
                                                    .justify_between()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .min_w_0()
                                                            .flex_1()
                                                            .text_sm()
                                                            .line_height(relative(1.0))
                                                            .truncate()
                                                            .child(label),
                                                    );
                                                if is_current {
                                                    row_content = row_content.child(
                                                        Tag::success()
                                                            .small()
                                                            .outline()
                                                            .rounded_full()
                                                            .child("Current"),
                                                    );
                                                }
                                                ListItem::new(format!(
                                                    "environment-manager-select-{environment_id}"
                                                ))
                                                .w_full()
                                                .cursor_pointer()
                                                .rounded(px(8.0))
                                                .px_3()
                                                .py_2()
                                                .selected(Some(environment_id) == self.selected_id)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.selected_id = Some(environment_id);
                                                        this.load_variables(
                                                            environment_id,
                                                            window,
                                                            cx,
                                                        );
                                                        cx.notify();
                                                    },
                                                ))
                                                .child(row_content)
                                            },
                                        ),
                                    ),
                                ),
                            ))
                            .child(
                                Button::new("environment-manager-add-environment")
                                    .small()
                                    .w_full()
                                    .label("Add environment")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_environment(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .p_2()
                            .child(if has_selected_environment {
                                div()
                                    .w_full()
                                    .h_full()
                                    .overflow_y_scrollbar()
                                    .child(variables_panel)
                                    .into_any_element()
                            } else {
                                v_flex()
                                    .w_full()
                                    .h_full()
                                    .items_center()
                                    .justify_center()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Select an environment from the left pane.")
                                    .into_any_element()
                            }),
                    ),
            )
            .into_any_element()
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
}

impl Render for TreeRenameDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target_view = self.target_view.clone();
        let node_id = self.node_id;
        let node_kind = self.node_kind;
        let name_input = self.name_input.clone();

        v_flex()
            .w(px(420.0))
            .p_3()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .px_1()
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
                            .cursor_pointer()
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

enum WorkspaceDialogMode {
    Create,
    Rename,
}

struct WorkspaceNameDialogView {
    target_view: Entity<BeamView>,
    mode: WorkspaceDialogMode,
    name_input: Entity<InputState>,
}

impl WorkspaceNameDialogView {
    fn new(
        target_view: Entity<BeamView>,
        mode: WorkspaceDialogMode,
        initial_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Workspace name")
                .default_value(initial_name)
        });
        Self {
            target_view,
            mode,
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

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            window.push_notification("Workspace name cannot be empty.", cx);
            return;
        }
        let is_create = matches!(self.mode, WorkspaceDialogMode::Create);
        let _ = self.target_view.update(cx, |this, cx| {
            if is_create {
                this.app_command_tx
                    .send(AppCommand::CreateWorkspace {
                        name,
                        command_id: next_command_id(),
                    })
                    .ok();
            } else if let Some(workspace_id) = this.shell.workspace.workspace_id {
                this.app_command_tx
                    .send(AppCommand::RenameWorkspace {
                        workspace_id,
                        new_name: name,
                        command_id: next_command_id(),
                    })
                    .ok();
            }
            window.close_dialog(cx);
        });
    }
}

impl Render for WorkspaceNameDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target_view = self.target_view.clone();
        let name_input = self.name_input.clone();
        let is_create = matches!(self.mode, WorkspaceDialogMode::Create);

        v_flex()
            .w(px(420.0))
            .p_3()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .px_1()
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
                        Button::new("workspace-dialog-cancel")
                            .small()
                            .ghost()
                            .cursor_pointer()
                            .label("Cancel")
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("workspace-dialog-submit")
                            .small()
                            .cursor_pointer()
                            .label(if is_create { "Create" } else { "Rename" })
                            .on_click(move |_, window, cx| {
                                let name = name_input.read(cx).value().trim().to_string();
                                if name.is_empty() {
                                    window.push_notification("Workspace name cannot be empty.", cx);
                                    return;
                                }
                                let _ = target_view.update(cx, |this, cx| {
                                    if is_create {
                                        this.app_command_tx
                                            .send(AppCommand::CreateWorkspace {
                                                name,
                                                command_id: next_command_id(),
                                            })
                                            .ok();
                                    } else if let Some(workspace_id) =
                                        this.shell.workspace.workspace_id
                                    {
                                        this.app_command_tx
                                            .send(AppCommand::RenameWorkspace {
                                                workspace_id,
                                                new_name: name,
                                                command_id: next_command_id(),
                                            })
                                            .ok();
                                    }
                                    window.close_dialog(cx);
                                });
                            }),
                    ),
            )
    }
}

struct WorkspaceDeleteDialogView {
    target_view: Entity<BeamView>,
    workspace_id: Ulid,
    workspace_name: String,
}

impl WorkspaceDeleteDialogView {
    fn new(
        target_view: Entity<BeamView>,
        workspace_id: Ulid,
        workspace_name: String,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            target_view,
            workspace_id,
            workspace_name,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace_id = self.workspace_id;
        let _ = self.target_view.update(cx, |this, cx| {
            if let Err(error) = this.publish_app_command(AppCommand::DeleteWorkspace {
                workspace_id,
                command_id: next_command_id(),
            }) {
                window.push_notification(error, cx);
                return;
            }
            window.close_dialog(cx);
        });
    }
}

impl Render for WorkspaceDeleteDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace_name = self.workspace_name.clone();

        v_flex()
            .w(px(460.0))
            .p_3()
            .gap_3()
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .child(format!("Delete workspace \"{workspace_name}\"?")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child("This deletes the workspace files from disk."),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("delete-workspace-cancel")
                            .small()
                            .ghost()
                            .cursor_pointer()
                            .label("Cancel")
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("delete-workspace-submit")
                            .small()
                            .danger()
                            .cursor_pointer()
                            .label("Delete")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit(window, cx);
                            })),
                    ),
            )
    }
}

impl BeamView {
    fn begin_request_run_for(&mut self, request_id: Ulid) -> u64 {
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

    fn cancel_request_run_for(&mut self, request_id: Ulid) {
        let Some(state) = self.request_execution_states.get_mut(&request_id) else {
            return;
        };
        if let Some(cancel_tx) = state.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }
        state.status = RequestExecutionStatus::Canceled;
    }

    fn clear_request_execution_state(&mut self, request_id: Ulid) {
        let Some(mut state) = self.request_execution_states.remove(&request_id) else {
            return;
        };
        if let Some(cancel_tx) = state.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }
    }

    fn prune_request_execution_states(&mut self) {
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

    fn is_request_sending(&self, request_id: Ulid) -> bool {
        self.request_execution_states
            .get(&request_id)
            .is_some_and(|state| state.status == RequestExecutionStatus::Sending)
    }

    fn cancel_active_request_wait(&mut self) {
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            return;
        };

        self.cancel_request_run_for(request_id);
        self.response_status = "Canceled".to_string();
        self.response_time = "—".to_string();
        self.response_size = "—".to_string();
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

    fn send_button_state_for_view(&self) -> SendButtonState {
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

    fn handle_send_or_cancel_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.send_button_state_for_view() {
            SendButtonState::Sending => self.cancel_active_request_wait(),
            SendButtonState::Disabled(SendDisabledReason::EmptyUrl) => {}
            _ => self.send_request(window, cx),
        }
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

    fn format_human_time(timestamp: &str) -> String {
        chrono::DateTime::parse_from_rfc3339(timestamp)
            .map(|parsed| parsed.with_timezone(&Local).format("%H:%M:%S").to_string())
            .unwrap_or_else(|_| timestamp.to_string())
    }

    fn active_environment_options(&self) -> Vec<(Ulid, String)> {
        self.shell
            .environments
            .iter()
            .map(|environment| (environment.environment_id, environment.name.clone()))
            .collect()
    }

    fn selected_environment_id_for_view(&self) -> Option<Ulid> {
        self.shell.effective_environment_id_for_selected_request()
    }

    fn selected_environment_label(&self) -> String {
        let Some(selected_id) = self.selected_environment_id_for_view() else {
            return "No environment".to_string();
        };
        let Some((_, label)) = self
            .active_environment_options()
            .into_iter()
            .find(|(environment_id, _)| *environment_id == selected_id)
        else {
            return "No environment".to_string();
        };
        label
    }

    fn set_selected_environment_for_view(&mut self, environment_id: Ulid) {
        self.shell
            .environment_selection
            .active_global_environment_id = Some(environment_id);
    }

    fn clear_selected_environment_for_view(&mut self) {
        self.shell
            .environment_selection
            .active_global_environment_id = None;
    }

    fn open_environment_variables_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let beam_view = cx.entity();
        let selected_option = self
            .selected_environment_id_for_view()
            .and_then(|selected_id| {
                self.shell
                    .environments
                    .iter()
                    .find(|environment| environment.environment_id == selected_id)
                    .map(|environment| {
                        (
                            environment.environment_id,
                            environment.name.clone(),
                            environment.file_name.clone(),
                        )
                    })
            });
        let sheet_view = cx.new(|cx| {
            EnvironmentManagerDialogView::new_for_sheet(
                beam_view.clone(),
                selected_option.clone(),
                window,
                cx,
            )
        });

        window.open_sheet_at(Placement::Right, cx, move |sheet, _, _| {
            sheet
                .title("Environment Variables")
                .size(px(520.0))
                .child(sheet_view.clone())
        });
    }

    fn apply_theme_mode(mode: ThemeMode, cx: &mut App) {
        Theme::change(mode, None, cx);
        #[cfg(target_os = "macos")]
        cx.set_menus(build_macos_system_menus(cx));
        cx.refresh_windows();
        if let Err(error) = Self::persist_theme_state_from_app(cx) {
            log::error!("{error}");
        }
    }

    fn apply_named_theme(theme_name: SharedString, cx: &mut App) {
        if Self::apply_named_theme_by_name(theme_name.as_ref(), cx, true) {
            return;
        }
    }

    fn apply_named_theme_by_name(theme_name: &str, cx: &mut App, persist: bool) -> bool {
        let stored_theme_name: SharedString = theme_name.to_string().into();
        let theme_config = ThemeRegistry::global(cx)
            .themes()
            .get(&stored_theme_name)
            .cloned();
        if let Some(theme_config) = theme_config {
            Theme::global_mut(cx).apply_config(&theme_config);
            #[cfg(target_os = "macos")]
            cx.set_menus(build_macos_system_menus(cx));
            cx.refresh_windows();
            if persist {
                if let Err(error) = Self::persist_theme_state_from_app(cx) {
                    log::error!("{error}");
                }
            }
            return true;
        }
        false
    }

    fn open_settings_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let beam_view = cx.entity();
        let settings_view = cx.new(|cx| SettingsDialogView::new(beam_view.clone(), _window, cx));
        self.settings_dialog_view = Some(settings_view.clone());
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    window.defer(cx, move |window, cx| {
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog
                                .title("Settings")
                                .w(px(920.0))
                                .max_w(px(1200.0))
                                .child(settings_view.clone())
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    fn environment_manager_options(&self) -> Vec<(Ulid, String)> {
        self.shell
            .environments
            .iter()
            .map(|environment| (environment.environment_id, environment.name.clone()))
            .collect()
    }

    fn environment_manager_file_names(&self) -> HashMap<Ulid, String> {
        self.shell
            .environments
            .iter()
            .map(|environment| (environment.environment_id, environment.file_name.clone()))
            .collect()
    }

    fn open_environment_manager(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let beam_view = cx.entity();
        let options = self.environment_manager_options();
        let environment_file_names = self.environment_manager_file_names();
        let fallback_id = options.first().map(|(environment_id, _)| *environment_id);
        let selected = self.selected_environment_id_for_view().or(fallback_id);
        let active_environment_id = self.selected_environment_id_for_view();
        let manager_view = cx.new(|cx| {
            EnvironmentManagerDialogView::new(
                beam_view.clone(),
                options.clone(),
                environment_file_names.clone(),
                selected,
                active_environment_id,
                window,
                cx,
            )
        });
        self.environment_manager_dialog_view = Some(manager_view.clone());
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

    fn refresh_environment_manager_dialog_if_open(
        &mut self,
        latest_upsert: Option<(Ulid, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog_view) = self.environment_manager_dialog_view.clone() else {
            return;
        };
        let options = self.environment_manager_options();
        let environment_file_names = self.environment_manager_file_names();
        let active_environment_id = self.selected_environment_id_for_view();
        dialog_view.update(cx, |dialog, cx| {
            dialog.refresh_from_snapshot(
                options,
                environment_file_names,
                active_environment_id,
                latest_upsert,
                window,
                cx,
            );
        });
    }

    fn invalidate_env_var_resolved_cache(&mut self) {
        self.env_var_resolved_cache = None;
    }

    fn environment_file_path_from_shell(&self, environment_id: Ulid) -> Option<PathBuf> {
        let environment = self
            .shell
            .environments
            .iter()
            .find(|environment| environment.environment_id == environment_id)?;
        let file_name = environment.file_name.trim();
        if file_name.is_empty() {
            return None;
        }
        let paths = BeamPaths::default_user_config();
        Some(paths.environments_dir.join(file_name))
    }

    fn hydrate_request_from_selection(request: &mut RequestAuthoringState, shell: &AppShellState) {
        let Some(request_id) = shell.workspace_tree.selected_request_id() else {
            return;
        };

        let Some(pane_data) = shell.request_pane_data.get(&request_id) else {
            let selected_node = shell.workspace_tree.node(request_id).cloned();
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
        request.ensure_trailing_empty_row();
    }

    fn sync_request_editor_from_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_active_request_cache();
        self.show_invalid_url_border = false;
        let active_tab = self.request.active_tab;
        self.request = RequestAuthoringState {
            active_tab,
            ..RequestAuthoringState::default()
        };
        Self::hydrate_request_from_selection(&mut self.request, &self.shell);
        self.request.ensure_trailing_empty_row();
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

    fn refresh_active_request_cache(&mut self) {
        let selected_request_id = self.shell.workspace_tree.selected_request_id();
        let cached_request_id = self
            .active_request_cache
            .as_ref()
            .map(|request_file| request_file.meta.request_id);
        if cached_request_id == selected_request_id {
            return;
        }

        self.active_request_cache = None;
        let Some(request_id) = selected_request_id else {
            return;
        };

        self.active_request_cache = self.shell.shared_store.requests.get(&request_id).cloned();
    }

    fn clear_response_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.response_status = "—".to_string();
        self.response_time = "—".to_string();
        self.response_size = "—".to_string();
        self.response_headers_raw.clear();
        self.response_content_type = None;
        self.response_body_editor.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
    }

    fn apply_response_snapshot(
        &mut self,
        snapshot: &StoredResponseSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content_type = snapshot.content_type.clone();
        let formatted_body =
            Self::auto_format_response_body(&snapshot.body, content_type.as_deref());
        self.response_status = snapshot.status.clone();
        self.response_time = snapshot.time.clone();
        self.response_size = snapshot.size.clone();
        self.response_headers_raw = snapshot.headers_raw.clone();
        self.response_content_type = content_type;
        self.response_body_editor.update(cx, |input, cx| {
            input.set_value(formatted_body.clone(), window, cx);
        });
    }

    fn sync_response_pane_from_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            self.response_history_entries.clear();
            self.clear_response_pane(window, cx);
            self.script_result = None;
            return;
        };

        self.response_history_entries = Self::load_response_history_entries(request_id);
        if let Some(snapshot) = self
            .response_history_entries
            .first()
            .map(Self::load_response_snapshot_for_history_entry)
        {
            self.apply_response_snapshot(&snapshot, window, cx);
        } else {
            self.clear_response_pane(window, cx);
        }
        self.script_result = Self::load_script_result(request_id);

        let (status, time, size) = response_summary_for_selected_request(
            Some(request_id),
            &self.request_execution_states,
            &self.response_status,
            &self.response_time,
            &self.response_size,
        );
        self.response_status = status;
        self.response_time = time;
        self.response_size = size;
    }

    fn load_request_history_file(request_id: Ulid) -> Option<RequestHistoryFile> {
        let paths = BeamPaths::default_user_config();
        let history_file_path = paths
            .local_dir
            .join("history/by-request")
            .join(format!("{request_id}.history.toml"));
        let content = fs::read_to_string(history_file_path).ok()?;
        let history_file: RequestHistoryFile = toml::from_str(&content).ok()?;

        Some(history_file)
    }

    fn response_snapshot_from_history_execution(
        paths: &BeamPaths,
        execution: &RequestHistoryExecution,
    ) -> StoredResponseSnapshot {
        let (status, time, size) = Self::response_history_summary_parts(execution);
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
            time,
            size,
            body,
            headers_raw,
            content_type,
        }
    }

    fn response_history_summary_parts(
        execution: &RequestHistoryExecution,
    ) -> (String, String, String) {
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

        (status, time, size)
    }

    fn load_response_snapshot_for_history_entry(
        entry: &ResponseHistoryEntry,
    ) -> StoredResponseSnapshot {
        let paths = BeamPaths::default_user_config();
        Self::response_snapshot_from_history_execution(&paths, &entry.execution)
    }

    fn load_response_history_entries(request_id: Ulid) -> Vec<ResponseHistoryEntry> {
        let Some(history_file) = Self::load_request_history_file(request_id) else {
            return Vec::new();
        };
        let total = history_file.executions.len();

        history_file
            .executions
            .iter()
            .enumerate()
            .rev()
            .map(|(index, execution)| {
                let (status, time, size) = Self::response_history_summary_parts(execution);
                let title = if index + 1 == total {
                    "Latest response".to_string()
                } else {
                    format!("Response #{}", index + 1)
                };
                let summary = format!("{status} | {time} | {size}");

                ResponseHistoryEntry {
                    title,
                    summary,
                    execution: execution.clone(),
                }
            })
            .collect()
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
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
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

    fn build_request_snapshot_for_save(
        &mut self,
        request_id: Ulid,
        pane_data: RequestPaneData,
    ) -> Result<RequestFile, String> {
        let mut request_file = self
            .active_request_cache
            .clone()
            .ok_or_else(|| "No active request cache available for save.".to_string())?;
        if request_file.meta.request_id != request_id {
            return Err(format!(
                "Active request cache mismatch: expected {request_id}, found {}.",
                request_file.meta.request_id
            ));
        }
        request_file.request.method = pane_data.method;
        request_file.request.url = pane_data.url;
        request_file.request.headers = pane_data.headers;
        request_file.request.query_params = pane_data.query_params;
        request_file.auth = pane_data.auth;
        request_file.body = pane_data.body;
        request_file.scripts.post_response = pane_data.post_script;
        request_file.meta.updated_at = Utc::now();
        self.active_request_cache = Some(request_file.clone());
        Ok(request_file)
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
        self.refresh_active_request_cache();
        match self.build_request_snapshot_for_save(request_id, pane_data) {
            Ok(request_file) => {
                let command = AppCommand::SaveRequest {
                    request_file,
                    command_id: next_command_id(),
                };
                if let Err(error) = self.publish_app_command(command) {
                    log::error!("{error}");
                    if error.starts_with("Backpressure:") {
                        self.pending_request_save_due_at =
                            Some(Instant::now() + Duration::from_millis(100));
                    }
                }
            }
            Err(error) => log::error!("{error}"),
        }
        self.request_save_in_flight = false;
        if self.pending_request_save_due_at.is_some() && !self.request_save_tick_scheduled {
            self.request_save_tick_scheduled = true;
            self.schedule_request_save_tick(cx);
        }
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
        if self.request.query_params.len() == 1 {
            if let Some(param) = self.request.query_params.get_mut(index) {
                param.name.clear();
                param.value.clear();
            }
        } else {
            self.request.query_params.remove(index);
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
        if self.request.headers.len() == 1 {
            if let Some(header) = self.request.headers.get_mut(index) {
                header.name.clear();
                header.value.clear();
            }
        } else {
            self.request.headers.remove(index);
        }
        self.rebuild_request_header_inputs(window, cx);
        self.schedule_request_save(cx);
        cx.notify();
    }

    fn parent_ref_for_add_request(&self, node_id: Ulid) -> Option<RequestParentRef> {
        self.request_parent_input_for_tree_node(node_id)
            .map(|(parent, _)| parent)
    }

    fn parent_ref_for_add_folder(&self, node_id: Ulid) -> Option<FolderParentRef> {
        self.folder_parent_input_for_tree_node(node_id)
            .map(|(parent, _)| parent)
    }

    fn request_parent_input_for_tree_node(
        &self,
        node_id: Ulid,
    ) -> Option<(RequestParentRef, Option<KnownParentManifestPath>)> {
        let node = self.shell.workspace_tree.node(node_id)?;
        match node.kind {
            TreeNodeKind::Folder => Some((
                RequestParentRef {
                    folder_id: Some(node.id),
                },
                node.manifest_path.clone().map(KnownParentManifestPath),
            )),
            TreeNodeKind::Request => {
                let parent_id = node.parent_id;
                match parent_id {
                    None => Some((RequestParentRef { folder_id: None }, None)),
                    Some(parent_id) => {
                        let parent_node = self.shell.workspace_tree.node(parent_id)?;
                        match parent_node.kind {
                            TreeNodeKind::Folder => Some((
                                RequestParentRef {
                                    folder_id: Some(parent_node.id),
                                },
                                parent_node
                                    .manifest_path
                                    .clone()
                                    .map(KnownParentManifestPath),
                            )),
                            TreeNodeKind::Request => None,
                        }
                    }
                }
            }
        }
    }

    fn folder_parent_input_for_tree_node(
        &self,
        node_id: Ulid,
    ) -> Option<(FolderParentRef, Option<KnownParentManifestPath>)> {
        let node = self.shell.workspace_tree.node(node_id)?;
        match node.kind {
            TreeNodeKind::Folder => Some((
                FolderParentRef {
                    folder_id: Some(node.id),
                },
                node.manifest_path.clone().map(KnownParentManifestPath),
            )),
            TreeNodeKind::Request => {
                let parent_id = node.parent_id;
                match parent_id {
                    None => Some((FolderParentRef { folder_id: None }, None)),
                    Some(parent_id) => {
                        let parent_node = self.shell.workspace_tree.node(parent_id)?;
                        match parent_node.kind {
                            TreeNodeKind::Folder => Some((
                                FolderParentRef {
                                    folder_id: Some(parent_node.id),
                                },
                                parent_node
                                    .manifest_path
                                    .clone()
                                    .map(KnownParentManifestPath),
                            )),
                            TreeNodeKind::Request => None,
                        }
                    }
                }
            }
        }
    }

    fn request_sibling_names_in_parent(&self, parent: RequestParentRef) -> Vec<String> {
        if let Some(folder_id) = parent.folder_id {
            let Some(parent_node) = self.shell.workspace_tree.node(folder_id) else {
                return Vec::new();
            };
            parent_node
                .children
                .iter()
                .filter_map(|child_id| self.shell.workspace_tree.node(*child_id))
                .filter(|child| child.kind == TreeNodeKind::Request)
                .map(|child| child.name.clone())
                .collect()
        } else {
            self.shell
                .workspace_tree
                .visible_rows()
                .into_iter()
                .filter(|row| row.depth == 0 && row.kind == TreeNodeKind::Request)
                .filter_map(|row| self.shell.workspace_tree.node(row.id))
                .map(|n| n.name.clone())
                .collect()
        }
    }

    fn folder_sibling_names_in_parent(&self, parent: FolderParentRef) -> Vec<String> {
        if let Some(folder_id) = parent.folder_id {
            let Some(parent_node) = self.shell.workspace_tree.node(folder_id) else {
                return Vec::new();
            };
            parent_node
                .children
                .iter()
                .filter_map(|child_id| self.shell.workspace_tree.node(*child_id))
                .filter(|child| child.kind == TreeNodeKind::Folder)
                .map(|child| child.name.clone())
                .collect()
        } else {
            self.shell
                .workspace_tree
                .visible_rows()
                .into_iter()
                .filter(|row| row.depth == 0 && row.kind == TreeNodeKind::Folder)
                .filter_map(|row| self.shell.workspace_tree.node(row.id))
                .map(|n| n.name.clone())
                .collect()
        }
    }

    fn request_file_path_for_tree_node(&self, node_id: Ulid) -> Option<PathBuf> {
        self.shell
            .workspace_tree
            .node(node_id)
            .filter(|node| node.kind == TreeNodeKind::Request)
            .and_then(|node| node.manifest_path.clone())
    }

    fn tree_drag_preview_for_request(
        dragged: &DraggedRequest,
        position: Point<Pixels>,
        _: &mut Window,
        cx: &mut App,
    ) -> Entity<TreeDragPreview> {
        cx.new(|_| TreeDragPreview::new(dragged.label.clone(), TreeNodeKind::Request, position))
    }

    fn tree_drag_preview_for_folder(
        dragged: &DraggedFolder,
        position: Point<Pixels>,
        _: &mut Window,
        cx: &mut App,
    ) -> Entity<TreeDragPreview> {
        cx.new(|_| TreeDragPreview::new(dragged.label.clone(), TreeNodeKind::Folder, position))
    }

    fn path_has_ancestor_in_tree(&self, start_id: Ulid, ancestor_id: Ulid) -> bool {
        let mut cursor = Some(start_id);
        while let Some(node_id) = cursor {
            if node_id == ancestor_id {
                return true;
            }
            cursor = self
                .shell
                .workspace_tree
                .node(node_id)
                .and_then(|node| node.parent_id);
        }
        false
    }

    fn has_name_conflict_in_scope(
        &self,
        parent_id: Option<Ulid>,
        moving_id: Ulid,
        name: &str,
    ) -> bool {
        let sibling_ids: Vec<Ulid> = if let Some(pid) = parent_id {
            self.shell
                .workspace_tree
                .node(pid)
                .map(|p| p.children.clone())
                .unwrap_or_default()
        } else {
            self.shell.workspace_tree.roots().to_vec()
        };
        sibling_ids
            .into_iter()
            .filter(|id| *id != moving_id)
            .filter_map(|id| self.shell.workspace_tree.node(id))
            .any(|child| child.name.eq_ignore_ascii_case(name))
    }

    fn request_parent_input_for_parent_node(
        &self,
        parent_id: Option<Ulid>,
    ) -> Option<(RequestParentRef, Option<KnownParentManifestPath>)> {
        let Some(pid) = parent_id else {
            return Some((RequestParentRef { folder_id: None }, None));
        };
        let parent = self.shell.workspace_tree.node(pid)?;
        match parent.kind {
            TreeNodeKind::Folder => Some((
                RequestParentRef {
                    folder_id: Some(parent.id),
                },
                parent.manifest_path.clone().map(KnownParentManifestPath),
            )),
            TreeNodeKind::Request => None,
        }
    }

    fn folder_parent_input_for_parent_node(
        &self,
        parent_id: Option<Ulid>,
    ) -> Option<(FolderParentRef, Option<KnownParentManifestPath>)> {
        let Some(pid) = parent_id else {
            return Some((FolderParentRef { folder_id: None }, None));
        };
        let parent = self.shell.workspace_tree.node(pid)?;
        match parent.kind {
            TreeNodeKind::Folder => Some((
                FolderParentRef {
                    folder_id: Some(parent.id),
                },
                parent.manifest_path.clone().map(KnownParentManifestPath),
            )),
            TreeNodeKind::Request => None,
        }
    }

    fn sibling_destination_for_target(
        &self,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> Option<(Option<Ulid>, usize)> {
        let target = self.shell.workspace_tree.node(target_id)?;
        let parent_id = target.parent_id;
        let siblings: Vec<Ulid> = if let Some(pid) = parent_id {
            self.shell.workspace_tree.node(pid)?.children.clone()
        } else {
            self.shell.workspace_tree.roots().to_vec()
        };
        let target_index = siblings.iter().position(|id| *id == target_id)?;
        let insertion_index = match placement {
            TreeDropPlacement::Before => target_index,
            TreeDropPlacement::After => target_index + 1,
            TreeDropPlacement::Into => return None,
        };
        Some((parent_id, insertion_index))
    }

    fn request_move_action(
        &self,
        request_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> Option<TreeMoveAction> {
        let request_node = self.shell.workspace_tree.node(request_id)?;
        if request_node.kind != TreeNodeKind::Request {
            return None;
        }

        let (destination_parent_id, insertion_index): (Option<Ulid>, usize) = match placement {
            TreeDropPlacement::Into => {
                let target = self.shell.workspace_tree.node(target_id)?;
                if target.kind != TreeNodeKind::Folder {
                    return None;
                }
                (Some(target.id), target.children.len())
            }
            TreeDropPlacement::Before | TreeDropPlacement::After => {
                if target_id == request_id {
                    return None;
                }
                self.sibling_destination_for_target(target_id, placement)?
            }
        };
        if self.has_name_conflict_in_scope(destination_parent_id, request_id, &request_node.name) {
            return None;
        }

        let (new_parent, known_target_manifest_path) =
            self.request_parent_input_for_parent_node(destination_parent_id)?;
        Some(TreeMoveAction::MoveRequest(MoveRequestInput {
            request_id,
            new_parent,
            insertion_index,
            known_request_path: request_node.manifest_path.clone(),
            known_target_manifest_path,
        }))
    }

    fn folder_move_action(
        &self,
        folder_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> Option<TreeMoveAction> {
        let folder_node = self.shell.workspace_tree.node(folder_id)?;
        if folder_node.kind != TreeNodeKind::Folder {
            return None;
        }

        let (destination_parent_id, insertion_index): (Option<Ulid>, usize) = match placement {
            TreeDropPlacement::Into => {
                if target_id == folder_id {
                    return None;
                }
                let target = self.shell.workspace_tree.node(target_id)?;
                if target.kind != TreeNodeKind::Folder {
                    return None;
                }
                (Some(target.id), target.children.len())
            }
            TreeDropPlacement::Before | TreeDropPlacement::After => {
                if target_id == folder_id {
                    return None;
                }
                self.sibling_destination_for_target(target_id, placement)?
            }
        };
        if destination_parent_id == Some(folder_id)
            || destination_parent_id.is_some_and(|id| self.path_has_ancestor_in_tree(id, folder_id))
        {
            return None;
        }
        if self.has_name_conflict_in_scope(destination_parent_id, folder_id, &folder_node.name) {
            return None;
        }

        let (new_parent, known_target_manifest_path) =
            self.folder_parent_input_for_parent_node(destination_parent_id)?;
        Some(TreeMoveAction::MoveFolder(MoveFolderInput {
            folder_id,
            new_parent,
            insertion_index,
            known_folder_manifest_path: folder_node.manifest_path.clone(),
            known_target_manifest_path,
        }))
    }

    fn can_accept_tree_drop(
        &self,
        dragged_value: &dyn Any,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> bool {
        if let Some(dragged) = dragged_value.downcast_ref::<DraggedRequest>() {
            let accepted = self
                .request_move_action(dragged.request_id, target_id, placement)
                .is_some();
            return accepted;
        }
        if let Some(dragged) = dragged_value.downcast_ref::<DraggedFolder>() {
            return self
                .folder_move_action(dragged.folder_id, target_id, placement)
                .is_some();
        }
        false
    }

    fn tree_row_body_drop_placement(target_kind: TreeNodeKind) -> Option<TreeDropPlacement> {
        match target_kind {
            TreeNodeKind::Folder => Some(TreeDropPlacement::Into),
            TreeNodeKind::Request => None,
        }
    }

    fn can_accept_tree_row_body_drop(
        &self,
        dragged_value: &dyn Any,
        target_id: Ulid,
        target_kind: TreeNodeKind,
    ) -> bool {
        Self::tree_row_body_drop_placement(target_kind)
            .is_some_and(|placement| self.can_accept_tree_drop(dragged_value, target_id, placement))
    }

    fn update_tree_drag_hover(
        &mut self,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        target_id: Ulid,
        target_kind: TreeNodeKind,
        dragged: &dyn Any,
        cx: &mut Context<Self>,
    ) {
        // Only process hover for the element actually under the mouse.
        if position.y < bounds.origin.y || position.y > bounds.origin.y + bounds.size.height {
            return;
        }

        let Some(placement) = Self::tree_row_body_drop_placement(target_kind) else {
            self.clear_tree_drag_hover(cx);
            return;
        };

        if !self.can_accept_tree_drop(dragged, target_id, placement) {
            self.clear_tree_drag_hover(cx);
            return;
        }

        let new_hover = Some((target_id, placement));
        if self.tree_drag_hover != new_hover {
            self.tree_drag_hover = new_hover;
            cx.notify();
        }
    }

    fn clear_tree_drag_hover(&mut self, cx: &mut Context<Self>) {
        if self.tree_drag_hover.is_some() {
            self.tree_drag_hover = None;
            cx.notify();
        }
    }

    fn perform_tree_move_action(
        &mut self,
        action: TreeMoveAction,
        preferred_selected_request_id: Option<Ulid>,
        expand_target_id: Option<Ulid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(target_id) = expand_target_id
            && !self.shell.workspace_tree.is_expanded(target_id)
        {
            self.shell.workspace_tree.toggle_expanded(target_id);
            if let Err(error) = self.persist_tree_expansion_state() {
                window.push_notification(error, cx);
            }
        }

        match action {
            TreeMoveAction::MoveRequest(input) => {
                let command = AppCommand::MoveRequest {
                    input,
                    command_id: next_command_id(),
                };
                if let Err(error) = self.publish_app_command(command) {
                    window.push_notification(error, cx);
                }
                return;
            }
            TreeMoveAction::MoveFolder(_) => {}
        }

        // Capture the info needed for the in-memory update before the action is moved.
        let (folder_id, new_parent_id, insertion_index, old_parent_id, folder_name) = match &action
        {
            TreeMoveAction::MoveFolder(input) => {
                let folder_id = input.folder_id;
                let new_parent_id = input.new_parent.folder_id;
                let insertion_index = input.insertion_index;
                let old_parent_id = self
                    .shell
                    .shared_store
                    .nodes
                    .get(&folder_id)
                    .and_then(|n| n.parent_id);
                let folder_name = self
                    .shell
                    .shared_store
                    .nodes
                    .get(&folder_id)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                (
                    folder_id,
                    new_parent_id,
                    insertion_index,
                    old_parent_id,
                    folder_name,
                )
            }
            TreeMoveAction::MoveRequest(_) => unreachable!(),
        };

        let old_folder_dir = crate::workspace_tree::folder_dir_path(
            &self.current_workspace_paths,
            &self.shell.shared_store,
            folder_id,
        )
        .ok();
        let paths = self.current_workspace_paths.clone();
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let result: std::result::Result<(), String> = cx
                .background_executor()
                .spawn(async move {
                    let backend = FileSystemStorage::new(paths);
                    let mut storage = WorkspaceRepository::new(backend)
                        .map_err(|error| format!("Failed to load workspace: {error}"))?;
                    match action {
                        TreeMoveAction::MoveRequest(_) => {
                            unreachable!()
                        }
                        TreeMoveAction::MoveFolder(input) => storage
                            .move_folder(input)
                            .map(|_| ())
                            .map_err(|error| format!("Failed to move folder: {error}")),
                    }
                })
                .await;
            let _ = view.update_in(cx, move |this, window, cx| match result {
                Ok(()) => {
                    this.shell.apply_folder_move(
                        folder_id,
                        old_parent_id,
                        new_parent_id,
                        insertion_index,
                        folder_name,
                    );
                    if let Some(old_folder_dir) = old_folder_dir.as_ref()
                        && let Ok(new_folder_dir) = crate::workspace_tree::folder_dir_path(
                            &this.current_workspace_paths,
                            &this.shell.shared_store,
                            folder_id,
                        )
                    {
                        this.shell.replace_moved_folder_subtree_paths(
                            folder_id,
                            old_folder_dir,
                            &new_folder_dir,
                        );
                        this.request_file_index = Self::build_request_file_index(&this.shell);
                        this.active_request_cache = None;
                        this.refresh_active_request_cache();
                    }
                    if let Some(request_id) = preferred_selected_request_id {
                        this.shell.workspace_tree.select_request(request_id);
                    }
                    cx.notify();
                }
                Err(error) => {
                    window.push_notification(error, cx);
                }
            });
        })
        .detach();
    }

    fn handle_request_tree_drop(
        &mut self,
        request_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.request_move_action(request_id, target_id, placement) else {
            return;
        };
        let expand_target_id = (placement == TreeDropPlacement::Into).then_some(target_id);
        self.perform_tree_move_action(action, Some(request_id), expand_target_id, window, cx);
    }

    fn handle_folder_tree_drop(
        &mut self,
        folder_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.folder_move_action(folder_id, target_id, placement) else {
            return;
        };
        let preferred_selected_request_id = self.shell.workspace_tree.selected_request_id();
        let expand_target_id = (placement == TreeDropPlacement::Into).then_some(target_id);
        self.perform_tree_move_action(
            action,
            preferred_selected_request_id,
            expand_target_id,
            window,
            cx,
        );
    }

    fn render_tree_drop_slot(
        &self,
        target_id: Ulid,
        target_kind: TreeNodeKind,
        placement: TreeDropPlacement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let base = div().h(px(2.0)).w_full().rounded(px(2.0)).can_drop(
            move |dragged_value, _window, app| {
                view.update(app, |this, _| {
                    this.can_accept_tree_drop(dragged_value, target_id, placement)
                })
            },
        );

        let slot = match target_kind {
            TreeNodeKind::Folder | TreeNodeKind::Request => base
                .drag_over::<DraggedRequest>(|style, _, _, cx| style.bg(cx.theme().drag_border))
                .drag_over::<DraggedFolder>(|style, _, _, cx| style.bg(cx.theme().drag_border))
                .on_drag_move(
                    cx.listener(move |this, _: &DragMoveEvent<DraggedRequest>, _, cx| {
                        this.clear_tree_drag_hover(cx);
                    }),
                )
                .on_drag_move(
                    cx.listener(move |this, _: &DragMoveEvent<DraggedFolder>, _, cx| {
                        this.clear_tree_drag_hover(cx);
                    }),
                )
                .on_drop(
                    cx.listener(move |this, dragged: &DraggedRequest, window, cx| {
                        this.handle_request_tree_drop(
                            dragged.request_id,
                            target_id,
                            placement,
                            window,
                            cx,
                        );
                        this.clear_tree_drag_hover(cx);
                    }),
                )
                .on_drop(
                    cx.listener(move |this, dragged: &DraggedFolder, window, cx| {
                        this.handle_folder_tree_drop(
                            dragged.folder_id,
                            target_id,
                            placement,
                            window,
                            cx,
                        );
                        this.clear_tree_drag_hover(cx);
                    }),
                ),
        };

        slot.into_any_element()
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
        let source = self.shell.workspace_tree.node(request_id)?;
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

    fn persist_last_opened_request_id(&self, request_id: Ulid) -> Result<(), String> {
        let backend = FileSystemStorage::new(self.current_workspace_paths.clone());
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
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
        let backend = FileSystemStorage::new(self.current_workspace_paths.clone());
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
        let mut local_state = match storage.load_local_state() {
            Ok(state) => state,
            Err(_) => LocalStateFile::default(),
        };

        let expanded_item_ids: Vec<Ulid> = self
            .shell
            .workspace_tree
            .expanded()
            .iter()
            .copied()
            .collect();
        if local_state.tree_state.expanded_item_ids == expanded_item_ids {
            return Ok(());
        }

        local_state.tree_state.expanded_item_ids = expanded_item_ids;
        local_state.local_state.updated_at = Utc::now();
        storage
            .save_local_state(&local_state)
            .map_err(|error| format!("Failed to save local state: {error}"))
    }

    fn persist_environment_selection_state(&self) -> Result<(), String> {
        let backend = FileSystemStorage::new(self.current_workspace_paths.clone());
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
        let mut local_state = match storage.load_local_state() {
            Ok(state) => state,
            Err(_) => LocalStateFile::default(),
        };

        let active_global_environment_id = self
            .shell
            .environment_selection
            .active_global_environment_id;
        if local_state.local_state.active_global_environment_id == active_global_environment_id {
            return Ok(());
        }

        local_state.local_state.active_global_environment_id = active_global_environment_id;
        local_state.local_state.updated_at = Utc::now();
        storage
            .save_local_state(&local_state)
            .map_err(|error| format!("Failed to save local state: {error}"))
    }

    fn persist_theme_state_from_app(cx: &App) -> Result<(), String> {
        let paths = BeamPaths::default_user_config();
        let backend = FileSystemStorage::new(paths);
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
        let active_theme_name = cx.theme().theme_name().to_string();
        storage
            .persist_theme_state(active_theme_name.as_str())
            .map_err(|error| format!("Failed to save local state: {error}"))
    }

    fn publish_app_command(&self, command: AppCommand) -> Result<(), String> {
        let operation = command.operation();
        self.app_command_tx
            .try_send(command)
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => format!(
                    "Backpressure: data sync queue is full for operation '{}'.",
                    operation.as_str()
                ),
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    "Failed to send command to data sync worker: worker disconnected.".to_string()
                }
            })
    }

    fn schedule_app_event_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.app_event_poll_scheduled {
            return;
        }
        self.app_event_poll_scheduled = true;
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor()
                .spawn(async move {
                    std::thread::sleep(Duration::from_millis(25));
                })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.app_event_poll_scheduled = false;
                this.process_app_events(window, cx);
            });
        })
        .detach();
    }

    fn apply_active_workspace_ui_state(
        &mut self,
        workspace_id: Option<Ulid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_env_var_resolved_cache();
        let data_root = DataRootPaths::default_user_config();
        if let Some(workspace_id) = workspace_id
            && let Some(entry) = self
                .shell
                .workspace
                .all_workspaces
                .iter()
                .find(|entry| entry.workspace_id == workspace_id)
        {
            self.current_workspace_paths = data_root.workspace_paths(&entry.path);
        }
        self.active_request_cache = None;
        self.request_file_index = Self::build_request_file_index(&self.shell);
        self.prune_request_execution_states();
        self.sync_request_editor_from_selection(window, cx);
    }

    fn process_app_events(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut did_apply_any = false;
        let mut should_sync_editor = false;
        let mut selected_request_to_persist = None;

        while let Ok(event) = self.app_event_rx.try_recv() {
            did_apply_any = true;
            match &event {
                AppEvent::RequestUpserted {
                    request,
                    command_id,
                } => {
                    if let Some(path) = request.file_path.clone() {
                        self.request_file_index
                            .insert(request.meta.request_id, path);
                    }
                    if self
                        .active_request_cache
                        .as_ref()
                        .is_some_and(|cached| cached.meta.request_id == request.meta.request_id)
                    {
                        self.active_request_cache = Some(request.clone());
                    }
                    self.shell.apply_event(&event);
                    if let Some(placement) = self.pending_request_placements.remove(command_id) {
                        match placement {
                            PendingRequestPlacement::Append { parent } => {
                                if let Some(parent_id) = parent.folder_id {
                                    self.shell
                                        .insert_request_into_shared_store(parent_id, None, request);
                                    self.shell.workspace_tree.insert_request_child(
                                        parent_id,
                                        request.meta.request_id,
                                        request.meta.name.clone(),
                                        request.request.method,
                                        request.request.url.clone(),
                                        request.file_path.clone(),
                                    );
                                } else {
                                    self.shell.insert_request_at_root(None, request);
                                }
                            }
                            PendingRequestPlacement::After {
                                parent,
                                after_request_id,
                            } => {
                                if let Some(parent_id) = parent.folder_id {
                                    self.shell.insert_request_into_shared_store(
                                        parent_id,
                                        Some(after_request_id),
                                        request,
                                    );
                                    self.shell.workspace_tree.insert_request_child_after(
                                        parent_id,
                                        after_request_id,
                                        request.meta.request_id,
                                        request.meta.name.clone(),
                                        request.request.method,
                                        request.request.url.clone(),
                                        request.file_path.clone(),
                                    );
                                } else {
                                    self.shell
                                        .insert_request_at_root(Some(after_request_id), request);
                                }
                            }
                        }
                        self.shell
                            .workspace_tree
                            .select_request(request.meta.request_id);
                        selected_request_to_persist = Some(request.meta.request_id);
                        should_sync_editor = true;
                    }
                }
                AppEvent::RequestDeleted { request_id, .. } => {
                    let deleted_selected =
                        self.shell.workspace_tree.selected_request_id() == Some(*request_id);
                    self.clear_request_execution_state(*request_id);
                    self.request_file_index.remove(request_id);
                    self.shell.apply_event(&event);
                    if deleted_selected {
                        should_sync_editor = true;
                    }
                }
                AppEvent::RequestMoved {
                    request,
                    new_parent_id: _,
                    insertion_index: _,
                    ..
                } => {
                    if let Some(path) = request.file_path.clone() {
                        self.request_file_index
                            .insert(request.meta.request_id, path);
                    }
                    if self
                        .active_request_cache
                        .as_ref()
                        .is_some_and(|cached| cached.meta.request_id == request.meta.request_id)
                    {
                        self.active_request_cache = Some(request.clone());
                    }
                    self.shell.apply_event(&event);
                    if self.shell.workspace_tree.selected_request_id()
                        == Some(request.meta.request_id)
                    {
                        should_sync_editor = true;
                    }
                }
                AppEvent::SyncFailed {
                    command_id,
                    operation,
                    error,
                } => {
                    self.pending_request_placements.remove(command_id);
                    self.shell.apply_event(&event);
                    log::error!(
                        "sync_failure command_id={} operation={} error={}",
                        command_id,
                        operation.as_str(),
                        error
                    );
                    window.push_notification(error.clone(), cx);
                }
                AppEvent::EnvironmentUpserted {
                    environment,
                    command_id,
                } => {
                    self.shell.apply_event(&event);
                    self.invalidate_env_var_resolved_cache();
                    self.refresh_environment_manager_dialog_if_open(
                        Some((environment.environment_id, command_id.clone())),
                        window,
                        cx,
                    );
                }
                AppEvent::EnvironmentDeleted { .. } => {
                    self.shell.apply_event(&event);
                    self.invalidate_env_var_resolved_cache();
                    if let Err(error) = self.persist_environment_selection_state() {
                        window.push_notification(error, cx);
                    }
                    self.refresh_environment_manager_dialog_if_open(None, window, cx);
                }
                AppEvent::WorkspaceSwitched { workspace_id, .. } => {
                    self.shell.apply_event(&event);
                    self.apply_active_workspace_ui_state(Some(*workspace_id), window, cx);
                }
                AppEvent::WorkspaceDeleted {
                    workspace_id,
                    new_active_workspace_id,
                    workspace_name,
                    new_active_workspace_name,
                    ..
                } => {
                    let deleted_active = self.shell.workspace.workspace_id == Some(*workspace_id);
                    self.shell.apply_event(&event);
                    if deleted_active {
                        self.apply_active_workspace_ui_state(*new_active_workspace_id, window, cx);
                        if !new_active_workspace_name.is_empty() {
                            window.push_notification(
                                format!(
                                    "Workspace \"{workspace_name}\" deleted. Switched to \"{new_active_workspace_name}\"."
                                ),
                                cx,
                            );
                        } else {
                            window.push_notification(
                                format!("Workspace \"{workspace_name}\" deleted."),
                                cx,
                            );
                        }
                    } else {
                        window.push_notification(
                            format!("Workspace \"{workspace_name}\" deleted."),
                            cx,
                        );
                    }
                }
                _ => self.shell.apply_event(&event),
            }
        }

        if let Some(request_id) = selected_request_to_persist
            && let Err(error) = self.persist_last_opened_request_id(request_id)
        {
            window.push_notification(error, cx);
        }
        if should_sync_editor {
            self.sync_request_editor_from_selection(window, cx);
        }
        self.schedule_app_event_poll(window, cx);
        if did_apply_any {
            cx.notify();
        }
    }

    fn add_request_from_tree_node(
        &mut self,
        node_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((parent, known_parent_manifest_path)) =
            self.request_parent_input_for_tree_node(node_id)
        else {
            window.push_notification("Unable to determine request parent.", cx);
            return;
        };
        let command_id = next_command_id();
        self.pending_request_placements.insert(
            command_id.clone(),
            PendingRequestPlacement::Append { parent },
        );
        let command = AppCommand::CreateRequest {
            input: CreateRequestInput {
                parent,
                known_parent_manifest_path,
                name: self.next_new_request_name(parent),
                method: HttpMethod::Get,
                url: String::new(),
            },
            command_id,
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
    }

    fn add_folder_from_tree_node(
        &mut self,
        _node_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((parent, _known_parent_manifest_path)) =
            self.folder_parent_input_for_tree_node(_node_id)
        else {
            window.push_notification("Unable to determine folder parent.", cx);
            return;
        };
        let folder_name = self.next_new_folder_name(parent);
        let command = AppCommand::CreateFolder {
            input: CreateFolderInput {
                parent,
                known_parent_manifest_path: None,
                name: folder_name,
            },
            command_id: next_command_id(),
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
    }

    fn add_request_at_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = RequestParentRef { folder_id: None };
        let command_id = next_command_id();
        self.pending_request_placements.insert(
            command_id.clone(),
            PendingRequestPlacement::Append { parent },
        );
        let command = AppCommand::CreateRequest {
            input: CreateRequestInput {
                parent,
                known_parent_manifest_path: None,
                name: self.next_new_request_name(parent),
                method: HttpMethod::Get,
                url: String::new(),
            },
            command_id,
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
    }

    fn add_folder_at_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = FolderParentRef { folder_id: None };
        let folder_name = self.next_new_folder_name(parent);
        let command = AppCommand::CreateFolder {
            input: CreateFolderInput {
                parent,
                known_parent_manifest_path: None,
                name: folder_name,
            },
            command_id: next_command_id(),
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
    }

    fn open_rename_dialog_for_tree_node(
        &mut self,
        node_id: Ulid,
        node_kind: TreeNodeKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.shell.workspace_tree.node(node_id).cloned() else {
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
                                .title("Rename")
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
        log::debug!(
            "rename_tree_node_from_modal invoked: id={}, kind={:?}, requested_name={}",
            node_id,
            node_kind,
            requested_name
        );
        let Some(node) = self.shell.workspace_tree.node(node_id).cloned() else {
            log::error!("rename: node not found for id={node_id}");
            window.push_notification("Unable to rename: item not found.", cx);
            return;
        };
        let next_name = requested_name.trim();
        if next_name.is_empty() {
            log::warn!("rename: rejected empty name");
            window.push_notification("Name cannot be empty.", cx);
            return;
        }

        let validated_name = match node_kind {
            TreeNodeKind::Folder => {
                let Some(_parent) = self.parent_ref_for_add_folder(node_id) else {
                    log::error!("rename: unable to determine folder parent for id={node_id}");
                    window.push_notification("Unable to determine folder parent.", cx);
                    return;
                };
                let validated = match validate_rename(&node.name, next_name) {
                    Ok(value) => value,
                    Err(RenameValidationError::EmptyName) => {
                        log::warn!("rename: folder empty name after validation");
                        window.push_notification("Folder name cannot be empty.", cx);
                        return;
                    }
                };
                validated
            }
            TreeNodeKind::Request => {
                let Some(_parent) = self.parent_ref_for_add_request(node_id) else {
                    log::error!("rename: unable to determine request parent for id={node_id}");
                    window.push_notification("Unable to determine request parent.", cx);
                    return;
                };
                let validated = match validate_rename(&node.name, next_name) {
                    Ok(value) => value,
                    Err(RenameValidationError::EmptyName) => {
                        log::warn!("rename: request empty name after validation");
                        window.push_notification("Request name cannot be empty.", cx);
                        return;
                    }
                };
                validated
            }
        };
        let confirmed_name = validated_name.clone();
        let persisted_name = validated_name;
        window.close_dialog(cx);
        cx.notify();
        if node_kind == TreeNodeKind::Request {
            let (_, known_parent_manifest_path) = self
                .request_parent_input_for_tree_node(node_id)
                .expect("request parent exists during rename");
            let _ = self
                .shell
                .workspace_tree
                .rename_node(node_id, confirmed_name.clone());
            let command = AppCommand::RenameRequest {
                input: RenameRequestInput {
                    request_id: node_id,
                    new_name: persisted_name,
                    known_request_path: node.manifest_path.clone(),
                    known_parent_manifest_path,
                },
                command_id: next_command_id(),
            };
            if let Err(error) = self.publish_app_command(command) {
                window.push_notification(error, cx);
            }
            cx.notify();
            return;
        }
        let _ = self
            .shell
            .workspace_tree
            .rename_node(node_id, confirmed_name.clone());
        let command = match node_kind {
            TreeNodeKind::Folder => AppCommand::RenameFolder {
                folder_id: node_id,
                new_name: persisted_name,
                command_id: next_command_id(),
            },
            TreeNodeKind::Request => unreachable!(),
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
        cx.notify();
    }

    fn send_request_from_tree_node(
        &mut self,
        request_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shell.workspace_tree.select_request(request_id);
        self.sync_request_editor_from_selection(window, cx);
        self.send_request(window, cx);
    }

    fn create_request_below_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_request_id) = self.shell.workspace_tree.selected_request_id() else {
            window.push_notification("No active request selected.", cx);
            return;
        };
        let Some((parent, known_parent_manifest_path)) =
            self.request_parent_input_for_tree_node(active_request_id)
        else {
            window.push_notification("Unable to determine request parent.", cx);
            return;
        };
        let command_id = next_command_id();
        self.pending_request_placements.insert(
            command_id.clone(),
            PendingRequestPlacement::After {
                parent,
                after_request_id: active_request_id,
            },
        );
        let command = AppCommand::CreateRequestAfter {
            input: CreateRequestInput {
                parent,
                known_parent_manifest_path,
                name: self.next_new_request_name(parent),
                method: HttpMethod::Get,
                url: String::new(),
            },
            source_request_id: active_request_id,
            command_id,
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
    }

    fn focus_url_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.url_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn on_action_create_request_below_active(
        &mut self,
        _: &CreateRequestBelowActive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_request_below_active(window, cx);
    }

    fn on_action_focus_url_input(
        &mut self,
        _: &FocusUrlInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_url_input(window, cx);
    }

    fn on_action_send_active_request(
        &mut self,
        _: &SendActiveRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_send_or_cancel_action(window, cx);
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
        let Some((parent, known_parent_manifest_path)) =
            self.request_parent_input_for_tree_node(request_id)
        else {
            window.push_notification("Unable to determine request parent.", cx);
            return;
        };
        let command_id = next_command_id();
        self.pending_request_placements.insert(
            command_id.clone(),
            PendingRequestPlacement::After {
                parent,
                after_request_id: request_id,
            },
        );
        let command = AppCommand::DuplicateRequest {
            input: DuplicateRequestInput {
                request_id,
                duplicate_name,
                parent,
                known_request_path: self.request_file_path_for_tree_node(request_id),
                known_parent_manifest_path,
            },
            command_id,
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
    }

    fn delete_request_from_tree_node(
        &mut self,
        request_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = AppCommand::DeleteRequest {
            input: DeleteRequestInput {
                request_id,
                known_request_path: self.request_file_path_for_tree_node(request_id),
                known_parent_manifest_path: self
                    .request_parent_input_for_tree_node(request_id)
                    .and_then(|(_, path)| path),
            },
            command_id: next_command_id(),
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
    }

    fn delete_folder_from_tree_node(
        &mut self,
        folder_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = AppCommand::DeleteFolder {
            folder_id,
            command_id: next_command_id(),
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
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

    fn method_badge_colors(method: HttpMethod, cx: &App) -> (Hsla, Hsla) {
        match method {
            HttpMethod::Get => (
                cx.theme().success.opacity(1.0),
                cx.theme().success_foreground,
            ),
            HttpMethod::Post => (
                cx.theme().warning.opacity(1.0),
                cx.theme().warning_foreground,
            ),
            HttpMethod::Put | HttpMethod::Patch => {
                (cx.theme().info.opacity(1.0), cx.theme().info_foreground)
            }
            HttpMethod::Delete => (cx.theme().danger.opacity(1.0), cx.theme().danger_foreground),
            HttpMethod::Head | HttpMethod::Options => {
                (cx.theme().secondary, cx.theme().secondary_foreground)
            }
        }
    }

    fn render_method_badge(method: HttpMethod, cx: &App) -> Div {
        let (badge_bg, badge_text) = Self::method_badge_colors(method, cx);
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

    fn render_title_bar_content(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let workspace_button = div()
            .flex_shrink_0()
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .child(self.render_workspace_picker(true, cx));

        h_flex()
            .items_center()
            .justify_between()
            .w_full()
            .h_full()
            .px_2()
            .text_sm()
            .text_color(cx.theme().foreground)
            .child(workspace_button)
            .child(
                Button::new("title-bar-environment-sheet")
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .h(px(22.0))
                    .px_1()
                    .rounded(px(6.0))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_environment_variables_sheet(window, cx);
                    }))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::default()
                                    .path("icons/variable.svg")
                                    .size(px(14.0))
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child("Environment variables"),
                    ),
            )
    }

    fn build_request_file_index(shell: &AppShellState) -> HashMap<Ulid, PathBuf> {
        shell
            .shared_store
            .requests
            .iter()
            .filter_map(|(request_id, request_file)| {
                request_file
                    .file_path
                    .clone()
                    .map(|path| (*request_id, path))
            })
            .collect()
    }

    fn new(
        shell: AppShellState,
        startup_messages: Vec<StartupMessage>,
        sync_runtime: DataSyncRuntime,
        workspace_paths: BeamPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut request = RequestAuthoringState::default();
        Self::hydrate_request_from_selection(&mut request, &shell);
        request.ensure_trailing_empty_row();
        let url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://api.example.com/resource")
                .default_value(request.url.clone())
        });
        let request_body_text = Self::body_editor_text(&request.body);
        let request_body_language = Self::body_editor_language(&request.body);
        let post_script_text = request.post_script.clone().unwrap_or_default();

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
                .replaceable(false)
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .searchable(true)
                .placeholder("Response body will appear here...")
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
                        this.show_invalid_url_border = false;
                        this.schedule_request_save(cx);
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        this.request.url = url_input.read(cx).value().to_string();
                        this.schedule_request_save(cx);
                        this.handle_send_or_cancel_action(window, cx);
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

        let request_file_index = Self::build_request_file_index(&shell);
        let mut view = Self {
            shell,
            request,
            startup_messages,
            url_input,
            request_body_editor,
            response_body_editor,
            response_headers_raw: String::new(),
            response_content_type: None,
            response_history_entries: Vec::new(),
            post_script_editor,
            active_response_tab: ResponseTab::Body,
            response_status: "—".to_string(),
            response_time: "—".to_string(),
            response_size: "—".to_string(),
            script_result: None,
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
            pending_request_save_due_at: None,
            request_save_tick_scheduled: false,
            request_save_in_flight: false,
            show_invalid_url_border: false,
            active_request_cache: None,
            request_file_index,
            environment_manager_dialog_view: None,
            settings_dialog_view: None,
            request_execution_states: HashMap::new(),
            next_request_run_id: 1,
            current_workspace_paths: workspace_paths,
            app_command_tx: sync_runtime.command_tx,
            app_event_rx: sync_runtime.event_rx,
            app_event_poll_scheduled: false,
            pending_request_placements: HashMap::new(),
            _subscriptions,
            collection_scroll_handle: UniformListScrollHandle::new(),
            collection_context_menu_row: None,
            tree_drag_hover: None,
            env_var_hover: None,
            env_var_resolved_cache: None,
        };
        view.refresh_active_request_cache();
        view.rebuild_request_param_inputs(window, cx);
        view.rebuild_request_header_inputs(window, cx);
        view.sync_request_auth_inputs(window, cx);
        view.rebuild_request_auth_input_subscriptions(window, cx);
        view.sync_response_pane_from_selection(window, cx);
        view.schedule_app_event_poll(window, cx);
        view
    }

    fn send_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        struct RequestRunCompletion {
            request_id: Ulid,
            run_id: u64,
            outcome: Option<SendRequestOutcome>,
        }

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
        let request_id = self.shell.workspace_tree.selected_request_id();
        let Some(request_id) = request_id else {
            return;
        };

        let selected_environment_id = self.selected_environment_id_for_view();
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
                self.response_body_editor.update(cx, |input, cx| {
                    input.set_value(error, window, cx);
                });
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
            let request_future = Self::execute_request_with_script(
                request_snapshot,
                Some(request_id),
                no_environment_selected,
                environment_variables,
            );
            let outcome = tokio::select! {
                _ = async {
                    let _ = cancel_rx.await;
                } => None,
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
                // Phase 2 assumption: only the latest run for this request is allowed to mutate
                // request-local execution state, so stale completions are dropped by run-id check.

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
                        this.response_time = "—".to_string();
                        this.response_size = "—".to_string();
                    }
                    cx.notify();
                    return;
                };
                let response = outcome.response;
                let response_status = response.status.clone();
                let response_time = response.time.clone();
                let response_size = response.size.clone();
                let response_body = Self::auto_format_response_body(
                    &response.body,
                    response.content_type.as_deref(),
                );
                let response_headers = response.headers.clone();
                if should_update_visible_response {
                    this.response_status = response_status;
                    this.response_time = response_time;
                    this.response_size = response_size;
                    this.response_body_editor.update(cx, |input, cx| {
                        input.set_value(response_body.clone(), window, cx);
                    });
                    this.response_headers_raw = response_headers;
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
                if let Err(error) = Self::persist_response_snapshot(request_id, &response) {
                    log::error!("Failed to persist response snapshot: {error}");
                }
                if Some(request_id) == this.shell.workspace_tree.selected_request_id() {
                    this.response_history_entries = Self::load_response_history_entries(request_id);
                }
                match outcome.script_result.as_ref() {
                    Some(script_result) => {
                        if let Err(error) = Self::persist_script_result(request_id, script_result) {
                            log::error!("Failed to persist script result: {error}");
                        }
                    }
                    None => {
                        if let Err(error) = Self::clear_script_result_for_request(request_id) {
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

    async fn execute_request_with_script(
        request: RequestAuthoringState,
        request_id: Option<Ulid>,
        no_environment_selected: bool,
        environment_variables: Vec<EnvironmentVariable>,
    ) -> SendRequestOutcome {
        let response = execute_http_request(request.clone()).await;
        let script_text = request.post_script.clone().unwrap_or_default();
        if script_text.trim().is_empty() {
            return SendRequestOutcome {
                response,
                script_result: None,
                updated_environment_variables: None,
            };
        }

        let runtime_response = ScriptRuntimeResponse {
            status: Self::parse_response_status_code(&response.status).unwrap_or(0),
            status_text: response.status.clone(),
            headers: Self::parse_response_headers(&response.headers),
            body: response.body.clone(),
            response_time_ms: Self::parse_response_duration_ms(&response.time).unwrap_or(0),
            body_size_bytes: response.body.len(),
        };
        let script_exec_result = execute_post_request_script(
            &script_text,
            &runtime_response,
            &environment_variables,
            !no_environment_selected,
        );
        let no_environment_selected_with_env_writes =
            no_environment_selected && Self::script_contains_environment_mutation(&script_text);
        let updated_environment_variables = if script_exec_result.environment_changes.is_empty()
            && script_exec_result.removed_env_keys.is_empty()
        {
            None
        } else {
            Some(Self::apply_script_environment_changes_to_variables(
                &environment_variables,
                &script_exec_result,
            ))
        };

        let request_id_text = request_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "unknown-request".to_string());
        SendRequestOutcome {
            response,
            script_result: Some(Self::to_persisted_script_result(
                &script_exec_result,
                request_id_text,
                no_environment_selected_with_env_writes,
            )),
            updated_environment_variables,
        }
    }

    fn load_environment_for_script(
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
                    secret: false,
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
            RequestBodyFormat::FormUrlEncoded => "Form URL",
            RequestBodyFormat::Multipart => "Multipart",
        }
    }

    fn body_tab_label(format: RequestBodyFormat) -> &'static str {
        match format {
            RequestBodyFormat::None => "Body",
            _ => Self::body_format_label(format),
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
            BodyConfig::None => BodyConfig::Raw {
                media_type: None,
                text,
            },
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

    fn format_xml_or_html(text: &str) -> Option<String> {
        let mut result = String::with_capacity(text.len() * 2);
        let mut depth = 0usize;
        let mut i = 0usize;
        let bytes = text.as_bytes();

        while i < bytes.len() {
            if bytes[i] == b'<' {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != b'>' {
                    j += 1;
                }
                if j >= bytes.len() {
                    let remainder = text[i..].trim();
                    if !remainder.is_empty() {
                        if !result.is_empty() && !result.ends_with('\n') {
                            result.push('\n');
                            for _ in 0..depth {
                                result.push_str("  ");
                            }
                        }
                        result.push_str(remainder);
                    }
                    break;
                }
                let tag = text[i..=j].trim();
                let is_closing = tag.starts_with("</");
                let is_self_closing = tag.ends_with("/>");
                let is_comment = tag.starts_with("<!--")
                    || tag.starts_with("<?")
                    || tag.starts_with("<!DOCTYPE")
                    || tag.starts_with("<![CDATA[");

                if is_closing {
                    depth = depth.saturating_sub(1);
                }

                if !result.is_empty() {
                    result.push('\n');
                }
                for _ in 0..depth {
                    result.push_str("  ");
                }
                result.push_str(tag);

                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                let next_is_tag = k < bytes.len() && bytes[k] == b'<';
                let next_is_closing = next_is_tag && k + 1 < bytes.len() && bytes[k + 1] == b'/';

                if !is_closing && !is_self_closing && !is_comment {
                    if !(next_is_tag && next_is_closing) {
                        depth += 1;
                    }
                }

                i = j + 1;
            } else if !bytes[i].is_ascii_whitespace() {
                let start = i;
                while i < bytes.len() && bytes[i] != b'<' {
                    i += 1;
                }
                let text_content = text[start..i].trim();
                if !text_content.is_empty() {
                    if !result.is_empty() && !result.ends_with('\n') {
                        result.push('\n');
                        for _ in 0..depth {
                            result.push_str("  ");
                        }
                    }
                    result.push_str(text_content);
                }
            } else {
                i += 1;
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    fn auto_format_response_body(body: &str, content_type: Option<&str>) -> String {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return body.to_string();
        }

        let ct = content_type.unwrap_or("").to_lowercase();

        if ct.contains("json") || trimmed.starts_with('{') || trimmed.starts_with('[') {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                    return pretty;
                }
            }
        } else if ct.contains("xml") || ct.contains("html") {
            if let Some(formatted) = Self::format_xml_or_html(trimmed) {
                return formatted;
            }
        }

        body.to_string()
    }

    fn format_response_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current_text = self.response_body_editor.read(cx).value().to_string();
        let trimmed = current_text.trim();
        if trimmed.is_empty() {
            return;
        }

        let formatted =
            Self::auto_format_response_body(&current_text, self.response_content_type.as_deref());

        if formatted == current_text {
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

    fn render_tree_row(
        &self,
        row: &crate::app_shell::TreeRow,
        show_before_slot: bool,
        show_after_slot: bool,
        trailing_after_slot_target: Option<Ulid>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let node = self.shell.workspace_tree.node(row.id).cloned();

        let label = node
            .as_ref()
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let chevron_icon = match row.kind {
            TreeNodeKind::Folder => {
                if self.shell.workspace_tree.is_expanded(row.id) {
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
                    .text_color(cx.theme().muted_foreground),
            );
        }
        let request_row_method = if row.kind == TreeNodeKind::Request {
            self.shell
                .request_pane_data
                .get(&row.id)
                .map(|pane_data| pane_data.method)
                .or_else(|| node.as_ref().and_then(|n| n.request_method))
        } else {
            None
        };
        if let Some(method) = request_row_method {
            row_content = row_content.child(Self::render_method_badge(method, cx));
        }
        row_content = row_content.child(label.clone());

        let row_data = *row;
        let row_id = row.id;
        let row_kind = row.kind;
        let body_drop_placement = Self::tree_row_body_drop_placement(row_kind);
        let body_view = cx.entity();
        let drag_hover = self.tree_drag_hover;
        let mut body = div()
            .id(format!("tree-row-body-{}", row_id))
            .cursor_pointer()
            .child(
                ListItem::new(format!("tree-row-{}", row_id))
                    .w_full()
                    .rounded(px(8.0))
                    .py_0p5()
                    .pr(px(6.0))
                    .pl(indent + px(6.0))
                    .selected(row.selected)
                    .when(
                        drag_hover
                            .is_some_and(|(id, p)| id == row_id && p == TreeDropPlacement::Into),
                        |this| this.bg(cx.theme().drop_target),
                    )
                    .child(row_content)
                    .on_click(cx.listener(move |this, _, window, cx| match row_kind {
                        TreeNodeKind::Folder => {
                            this.shell.workspace_tree.toggle_expanded(row_id);
                            if let Err(error) = this.persist_tree_expansion_state() {
                                window.push_notification(error, cx);
                            }
                        }
                        TreeNodeKind::Request => {
                            this.shell.workspace_tree.select_request(row_id);
                            if let Err(error) = this.persist_last_opened_request_id(row_id) {
                                window.push_notification(error, cx);
                            }
                            this.sync_request_editor_from_selection(window, cx);
                        }
                    })),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| {
                    this.collection_context_menu_row = Some(row_data);
                    cx.notify();
                }),
            );
        if let Some(placement) = body_drop_placement {
            body = body
                .can_drop(move |dragged_value, _window, app| {
                    body_view.update(app, |this, _| {
                        this.can_accept_tree_row_body_drop(dragged_value, row_id, row_kind)
                    })
                })
                .drag_over::<DraggedRequest>(|style, _, _, cx| style.bg(cx.theme().selection))
                .drag_over::<DraggedFolder>(|style, _, _, cx| style.bg(cx.theme().selection))
                .on_drag_move(cx.listener(
                    move |this, drag: &DragMoveEvent<DraggedRequest>, _, cx| {
                        let dragged = drag.drag(cx).clone();
                        this.update_tree_drag_hover(
                            drag.bounds,
                            drag.event.position,
                            row_id,
                            row_kind,
                            &dragged,
                            cx,
                        );
                    },
                ))
                .on_drag_move(cx.listener(
                    move |this, drag: &DragMoveEvent<DraggedFolder>, _, cx| {
                        let dragged = drag.drag(cx).clone();
                        this.update_tree_drag_hover(
                            drag.bounds,
                            drag.event.position,
                            row_id,
                            row_kind,
                            &dragged,
                            cx,
                        );
                    },
                ))
                .on_drop(
                    cx.listener(move |this, dragged: &DraggedRequest, window, cx| {
                        this.handle_request_tree_drop(
                            dragged.request_id,
                            row_id,
                            placement,
                            window,
                            cx,
                        );
                        this.clear_tree_drag_hover(cx);
                    }),
                )
                .on_drop(
                    cx.listener(move |this, dragged: &DraggedFolder, window, cx| {
                        this.handle_folder_tree_drop(
                            dragged.folder_id,
                            row_id,
                            placement,
                            window,
                            cx,
                        );
                        this.clear_tree_drag_hover(cx);
                    }),
                );
        } else {
            body = body
                .on_drag_move(
                    cx.listener(move |this, _: &DragMoveEvent<DraggedRequest>, _, cx| {
                        this.clear_tree_drag_hover(cx);
                    }),
                )
                .on_drag_move(
                    cx.listener(move |this, _: &DragMoveEvent<DraggedFolder>, _, cx| {
                        this.clear_tree_drag_hover(cx);
                    }),
                );
        }
        match row_kind {
            TreeNodeKind::Folder => body.interactivity().on_drag(
                DraggedFolder {
                    folder_id: row_id,
                    label: label.clone(),
                },
                Self::tree_drag_preview_for_folder,
            ),
            TreeNodeKind::Request => body.interactivity().on_drag(
                DraggedRequest {
                    request_id: row_id,
                    label: label.clone(),
                },
                Self::tree_drag_preview_for_request,
            ),
        }

        let tree_row = v_flex().w_full().when(show_before_slot, |this| {
            this.child(self.render_tree_drop_slot(row_id, row_kind, TreeDropPlacement::Before, cx))
        });
        tree_row
            .child(body)
            .when(show_after_slot, |this| {
                this.child(self.render_tree_drop_slot(
                    row_id,
                    row_kind,
                    TreeDropPlacement::After,
                    cx,
                ))
            })
            .when_some(trailing_after_slot_target, |this, target_id| {
                this.child(self.render_tree_drop_slot(
                    target_id,
                    TreeNodeKind::Folder,
                    TreeDropPlacement::After,
                    cx,
                ))
            })
            .into_any_element()
    }

    fn build_tree_row_context_menu(
        &self,
        row: crate::app_shell::TreeRow,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PopupMenu {
        let row_id = row.id;
        let row_kind = row.kind;
        let view = cx.entity();
        let muted_foreground = cx.theme().muted_foreground;
        let mut menu = menu.min_w(px(180.0));
        match row_kind {
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
                                    .text_color(muted_foreground),
                            )
                            .child("Add Request")
                    })
                    .on_click(window.listener_for(
                        &view,
                        move |this, _, window, cx| {
                            this.add_request_from_tree_node(row_id, window, cx);
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
                                    .text_color(muted_foreground),
                            )
                            .child("Add Folder")
                    })
                    .on_click(window.listener_for(
                        &view,
                        move |this, _, window, cx| {
                            this.add_folder_from_tree_node(row_id, window, cx);
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
                                    .text_color(muted_foreground),
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
                                    .text_color(muted_foreground),
                            )
                            .child("Delete")
                    })
                    .on_click(window.listener_for(
                        &view,
                        move |this, _, window, cx| {
                            this.delete_folder_from_tree_node(row_id, window, cx);
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
                                    .text_color(muted_foreground),
                            )
                            .child("Send Request")
                    })
                    .on_click(window.listener_for(
                        &view,
                        move |this, _, window, cx| {
                            this.send_request_from_tree_node(row_id, window, cx);
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
                                    .text_color(muted_foreground),
                            )
                            .child("Copy as cURL")
                    })
                    .on_click(window.listener_for(
                        &view,
                        move |this, _, window, cx| {
                            this.copy_request_as_curl_from_tree_node(row_id, window, cx);
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
                                    .text_color(muted_foreground),
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
                                    .text_color(muted_foreground),
                            )
                            .child("Duplicate")
                    })
                    .on_click(window.listener_for(
                        &view,
                        move |this, _, window, cx| {
                            this.duplicate_request_from_tree_node(row_id, window, cx);
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
                                    .text_color(muted_foreground),
                            )
                            .child("Delete")
                    })
                    .on_click(window.listener_for(
                        &view,
                        move |this, _, window, cx| {
                            this.delete_request_from_tree_node(row_id, window, cx);
                        },
                    )),
                );
            }
        }
        menu
    }

    fn build_empty_space_context_menu(
        &self,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PopupMenu {
        let view = cx.entity();
        let view2 = view.clone();
        let muted_foreground = cx.theme().muted_foreground;
        menu.min_w(px(180.0))
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
                                .path("icons/add.svg")
                                .size(px(14.0))
                                .text_color(muted_foreground),
                        )
                        .child("HTTP")
                })
                .on_click(window.listener_for(
                    &view,
                    move |this, _, window, cx| {
                        this.add_request_at_root(window, cx);
                    },
                )),
            )
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
                                .path("icons/add.svg")
                                .size(px(14.0))
                                .text_color(muted_foreground),
                        )
                        .child("Folder")
                })
                .on_click(window.listener_for(
                    &view2,
                    move |this, _, window, cx| {
                        this.add_folder_at_root(window, cx);
                    },
                )),
            )
    }

    fn render_workspace_picker(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = &self.shell.workspace;
        let workspace_name = if workspace.workspace_name.is_empty() {
            "Workspace".to_string()
        } else {
            workspace.workspace_name.clone()
        };

        let all_workspaces = workspace.all_workspaces.clone();
        let current_workspace_id = workspace.workspace_id;
        let can_delete = all_workspaces.len() > 1;

        let view = cx.entity();
        let view_for_new = view.clone();
        let view_for_delete = view.clone();
        let view_for_rename = view.clone();

        let filled_bg_color = cx.theme().secondary;
        let default_bg_color = cx.theme().background;
        let filled_fg_color = cx.theme().secondary_foreground;
        let default_fg_color = cx.theme().foreground;
        let filled_icon_color = cx.theme().secondary_foreground.opacity(0.8);
        let default_icon_color = cx.theme().muted_foreground;

        Button::new("workspace-picker")
            .ghost()
            .small()
            // Maybe match the height of the environment picker: 22px?
            .h(px(28.0))
            .px_2()
            .rounded(px(6.0))
            .cursor_pointer()
            .justify_start()
            .bg(if compact {
                filled_bg_color
            } else {
                default_bg_color
            })
            .when(compact, |b| b.min_w(px(130.0)))
            .when(!compact, |b| b.w_full())
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(if compact {
                                filled_fg_color
                            } else {
                                default_fg_color
                            })
                            .truncate()
                            .child(workspace_name.clone()),
                    )
                    .child(
                        Icon::default()
                            .path("icons/chevron-down.svg")
                            .size(px(12.0))
                            .text_color(if compact {
                                filled_icon_color
                            } else {
                                default_icon_color
                            }),
                    ),
            )
            .dropdown_menu(move |menu, window, _| {
                let mut menu = menu.min_w(px(200.));

                // List existing workspaces.
                for entry in &all_workspaces {
                    let checked = Some(entry.workspace_id) == current_workspace_id;
                    let workspace_id = entry.workspace_id;
                    let entry_name = entry.name.clone();
                    let item_view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            div().w_full().cursor_pointer().child(entry_name.clone())
                        })
                        .checked(checked)
                        .on_click(window.listener_for(
                            &item_view,
                            move |this, _, _, _cx| {
                                if Some(workspace_id) != this.shell.workspace.workspace_id {
                                    this.app_command_tx
                                        .send(AppCommand::SwitchWorkspace {
                                            workspace_id,
                                            command_id: next_command_id(),
                                        })
                                        .ok();
                                }
                            },
                        )),
                    );
                }

                menu = menu.separator();

                // New workspace.
                let view_new = view_for_new.clone();
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div().w_full().cursor_pointer().child("New Workspace")
                    })
                    .on_click(window.listener_for(
                        &view_new,
                        |this, _, _, cx| {
                            this.show_create_workspace_dialog(cx);
                        },
                    )),
                );

                // Delete workspace (only shown if more than one exists).
                if can_delete {
                    let view_del = view_for_delete.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            div().w_full().cursor_pointer().child("Delete Workspace")
                        })
                        .on_click(window.listener_for(
                            &view_del,
                            |this, _, _, cx| {
                                this.show_delete_workspace_dialog(cx);
                            },
                        )),
                    );
                }

                // Rename workspace.
                let view_ren = view_for_rename.clone();
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div().w_full().cursor_pointer().child("Rename Workspace")
                    })
                    .on_click(window.listener_for(
                        &view_ren,
                        |this, _, _, cx| {
                            this.show_rename_workspace_dialog(cx);
                        },
                    )),
                );

                menu
            })
    }

    fn show_create_workspace_dialog(&mut self, cx: &mut Context<Self>) {
        self.open_workspace_name_dialog(WorkspaceDialogMode::Create, String::new(), cx);
    }

    fn show_rename_workspace_dialog(&mut self, cx: &mut Context<Self>) {
        let current_name = self.shell.workspace.workspace_name.clone();
        self.open_workspace_name_dialog(WorkspaceDialogMode::Rename, current_name, cx);
    }

    fn show_delete_workspace_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.shell.workspace.workspace_id else {
            return;
        };

        let workspace_name = self.shell.workspace.workspace_name.clone();
        let view = cx.entity();
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    let dialog_view = cx.new(|cx| {
                        WorkspaceDeleteDialogView::new(
                            view.clone(),
                            workspace_id,
                            workspace_name.clone(),
                            window,
                            cx,
                        )
                    });
                    window.defer(cx, move |window, cx| {
                        let submit_dv = dialog_view.clone();
                        window.open_dialog(cx, move |dialog, _, _| {
                            let submit_dv_for_ok = submit_dv.clone();
                            dialog
                                .title("Delete Workspace")
                                .w(px(500.0))
                                .child(dialog_view.clone())
                                .on_ok(move |_, window, cx| {
                                    let _ = submit_dv_for_ok.update(cx, |this, cx| {
                                        this.submit(window, cx);
                                    });
                                    false
                                })
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    fn open_workspace_name_dialog(
        &mut self,
        mode: WorkspaceDialogMode,
        initial_name: String,
        cx: &mut Context<Self>,
    ) {
        let title = match mode {
            WorkspaceDialogMode::Create => "New Workspace",
            WorkspaceDialogMode::Rename => "Rename Workspace",
        };
        let view = cx.entity();
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    let dialog_view = cx.new(|cx| {
                        WorkspaceNameDialogView::new(view.clone(), mode, initial_name, window, cx)
                    });
                    let focus_dv = dialog_view.clone();
                    window.defer(cx, move |window, cx| {
                        let submit_dv = dialog_view.clone();
                        window.open_dialog(cx, move |dialog, _, _| {
                            let ok_dv = submit_dv.clone();
                            dialog
                                .title(title)
                                .w(px(460.0))
                                .child(dialog_view.clone())
                                .on_ok(move |_, window, cx| {
                                    let _ = ok_dv.update(cx, |this, cx| {
                                        this.submit(window, cx);
                                    });
                                    false
                                })
                        });
                        window.defer(cx, move |window, cx| {
                            let _ = focus_dv.update(cx, |this, cx| {
                                this.focus_name_input(window, cx);
                            });
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    fn render_collections_panel(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut panel = v_flex()
            .h_full()
            .w_full()
            .gap(px(2.0))
            .p_2()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);

        if !self.startup_messages.is_empty() {
            for msg in &self.startup_messages {
                panel = panel.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child(msg.text.clone()),
                );
            }
        }

        let rows = self.shell.workspace_tree.visible_rows();
        if rows.is_empty() {
            let view = cx.entity();
            panel.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .context_menu(move |menu, window, cx| {
                        view.update(cx, |this, cx| {
                            this.build_empty_space_context_menu(menu, window, cx)
                        })
                    })
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("No requests yet"),
                            ),
                    ),
            )
        } else {
            let view = cx.entity();
            let list_view = view.clone();
            let menu_view = view.clone();
            panel.child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list("workspace_tree", rows.len(), {
                            let rows = rows.clone();
                            move |visible_range, window, app| {
                                let mut elements = Vec::with_capacity(visible_range.len());
                                for ix in visible_range {
                                    let row = rows[ix].clone();
                                    let show_before_slot =
                                        tree_row_shows_before_drop_slot(&rows, ix);
                                    let show_after_slot = tree_row_shows_after_drop_slot(&rows, ix);
                                    let trailing_after_slot_target =
                                        list_view.update(app, |this, _| {
                                            trailing_tree_drop_slot_target(&rows, ix, |id| {
                                                this.shell
                                                    .workspace_tree
                                                    .node(id)
                                                    .and_then(|node| node.parent_id)
                                            })
                                        });
                                    let el = list_view.update(app, |this, cx| {
                                        this.render_tree_row(
                                            &row,
                                            show_before_slot,
                                            show_after_slot,
                                            trailing_after_slot_target,
                                            window,
                                            cx,
                                        )
                                    });
                                    elements.push(el);
                                }
                                elements
                            }
                        })
                        .flex_grow()
                        .size_full()
                        .with_sizing_behavior(ListSizingBehavior::Auto)
                        .track_scroll(&self.collection_scroll_handle),
                    )
                    .vertical_scrollbar(&self.collection_scroll_handle)
                    .context_menu({
                        let view = menu_view;
                        move |menu, window, cx| {
                            view.update(cx, |this, cx| {
                                if let Some(row) = this.collection_context_menu_row.take() {
                                    this.build_tree_row_context_menu(row, menu, window, cx)
                                } else {
                                    this.build_empty_space_context_menu(menu, window, cx)
                                }
                            })
                        }
                    }),
            )
        }
    }

    fn render_url_bar(&self, cx: &mut Context<Self>) -> Div {
        let send_state = self.send_button_state_for_view();
        let send_disabled = matches!(
            send_state,
            SendButtonState::Disabled(SendDisabledReason::EmptyUrl)
        );
        let send_icon_path = if matches!(send_state, SendButtonState::Sending) {
            "icons/cancel.svg"
        } else {
            "icons/send.svg"
        };
        let highlight_invalid_url = self.show_invalid_url_border;
        let current_method = self.request.method;
        let url_has_selection = !self.url_input.read(cx).selected_range().is_empty();
        let selected_environment_id = self.selected_environment_id_for_view();
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
                    .border_color(if highlight_invalid_url {
                        cx.theme().danger
                    } else {
                        cx.theme().border
                    })
                    .bg(cx.theme().background)
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
                                                        this.schedule_request_save(cx);
                                                        cx.notify();
                                                    },
                                                )),
                                            );
                                        }
                                        menu
                                    }),
                            )
                            .child({
                                let url_entity = self.url_input.clone();
                                div()
                                    .id("env-hover-url")
                                    .flex_1()
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &MouseMoveEvent, _, cx| {
                                            this.update_env_var_hover_for_input(
                                                &url_entity,
                                                event.position,
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if !hovered {
                                            this.clear_env_var_hover(cx);
                                        }
                                    }))
                                    .child(
                                        Input::new(&self.url_input)
                                            .flex_1()
                                            .small()
                                            .appearance(false)
                                            .context_menu({
                                                move |menu, _, cx| {
                                                    Self::build_text_edit_context_menu(
                                                        menu,
                                                        url_has_selection,
                                                        cx.theme().muted_foreground,
                                                    )
                                                }
                                            }),
                                    )
                            })
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
                                            .path(send_icon_path)
                                            .size(px(16.0))
                                            .text_color(cx.theme().muted_foreground),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.handle_send_or_cancel_action(window, cx);
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
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .cursor_pointer()
                    .justify_start()
                    .child(div().w_full().child(env_label))
                    .dropdown_menu(move |menu, window, _| {
                        let mut menu = menu.min_w(px(220.));
                        let environment_view = environment_view.clone();

                        let no_env_selected = selected_environment_id.is_none_or(|selected_id| {
                            !environment_options
                                .iter()
                                .any(|(environment_id, _)| *environment_id == selected_id)
                        });
                        menu = menu.item(
                            PopupMenuItem::element(move |_, _| {
                                div().w_full().cursor_pointer().child("No environment")
                            })
                            .checked(no_env_selected)
                            .on_click(window.listener_for(
                                &environment_view,
                                move |this, _, window, cx| {
                                    this.clear_selected_environment_for_view();
                                    if let Err(error) = this.persist_environment_selection_state() {
                                        window.push_notification(error, cx);
                                    }
                                    cx.notify();
                                },
                            )),
                        );

                        for (environment_id, label) in environment_options.clone() {
                            let checked = Some(environment_id) == selected_environment_id;
                            let item_view = environment_view.clone();
                            menu = menu.item(
                                PopupMenuItem::element(move |_, _| {
                                    div().w_full().cursor_pointer().child(label.clone())
                                })
                                .checked(checked)
                                .on_click(window.listener_for(
                                    &item_view,
                                    move |this, _, window, cx| {
                                        this.set_selected_environment_for_view(environment_id);
                                        if let Err(error) =
                                            this.persist_environment_selection_state()
                                        {
                                            window.push_notification(error, cx);
                                        }
                                        cx.notify();
                                    },
                                )),
                            );
                        }
                        menu = menu.separator().item(
                            PopupMenuItem::element(move |_, _| {
                                div().w_full().cursor_pointer().child("Manage environment")
                            })
                            .on_click(window.listener_for(
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

    fn update_env_var_hover_for_input(
        &mut self,
        input_entity: &Entity<InputState>,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let env_id = self.selected_environment_id_for_view();
        let cache_stale = self
            .env_var_resolved_cache
            .as_ref()
            .map(|(cached_id, _)| *cached_id != env_id)
            .unwrap_or(true);
        if cache_stale {
            let env_vars = self.load_environment_for_script(env_id);
            let resolved = build_enabled_environment_lookup(&env_vars);
            self.env_var_resolved_cache = Some((env_id, resolved));
        }
        let resolved_env = self
            .env_var_resolved_cache
            .as_ref()
            .map(|(_, m)| m.clone())
            .unwrap_or_default();

        let input = input_entity.read(cx);
        let text = input.value().to_string();
        let mut found: Option<EnvVarHoverInfo> = None;
        for (byte_range, var_name) in find_env_var_ranges(&text) {
            let Some(bounds) = input.range_to_bounds(&byte_range) else {
                continue;
            };
            if bounds.contains(&pos) {
                let resolved = resolved_env.get(&var_name).cloned();
                found = Some(EnvVarHoverInfo {
                    var_name,
                    resolved_value: resolved,
                    token_bounds: bounds,
                });
                break;
            }
        }

        if self.env_var_hover.as_ref().map(|h| &h.token_bounds)
            != found.as_ref().map(|h| &h.token_bounds)
        {
            self.env_var_hover = found;
            cx.notify();
        }
    }

    fn clear_env_var_hover(&mut self, cx: &mut Context<Self>) {
        if self.env_var_hover.is_some() {
            self.env_var_hover = None;
            cx.notify();
        }
    }

    fn render_env_var_hover_overlay(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut elements: Vec<AnyElement> = Vec::new();

        if let Some(hover_info) = &self.env_var_hover {
            let popup_x = hover_info.token_bounds.origin.x;
            let popup_y = hover_info.token_bounds.bottom();
            let var_name = hover_info.var_name.clone();
            let resolved_value = hover_info.resolved_value.clone();

            let content = h_flex()
                .gap_1p5()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{}:", var_name)),
                )
                .child(match &resolved_value {
                    Some(val) => div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(val.clone())
                        .into_any_element(),
                    None => div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .italic()
                        .child("not set")
                        .into_any_element(),
                });

            elements.push(
                deferred(
                    anchored()
                        .snap_to_window_with_margin(px(8.))
                        .anchor(gpui::Anchor::TopLeft)
                        .position(point(popup_x, popup_y))
                        .child(
                            div()
                                .occlude()
                                .popover_style(cx)
                                .px_2()
                                .py_1p5()
                                .child(content),
                        ),
                )
                .with_priority(2)
                .into_any_element(),
            );
        }

        elements
    }

    fn render_request_tabs(&self, cx: &mut Context<Self>) -> Div {
        let mut tabs = h_flex().items_center().gap_1().w_full();
        let body_tab_view = cx.entity();
        let current_body_format = Self::body_format_from_config(&self.request.body);
        let body_tab_label = Self::body_tab_label(current_body_format);
        let body_tab_button = Button::new("tab-Body")
            .small()
            .ghost()
            .cursor_pointer()
            .selected(self.request.active_tab == RequestTab::Body)
            .child(
                h_flex().items_center().gap_1().child(body_tab_label).child(
                    Icon::default()
                        .path("icons/chevron-down.svg")
                        .size(px(12.0))
                        .text_color(cx.theme().muted_foreground),
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
                cx.theme().success
            } else {
                cx.theme().danger
            })
        } else {
            Some(cx.theme().muted_foreground)
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
        let post_script_help_trigger = HoverCard::new("tab-Post Script-help")
            .anchor(gpui::Anchor::BottomLeft)
            .open_delay(Duration::from_millis(100))
            .close_delay(Duration::from_millis(150))
            .trigger(
                div()
                    .id("tab-Post Script-help-trigger")
                    .flex()
                    .flex_shrink_0()
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .cursor_pointer()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .line_height(relative(1.0))
                            .text_color(cx.theme().secondary_foreground)
                            .child("i"),
                    ),
            )
            .child(
                div().w(px(360.0)).h(px(520.0)).overflow_hidden().child(
                    div().size_full().overflow_y_scrollbar().child(
                        markdown(POST_SCRIPT_API_HELP_MARKDOWN)
                            .w_full()
                            .text_sm()
                            .selectable(true),
                    ),
                ),
            );
        tabs = tabs.child(
            h_flex()
                .items_center()
                .gap_1()
                .child(
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
                )
                .child(post_script_help_trigger),
        );

        tabs
    }

    fn context_menu_item_row(
        label: &'static str,
        icon_path: &'static str,
        shortcut: &'static str,
        muted_color: Hsla,
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
                            .text_color(muted_color),
                    )
                    .child(label),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted_color.opacity(0.7))
                    .child(shortcut),
            )
    }

    fn context_menu_action_item(
        label: &'static str,
        icon_path: &'static str,
        shortcut: &'static str,
        muted_color: Hsla,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> PopupMenuItem {
        PopupMenuItem::element(move |_, _| {
            Self::context_menu_item_row(label, icon_path, shortcut, muted_color)
        })
        .action(action)
        .disabled(disabled)
    }

    fn build_text_edit_context_menu(
        menu: PopupMenu,
        has_selection: bool,
        muted_color: Hsla,
    ) -> PopupMenu {
        menu.min_w(px(180.0))
            .item(Self::context_menu_action_item(
                "Cut",
                "icons/cut.svg",
                "Cmd+X",
                muted_color,
                Box::new(input::Cut),
                !has_selection,
            ))
            .item(Self::context_menu_action_item(
                "Copy",
                "icons/copy.svg",
                "Cmd+C",
                muted_color,
                Box::new(input::Copy),
                !has_selection,
            ))
            .item(Self::context_menu_action_item(
                "Paste",
                "icons/clipboard-paste.svg",
                "Cmd+V",
                muted_color,
                Box::new(input::Paste),
                false,
            ))
            .separator()
            .item(Self::context_menu_action_item(
                "Select All",
                "icons/square-dashed-text.svg",
                "Cmd+A",
                muted_color,
                Box::new(input::SelectAll),
                false,
            ))
    }

    fn build_text_edit_context_menu_with_find(
        menu: PopupMenu,
        has_selection: bool,
        muted_color: Hsla,
    ) -> PopupMenu {
        let menu = menu
            .min_w(px(180.0))
            .item(Self::context_menu_action_item(
                "Find",
                "icons/search.svg",
                "Cmd+F",
                muted_color,
                Box::new(input::Search),
                false,
            ))
            .separator();
        Self::build_text_edit_context_menu(menu, has_selection, muted_color)
    }

    fn render_request_editor_surface(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.request.active_tab {
            RequestTab::Body => {
                if self.request.method == HttpMethod::Get {
                    return v_flex()
                        .h_full()
                        .w_full()
                        .child(div().flex_1())
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("No Body"),
                                )
                                .child(div().flex_1()),
                        )
                        .child(div().flex_1())
                        .into_any_element();
                }

                let request_body_has_selection = !self
                    .request_body_editor
                    .read(cx)
                    .selected_range()
                    .is_empty();
                {
                    let request_body_editor = self.request_body_editor.clone();
                    div()
                        .id("env-hover-request-body")
                        .h_full()
                        .w_full()
                        .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                            this.update_env_var_hover_for_input(
                                &request_body_editor,
                                event.position,
                                cx,
                            );
                        }))
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            if !hovered {
                                this.clear_env_var_hover(cx);
                            }
                        }))
                        .child(
                            Input::new(&self.request_body_editor)
                                .h_full()
                                .p_0()
                                .border_0()
                                .focus_bordered(false)
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(cx.theme().mono_font_size)
                                .context_menu({
                                    let view = cx.entity();
                                    move |menu, window, cx| {
                                        let muted_foreground = cx.theme().muted_foreground;
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
                                                            .text_color(muted_foreground),
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
                                            muted_foreground,
                                        )
                                    }
                                }),
                        )
                        .into_any_element()
                }
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
                            .border_color(cx.theme().border)
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
                            .child({
                                let key_entity = key_input.clone();
                                div()
                                    .id(("env-hover-param-key", index as u64))
                                    .flex_1()
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &MouseMoveEvent, _, cx| {
                                            this.update_env_var_hover_for_input(
                                                &key_entity,
                                                event.position,
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if !hovered {
                                            this.clear_env_var_hover(cx);
                                        }
                                    }))
                                    .child(
                                        Input::new(&key_input)
                                            .small()
                                            .w_full()
                                            .appearance(false)
                                            .context_menu({
                                                move |menu, _, cx| {
                                                    Self::build_text_edit_context_menu(
                                                        menu,
                                                        key_has_selection,
                                                        cx.theme().muted_foreground,
                                                    )
                                                }
                                            }),
                                    )
                            })
                            .child({
                                let value_entity = value_input.clone();
                                div()
                                    .id(("env-hover-param-value", index as u64))
                                    .flex_1()
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &MouseMoveEvent, _, cx| {
                                            this.update_env_var_hover_for_input(
                                                &value_entity,
                                                event.position,
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if !hovered {
                                            this.clear_env_var_hover(cx);
                                        }
                                    }))
                                    .child(
                                        Input::new(&value_input)
                                            .small()
                                            .w_full()
                                            .appearance(false)
                                            .context_menu({
                                                move |menu, _, cx| {
                                                    Self::build_text_edit_context_menu(
                                                        menu,
                                                        value_has_selection,
                                                        cx.theme().muted_foreground,
                                                    )
                                                }
                                            }),
                                    )
                            })
                            .child(
                                div().w(px(28.0)).child(
                                    Button::new(format!("delete-request-param-{index}"))
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .icon(
                                            Icon::default()
                                                .path("icons/delete.svg")
                                                .size(px(14.0))
                                                .text_color(cx.theme().muted_foreground),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.delete_request_param_row(index, window, cx);
                                        })),
                                ),
                            ),
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
                            .border_color(cx.theme().border)
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
                            .child({
                                let key_entity = key_input.clone();
                                div()
                                    .id(("env-hover-header-key", index as u64))
                                    .flex_1()
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &MouseMoveEvent, _, cx| {
                                            this.update_env_var_hover_for_input(
                                                &key_entity,
                                                event.position,
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if !hovered {
                                            this.clear_env_var_hover(cx);
                                        }
                                    }))
                                    .child(
                                        Input::new(&key_input)
                                            .small()
                                            .w_full()
                                            .appearance(false)
                                            .context_menu({
                                                move |menu, _, cx| {
                                                    Self::build_text_edit_context_menu(
                                                        menu,
                                                        key_has_selection,
                                                        cx.theme().muted_foreground,
                                                    )
                                                }
                                            }),
                                    )
                            })
                            .child({
                                let value_entity = value_input.clone();
                                div()
                                    .id(("env-hover-header-value", index as u64))
                                    .flex_1()
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &MouseMoveEvent, _, cx| {
                                            this.update_env_var_hover_for_input(
                                                &value_entity,
                                                event.position,
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if !hovered {
                                            this.clear_env_var_hover(cx);
                                        }
                                    }))
                                    .child(
                                        Input::new(&value_input)
                                            .small()
                                            .w_full()
                                            .appearance(false)
                                            .context_menu({
                                                move |menu, _, cx| {
                                                    Self::build_text_edit_context_menu(
                                                        menu,
                                                        value_has_selection,
                                                        cx.theme().muted_foreground,
                                                    )
                                                }
                                            }),
                                    )
                            })
                            .child(
                                div().w(px(28.0)).child(
                                    Button::new(format!("delete-request-header-{index}"))
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .icon(
                                            Icon::default()
                                                .path("icons/delete.svg")
                                                .size(px(14.0))
                                                .text_color(cx.theme().muted_foreground),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.delete_request_header_row(index, window, cx);
                                        })),
                                ),
                            ),
                    );
                }

                table.into_any_element()
            }
            RequestTab::Auth => div()
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
                                        username: (!username.trim().is_empty()).then_some(username),
                                        password: (!password.trim().is_empty()).then_some(password),
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
                                    let value = api_key_value_input.read(cx).value().to_string();
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
                            .text_color(cx.theme().muted_foreground)
                            .child("No auth header will be added.")
                            .into_any_element(),
                        AuthConfig::Bearer { .. } => {
                            let bearer_entity = self.request_auth_bearer_token_input.clone();
                            v_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Token"),
                                )
                                .child(
                                    div()
                                        .id("env-hover-auth-bearer")
                                        .on_mouse_move(cx.listener(
                                            move |this, event: &MouseMoveEvent, _, cx| {
                                                this.update_env_var_hover_for_input(
                                                    &bearer_entity,
                                                    event.position,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                            if !hovered {
                                                this.clear_env_var_hover(cx);
                                            }
                                        }))
                                        .child(
                                            Input::new(&self.request_auth_bearer_token_input)
                                                .small()
                                                .w_full()
                                                .context_menu({
                                                    move |menu, _, cx| {
                                                        Self::build_text_edit_context_menu(
                                                            menu,
                                                            bearer_has_selection,
                                                            cx.theme().muted_foreground,
                                                        )
                                                    }
                                                }),
                                        ),
                                )
                                .into_any_element()
                        }
                        AuthConfig::Basic { .. } => {
                            let username_entity = self.request_auth_basic_username_input.clone();
                            let password_entity = self.request_auth_basic_password_input.clone();
                            v_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Username"),
                                )
                                .child(
                                    div()
                                        .id("env-hover-auth-basic-username")
                                        .on_mouse_move(cx.listener(
                                            move |this, event: &MouseMoveEvent, _, cx| {
                                                this.update_env_var_hover_for_input(
                                                    &username_entity,
                                                    event.position,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                            if !hovered {
                                                this.clear_env_var_hover(cx);
                                            }
                                        }))
                                        .child(
                                            Input::new(&self.request_auth_basic_username_input)
                                                .small()
                                                .w_full()
                                                .context_menu({
                                                    move |menu, _, cx| {
                                                        Self::build_text_edit_context_menu(
                                                            menu,
                                                            basic_username_has_selection,
                                                            cx.theme().muted_foreground,
                                                        )
                                                    }
                                                }),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Password"),
                                )
                                .child(
                                    div()
                                        .id("env-hover-auth-basic-password")
                                        .on_mouse_move(cx.listener(
                                            move |this, event: &MouseMoveEvent, _, cx| {
                                                this.update_env_var_hover_for_input(
                                                    &password_entity,
                                                    event.position,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                            if !hovered {
                                                this.clear_env_var_hover(cx);
                                            }
                                        }))
                                        .child(
                                            Input::new(&self.request_auth_basic_password_input)
                                                .small()
                                                .w_full()
                                                .context_menu({
                                                    move |menu, _, cx| {
                                                        Self::build_text_edit_context_menu(
                                                            menu,
                                                            basic_password_has_selection,
                                                            cx.theme().muted_foreground,
                                                        )
                                                    }
                                                }),
                                        ),
                                )
                                .into_any_element()
                        }
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
                                                    if let AuthConfig::ApiKey {
                                                        key, value, ..
                                                    } = &this.request.auth
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
                                                .on_click(
                                                    cx.listener(move |this, _, _, cx| {
                                                        if let AuthConfig::ApiKey {
                                                            key,
                                                            value,
                                                            ..
                                                        } = &this.request.auth
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
                                                    }),
                                                ),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Key Value"),
                                )
                                .child({
                                    let api_key_value_entity =
                                        self.request_auth_api_key_value_input.clone();
                                    div()
                                        .id("env-hover-auth-apikey-value")
                                        .on_mouse_move(cx.listener(
                                            move |this, event: &MouseMoveEvent, _, cx| {
                                                this.update_env_var_hover_for_input(
                                                    &api_key_value_entity,
                                                    event.position,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                            if !hovered {
                                                this.clear_env_var_hover(cx);
                                            }
                                        }))
                                        .child(
                                            Input::new(&self.request_auth_api_key_value_input)
                                                .small()
                                                .w_full()
                                                .context_menu({
                                                    move |menu, _, cx| {
                                                        Self::build_text_edit_context_menu(
                                                            menu,
                                                            api_key_value_has_selection,
                                                            cx.theme().muted_foreground,
                                                        )
                                                    }
                                                }),
                                        )
                                })
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Header / Query Name"),
                                )
                                .child({
                                    let api_key_name_entity =
                                        self.request_auth_api_key_name_input.clone();
                                    div()
                                        .id("env-hover-auth-apikey-name")
                                        .on_mouse_move(cx.listener(
                                            move |this, event: &MouseMoveEvent, _, cx| {
                                                this.update_env_var_hover_for_input(
                                                    &api_key_name_entity,
                                                    event.position,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                            if !hovered {
                                                this.clear_env_var_hover(cx);
                                            }
                                        }))
                                        .child(
                                            Input::new(&self.request_auth_api_key_name_input)
                                                .small()
                                                .w_full()
                                                .context_menu({
                                                    move |menu, _, cx| {
                                                        Self::build_text_edit_context_menu(
                                                            menu,
                                                            api_key_name_has_selection,
                                                            cx.theme().muted_foreground,
                                                        )
                                                    }
                                                }),
                                        )
                                })
                                .into_any_element()
                        }
                    }
                })
                .into_any_element(),
        }
    }

    fn render_script_tests_section(&self, result: &PersistedScriptResult, cx: &App) -> AnyElement {
        if result.test_results.is_empty() {
            return div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No tests recorded.")
                .into_any_element();
        }
        let mut column = v_flex().w_full().gap_1();
        for test in &result.test_results {
            let status = if test.passed { "PASS" } else { "FAIL" };
            let color = if test.passed {
                cx.theme().success
            } else {
                cx.theme().danger
            };
            let summary = match (&test.expected, &test.actual) {
                (Some(expected), Some(actual)) if expected != actual => {
                    format!("expected={expected}, actual={actual}")
                }
                _ => String::new(),
            };
            let detail = test
                .error_message
                .as_ref()
                .filter(|message| !message.is_empty())
                .cloned()
                .or_else(|| {
                    if summary.is_empty() {
                        None
                    } else {
                        Some(summary)
                    }
                });
            let line = if let Some(detail) = detail {
                format!("[{status}] {} ({detail})", test.name)
            } else {
                format!("[{status}] {}", test.name)
            };
            column = column.child(div().text_xs().text_color(color).child(line));
        }
        column.into_any_element()
    }

    fn render_script_env_changes_section(
        &self,
        result: &PersistedScriptResult,
        cx: &App,
    ) -> AnyElement {
        let mut column = v_flex().w_full().gap_1();
        if result.no_environment_selected_with_env_writes {
            column = column.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child("Warning: Script wrote environment changes, but \"No environment\" was selected. Changes were not persisted."),
            );
        }
        if result.environment_diff.is_empty() {
            return column
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No environment changes."),
                )
                .into_any_element();
        }
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

    fn render_script_console_section(
        &self,
        result: &PersistedScriptResult,
        cx: &App,
    ) -> AnyElement {
        if result.console_output.is_empty() {
            return div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
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
                                        this.shell.workspace_tree.selected_request_id()
                                    {
                                        if let Err(error) =
                                            Self::clear_script_result_for_request(request_id)
                                        {
                                            log::error!("Failed to clear script result: {error}");
                                        }
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "Updated {}",
                            Self::format_human_timestamp(&result.updated_at)
                        )),
                );

            if let Some(error_message) = &result.error_message {
                if !error_message.is_empty() {
                    content = content.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(format!("Error: {error_message}")),
                    );
                }
            }

            content = content.child(div().text_xs().font_semibold().child("Tests"));
            content = content.child(self.render_script_tests_section(result, cx));
            content = content.child(div().text_xs().font_semibold().child("Environment Changes"));
            content = content.child(self.render_script_env_changes_section(result, cx));
            content = content.child(div().text_xs().font_semibold().child("Console"));
            content = content.child(self.render_script_console_section(result, cx));

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
                    .text_color(cx.theme().muted_foreground)
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
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .h_1_2()
                    .min_h_0()
                    .w_full()
                    .border_b_1()
                    .border_color(cx.theme().border)
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
                                    move |menu, _, cx| {
                                        Self::build_text_edit_context_menu_with_find(
                                            menu,
                                            post_script_has_selection,
                                            cx.theme().muted_foreground,
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
                .border_color(cx.theme().border)
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
                .border_color(cx.theme().border)
                .p_3()
                .child(self.render_request_editor_surface(window, cx)),
        };

        v_flex()
            .h_full()
            .w_full()
            .gap_2()
            .p_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_request_tabs(cx))
            .child(editor_container)
    }

    fn render_response_tabs(&self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
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

        let response_histories = self.response_history_entries.clone();
        let response_history_view = cx.entity();
        tabs = tabs.child(
            Button::new("response-history-dropdown")
                .small()
                .ghost()
                .cursor_pointer()
                .rounded(px(6.0))
                .disabled(self.shell.workspace_tree.selected_request_id().is_none())
                .icon(
                    Icon::default()
                        .path("icons/history.svg")
                        .size(px(14.0))
                        .text_color(cx.theme().muted_foreground),
                )
                .dropdown_menu(move |menu, window, _| {
                    let mut menu = menu.min_w(px(220.0)).scrollable(true).max_h(px(280.0));
                    if response_histories.is_empty() {
                        return menu.item(
                            PopupMenuItem::element(move |_, cx| {
                                div()
                                    .w_full()
                                    .px_2()
                                    .py_1()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("No response history")
                            })
                            .disabled(true),
                        );
                    }

                    for entry in response_histories.clone() {
                        let item_view = response_history_view.clone();
                        let title = entry.title.clone();
                        let summary = entry.summary.clone();
                        let history_entry = entry.clone();
                        menu = menu.item(
                            PopupMenuItem::element(move |_, cx| {
                                v_flex()
                                    .w_full()
                                    .cursor_pointer()
                                    .gap_0p5()
                                    .px_2()
                                    .py_1()
                                    .child(div().text_sm().child(title.clone()))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(summary.clone()),
                                    )
                            })
                            .on_click(window.listener_for(
                                &item_view,
                                move |this, _, window, cx| {
                                    let snapshot = Self::load_response_snapshot_for_history_entry(
                                        &history_entry,
                                    );
                                    this.apply_response_snapshot(&snapshot, window, cx);
                                    cx.notify();
                                },
                            )),
                        );
                    }

                    menu
                }),
        );

        tabs
    }

    fn render_response_editor_surface(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.active_response_tab {
            ResponseTab::Body => {
                let response_body_text = self.response_body_editor.read(cx).value().to_string();
                let trimmed_body = response_body_text.trim();
                if trimmed_body.is_empty() {
                    return self.render_response_body_shortcuts_empty_state(cx);
                }
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
                        move |menu, window, cx| {
                            let muted_foreground = cx.theme().muted_foreground;
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
                                                    .text_color(muted_foreground),
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
                            Self::build_text_edit_context_menu(
                                menu,
                                response_body_has_selection,
                                muted_foreground,
                            )
                        }
                    })
                    .into_any_element()
            }
            ResponseTab::Headers => self.render_response_headers_table(cx),
        }
    }

    fn command_key_icon_path() -> &'static str {
        if cfg!(target_os = "macos") {
            MACOS_COMMAND_ICON_PATH
        } else {
            NON_MACOS_COMMAND_ICON_PATH
        }
    }

    fn render_shortcut_key_with_icon(
        &self,
        icon_path: &'static str,
        key_text: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut key = h_flex()
            .items_center()
            .gap_1()
            .px_1p5()
            .h(px(22.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .child(
                Icon::default()
                    .path(icon_path)
                    .size(px(12.0))
                    .text_color(cx.theme().muted_foreground),
            );
        if let Some(key_text) = key_text {
            key = key.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(key_text),
            );
        }
        key
    }

    fn render_response_shortcut_row(
        &self,
        label: &'static str,
        key: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        h_flex()
            .items_center()
            .justify_between()
            .w(px(240.0))
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(label)
            .child(key)
    }

    fn render_response_body_shortcuts_empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        let command_icon_path = Self::command_key_icon_path();
        let send_key = self.render_shortcut_key_with_icon(command_icon_path, Some("Enter"), cx);
        let new_request_key = self.render_shortcut_key_with_icon(command_icon_path, Some("N"), cx);
        let focus_url_key = self.render_shortcut_key_with_icon(command_icon_path, Some("L"), cx);

        h_flex()
            .h_full()
            .w_full()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .child(self.render_response_shortcut_row("Send Request", send_key, cx))
                    .child(self.render_response_shortcut_row("New Request", new_request_key, cx))
                    .child(self.render_response_shortcut_row("Focus URL", focus_url_key, cx)),
            )
            .into_any_element()
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

    fn render_response_headers_table(&self, cx: &App) -> AnyElement {
        let rows = Self::parse_response_headers(&self.response_headers_raw);
        if rows.is_empty() {
            return div()
                .h_full()
                .w_full()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
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
             <tr>\
             <th style=\"text-align:left; padding:6px 8px; border-bottom:1px solid currentColor; width:220px;\">Header</th>\
             <th style=\"text-align:left; padding:6px 8px; border-bottom:1px solid currentColor;\">Value</th>\
             </tr>\
             </thead><tbody>",
        );

        for (key, value) in rows {
            table.push_str(&format!(
                "<tr>\
                 <td style=\"padding:6px 8px; border-bottom:1px solid currentColor; vertical-align:top; white-space:pre-wrap;\">{}</td>\
                 <td style=\"padding:6px 8px; border-bottom:1px solid currentColor; vertical-align:top; white-space:pre-wrap;\">{}</td>\
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

    fn render_shimmer_loading_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let color = cx.theme().progress_bar;
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(2.0))
            .overflow_hidden()
            .rounded_t(px(8.0))
            .bg(color.opacity(0.18))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .w(relative(0.28))
                    .bg(color.opacity(0.85))
                    .rounded_full()
                    .with_animation(
                        "shimmer-loading",
                        Animation::new(Duration::from_millis(1400)).repeat(),
                        move |this, delta| this.left(relative(delta)),
                    ),
            )
    }

    fn render_response_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let is_sending = self
            .shell
            .workspace_tree
            .selected_request_id()
            .map(|id| self.is_request_sending(id))
            .unwrap_or(false);

        let response_container = match self.active_response_tab {
            ResponseTab::Body => div()
                .flex_1()
                .w_full()
                .relative()
                .rounded(px(8.0))
                .border_1()
                .border_color(cx.theme().border)
                .p_0()
                .child(
                    div()
                        .w_full()
                        .h_full()
                        .when(is_sending, |d| d.opacity(0.45))
                        .child(self.render_response_editor_surface(cx)),
                )
                .when(is_sending, |d| d.child(self.render_shimmer_loading_bar(cx))),
            ResponseTab::Headers => div()
                .flex_1()
                .w_full()
                .relative()
                .rounded(px(8.0))
                .border_1()
                .border_color(cx.theme().border)
                .p_3()
                .when(is_sending, |d| d.opacity(0.45))
                .child(self.render_response_editor_surface(cx))
                .when(is_sending, |d| d.child(self.render_shimmer_loading_bar(cx))),
        };

        v_flex()
            .h_full()
            .w_full()
            .gap_2()
            .p_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .gap_2()
                    .child(self.render_response_tabs(window, cx))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.render_response_status_summary(cx))
                            .child(format!("Time: {}", self.response_time))
                            .child(format!("Size: {}", self.response_size)),
                    ),
            )
            .child(response_container)
    }

    fn render_response_status_summary(&self, _cx: &mut Context<Self>) -> AnyElement {
        let (status_code, status_text) = Self::response_status_code_and_text(&self.response_status);
        let trigger = h_flex()
            .items_center()
            .gap_1()
            .child("Status:")
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .when(status_text.is_some(), |div| div.cursor_pointer())
                    .child(status_code),
            )
            .cursor_pointer();

        match status_text {
            Some(status_text) => HoverCard::new("response-status-summary")
                .anchor(gpui::Anchor::BottomRight)
                .open_delay(Duration::from_millis(100))
                .close_delay(Duration::from_millis(150))
                .trigger(trigger)
                .child(div().occlude().text_sm().child(status_text))
                .into_any_element(),
            None => trigger.into_any_element(),
        }
    }

    fn response_status_code_and_text(status: &str) -> (String, Option<String>) {
        let Some(status_code) = Self::parse_response_status_code(status) else {
            return (status.to_string(), None);
        };

        let status_code_text = status_code.to_string();
        let status_text = status
            .strip_prefix(&status_code_text)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .or_else(|| {
                reqwest::StatusCode::from_u16(status_code)
                    .ok()
                    .and_then(|status| status.canonical_reason())
                    .map(str::to_string)
            });

        (status_code_text, status_text)
    }

    fn render_status_bar(&mut self, cx: &mut Context<Self>) -> Div {
        h_flex()
            .items_center()
            .w_full()
            .h(px(28.0))
            .px_3()
            .bg(cx.theme().secondary)
            .border_t_1()
            .border_color(cx.theme().border)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(
                Button::new("status-bar-settings-modal")
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .h(px(22.0))
                    .px_1()
                    .rounded(px(6.0))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings_dialog(window, cx);
                    }))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::default()
                                    .path("icons/settings.svg")
                                    .size(px(14.0))
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child("Settings"),
                    ),
            )
    }
}

struct HttpResponseView {
    status: String,
    time: String,
    size: String,
    body: String,
    headers: String,
    content_type: Option<String>,
}

struct SendRequestOutcome {
    response: HttpResponseView,
    script_result: Option<PersistedScriptResult>,
    updated_environment_variables: Option<Vec<EnvironmentVariable>>,
}

static HTTP_CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
static HTTP_RUNTIME: OnceLock<Result<TokioRuntime, String>> = OnceLock::new();

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

fn request_run_completion_is_current(
    execution_states: &HashMap<Ulid, RequestExecutionState>,
    request_id: Ulid,
    run_id: u64,
) -> bool {
    execution_states
        .get(&request_id)
        .is_some_and(|state| state.run_id == run_id)
}

fn apply_request_run_completion_status(
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

fn completion_updates_selected_request_ui(
    selected_request_id: Option<Ulid>,
    completed_request_id: Ulid,
) -> bool {
    selected_request_id == Some(completed_request_id)
}

fn response_summary_for_selected_request(
    selected_request_id: Option<Ulid>,
    execution_states: &HashMap<Ulid, RequestExecutionState>,
    fallback_status: &str,
    fallback_time: &str,
    fallback_size: &str,
) -> (String, String, String) {
    if let Some(request_id) = selected_request_id {
        if execution_states
            .get(&request_id)
            .is_some_and(|state| state.status == RequestExecutionStatus::Sending)
        {
            return ("Sending...".to_string(), "—".to_string(), "—".to_string());
        }
    }

    (
        fallback_status.to_string(),
        fallback_time.to_string(),
        fallback_size.to_string(),
    )
}

fn send_button_state_for_selected_request(
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

fn resolve_request_with_environment(
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

fn build_enabled_environment_lookup(
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

/// Find all `{{var_name}}` tokens in `text`, returning their byte ranges and variable names.
fn find_env_var_ranges(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut result = Vec::new();
    let mut index = 0usize;

    while let Some(start_offset) = text[index..].find("{{") {
        let start = index + start_offset;
        let token_start = start + 2;
        let Some(end_offset) = text[token_start..].find("}}") else {
            break;
        };
        let end = token_start + end_offset;
        let var_name = text[token_start..end].trim().to_string();
        if !var_name.is_empty() {
            result.push((start..end + 2, var_name));
        }
        index = end + 2;
    }

    result
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

async fn execute_http_request(request: RequestAuthoringState) -> HttpResponseView {
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
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| {
                    let value = value.to_str().unwrap_or("<non-utf8>");
                    format!("{}: {value}", name.as_str())
                })
                .collect::<Vec<_>>()
                .join("\n");
            match response.bytes().await {
                Ok(bytes) => {
                    let body = String::from_utf8_lossy(&bytes).to_string();
                    HttpResponseView {
                        status: status_text,
                        time: format!("{} ms", start.elapsed().as_millis()),
                        size: format_bytes(bytes.len()),
                        body,
                        headers,
                        content_type,
                    }
                }
                Err(error) => HttpResponseView {
                    status: status_text,
                    time: format!("{} ms", start.elapsed().as_millis()),
                    size: "—".to_string(),
                    body: format!("Failed to read response body: {error}"),
                    headers,
                    content_type,
                },
            }
        }
        Err(error) => HttpResponseView {
            status: "Error".to_string(),
            time: format!("{} ms", start.elapsed().as_millis()),
            size: "—".to_string(),
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
            .on_action(cx.listener(Self::on_action_send_active_request))
            .on_action(cx.listener(Self::on_action_create_request_below_active))
            .on_action(cx.listener(Self::on_action_focus_url_input))
            .bg(cx.theme().background)
            .child(TitleBar::new().child(self.render_title_bar_content(window, cx)))
            .child(
                h_flex().flex_1().w_full().child(
                    h_resizable("beam-main-shell")
                        .child(
                            resizable_panel()
                                .size(px(left_size))
                                .child(self.render_collections_panel(window, cx)),
                        )
                        .child(resizable_panel().child({
                            let workspace =
                                v_flex().h_full().w_full().bg(cx.theme().background).child(
                                    div()
                                        .w_full()
                                        .p_3()
                                        .border_b_1()
                                        .border_color(cx.theme().border)
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
            .child(self.render_status_bar(cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
            .children(self.render_env_var_hover_overlay(cx))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ulid::Ulid;

    use super::{
        EnvironmentManagerDialogView, RequestExecutionState, RequestExecutionStatus,
        apply_request_run_completion_status, completion_updates_selected_request_ui,
        request_run_completion_is_current, response_summary_for_selected_request,
        send_button_state_for_selected_request, trailing_tree_drop_slot_target,
        tree_row_shows_after_drop_slot, tree_row_shows_before_drop_slot,
    };
    use crate::app_shell::{TreeNodeKind, TreeRow};
    use crate::request_authoring::{RequestAuthoringState, SendButtonState};

    fn ready_request() -> RequestAuthoringState {
        RequestAuthoringState {
            url: "https://example.com".to_string(),
            ..RequestAuthoringState::default()
        }
    }

    #[::core::prelude::v1::test]
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

    #[::core::prelude::v1::test]
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

    #[::core::prelude::v1::test]
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

    #[::core::prelude::v1::test]
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

    #[::core::prelude::v1::test]
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
            "120 ms",
            "1.2 KB",
        );
        let selected_b = response_summary_for_selected_request(
            Some(request_b),
            &execution_states,
            "200",
            "120 ms",
            "1.2 KB",
        );

        assert_eq!(
            selected_a,
            ("Sending...".to_string(), "—".to_string(), "—".to_string())
        );
        assert_eq!(
            selected_b,
            (
                "200".to_string(),
                "120 ms".to_string(),
                "1.2 KB".to_string()
            )
        );
    }

    #[::core::prelude::v1::test]
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

    #[::core::prelude::v1::test]
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

    #[::core::prelude::v1::test]
    fn trailing_tree_drop_slot_targets_last_root_ancestor_for_nested_tail_row() {
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let rows = vec![
            TreeRow {
                id: folder_id,
                kind: TreeNodeKind::Folder,
                depth: 0,
                selected: false,
            },
            TreeRow {
                id: request_id,
                kind: TreeNodeKind::Request,
                depth: 1,
                selected: false,
            },
        ];

        assert_eq!(
            trailing_tree_drop_slot_target(&rows, 1, |id| match id {
                value if value == request_id => Some(folder_id),
                _ => None,
            }),
            Some(folder_id)
        );
    }

    #[::core::prelude::v1::test]
    fn trailing_tree_drop_slot_is_absent_for_last_root_row() {
        let request_id = Ulid::new();
        let rows = vec![TreeRow {
            id: request_id,
            kind: TreeNodeKind::Request,
            depth: 0,
            selected: false,
        }];

        assert_eq!(trailing_tree_drop_slot_target(&rows, 0, |_| None), None);
    }

    #[::core::prelude::v1::test]
    fn folder_to_first_child_transition_uses_child_before_slot() {
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let rows = vec![
            TreeRow {
                id: folder_id,
                kind: TreeNodeKind::Folder,
                depth: 0,
                selected: false,
            },
            TreeRow {
                id: request_id,
                kind: TreeNodeKind::Request,
                depth: 1,
                selected: false,
            },
        ];

        assert!(tree_row_shows_before_drop_slot(&rows, 1));
        assert!(!tree_row_shows_after_drop_slot(&rows, 0));
    }

    #[::core::prelude::v1::test]
    fn child_to_parent_transition_keeps_both_drop_slots() {
        let folder_id = Ulid::new();
        let nested_request_id = Ulid::new();
        let root_request_id = Ulid::new();
        let rows = vec![
            TreeRow {
                id: folder_id,
                kind: TreeNodeKind::Folder,
                depth: 0,
                selected: false,
            },
            TreeRow {
                id: nested_request_id,
                kind: TreeNodeKind::Request,
                depth: 1,
                selected: false,
            },
            TreeRow {
                id: root_request_id,
                kind: TreeNodeKind::Request,
                depth: 0,
                selected: false,
            },
        ];

        assert!(tree_row_shows_after_drop_slot(&rows, 1));
        assert!(tree_row_shows_before_drop_slot(&rows, 2));
    }
}
