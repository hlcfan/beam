mod actions;
mod beam_view;
mod dialogs;
mod request;
mod response;
mod text_edit_menu;
mod theme;
mod tree;

use beam_view::BeamView;

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
