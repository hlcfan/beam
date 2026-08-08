use super::*;

pub(super) struct BeamView {
    pub(super) shell: AppShellState,
    pub(super) focus_handle: FocusHandle,
    pub(super) current_workspace_paths: BeamPaths,
    pub(super) request: RequestAuthoringState,
    pub(super) startup_messages: Vec<StartupMessage>,
    pub(super) url_input: Entity<InputState>,
    pub(super) request_body_editor: Entity<InputState>,
    pub(super) response_body_editor: Entity<InputState>,
    pub(super) response_headers_raw: String,
    pub(super) response_content_type: Option<String>,
    pub(super) response_body_language: &'static str,
    pub(super) response_history_entries: Vec<ResponseHistoryEntry>,
    pub(super) selected_response_history_index: Option<usize>,
    pub(super) post_script_editor: Entity<InputState>,
    pub(super) active_response_tab: ResponseTab,
    pub(super) response_status: String,
    pub(super) response_status_code: Option<u16>,
    pub(super) response_time: String,
    pub(super) response_size: String,
    pub(super) script_result: Option<PersistedScriptResult>,
    pub(super) request_param_name_inputs: Vec<Entity<InputState>>,
    pub(super) request_param_value_inputs: Vec<Entity<InputState>>,
    pub(super) request_param_input_subscriptions: Vec<Subscription>,
    pub(super) request_header_name_inputs: Vec<Entity<InputState>>,
    pub(super) request_header_value_inputs: Vec<Entity<InputState>>,
    pub(super) request_header_input_subscriptions: Vec<Subscription>,
    pub(super) request_auth_bearer_token_input: Entity<InputState>,
    pub(super) request_auth_basic_username_input: Entity<InputState>,
    pub(super) request_auth_basic_password_input: Entity<InputState>,
    pub(super) request_auth_api_key_name_input: Entity<InputState>,
    pub(super) request_auth_api_key_value_input: Entity<InputState>,
    pub(super) request_auth_input_subscriptions: Vec<Subscription>,
    pub(super) suppress_request_auth_change_events: bool,
    pub(super) pending_request_save_due_at: Option<Instant>,
    pub(super) request_save_tick_scheduled: bool,
    pub(super) request_save_in_flight: bool,
    pub(super) pending_response_scroll_offset_persistence_due_at: Option<Instant>,
    pub(super) response_scroll_offset_persistence_tick_scheduled: bool,
    pub(super) suppress_response_scroll_offset_persistence: bool,
    pub(super) show_invalid_url_border: bool,
    pub(super) active_request_cache: Option<RequestFile>,
    pub(super) request_file_index: HashMap<Ulid, PathBuf>,
    pub(super) environment_manager_dialog_view: Option<Entity<EnvironmentManagerDialogView>>,
    pub(super) settings_dialog_view: Option<Entity<SettingsDialogView>>,
    pub(super) key_bindings_dialog_view: Option<Entity<KeyBindingsDialogView>>,
    pub(super) import_dialog_view: Option<Entity<ImportDialogView>>,
    pub(super) command_palette_dialog_view: Option<Entity<CommandPaletteDialogView>>,
    pub(super) request_execution_states: HashMap<Ulid, RequestExecutionState>,
    pub(super) next_request_run_id: u64,
    pub(super) app_command_tx: std::sync::mpsc::SyncSender<AppCommand>,
    pub(super) app_event_rx: std::sync::mpsc::Receiver<AppEvent>,
    pub(super) app_event_poll_scheduled: bool,
    pub(super) pending_request_creations: HashSet<String>,
    pub(super) pending_folder_placements: HashMap<String, PendingFolderPlacement>,
    pub(super) _subscriptions: Vec<Subscription>,
    pub(super) collection_scroll_handle: VirtualListScrollHandle,
    pub(super) collection_context_menu_row: Option<crate::app_shell::TreeRow>,
    pub(super) tree_drag_hover: Option<(Ulid, TreeDropPlacement)>,
    pub(super) tree_drag_slot_hover: Option<TreeDropSlot>,
    pub(super) tree_drag_scroll_task: Option<Task<()>>,
    pub(super) env_var_hover: Option<EnvVarHoverInfo>,
    /// Cached resolved env variables for the overlay: (active_env_id, resolved_map).
    /// Invalidated when the effective environment changes or environment data updates.
    pub(super) env_var_resolved_cache: Option<(Option<Ulid>, HashMap<String, String>)>,
    /// In-memory sequence of requests the user has selected across all workspaces.
    pub(super) request_view_histories: WorkspaceRequestViewHistories,
    pub(super) request_body_editor_cache: HashMap<Ulid, Entity<InputState>>,
    pub(super) request_body_editor_cache_order: Vec<Ulid>,
    pub(super) request_body_editor_change_sub: Option<Subscription>,
    pub(super) request_url_editor_cache: HashMap<Ulid, Entity<InputState>>,
    pub(super) request_url_editor_cache_order: Vec<Ulid>,
    pub(super) request_url_editor_change_sub: Option<Subscription>,
}

impl BeamView {
    pub(in crate::ui) fn build_request_file_index(shell: &AppShellState) -> HashMap<Ulid, PathBuf> {
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

    pub(super) fn new(
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
            selected_response_history_index: None,
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
            command_palette_dialog_view: None,
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
            request_view_histories: WorkspaceRequestViewHistories::default(),
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
            .on_action(cx.listener(Self::on_action_delete_selected_tree_node))
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
            .on_action(cx.listener(Self::on_action_tree_menu_duplicate_folder))
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
