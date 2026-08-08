use super::*;

impl BeamView {
    pub(in crate::ui) fn on_action_toggle_selected_folder(
        &mut self,
        _: &ToggleSelectedFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(folder_id) = self.shell.workspace_tree.selected_node_id() else {
            return;
        };
        if !self
            .shell
            .workspace_tree
            .node(folder_id)
            .is_some_and(|node| node.kind == TreeNodeKind::Folder)
        {
            return;
        }

        self.shell.workspace_tree.toggle_expanded(folder_id);
        if let Err(error) = self.persist_tree_expansion_state() {
            window.push_notification(error, cx);
        }
        cx.notify();
    }

    pub(in crate::ui) fn select_request(
        &mut self,
        request_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.persist_current_response_scroll_offset(cx);
        self.pending_response_scroll_offset_persistence_due_at = None;
        self.shell.workspace_tree.select_request(request_id);
        self.request_view_histories.visit(request_id);
        self.sync_request_editor_from_selection(window, cx);
    }

    /// Moves the workspace tree selection to the next or previous visible row.
    /// Collapsed folder descendants are skipped and folders are selected without
    /// changing the request shown in the editor.
    pub(in crate::ui) fn select_neighbor_request(
        &mut self,
        direction: TreeNeighborDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ordered: Vec<Ulid> = self
            .shell
            .workspace_tree
            .visible_rows()
            .into_iter()
            .map(|row| row.id)
            .collect();
        if ordered.is_empty() {
            return;
        }

        let next_id = match self
            .shell
            .workspace_tree
            .selected_node_id()
            .and_then(|current| ordered.iter().position(|id| *id == current))
        {
            Some(index) => match direction {
                TreeNeighborDirection::Next => ordered[(index + 1) % ordered.len()],
                TreeNeighborDirection::Prev => {
                    if index == 0 {
                        ordered[ordered.len() - 1]
                    } else {
                        ordered[index - 1]
                    }
                }
            },
            None => ordered[0],
        };

        if Some(next_id) == self.shell.workspace_tree.selected_node_id() {
            return;
        }

        match self
            .shell
            .workspace_tree
            .node(next_id)
            .map(|node| node.kind)
        {
            Some(TreeNodeKind::Request) => {
                self.select_request(next_id, window, cx);
                self.commit_request_selection(window, cx);
            }
            Some(TreeNodeKind::Folder) => {
                self.tree_focus_handle.focus(window, cx);
                self.shell.workspace_tree.select_node(next_id);
                self.scroll_tree_node_into_view(next_id);
                cx.notify();
            }
            None => {}
        }
    }

    /// Navigates the request view history (the in-memory sequence of requests
    /// the user has selected). `ctrl+[` moves the cursor back; `ctrl+]`
    /// moves it forward. When the cursor is already at the corresponding
    /// end of the history the call is a no-op.
    pub(in crate::ui) fn navigate_request_view_history(
        &mut self,
        direction: RequestViewHistoryDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request_id = match direction {
            RequestViewHistoryDirection::Prev => self.request_view_histories.go_back(),
            RequestViewHistoryDirection::Next => self.request_view_histories.go_forward(),
        };
        let Some(request_id) = request_id else {
            return;
        };
        if Some(request_id) == self.shell.workspace_tree.selected_request_id() {
            return;
        }
        self.select_request(request_id, window, cx);
        self.commit_request_selection(window, cx);
    }

    /// Persists all local-state side effects of a request selection change (tree expansion, last
    /// opened request id) , scrolls the tree so the selection is visible, and notifies the
    /// framework. Shared by `select_neighbor_request`, `navigate_request_view_history`, and the
    /// tree row click handler.
    pub(in crate::ui) fn commit_request_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.persist_tree_expansion_state() {
            window.push_notification(error, cx);
        }
        if let Some(request_id) = self.shell.workspace_tree.selected_request_id() {
            if let Err(error) = self.persist_last_opened_request_id(request_id) {
                window.push_notification(error, cx);
            }

            self.scroll_selected_request_into_view(request_id);
        }
        cx.notify();
    }

    /// Scrolls the workspace tree just enough to bring `request_id`'s row into view, leaving the
    /// scroll offset untouched if it's already visible. Needed because keyword-driven navigation
    /// (cmd-alt-up/down/left/right) can select a request whose row is scrolled out of the
    /// virtualized tree's viewport.
    pub(in crate::ui) fn scroll_selected_request_into_view(&self, request_id: Ulid) {
        self.scroll_tree_node_into_view(request_id);
    }

    fn scroll_tree_node_into_view(&self, node_id: Ulid) {
        let items = build_tree_render_items(&self.shell.workspace_tree);
        if let Some(index) = items
            .iter()
            .position(|item| matches!(item, TreeRenderItem::Row(row) if row.id == node_id))
        {
            self.collection_scroll_handle
                .scroll_to_item(index, ScrollStrategy::Top);
        }
    }

    /// Expands and reveals a folder without changing the active request or request view history.
    pub(in crate::ui) fn reveal_tree_folder(
        &mut self,
        folder_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.shell.workspace_tree.reveal_folder(folder_id) {
            return;
        }
        if let Err(error) = self.persist_tree_expansion_state() {
            window.push_notification(error, cx);
        }
        let items = build_tree_render_items(&self.shell.workspace_tree);
        if let Some(index) = items
            .iter()
            .position(|item| matches!(item, TreeRenderItem::Row(row) if row.id == folder_id))
        {
            self.collection_scroll_handle
                .scroll_to_item(index, ScrollStrategy::Top);
        }
        cx.notify();
    }

    /// Initializes the request view history with whatever request the shell
    /// already has selected at startup, so the very first `cmd-alt-down` / `cmd-alt-up`
    /// keypress has a meaningful anchor to step from.
    pub(in crate::ui) fn seed_request_view_history(&mut self) {
        self.request_view_histories
            .set_active_workspace(self.shell.workspace.workspace_id);
        if let Some(request_id) = self.shell.workspace_tree.selected_request_id() {
            self.request_view_histories.visit(request_id);
        }
    }

    pub(in crate::ui) fn refresh_active_request_cache(&mut self) {
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

    pub(in crate::ui) fn parent_ref_for_add_request(
        &self,
        node_id: Ulid,
    ) -> Option<RequestParentRef> {
        self.request_parent_input_for_tree_node(node_id)
            .map(|(parent, _)| parent)
    }

    pub(in crate::ui) fn parent_ref_for_add_folder(
        &self,
        node_id: Ulid,
    ) -> Option<FolderParentRef> {
        self.folder_parent_input_for_tree_node(node_id)
            .map(|(parent, _)| parent)
    }

    pub(in crate::ui) fn request_parent_input_for_tree_node(
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

    pub(in crate::ui) fn folder_parent_input_for_tree_node(
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

    pub(in crate::ui) fn request_sibling_names_in_parent(
        &self,
        parent: RequestParentRef,
    ) -> Vec<String> {
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

    pub(in crate::ui) fn folder_sibling_names_in_parent(
        &self,
        parent: FolderParentRef,
    ) -> Vec<String> {
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

    pub(in crate::ui) fn request_file_path_for_tree_node(&self, node_id: Ulid) -> Option<PathBuf> {
        self.shell
            .workspace_tree
            .node(node_id)
            .filter(|node| node.kind == TreeNodeKind::Request)
            .and_then(|node| node.manifest_path.clone())
    }

    pub(in crate::ui) fn next_new_request_name(&self, parent: RequestParentRef) -> String {
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

    pub(in crate::ui) fn next_new_folder_name(&self, parent: FolderParentRef) -> String {
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

    pub(in crate::ui) fn next_duplicate_request_name(&self, request_id: Ulid) -> Option<String> {
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

    pub(in crate::ui) fn next_duplicate_folder_name(&self, folder_id: Ulid) -> Option<String> {
        let source = self.shell.workspace_tree.node(folder_id)?;
        let parent = FolderParentRef {
            folder_id: source.parent_id,
        };
        let siblings = self.folder_sibling_names_in_parent(parent);
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

    pub(in crate::ui) fn add_request_from_tree_node(
        &mut self,
        node_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.shell.workspace_tree.node(node_id).cloned() else {
            window.push_notification("Unable to determine request parent.", cx);
            return;
        };
        let Some((parent, known_parent_manifest_path)) =
            self.request_parent_input_for_tree_node(node_id)
        else {
            window.push_notification("Unable to determine request parent.", cx);
            return;
        };
        let command_id = next_command_id();
        let command = match node.kind {
            TreeNodeKind::Folder => {
                self.pending_request_creations.insert(command_id.clone());
                AppCommand::CreateRequest {
                    input: CreateRequestInput {
                        parent,
                        known_parent_manifest_path,
                        name: self.next_new_request_name(parent),
                        method: HttpMethod::Get,
                        url: String::new(),
                    },
                    command_id,
                }
            }
            TreeNodeKind::Request => {
                self.pending_request_creations.insert(command_id.clone());
                AppCommand::CreateRequestAfter {
                    input: CreateRequestInput {
                        parent,
                        known_parent_manifest_path,
                        name: self.next_new_request_name(parent),
                        method: HttpMethod::Get,
                        url: String::new(),
                    },
                    source_request_id: node_id,
                    command_id,
                }
            }
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
    }

    pub(in crate::ui) fn add_folder_from_tree_node(
        &mut self,
        node_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.shell.workspace_tree.node(node_id).cloned() else {
            window.push_notification("Unable to determine folder parent.", cx);
            return;
        };
        let Some((parent, known_parent_manifest_path)) =
            self.folder_parent_input_for_tree_node(node_id)
        else {
            window.push_notification("Unable to determine folder parent.", cx);
            return;
        };
        let folder_name = self.next_new_folder_name(parent);
        let command_id = next_command_id();
        if node.kind == TreeNodeKind::Request {
            let Some((_, insertion_index)) =
                self.sibling_destination_for_target(node_id, TreeDropPlacement::After)
            else {
                window.push_notification("Unable to determine folder position.", cx);
                return;
            };
            self.pending_folder_placements.insert(
                command_id.clone(),
                PendingFolderPlacement::After {
                    parent,
                    insertion_index,
                    known_target_manifest_path: known_parent_manifest_path.clone(),
                },
            );
        }
        let command = AppCommand::CreateFolder {
            input: CreateFolderInput {
                parent,
                known_parent_manifest_path,
                name: folder_name,
            },
            command_id,
        };
        if let Err(error) = self.publish_app_command(command) {
            window.push_notification(error, cx);
        }
    }

    pub(in crate::ui) fn add_request_at_root(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = RequestParentRef { folder_id: None };
        let command_id = next_command_id();
        self.pending_request_creations.insert(command_id.clone());
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

    pub(in crate::ui) fn add_folder_at_root(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(in crate::ui) fn open_rename_dialog_for_tree_node(
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

    pub(in crate::ui) fn rename_tree_node_from_modal(
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

    pub(in crate::ui) fn send_request_from_tree_node(
        &mut self,
        request_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_request(request_id, window, cx);
        self.send_request(window, cx);
    }

    pub(in crate::ui) fn create_request_below_active(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.pending_request_creations.insert(command_id.clone());
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

    pub(in crate::ui) fn duplicate_selected_tree_node(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node_id) = self.shell.workspace_tree.selected_node_id() else {
            return;
        };
        let Some(node_kind) = self
            .shell
            .workspace_tree
            .node(node_id)
            .map(|node| node.kind)
        else {
            return;
        };
        match node_kind {
            TreeNodeKind::Folder => self.duplicate_folder_from_tree_node(node_id, window, cx),
            TreeNodeKind::Request => self.duplicate_request_from_tree_node(node_id, window, cx),
        }
    }

    pub(in crate::ui) fn rename_selected_tree_node(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node_id) = self.shell.workspace_tree.selected_node_id() else {
            return;
        };
        let Some(node_kind) = self
            .shell
            .workspace_tree
            .node(node_id)
            .map(|node| node.kind)
        else {
            return;
        };
        self.open_rename_dialog_for_tree_node(node_id, node_kind, window, cx);
    }

    pub(in crate::ui) fn delete_selected_tree_node(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node_id) = self.shell.workspace_tree.selected_node_id() else {
            return;
        };
        let Some(node_kind) = self
            .shell
            .workspace_tree
            .node(node_id)
            .map(|node| node.kind)
        else {
            return;
        };
        self.show_delete_tree_node_dialog(node_id, node_kind, cx);
    }

    pub(in crate::ui) fn focus_url_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.url_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(in crate::ui) fn on_action_create_request_below_active(
        &mut self,
        _: &CreateRequestBelowActive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_request_below_active(window, cx);
    }

    pub(in crate::ui) fn on_action_duplicate_active_request(
        &mut self,
        _: &DuplicateActiveRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.duplicate_selected_tree_node(window, cx);
    }

    pub(in crate::ui) fn on_action_rename_active_request(
        &mut self,
        _: &RenameActiveRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rename_selected_tree_node(window, cx);
    }

    pub(in crate::ui) fn on_action_delete_selected_tree_node(
        &mut self,
        _: &DeleteSelectedTreeNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selected_tree_node(window, cx);
    }

    pub(in crate::ui) fn on_action_focus_url_input(
        &mut self,
        _: &FocusUrlInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_url_input(window, cx);
    }

    pub(in crate::ui) fn on_action_send_active_request(
        &mut self,
        _: &SendActiveRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_send_or_cancel_action(window, cx);
    }

    pub(in crate::ui) fn on_action_format_request_body(
        &mut self,
        _: &FormatRequestBody,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_request_body(window, cx);
    }

    pub(in crate::ui) fn on_action_format_response_body(
        &mut self,
        _: &FormatResponseBody,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_response_body(window, cx);
    }

    pub(in crate::ui) fn on_action_tree_menu_send_request(
        &mut self,
        action: &TreeMenuSendRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_request_from_tree_node(action.0, window, cx);
    }

    pub(in crate::ui) fn on_action_tree_menu_copy_as_curl(
        &mut self,
        action: &TreeMenuCopyAsCurl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_request_as_curl_from_tree_node(action.0, window, cx);
    }

    pub(in crate::ui) fn on_action_tree_menu_add_request_in_folder(
        &mut self,
        action: &TreeMenuAddRequestInFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_request_from_tree_node(action.0, window, cx);
    }

    pub(in crate::ui) fn on_action_tree_menu_add_folder_in_folder(
        &mut self,
        action: &TreeMenuAddFolderInFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_folder_from_tree_node(action.0, window, cx);
    }

    pub(in crate::ui) fn on_action_tree_menu_rename(
        &mut self,
        action: &TreeMenuRename,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row_id = action.0;
        let kind = self
            .shell
            .workspace_tree
            .node(row_id)
            .map(|n| n.kind)
            .unwrap_or(TreeNodeKind::Request);
        self.open_rename_dialog_for_tree_node(row_id, kind, window, cx);
    }

    pub(in crate::ui) fn on_action_tree_menu_delete(
        &mut self,
        action: &TreeMenuDelete,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row_id = action.0;
        let kind = self
            .shell
            .workspace_tree
            .node(row_id)
            .map(|n| n.kind)
            .unwrap_or(TreeNodeKind::Request);
        self.show_delete_tree_node_dialog(row_id, kind, cx);
    }

    pub(in crate::ui) fn on_action_tree_menu_duplicate_request(
        &mut self,
        action: &TreeMenuDuplicateRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.duplicate_request_from_tree_node(action.0, window, cx);
    }

    pub(in crate::ui) fn on_action_tree_menu_duplicate_folder(
        &mut self,
        action: &TreeMenuDuplicateFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.duplicate_folder_from_tree_node(action.0, window, cx);
    }

    pub(in crate::ui) fn on_action_tree_menu_add_request_at_root(
        &mut self,
        _: &TreeMenuAddRequestAtRoot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_request_at_root(window, cx);
    }

    pub(in crate::ui) fn on_action_tree_menu_add_folder_at_root(
        &mut self,
        _: &TreeMenuAddFolderAtRoot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_folder_at_root(window, cx);
    }

    pub(in crate::ui) fn duplicate_request_from_tree_node(
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
        self.pending_request_creations.insert(command_id.clone());
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

    pub(in crate::ui) fn duplicate_folder_from_tree_node(
        &mut self,
        folder_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(duplicate_name) = self.next_duplicate_folder_name(folder_id) else {
            window.push_notification("Unable to duplicate this folder.", cx);
            return;
        };
        let Some(source) = self.shell.workspace_tree.node(folder_id) else {
            window.push_notification("Unable to determine folder parent.", cx);
            return;
        };
        let parent = FolderParentRef {
            folder_id: source.parent_id,
        };
        let siblings = source
            .parent_id
            .and_then(|parent_id| {
                self.shell
                    .workspace_tree
                    .node(parent_id)
                    .map(|node| node.children.as_slice())
            })
            .unwrap_or(self.shell.workspace_tree.roots());
        let insertion_index = siblings
            .iter()
            .position(|id| *id == folder_id)
            .map(|index| index + 1)
            .unwrap_or(siblings.len());
        let command_id = next_command_id();
        self.pending_folder_placements.insert(
            command_id.clone(),
            PendingFolderPlacement::After {
                parent,
                insertion_index,
                known_target_manifest_path: self
                    .folder_parent_input_for_tree_node(folder_id)
                    .and_then(|(_, path)| path),
            },
        );
        if let Err(error) = self.publish_app_command(AppCommand::DuplicateFolder {
            input: DuplicateFolderInput {
                folder_id,
                duplicate_name,
                parent,
            },
            command_id,
        }) {
            window.push_notification(error, cx);
        }
    }

    pub(in crate::ui) fn delete_request_from_tree_node(
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

    pub(in crate::ui) fn delete_folder_from_tree_node(
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
}
