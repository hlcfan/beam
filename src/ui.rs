mod actions;
mod dialogs;
mod request;
mod response;
mod text_edit_menu;
mod theme;
mod tree;

use actions::*;
use tree::*;

use dialogs::{
    EnvironmentManagerDialogView, ImportDialogView, KeyBindingsDialogView, SettingsDialogView,
    TreeNodeDeleteDialogView, TreeRenameDialogView, WorkspaceDeleteDialogView, WorkspaceDialogMode,
    WorkspaceNameDialogView,
};
use request::body::{
    BodyFormatHint, RequestBodyFormat, body_editor_language, body_editor_text,
    body_format_from_config, body_format_label, body_from_format, body_tab_label,
    body_with_updated_text, format_body_text, response_body_editor_language,
    supported_body_formats,
};
use request::execution::{
    DEFAULT_API_KEY_HEADER_NAME, HttpResponseSnapshot, RequestExecutionState,
    build_enabled_environment_lookup, format_bytes, parse_response_headers,
    response_summary_for_selected_request,
};
use response::history::{
    ResponseHistoryEntry, StoredResponseSnapshot, load_response_history_entries,
    load_response_snapshot_for_history_entry,
};
use response::persistence::{
    PersistedScriptResult, clear_script_result_for_request, load_script_result,
};
use text_edit_menu::{
    append_with_image_or_plain, build_text_edit_context_menu,
    build_text_edit_context_menu_with_find,
};
use theme::init_theme_registry;

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use std::{fs, path::PathBuf};

use chrono::{Local, Utc};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, Disableable, Icon, Placement, Root, Selectable, Sizable, StyledExt, Theme,
    ThemeRegistry, TitleBar, VirtualListScrollHandle, WindowExt as _,
    button::{Button, ButtonVariants as _, DropdownButton},
    h_flex,
    hover_card::HoverCard,
    input::{Input, InputEvent, InputState, Position, TabSize},
    list::ListItem,
    menu::{DropdownMenu as _, PopupMenuItem},
    native_menu::NativeMenu,
    resizable::{h_resizable, resizable_panel},
    scroll::ScrollableElement,
    switch::Switch,
    tag::Tag,
    text::{html, markdown},
    tooltip::Tooltip,
    v_flex, v_virtual_list,
};
use ulid::Ulid;

use crate::app_shell::next_command_id;
use crate::app_shell::{
    AppCommand, AppEvent, AppShellState, DataSyncRuntime, ImportJob, ImportResult, RequestPaneData,
    StartupMessage, TreeNodeKind,
};
use crate::assets::Assets;
use crate::importers::{
    CurlPlan, DetectedSource, ImportPlan, is_curl, parse_curl, parser_for, scanner, tag_label,
};
use crate::models::{
    AppFontSize, AuthConfig, BodyConfig, EnvironmentFile, EnvironmentScope, EnvironmentVariable,
    HttpMethod, LocalStateFile, RequestFile,
};
use crate::paths::{BeamPaths, DataRootPaths};
use crate::post_script_help::POST_SCRIPT_API_HELP_MARKDOWN;
use crate::request_authoring::{
    RenameValidationError, RequestAuthoringState, RequestTab, SendButtonState, SendDisabledReason,
    validate_rename,
};
use crate::script::EnvironmentChangeKind;
use crate::storage::fs_backend::FileSystemStorage;
use crate::storage::workspace_repo::WorkspaceRepository;
use crate::storage::{
    CreateFolderInput, CreateRequestInput, DeleteRequestInput, DuplicateRequestInput,
    FolderParentRef, KnownParentManifestPath, MoveFolderInput, MoveRequestInput,
    RenameRequestInput, RequestParentRef,
};
use crate::tree_dnd::{
    SLOT_BAR_HEIGHT_PX, SLOT_DEPTH_GAP_PX, SLOT_DRAG_PROXIMITY_PX, SLOT_HIT_HEIGHT_PX,
    SLOT_RIGHT_PAD_PX, TREE_ROW_HEIGHT_PX, TreeDropPlacement, TreeDropSlot, TreeRenderItem,
    TreeRowViewModel, build_tree_render_items, tree_depth_inset,
};

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
        init_theme_registry(
            state.theme.theme_name.clone().map(Into::into),
            state.theme.font_size,
            cx,
        );
        cx.bind_keys([
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-q", QuitApp, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-enter", SendActiveRequest, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-r", SendActiveRequest, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-n", CreateRequestBelowActive, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-l", FocusUrlInput, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-,", OpenSettings, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-d", DuplicateActiveRequest, None),
            KeyBinding::new("f2", RenameActiveRequest, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("alt-f4", QuitApp, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-enter", SendActiveRequest, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-r", SendActiveRequest, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-n", CreateRequestBelowActive, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-l", FocusUrlInput, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-,", OpenSettings, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-d", DuplicateActiveRequest, None),
            KeyBinding::new("cmd-alt-down", SelectNextRequestInTree, None),
            KeyBinding::new("cmd-alt-up", SelectPrevRequestInTree, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-alt-down", SelectNextRequestInTree, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-alt-up", SelectPrevRequestInTree, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-alt-right", SelectNextRequestInViewHistory, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-alt-left", SelectPrevRequestInViewHistory, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-alt-right", SelectNextRequestInViewHistory, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-alt-left", SelectPrevRequestInViewHistory, None),
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
        cx.on_action(|_: &DuplicateActiveRequest, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.duplicate_active_request(window, cx);
                                });
                            });
                        }
                    }
                }
            });
        });
        cx.on_action(|_: &RenameActiveRequest, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.rename_active_request(window, cx);
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
        cx.on_action(|_: &SelectNextRequestInTree, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.select_neighbor_request(
                                        TreeNeighborDirection::Next,
                                        window,
                                        cx,
                                    );
                                });
                            });
                        }
                    }
                }
            });
        });
        cx.on_action(|_: &SelectPrevRequestInTree, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.select_neighbor_request(
                                        TreeNeighborDirection::Prev,
                                        window,
                                        cx,
                                    );
                                });
                            });
                        }
                    }
                }
            });
        });
        cx.on_action(|_: &SelectNextRequestInViewHistory, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.navigate_request_view_history(
                                        RequestViewHistoryDirection::Next,
                                        window,
                                        cx,
                                    );
                                });
                            });
                        }
                    }
                }
            });
        });
        cx.on_action(|_: &SelectPrevRequestInViewHistory, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.navigate_request_view_history(
                                        RequestViewHistoryDirection::Prev,
                                        window,
                                        cx,
                                    );
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
    focus_handle: FocusHandle,
    current_workspace_paths: BeamPaths,
    request: RequestAuthoringState,
    startup_messages: Vec<StartupMessage>,
    url_input: Entity<InputState>,
    request_body_editor: Entity<InputState>,
    response_body_editor: Entity<InputState>,
    response_headers_raw: String,
    response_content_type: Option<String>,
    response_body_language: &'static str,
    response_history_entries: Vec<ResponseHistoryEntry>,
    post_script_editor: Entity<InputState>,
    active_response_tab: ResponseTab,
    response_status: String,
    response_status_code: Option<u16>,
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
    pending_response_scroll_offset_persistence_due_at: Option<Instant>,
    response_scroll_offset_persistence_tick_scheduled: bool,
    suppress_response_scroll_offset_persistence: bool,
    show_invalid_url_border: bool,
    active_request_cache: Option<RequestFile>,
    request_file_index: HashMap<Ulid, PathBuf>,
    environment_manager_dialog_view: Option<Entity<EnvironmentManagerDialogView>>,
    settings_dialog_view: Option<Entity<SettingsDialogView>>,
    key_bindings_dialog_view: Option<Entity<KeyBindingsDialogView>>,
    import_dialog_view: Option<Entity<ImportDialogView>>,
    request_execution_states: HashMap<Ulid, RequestExecutionState>,
    next_request_run_id: u64,
    app_command_tx: std::sync::mpsc::SyncSender<AppCommand>,
    app_event_rx: std::sync::mpsc::Receiver<AppEvent>,
    app_event_poll_scheduled: bool,
    pending_request_creations: HashSet<String>,
    pending_folder_placements: HashMap<String, PendingFolderPlacement>,
    _subscriptions: Vec<Subscription>,
    collection_scroll_handle: VirtualListScrollHandle,
    collection_context_menu_row: Option<crate::app_shell::TreeRow>,
    tree_drag_hover: Option<(Ulid, TreeDropPlacement)>,
    tree_drag_slot_hover: Option<TreeDropSlot>,
    tree_drag_scroll_task: Option<Task<()>>,
    env_var_hover: Option<EnvVarHoverInfo>,
    /// Cached resolved env variables for the overlay: (active_env_id, resolved_map).
    /// Invalidated when the effective environment changes or environment data updates.
    env_var_resolved_cache: Option<(Option<Ulid>, HashMap<String, String>)>,
    /// In-memory sequence of requests the user has selected. Cleared on workspace switch.
    request_view_history: RequestViewHistory,
    request_body_editor_cache: HashMap<Ulid, Entity<InputState>>,
    request_body_editor_cache_order: Vec<Ulid>,
    request_body_editor_change_sub: Option<Subscription>,
    request_url_editor_cache: HashMap<Ulid, Entity<InputState>>,
    request_url_editor_cache_order: Vec<Ulid>,
    request_url_editor_change_sub: Option<Subscription>,
}

const BODY_EDITOR_CACHE_CAP: usize = 32;
const URL_EDITOR_CACHE_CAP: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseTab {
    Body,
    Headers,
}

/// Tracks the in-memory sequence of requests the user has viewed. The cursor
/// always points at the currently selected request so back/forward navigation
/// can move the cursor without mutating the history itself.
#[derive(Clone, Debug, Default)]
struct RequestViewHistory {
    entries: Vec<Ulid>,
    cursor: Option<usize>,
}

impl RequestViewHistory {
    fn clear(&mut self) {
        self.entries.clear();
        self.cursor = None;
    }

    /// Records a request selection in chronological order.
    ///
    /// Selecting the request already at the cursor is a no-op. Otherwise, any
    /// entries ahead of the cursor are discarded before the selected request
    /// is appended. Request ids may therefore appear more than once: visiting
    /// A, B, C, D, then C produces `[A, B, C, D, C]`.
    fn visit(&mut self, request_id: Ulid) {
        if let Some(cursor) = self.cursor
            && self.entries.get(cursor) == Some(&request_id)
        {
            return;
        }
        if let Some(cursor) = self.cursor
            && cursor + 1 < self.entries.len()
        {
            self.entries.truncate(cursor + 1);
        }
        self.entries.push(request_id);
        self.cursor = self.entries.len().checked_sub(1);
    }

    /// Drops the entry for `request_id` (if any) and shifts the cursor so it
    /// keeps pointing at the same logical selection, or clamps to the new
    /// tip / `None` if there is nothing left to point at. This is called
    /// when a request is deleted so that `go_back` / `go_forward` cannot
    /// land on a missing id.
    fn prune(&mut self, request_id: Ulid) {
        let Some(index) = self.entries.iter().position(|id| *id == request_id) else {
            return;
        };
        self.entries.remove(index);
        if self.entries.is_empty() {
            self.cursor = None;
            return;
        }
        let new_tip = self.entries.len() - 1;
        self.cursor = Some(match self.cursor {
            Some(c) if c > index => c - 1,
            Some(c) => c.min(new_tip),
            None => new_tip,
        });
    }

    /// Moves the cursor back by one entry. Returns the request id at the new
    /// position, or `None` if the cursor is already at the start.
    fn go_back(&mut self) -> Option<Ulid> {
        let cursor = self.cursor?;
        if cursor == 0 {
            return None;
        }
        self.cursor = Some(cursor - 1);
        self.entries.get(cursor - 1).copied()
    }

    /// Moves the cursor forward by one entry. Returns the request id at the
    /// new position, or `None` if the cursor is already at the tip.
    fn go_forward(&mut self) -> Option<Ulid> {
        let cursor = self.cursor?;
        let next = cursor + 1;
        let id = self.entries.get(next).copied()?;
        self.cursor = Some(next);
        Some(id)
    }
}

#[derive(Clone, Debug)]
struct EnvVarHoverInfo {
    var_name: String,
    resolved_value: Option<String>,
    token_bounds: Bounds<Pixels>,
}

const MACOS_COMMAND_ICON_PATH: &str = "icons/command.svg";
const NON_MACOS_COMMAND_ICON_PATH: &str = "icons/chevron-up.svg";

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
        let workspace_paths = self.current_workspace_paths.clone();
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
                workspace_paths.clone(),
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

    fn open_import_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let import_view =
            cx.new(|cx| ImportDialogView::new(self.app_command_tx.clone(), _window, cx));
        self.import_dialog_view = Some(import_view.clone());
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    window.defer(cx, move |window, cx| {
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog
                                .title("Import requests")
                                .w(px(640.0))
                                .child(import_view.clone())
                                .keyboard(false)
                                .overlay_closable(false)
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    fn open_key_bindings_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|_cx| KeyBindingsDialogView);
        self.key_bindings_dialog_view = Some(view.clone());
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    window.defer(cx, move |window, cx| {
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog
                                .title("Key Bindings")
                                .w(px(520.0))
                                .child(view.clone())
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
        let workspace_paths = self.current_workspace_paths.clone();
        let manager_view = cx.new(|cx| {
            EnvironmentManagerDialogView::new(
                beam_view.clone(),
                workspace_paths.clone(),
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
        let workspace_paths = self.current_workspace_paths.clone();
        dialog_view.update(cx, |dialog, cx| {
            dialog.refresh_from_snapshot(
                workspace_paths,
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
        environment_file_path_for_workspace(&self.current_workspace_paths, &environment.file_name)
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
        let next_script = self.request.post_script.clone().unwrap_or_default();
        let next_body_language = body_editor_language(&self.request.body);
        let selected_request_id = self.shell.workspace_tree.selected_request_id();
        if let Some(request_id) = selected_request_id {
            // Cache URL editor
            if let Some(cached_url) = self.request_url_editor_cache.get(&request_id) {
                self.url_input = cached_url.clone();
                self.resubscribe_request_url_editor(window, cx);
            } else {
                let url = Self::build_request_url_editor(&self.request, window, cx);
                self.url_input = url.clone();
                self.cache_url_editor(request_id, url);
                self.resubscribe_request_url_editor(window, cx);
            }
            // Cache body editor
            if let Some(cached_editor) = self.request_body_editor_cache.get(&request_id) {
                let cached_editor = cached_editor.clone();
                self.request_body_editor = cached_editor.clone();
                self.request_body_editor.update(cx, |input, cx| {
                    input.set_soft_wrap(self.shell.theme.wrap_body_editor, window, cx);
                });
                self.resubscribe_request_body_editor(window, cx);
            } else {
                let editor = Self::build_request_body_editor(
                    &self.request,
                    self.shell.theme.wrap_body_editor,
                    window,
                    cx,
                );
                self.request_body_editor = editor.clone();
                self.cache_body_editor(request_id, editor);
                self.resubscribe_request_body_editor(window, cx);
            }
        } else {
            let next_url = self.request.url.clone();
            self.url_input.update(cx, |input, cx| {
                input.set_value(next_url, window, cx);
            });
            self.request_body_editor.update(cx, |input, cx| {
                input.set_highlighter(next_body_language, cx);
                input.set_value(body_editor_text(&self.request.body), window, cx);
            });
            self.resubscribe_request_body_editor(window, cx);
        }
        self.post_script_editor.update(cx, |input, cx| {
            input.set_value(next_script, window, cx);
        });
        self.sync_request_auth_inputs(window, cx);
        self.sync_response_pane_from_selection(window, cx);
    }

    fn clear_response_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.response_status = "—".to_string();
        self.response_status_code = None;
        self.response_time = "—".to_string();
        self.response_size = "—".to_string();
        self.response_headers_raw.clear();
        self.response_content_type = None;
        self.update_response_body_editor_with_scroll_persistence_suppressed(
            window,
            cx,
            |input, window, cx| {
                input.set_value(String::new(), window, cx);
            },
        );
    }

    fn apply_response_snapshot(
        &mut self,
        snapshot: &StoredResponseSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content_type = snapshot.content_type.clone();
        let formatted_body =
            self.response_body_for_display(&snapshot.body, content_type.as_deref());
        let language = response_body_editor_language(content_type.as_deref());
        self.response_status = snapshot.status.clone();
        self.response_status_code = snapshot.status_code;
        self.response_time = snapshot.time.clone();
        self.response_size = snapshot.size.clone();
        self.response_headers_raw = snapshot.headers_raw.clone();
        self.response_content_type = content_type;
        self.update_response_body_editor_with_scroll_persistence_suppressed(
            window,
            cx,
            |input, window, cx| {
                input.set_highlighter(language, cx);
                input.set_value(formatted_body.clone(), window, cx);
            },
        );
        self.response_body_language = language;
    }

    fn sync_response_pane_from_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            self.response_history_entries.clear();
            self.clear_response_pane(window, cx);
            self.script_result = None;
            return;
        };

        self.response_history_entries =
            load_response_history_entries(&self.current_workspace_paths, request_id);
        if let Some(snapshot) = self.response_history_entries.first().map(|entry| {
            load_response_snapshot_for_history_entry(&self.current_workspace_paths, entry)
        }) {
            self.apply_response_snapshot(&snapshot, window, cx);
            self.restore_selected_request_response_scroll_offset(window, cx);
        } else {
            self.clear_response_pane(window, cx);
        }
        self.script_result = load_script_result(&self.current_workspace_paths, request_id);

        let (status, status_code, time, size) = response_summary_for_selected_request(
            Some(request_id),
            &self.request_execution_states,
            &self.response_status,
            self.response_status_code,
            &self.response_time,
            &self.response_size,
        );
        self.response_status = status;
        self.response_status_code = status_code;
        self.response_time = time;
        self.response_size = size;
    }

    fn status_code_in_color(status: Option<u16>, cx: &App) -> Hsla {
        match status {
            Some(200..=299) => cx.theme().success,
            Some(300..=399) => cx.theme().warning,
            Some(400..=599) => cx.theme().danger,
            Some(100..=199) => cx.theme().info,
            _ => cx.theme().muted_foreground,
        }
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
        let response_scroll_offset = self
            .shell
            .request_pane_data
            .get(&request_id)
            .map(|pane_data| pane_data.response_scroll_offset)
            .unwrap_or(point(px(0.), px(0.)));
        let pane_data = RequestPaneData {
            method: self.request.method,
            url: self.request.url.clone(),
            headers: self.request.headers.clone(),
            query_params: self.request.query_params.clone(),
            auth: self.request.auth.clone(),
            body: self.request.body.clone(),
            post_script: self.request.post_script.clone(),
            response_scroll_offset,
        };
        self.shell
            .request_pane_data
            .insert(request_id, pane_data.clone());
        Some((request_id, pane_data))
    }

    fn schedule_request_save_with_delay(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if self.shell.workspace_tree.selected_request_id().is_none() {
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
        // TODO: can we not initialize WorkspaceRepository everytime
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
        self.request_view_history.clear();
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
        self.request_body_editor_cache.clear();
        self.request_body_editor_cache_order.clear();
        self.request_url_editor_cache.clear();
        self.request_url_editor_cache_order.clear();
        self.request_file_index = Self::build_request_file_index(&self.shell);
        self.prune_request_execution_states();
        self.sync_request_editor_from_selection(window, cx);
        self.seed_request_view_history();
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
                    if self.pending_request_creations.remove(command_id) {
                        self.select_request(request.meta.request_id, window, cx);
                        self.commit_request_selection(window, cx);
                        selected_request_to_persist = Some(request.meta.request_id);
                    }
                }
                AppEvent::RequestDeleted { request_id, .. } => {
                    let deleted_selected =
                        self.shell.workspace_tree.selected_request_id() == Some(*request_id);
                    self.clear_request_execution_state(*request_id);
                    self.request_file_index.remove(request_id);
                    self.request_view_history.prune(*request_id);
                    self.request_body_editor_cache.remove(request_id);
                    self.request_body_editor_cache_order
                        .retain(|id| id != request_id);
                    self.request_url_editor_cache.remove(request_id);
                    self.request_url_editor_cache_order
                        .retain(|id| id != request_id);
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
                    self.pending_request_creations.remove(command_id);
                    self.pending_folder_placements.remove(command_id);
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
                AppEvent::FolderUpserted {
                    folder,
                    manifest_path,
                    command_id,
                } => {
                    self.shell.apply_event(&event);
                    if let Some(placement) = self.pending_folder_placements.remove(command_id) {
                        match placement {
                            PendingFolderPlacement::After {
                                parent,
                                insertion_index,
                                known_target_manifest_path,
                            } => {
                                self.perform_tree_move_action(
                                    TreeMoveAction::MoveFolder(MoveFolderInput {
                                        folder_id: folder.folder_id,
                                        new_parent: parent,
                                        insertion_index,
                                        known_folder_manifest_path: manifest_path.clone(),
                                        known_target_manifest_path,
                                    }),
                                    None,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        }
                    }
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
                AppEvent::ImportResult { result, command_id } => {
                    if let Some(ref import_dialog) = self.import_dialog_view {
                        import_dialog.update(cx, |dialog, cx| {
                            dialog.handle_import_result(result.clone(), command_id.clone(), cx);
                        });
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

    fn method_label(method: HttpMethod) -> &'static str {
        match method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
            HttpMethod::Query => "QUERY",
        }
    }

    fn supported_http_methods() -> [HttpMethod; 8] {
        [
            HttpMethod::Get,
            HttpMethod::Post,
            HttpMethod::Put,
            HttpMethod::Delete,
            HttpMethod::Patch,
            HttpMethod::Head,
            HttpMethod::Options,
            HttpMethod::Query,
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
            HttpMethod::Put | HttpMethod::Patch | HttpMethod::Query => {
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
                div().flex().occlude().child(
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
        let url_input = Self::build_request_url_editor(&request, window, cx);
        let post_script_text = request.post_script.clone().unwrap_or_default();
        let wrap_body_editor = shell.theme.wrap_body_editor;

        let request_body_editor =
            Self::build_request_body_editor(&request, wrap_body_editor, window, cx);

        let response_body_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("text")
                .replaceable(false)
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .searchable(true)
                .soft_wrap(wrap_body_editor)
                .placeholder("Response body will appear here...")
                .default_value("aa")
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
            cx.observe(&response_body_editor, |this: &mut Self, _, cx| {
                this.on_response_body_editor_updated(cx);
            }),
        ];

        let request_file_index = Self::build_request_file_index(&shell);
        let focus_handle = cx.focus_handle();
        let mut view = Self {
            shell,
            focus_handle,
            request,
            startup_messages,
            url_input,
            request_body_editor,
            response_body_editor,
            response_headers_raw: String::new(),
            response_content_type: None,
            response_body_language: "text",
            response_history_entries: Vec::new(),
            post_script_editor,
            active_response_tab: ResponseTab::Body,
            response_status: "—".to_string(),
            response_status_code: None,
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
            pending_response_scroll_offset_persistence_due_at: None,
            response_scroll_offset_persistence_tick_scheduled: false,
            suppress_response_scroll_offset_persistence: false,
            show_invalid_url_border: false,
            active_request_cache: None,
            request_file_index,
            environment_manager_dialog_view: None,
            settings_dialog_view: None,
            key_bindings_dialog_view: None,
            import_dialog_view: None,
            request_execution_states: HashMap::new(),
            next_request_run_id: 1,
            current_workspace_paths: workspace_paths,
            app_command_tx: sync_runtime.command_tx,
            app_event_rx: sync_runtime.event_rx,
            app_event_poll_scheduled: false,
            pending_request_creations: HashSet::new(),
            pending_folder_placements: HashMap::new(),
            _subscriptions,
            collection_scroll_handle: VirtualListScrollHandle::new(),
            collection_context_menu_row: None,
            tree_drag_hover: None,
            tree_drag_slot_hover: None,
            tree_drag_scroll_task: None,
            env_var_hover: None,
            env_var_resolved_cache: None,
            request_view_history: RequestViewHistory::default(),
            request_body_editor_cache: HashMap::new(),
            request_body_editor_cache_order: Vec::new(),
            request_body_editor_change_sub: None,
            request_url_editor_cache: HashMap::new(),
            request_url_editor_cache_order: Vec::new(),
            request_url_editor_change_sub: None,
        };
        view.resubscribe_request_body_editor(window, cx);
        view.resubscribe_request_url_editor(window, cx);
        view.refresh_active_request_cache();
        if let Some(request_id) = view.shell.workspace_tree.selected_request_id() {
            view.cache_body_editor(request_id, view.request_body_editor.clone());
            view.cache_url_editor(request_id, view.url_input.clone());
        }
        view.rebuild_request_param_inputs(window, cx);
        view.rebuild_request_header_inputs(window, cx);
        view.sync_request_auth_inputs(window, cx);
        view.rebuild_request_auth_input_subscriptions(window, cx);
        view.sync_response_pane_from_selection(window, cx);
        view.seed_request_view_history();
        view.schedule_app_event_poll(window, cx);
        view
    }

    fn set_request_body_format(
        &mut self,
        format: RequestBodyFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current_text = self.request_body_editor.read(cx).value().to_string();
        self.request.body = body_from_format(format, current_text);
        self.request.active_tab = RequestTab::Body;

        let editor_text = body_editor_text(&self.request.body);
        let language = body_editor_language(&self.request.body);
        self.request_body_editor.update(cx, |input, cx| {
            input.set_highlighter(language, cx);
            input.set_value(editor_text, window, cx);
            input.focus(window, cx);
        });

        self.schedule_request_save(cx);
        cx.notify();
    }

    fn format_request_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let body = self.request.body.clone();
        let current_text = self.request_body_editor.read(cx).value().to_string();
        let source_text = current_text.clone();

        cx.spawn_in(window, async move |_, cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        format_body_text(&current_text, BodyFormatHint::FromConfig(&body))
                    })
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

                this.request.body = body_with_updated_text(&this.request.body, formatted.clone());
                this.request.active_tab = RequestTab::Body;
                this.request_body_editor.update(cx, |input, cx| {
                    Self::replace_editor_text(input, formatted, window, cx);
                });
                this.schedule_request_save(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn response_body_for_display(&self, body: &str, content_type: Option<&str>) -> String {
        if !self.shell.theme.auto_format_response {
            return body.to_string();
        }
        format_body_text(body, BodyFormatHint::FromContentType(content_type))
            .unwrap_or_else(|_| body.to_string())
    }

    fn format_response_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current_text = self.response_body_editor.read(cx).value().to_string();
        if current_text.trim().is_empty() {
            return;
        }

        let formatted = format_body_text(
            &current_text,
            BodyFormatHint::FromContentType(self.response_content_type.as_deref()),
        )
        .unwrap_or_else(|_| current_text.clone());

        if formatted == current_text {
            return;
        }

        self.update_response_body_editor_with_scroll_persistence_suppressed(
            window,
            cx,
            |input, window, cx| {
                Self::replace_editor_text(input, formatted, window, cx);
            },
        );
        cx.notify();
    }

    fn replace_editor_text(
        input: &mut InputState,
        text: String,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        let scroll_offset = input.scroll_offset();
        input.replace_all(text, window, cx);
        input.set_scroll_offset(scroll_offset, cx);
        input.focus(window, cx);
    }

    fn build_request_body_editor(
        request: &RequestAuthoringState,
        wrap_body_editor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let body_text = body_editor_text(&request.body);
        let body_language = body_editor_language(&request.body);
        cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(body_language)
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .soft_wrap(wrap_body_editor)
                .searchable(true)
                .placeholder("Enter request body...")
                .default_value(body_text)
        })
    }

    fn resubscribe_request_body_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.request_body_editor.clone();
        self.request_body_editor_change_sub =
            Some(
                cx.subscribe_in(&editor, window, move |this, _, ev: &InputEvent, _, cx| {
                    if !matches!(ev, InputEvent::Change) {
                        return;
                    }
                    let next_body_text = this.request_body_editor.read(cx).value().to_string();
                    this.request.body = body_with_updated_text(&this.request.body, next_body_text);
                    this.schedule_request_save(cx);
                    cx.notify();
                }),
            );
    }

    fn build_request_url_editor(
        request: &RequestAuthoringState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://api.example.com/resource")
                .default_value(request.url.clone())
        })
    }

    fn apply_curl_plan(request: &mut RequestAuthoringState, plan: CurlPlan) {
        request.method = plan.method;
        request.url = plan.url;
        request.headers = plan.headers;
        request.query_params = plan.query;
        request.body = plan.body;
        request.auth = plan.auth;
        request.ensure_trailing_empty_row();
    }

    fn import_curl_from_url_input(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !is_curl(value) {
            return false;
        }

        let plan = match parse_curl(value) {
            Ok(plan) => plan,
            Err(error) => {
                window.push_notification(format!("Could not parse cURL command: {error}"), cx);
                return true;
            }
        };

        Self::apply_curl_plan(&mut self.request, plan);
        self.rebuild_request_param_inputs(window, cx);
        self.rebuild_request_header_inputs(window, cx);
        self.sync_request_auth_inputs(window, cx);

        let url = self.request.url.clone();
        self.url_input.update(cx, |input, cx| {
            input.set_value(url, window, cx);
            input.focus(window, cx);
        });

        let body_language = body_editor_language(&self.request.body);
        let body_text = body_editor_text(&self.request.body);
        self.request_body_editor.update(cx, |input, cx| {
            input.set_highlighter(body_language, cx);
            input.set_value(body_text, window, cx);
        });

        self.show_invalid_url_border = false;
        self.schedule_request_save(cx);
        cx.notify();
        true
    }

    fn resubscribe_request_url_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.url_input.clone();
        self.request_url_editor_change_sub = Some(cx.subscribe_in(
            &editor,
            window,
            move |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let value = this.url_input.read(cx).value().to_string();
                    if this.import_curl_from_url_input(&value, window, cx) {
                        return;
                    }
                    this.request.url = value;
                    this.show_invalid_url_border = false;
                    this.schedule_request_save(cx);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    let value = this.url_input.read(cx).value().to_string();
                    if this.import_curl_from_url_input(&value, window, cx) {
                        return;
                    }
                    this.request.url = value;
                    this.schedule_request_save(cx);
                    this.handle_send_or_cancel_action(window, cx);
                    cx.notify();
                }
                _ => {}
            },
        ));
    }

    fn cache_url_editor(&mut self, request_id: Ulid, editor: Entity<InputState>) {
        Self::insert_editor_cache_entry(
            &mut self.request_url_editor_cache,
            &mut self.request_url_editor_cache_order,
            URL_EDITOR_CACHE_CAP,
            request_id,
            editor,
        );
    }

    fn cache_body_editor(&mut self, request_id: Ulid, editor: Entity<InputState>) {
        Self::insert_editor_cache_entry(
            &mut self.request_body_editor_cache,
            &mut self.request_body_editor_cache_order,
            BODY_EDITOR_CACHE_CAP,
            request_id,
            editor,
        );
    }

    fn insert_editor_cache_entry(
        cache: &mut HashMap<Ulid, Entity<InputState>>,
        order: &mut Vec<Ulid>,
        cap: usize,
        request_id: Ulid,
        editor: Entity<InputState>,
    ) {
        cache.insert(request_id, editor);
        order.push(request_id);
        if cache.len() > cap
            && let Some(oldest) = order.first().copied()
        {
            cache.remove(&oldest);
            order.remove(0);
        }
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

    fn show_delete_tree_node_dialog(
        &mut self,
        node_id: Ulid,
        node_kind: TreeNodeKind,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.shell.workspace_tree.node(node_id) else {
            return;
        };
        let node_name = node.name.clone();
        let view = cx.entity();
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    let dialog_view = cx.new(|cx| {
                        TreeNodeDeleteDialogView::new(
                            view.clone(),
                            node_id,
                            node_kind,
                            node_name.clone(),
                            window,
                            cx,
                        )
                    });
                    let submit_dv = dialog_view.clone();
                    window.defer(cx, move |window, cx| {
                        window.open_dialog(cx, move |dialog, _, _| {
                            let submit_dv_for_ok = submit_dv.clone();
                            let title = match node_kind {
                                TreeNodeKind::Folder => "Delete Folder",
                                TreeNodeKind::Request => "Delete Request",
                            };
                            dialog
                                .title(title)
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
                                                    build_text_edit_context_menu(
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

        let found = {
            let input = input_entity.read(cx);
            let text = input.value();
            let line_height = input.line_height().unwrap_or(px(20.));
            let resolved_env = self.env_var_resolved_cache.as_ref().map(|(_, m)| m);

            find_env_var_ranges(text.as_ref())
                .into_iter()
                .find_map(|(byte_range, var_name)| {
                    let bounds = find_token_hover_bounds(input, &byte_range, pos, line_height)?;
                    let resolved_value = resolved_env.and_then(|m| m.get(&var_name).cloned());

                    Some(EnvVarHoverInfo {
                        var_name,
                        resolved_value,
                        token_bounds: bounds,
                    })
                })
        };

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
        let current_body_format = body_format_from_config(&self.request.body);
        let body_tab_label = body_tab_label(current_body_format);
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
                for format in supported_body_formats() {
                    let item_label = body_format_label(format);
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
                .child("Script")
                .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(color))
        } else {
            h_flex().items_center().gap_1().child("Script")
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
                                .context_menu(move |menu, _window, cx| {
                                    let muted_foreground = cx.theme().muted_foreground;
                                    let menu = menu
                                        .menu_with_icon(
                                            "Format",
                                            Icon::default().path("icons/indent.svg"),
                                            Box::new(FormatRequestBody),
                                        )
                                        .separator();
                                    build_text_edit_context_menu_with_find(
                                        menu,
                                        request_body_has_selection,
                                        muted_foreground,
                                    )
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
                                                    build_text_edit_context_menu(
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
                                                    build_text_edit_context_menu(
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
                                                    build_text_edit_context_menu(
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
                                                    build_text_edit_context_menu(
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
                                                        build_text_edit_context_menu(
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
                                                        build_text_edit_context_menu(
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
                                                        build_text_edit_context_menu(
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
                                                        build_text_edit_context_menu(
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
                                                        build_text_edit_context_menu(
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
                                        if let Err(error) = clear_script_result_for_request(
                                            &this.current_workspace_paths,
                                            request_id,
                                        ) {
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
            .border_dashed()
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
                                        build_text_edit_context_menu_with_find(
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
                .dropdown_menu(move |menu, _window, menu_cx| {
                    let list_width_px = 180.0;
                    let mut menu = menu.min_w(px(list_width_px));

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

                    let popup_menu = menu_cx.entity().clone();
                    let row_width_px = 172.0;
                    let row_height_px = 32.0;
                    let row_height = px(row_height_px);
                    let row_content_height = px(28.0);
                    let list_height =
                        px((response_histories.len() as f32 * row_height_px).min(280.0));
                    let row_sizes = Rc::new(
                        response_histories
                            .iter()
                            .map(|_| size(px(row_width_px), row_height))
                            .collect::<Vec<_>>(),
                    );
                    let menu_response_histories = response_histories.clone();
                    let menu_response_history_view = response_history_view.clone();
                    let menu_popup_menu = popup_menu.clone();
                    let scroll_handle = VirtualListScrollHandle::new();

                    menu = menu.item(
                        PopupMenuItem::element(move |_, _cx| {
                            let row_sizes = row_sizes.clone();
                            let list_response_histories = menu_response_histories.clone();
                            let list_response_history_view = menu_response_history_view.clone();
                            let list_popup_menu = menu_popup_menu.clone();
                            let scroll_handle = scroll_handle.clone();

                            div()
                                .min_w(px(list_width_px))
                                .mx(px(-8.0))
                                .p_1()
                                .h(list_height)
                                .child(
                                v_virtual_list(
                                    list_response_history_view,
                                    "response-history-dropdown-list",
                                    row_sizes,
                                    move |_, visible_range, _, cx| {
                                        visible_range
                                            .map(|ix| {
                                                let entry = list_response_histories[ix].clone();
                                                let popup_menu = list_popup_menu.clone();
                                                let timestamp_text = entry.timestamp_text.clone();
                                                let status_text = entry.status_text.clone();
                                                let history_entry = entry.clone();
                                                let status_color =
                                                    Self::status_code_in_color(
                                                        entry.execution.status,
                                                        cx,
                                                    );
                                                div().w_full().h(row_height).pb(px(4.0)).child(
                                                    ListItem::new(format!(
                                                        "response-history-dropdown-row-{ix}"
                                                    ))
                                                    .w_full()
                                                    .h(row_content_height)
                                                    .rounded(px(6.0))
                                                    .cursor_pointer()
                                                    .px_1()
                                                    .py_1()
                                                    .child(
                                                        h_flex()
                                                            .w_full()
                                                            .items_center()
                                                            .justify_between()
                                                            .text_sm()
                                                            .child(
                                                                div()
                                                                    .text_color(
                                                                        cx.theme().muted_foreground,
                                                                    )
                                                                    .child(timestamp_text),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_color(status_color)
                                                                    .child(status_text),
                                                            ),
                                                    )
                                                    .on_click(
                                                        cx.listener(move |this, _, window, cx| {
                                                            cx.stop_propagation();
                                                            window.prevent_default();
                                                            let snapshot =
                                                                load_response_snapshot_for_history_entry(
                                                                    &this.current_workspace_paths,
                                                                    &history_entry,
                                                                );
                                                            this.apply_response_snapshot(
                                                                &snapshot,
                                                                window,
                                                                cx,
                                                            );
                                                            popup_menu.update(cx, |_, cx| {
                                                                cx.emit(DismissEvent)
                                                            });
                                                            cx.notify();
                                                        }),
                                                    ),
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                    },
                                )
                                .track_scroll(&scroll_handle),
                            )
                        })
                        .disabled(true),
                    );

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
                    .context_menu(move |menu, _window, cx| {
                        let muted_foreground = cx.theme().muted_foreground;
                        let menu = menu
                            .menu_with_icon(
                                "Format",
                                Icon::default().path("icons/indent.svg"),
                                Box::new(FormatResponseBody),
                            )
                            .separator();
                        build_text_edit_context_menu(
                            menu,
                            response_body_has_selection,
                            muted_foreground,
                        )
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

    fn render_response_headers_table(&self, cx: &App) -> AnyElement {
        let rows = parse_response_headers(&self.response_headers_raw);
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

    fn render_response_status_summary(&self, cx: &mut Context<Self>) -> AnyElement {
        let (status_code, status_text) =
            Self::response_status_code_and_text(&self.response_status, self.response_status_code);
        let status_color = Self::status_code_in_color(self.response_status_code, cx);
        let trigger = h_flex()
            .items_center()
            .gap_1()
            .child("Status:")
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(status_color)
                    .when(status_text.is_some(), |div| div.cursor_pointer())
                    .child(status_code),
            )
            .cursor_pointer();

        match status_text {
            Some(status_text) => HoverCard::new("response-status-summary")
                .anchor(gpui::Anchor::BottomRight)
                .appearance(false)
                .open_delay(Duration::from_millis(100))
                .close_delay(Duration::from_millis(150))
                .trigger(trigger)
                .child(
                    div()
                        .occlude()
                        .popover_style(cx)
                        .px_2()
                        .py_0()
                        .text_sm()
                        .child(status_text),
                )
                .into_any_element(),
            None => trigger.into_any_element(),
        }
    }

    fn response_status_code_and_text(
        status: &str,
        status_code: Option<u16>,
    ) -> (String, Option<String>) {
        let Some(status_code) = status_code else {
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
            .gap_2()
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
            .child({
                let is_importing = self
                    .import_dialog_view
                    .as_ref()
                    .is_some_and(|view| view.read(cx).is_importing());
                Button::new("status-bar-import-modal")
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .ml_1()
                    .h(px(22.0))
                    .px_1()
                    .rounded(px(6.0))
                    .disabled(is_importing)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_import_dialog(window, cx);
                    }))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::default()
                                    .path("icons/import.svg")
                                    .size(px(14.0))
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child("Import"),
                    )
            })
            .child(div().flex_1())
            .child(
                Button::new("status-bar-key-bindings-modal")
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .h(px(22.0))
                    .px_1()
                    .rounded(px(6.0))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_key_bindings_dialog(window, cx);
                    }))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::default()
                                    .path("icons/command.svg")
                                    .size(px(14.0))
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child("Key bindings"),
                    ),
            )
    }
}

fn environment_file_path_for_workspace(
    workspace_paths: &BeamPaths,
    file_name: &str,
) -> Option<PathBuf> {
    let trimmed = file_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(workspace_paths.environments_dir.join(trimmed))
}

/// Return the visual-line segment of `byte_range` (a `{{var}}` token) that contains `pos`,
/// or `None` if the cursor is not over the token. When soft-wrap splits the token across
/// visual lines, `InputState::range_to_bounds` collapses it into a single rect with negative
/// width, so we walk byte-by-byte and reassemble per-line segments here.
fn find_token_hover_bounds(
    input: &InputState,
    byte_range: &std::ops::Range<usize>,
    pos: Point<Pixels>,
    line_height: Pixels,
) -> Option<Bounds<Pixels>> {
    let token_bounds = input.range_to_bounds(byte_range)?;
    if token_bounds.size.width > px(0.) && token_bounds.size.height < line_height + px(1.) {
        return token_bounds.contains(&pos).then_some(token_bounds);
    }

    let mut seg_origin: Option<Point<Pixels>> = None;
    let mut seg_right = px(0.);

    let close = |origin: Point<Pixels>, right: Pixels| Bounds {
        origin,
        size: Size {
            width: right - origin.x,
            height: line_height,
        },
    };

    for byte_offset in byte_range.start..byte_range.end {
        let Some(b) = input.range_to_bounds(&(byte_offset..byte_offset + 1)) else {
            continue;
        };

        let byte_wraps = b.size.height > line_height + px(1.) || b.size.width <= px(0.);
        if byte_wraps {
            if let Some(origin) = seg_origin.take() {
                let segment = close(origin, seg_right);
                if segment.contains(&pos) {
                    return Some(segment);
                }
            }
            continue;
        }

        let byte_right = b.origin.x + b.size.width;
        match seg_origin {
            None => {
                seg_origin = Some(b.origin);
                seg_right = byte_right;
            }
            Some(origin) if (b.origin.y - origin.y).abs() < px(1.) => {
                seg_right = byte_right;
            }
            Some(origin) => {
                let segment = close(origin, seg_right);
                if segment.contains(&pos) {
                    return Some(segment);
                }

                seg_origin = Some(b.origin);
                seg_right = byte_right;
            }
        }
    }

    if let Some(origin) = seg_origin {
        let segment = close(origin, seg_right);
        if segment.contains(&pos) {
            return Some(segment);
        }
    }
    None
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

impl Focusable for BeamView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for BeamView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let left_size = 1280.0 * self.shell.layout.collections_workspace.ratio();
        let request_size = (1280.0 - left_size) * 0.5;

        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .on_action(cx.listener(Self::on_action_send_active_request))
            .on_action(cx.listener(Self::on_action_create_request_below_active))
            .on_action(cx.listener(Self::on_action_duplicate_active_request))
            .on_action(cx.listener(Self::on_action_rename_active_request))
            .on_action(cx.listener(Self::on_action_focus_url_input))
            .on_action(cx.listener(Self::on_action_format_request_body))
            .on_action(cx.listener(Self::on_action_format_response_body))
            .on_action(cx.listener(Self::on_action_tree_menu_send_request))
            .on_action(cx.listener(Self::on_action_tree_menu_copy_as_curl))
            .on_action(cx.listener(Self::on_action_tree_menu_add_request_in_folder))
            .on_action(cx.listener(Self::on_action_tree_menu_add_folder_in_folder))
            .on_action(cx.listener(Self::on_action_tree_menu_rename))
            .on_action(cx.listener(Self::on_action_tree_menu_delete))
            .on_action(cx.listener(Self::on_action_tree_menu_duplicate_request))
            .on_action(cx.listener(Self::on_action_tree_menu_add_request_at_root))
            .on_action(cx.listener(Self::on_action_tree_menu_add_folder_at_root))
            .bg(cx.theme().background)
            .child(TitleBar::new().child(self.render_title_bar_content(window, cx)))
            .child(
                h_flex().flex_1().w_full().child(
                    h_resizable("beam-main-shell")
                        .child(
                            resizable_panel()
                                .size(px(left_size))
                                .child(self.render_workspace_panel(window, cx)),
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
    use std::path::PathBuf;

    use ulid::Ulid;

    use super::{BeamView, RequestViewHistory, environment_file_path_for_workspace};
    use crate::importers::parse_curl;
    use crate::models::{AuthConfig, BodyConfig, HttpMethod};
    use crate::paths::BeamPaths;
    use crate::request_authoring::RequestAuthoringState;

    #[test]
    fn apply_curl_plan_replaces_current_request_authoring_fields() {
        let mut request = RequestAuthoringState {
            method: HttpMethod::Delete,
            url: "https://old.example.com".to_string(),
            ..RequestAuthoringState::default()
        };
        let plan = parse_curl(
            r#"curl -X POST -u user:pass -H 'X-Test: yes' -H 'Content-Type: application/json' -d '{"ok":true}' https://new.example.com/items"#,
        )
        .unwrap();

        BeamView::apply_curl_plan(&mut request, plan);

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://new.example.com/items");
        assert_eq!(
            request.auth,
            AuthConfig::Basic {
                username: Some("user".to_string()),
                password: Some("pass".to_string()),
            }
        );
        assert!(
            request.headers.iter().any(|header| {
                header.name == "X-Test" && header.value == "yes" && header.enabled
            })
        );
        assert_eq!(
            request.body,
            BodyConfig::Json {
                text: r#"{"ok":true}"#.to_string(),
            }
        );
        assert!(
            request
                .headers
                .last()
                .is_some_and(|header| { header.name.is_empty() && header.value.is_empty() })
        );
        assert!(
            request
                .query_params
                .last()
                .is_some_and(|param| { param.name.is_empty() && param.value.is_empty() })
        );
    }

    #[test]
    fn environment_file_path_uses_selected_workspace_directory() {
        let workspace_paths =
            BeamPaths::from_root(PathBuf::from("/tmp/beam-tests/other-workspace"));

        assert_eq!(
            environment_file_path_for_workspace(&workspace_paths, "prod.env.toml"),
            Some(
                PathBuf::from("/tmp/beam-tests/other-workspace")
                    .join("environments")
                    .join("prod.env.toml")
            )
        );
        assert_eq!(
            environment_file_path_for_workspace(&workspace_paths, "   "),
            None
        );
    }

    #[test]
    fn request_view_history_records_and_steps_back_forward() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);

        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.go_back(), Some(r1));
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r2));
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_truncates_forward_on_new_visit_after_back() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let r4 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        assert_eq!(history.go_back(), Some(r2));

        // New visit after stepping back should drop the forward entries.
        history.visit(r4);
        assert_eq!(history.go_forward(), None);
        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.go_back(), Some(r1));
        assert_eq!(history.go_back(), None);
    }

    #[test]
    fn request_view_history_visit_at_cursor_is_no_op() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let mut history = RequestViewHistory::default();
        history.visit(r1);
        history.visit(r2);
        assert_eq!(history.go_back(), Some(r1));

        // Visiting the same entry already at the cursor should not duplicate
        // and should keep the cursor pinned at that entry.
        history.visit(r1);
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r2));
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_clear_resets_state() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let mut history = RequestViewHistory::default();
        history.visit(r1);
        history.visit(r2);
        assert_eq!(history.go_back(), Some(r1));

        history.clear();
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_revisit_existing_entry_records_transition() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let r4 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        history.visit(r4);
        history.visit(r3);

        assert_eq!(history.go_back(), Some(r4));
        assert_eq!(history.go_back(), Some(r3));
        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.go_back(), Some(r1));
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r2));
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.go_forward(), Some(r4));
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_new_visit_after_back_replaces_forward_branch() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let r4 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        assert_eq!(history.go_back(), Some(r2));
        history.visit(r4);

        assert_eq!(history.go_forward(), None);
        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.go_back(), Some(r1));
        assert_eq!(history.go_back(), None);
    }

    #[test]
    fn request_view_history_prune_drops_deleted_request() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        // Step back so the cursor is parked on r2; this is the request the
        // user is going to delete.
        assert_eq!(history.go_back(), Some(r2));

        history.prune(r2);
        // r2 is gone; r1 (back) and r3 (forward of where the cursor sat)
        // remain, and the cursor clamps to the new tip (r3).
        assert_eq!(history.go_back(), Some(r1));
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_prune_unselected_request_keeps_cursor() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);

        // Deleting r1 (a backward entry) must not affect the cursor (r3).
        history.prune(r1);
        assert_eq!(history.go_back(), Some(r2));
        // After removing r1 the cursor sits on r2; one more step back must
        // return None rather than a stale id.
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_prune_tip_only_entry_clears_history() {
        let r1 = Ulid::new();
        let mut history = RequestViewHistory::default();
        history.visit(r1);

        history.prune(r1);
        assert!(history.entries.is_empty());
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), None);
    }
}
