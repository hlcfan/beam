mod actions;
mod app_events;
mod beam_view;
mod dialogs;
mod environment;
mod persistence;
mod request;
mod response;
mod text_edit_menu;
mod theme;
mod tree;

use beam_view::BeamView;
use environment::{EnvVarHoverInfo, environment_file_path_for_workspace};

use actions::*;
use tree::*;

use dialogs::{
    CommandPaletteDialogView, ConfirmPaletteItem, DismissCommandPalette,
    EnvironmentManagerDialogView, ImportDialogView, KeyBindingsDialogView, SelectNextPaletteItem,
    SelectPreviousPaletteItem, SettingsDialogView, TreeRenameDialogView,
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
use response::{MACOS_COMMAND_ICON_PATH, NON_MACOS_COMMAND_ICON_PATH, ResponseTab};
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
    CreateFolderInput, CreateRequestInput, DeleteRequestInput, DuplicateFolderInput,
    DuplicateRequestInput, FolderParentRef, KnownParentManifestPath, MoveFolderInput,
    MoveRequestInput, RenameRequestInput, RequestParentRef,
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
            KeyBinding::new("up", SelectPreviousPaletteItem, Some("CommandPalette")),
            KeyBinding::new("down", SelectNextPaletteItem, Some("CommandPalette")),
            KeyBinding::new("ctrl-p", SelectPreviousPaletteItem, Some("CommandPalette")),
            KeyBinding::new("ctrl-n", SelectNextPaletteItem, Some("CommandPalette")),
            KeyBinding::new("enter", ConfirmPaletteItem, Some("CommandPalette")),
            KeyBinding::new("escape", DismissCommandPalette, Some("CommandPalette")),
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
            KeyBinding::new("cmd-k", OpenCommandPalette, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-d", DuplicateActiveRequest, None),
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-backspace", DeleteActiveRequest, None),
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
            KeyBinding::new("ctrl-k", OpenCommandPalette, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-d", DuplicateActiveRequest, None),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            KeyBinding::new("ctrl-delete", DeleteActiveRequest, None),
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
                                    beam_view.duplicate_selected_tree_node(window, cx);
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
                                    beam_view.rename_selected_tree_node(window, cx);
                                });
                            });
                        }
                    }
                }
            });
        });
        cx.on_action(|_: &DeleteActiveRequest, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.delete_selected_tree_node(window, cx);
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
        cx.on_action(|_: &OpenCommandPalette, cx: &mut App| {
            cx.defer(move |cx| {
                if let Some(window_handle) = cx.active_window() {
                    if let Some(root) = window_handle
                        .downcast::<Root>()
                        .and_then(|h| h.read(cx).ok())
                    {
                        if let Ok(beam_view) = root.view().clone().downcast::<BeamView>() {
                            let _ = window_handle.update(cx, |_root_view, window, cx| {
                                beam_view.update(cx, |beam_view, cx| {
                                    beam_view.open_command_palette(window, cx);
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
