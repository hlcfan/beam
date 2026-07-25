use super::*;

impl BeamView {
    pub(in crate::ui) fn publish_app_command(&self, command: AppCommand) -> Result<(), String> {
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

    pub(in crate::ui) fn schedule_app_event_poll(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.request_view_histories
            .set_active_workspace(workspace_id);
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
                    self.request_view_histories.prune(*request_id);
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
                AppEvent::RequestMoved { request, .. } => {
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
                    self.seed_request_view_history();
                }
                AppEvent::WorkspaceDeleted {
                    workspace_id,
                    new_active_workspace_id,
                    workspace_name,
                    new_active_workspace_name,
                    ..
                } => {
                    let deleted_active = self.shell.workspace.workspace_id == Some(*workspace_id);
                    self.request_view_histories.prune_workspace(*workspace_id);
                    self.shell.apply_event(&event);
                    if deleted_active {
                        self.apply_active_workspace_ui_state(*new_active_workspace_id, window, cx);
                        self.seed_request_view_history();
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
}
