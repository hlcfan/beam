use super::*;

impl BeamView {
    pub(in crate::ui) fn tree_drag_preview_for_request(
        dragged: &DraggedRequest,
        position: Point<Pixels>,
        _: &mut Window,
        cx: &mut App,
    ) -> Entity<TreeDragPreview> {
        cx.new(|_| TreeDragPreview::new(dragged.label.clone(), TreeNodeKind::Request, position))
    }

    pub(in crate::ui) fn tree_drag_preview_for_folder(
        dragged: &DraggedFolder,
        position: Point<Pixels>,
        _: &mut Window,
        cx: &mut App,
    ) -> Entity<TreeDragPreview> {
        cx.new(|_| TreeDragPreview::new(dragged.label.clone(), TreeNodeKind::Folder, position))
    }

    pub(in crate::ui) fn path_has_ancestor_in_tree(
        &self,
        start_id: Ulid,
        ancestor_id: Ulid,
    ) -> bool {
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

    pub(in crate::ui) fn has_name_conflict_in_scope(
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

    /// Tells the user why a drop was rejected when the destination folder
    /// already contains an item with the same name. Called instead of
    /// silently no-op'ing the drop.
    pub(in crate::ui) fn notify_tree_move_name_conflict(
        &self,
        moving_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.shell.workspace_tree.node(moving_id) else {
            return;
        };
        window.push_notification(
            format!(
                "Can't move \"{}\" here — an item with that name already exists in the destination.",
                node.name
            ),
            cx,
        );
    }

    pub(in crate::ui) fn request_parent_input_for_parent_node(
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

    pub(in crate::ui) fn folder_parent_input_for_parent_node(
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

    pub(in crate::ui) fn sibling_destination_for_target(
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

    /// Resolves the `(destination_parent_id, insertion_index)` a request drop
    /// would target, without checking whether the move is actually allowed
    /// (e.g. name conflicts). Shared by the real move-action builder and by
    /// the name-conflict-only check used to decide hover/notification UX.
    pub(in crate::ui) fn request_move_destination(
        &self,
        request_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> Option<(Option<Ulid>, usize)> {
        let request_node = self.shell.workspace_tree.node(request_id)?;
        if request_node.kind != TreeNodeKind::Request {
            return None;
        }

        match placement {
            TreeDropPlacement::Into => {
                let target = self.shell.workspace_tree.node(target_id)?;
                if target.kind != TreeNodeKind::Folder {
                    return None;
                }
                Some((Some(target.id), target.children.len()))
            }
            TreeDropPlacement::Before | TreeDropPlacement::After => {
                if target_id == request_id {
                    return None;
                }
                self.sibling_destination_for_target(target_id, placement)
            }
        }
    }

    pub(in crate::ui) fn request_move_action(
        &self,
        request_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> Option<TreeMoveAction> {
        let (destination_parent_id, insertion_index) =
            self.request_move_destination(request_id, target_id, placement)?;
        self.request_move_action_for_destination(request_id, destination_parent_id, insertion_index)
    }

    /// True when a request drop at `target_id`/`placement` would otherwise be
    /// valid and is rejected solely because of a sibling name conflict at the
    /// destination. Lets callers still highlight the target and explain the
    /// rejection instead of behaving as if the drop were structurally invalid.
    pub(in crate::ui) fn request_move_blocked_by_name_conflict(
        &self,
        request_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> bool {
        let Some((destination_parent_id, insertion_index)) =
            self.request_move_destination(request_id, target_id, placement)
        else {
            return false;
        };
        self.request_move_destination_viable(request_id, destination_parent_id, insertion_index)
            && self
                .request_move_action_for_destination(
                    request_id,
                    destination_parent_id,
                    insertion_index,
                )
                .is_none()
    }

    pub(in crate::ui) fn request_move_action_for_destination(
        &self,
        request_id: Ulid,
        destination_parent_id: Option<Ulid>,
        insertion_index: usize,
    ) -> Option<TreeMoveAction> {
        self.request_move_action_for_destination_impl(
            request_id,
            destination_parent_id,
            insertion_index,
            true,
        )
    }

    /// Same as [`Self::request_move_action_for_destination`] but ignores
    /// sibling name conflicts, so it reports whether the destination is
    /// otherwise structurally valid.
    pub(in crate::ui) fn request_move_destination_viable(
        &self,
        request_id: Ulid,
        destination_parent_id: Option<Ulid>,
        insertion_index: usize,
    ) -> bool {
        self.request_move_action_for_destination_impl(
            request_id,
            destination_parent_id,
            insertion_index,
            false,
        )
        .is_some()
    }

    pub(in crate::ui) fn request_move_action_for_destination_impl(
        &self,
        request_id: Ulid,
        destination_parent_id: Option<Ulid>,
        insertion_index: usize,
        check_name_conflict: bool,
    ) -> Option<TreeMoveAction> {
        let request_node = self.shell.workspace_tree.node(request_id)?;
        if request_node.kind != TreeNodeKind::Request {
            return None;
        }
        if check_name_conflict
            && self.has_name_conflict_in_scope(
                destination_parent_id,
                request_id,
                &request_node.name,
            )
        {
            return None;
        }

        let (new_parent, known_target_manifest_path) =
            self.request_parent_input_for_parent_node(destination_parent_id)?;
        Some(TreeMoveAction::MoveRequest(MoveRequestInput {
            request_id,
            new_parent,
            insertion_index: self.adjust_insertion_index_for_same_container_move(
                destination_parent_id,
                insertion_index,
                request_id,
            ),
            known_request_path: request_node.manifest_path.clone(),
            known_target_manifest_path,
        }))
    }

    /// Adjusts `insertion_index` for an in-container move so the dragged item
    /// lands at the intended position.
    ///
    /// Callers compute `insertion_index` against the destination container's
    /// current sibling list (before the move). The move helpers remove the
    /// dragged item from its old slot *before* inserting at `insertion_index`.
    /// When the dragged item already lives in the destination container at an
    /// index below the insertion point, that removal shifts every later item
    /// down by one, so the raw index would land one position too far. We
    /// compensate by decrementing here. Cross-container moves and moves where
    /// the dragged item sits at or above the insertion point are unaffected.
    pub(in crate::ui) fn adjust_insertion_index_for_same_container_move(
        &self,
        destination_parent_id: Option<Ulid>,
        insertion_index: usize,
        dragged_id: Ulid,
    ) -> usize {
        let siblings: Vec<Ulid> = if let Some(folder_id) = destination_parent_id {
            self.shell
                .workspace_tree
                .node(folder_id)
                .map(|node| node.children.clone())
                .unwrap_or_default()
        } else {
            self.shell.workspace_tree.roots().to_vec()
        };
        let Some(dragged_idx) = siblings.iter().position(|id| *id == dragged_id) else {
            return insertion_index;
        };
        if dragged_idx < insertion_index {
            insertion_index.saturating_sub(1)
        } else {
            insertion_index
        }
    }

    /// Resolves the `(destination_parent_id, insertion_index)` a folder drop
    /// would target, without checking whether the move is actually allowed
    /// (e.g. name conflicts). Shared by the real move-action builder and by
    /// the name-conflict-only check used to decide hover/notification UX.
    pub(in crate::ui) fn folder_move_destination(
        &self,
        folder_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> Option<(Option<Ulid>, usize)> {
        let folder_node = self.shell.workspace_tree.node(folder_id)?;
        if folder_node.kind != TreeNodeKind::Folder {
            return None;
        }

        match placement {
            TreeDropPlacement::Into => {
                if target_id == folder_id {
                    return None;
                }
                let target = self.shell.workspace_tree.node(target_id)?;
                if target.kind != TreeNodeKind::Folder {
                    return None;
                }
                Some((Some(target.id), target.children.len()))
            }
            TreeDropPlacement::Before | TreeDropPlacement::After => {
                if target_id == folder_id {
                    return None;
                }
                self.sibling_destination_for_target(target_id, placement)
            }
        }
    }

    pub(in crate::ui) fn folder_move_action(
        &self,
        folder_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> Option<TreeMoveAction> {
        let (destination_parent_id, insertion_index) =
            self.folder_move_destination(folder_id, target_id, placement)?;
        self.folder_move_action_for_destination(folder_id, destination_parent_id, insertion_index)
    }

    /// True when a folder drop at `target_id`/`placement` would otherwise be
    /// valid and is rejected solely because of a sibling name conflict at the
    /// destination. Lets callers still highlight the target and explain the
    /// rejection instead of behaving as if the drop were structurally invalid.
    pub(in crate::ui) fn folder_move_blocked_by_name_conflict(
        &self,
        folder_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> bool {
        let Some((destination_parent_id, insertion_index)) =
            self.folder_move_destination(folder_id, target_id, placement)
        else {
            return false;
        };
        self.folder_move_destination_viable(folder_id, destination_parent_id, insertion_index)
            && self
                .folder_move_action_for_destination(
                    folder_id,
                    destination_parent_id,
                    insertion_index,
                )
                .is_none()
    }

    pub(in crate::ui) fn folder_move_action_for_destination(
        &self,
        folder_id: Ulid,
        destination_parent_id: Option<Ulid>,
        insertion_index: usize,
    ) -> Option<TreeMoveAction> {
        self.folder_move_action_for_destination_impl(
            folder_id,
            destination_parent_id,
            insertion_index,
            true,
        )
    }

    /// Same as [`Self::folder_move_action_for_destination`] but ignores
    /// sibling name conflicts, so it reports whether the destination is
    /// otherwise structurally valid.
    pub(in crate::ui) fn folder_move_destination_viable(
        &self,
        folder_id: Ulid,
        destination_parent_id: Option<Ulid>,
        insertion_index: usize,
    ) -> bool {
        self.folder_move_action_for_destination_impl(
            folder_id,
            destination_parent_id,
            insertion_index,
            false,
        )
        .is_some()
    }

    pub(in crate::ui) fn folder_move_action_for_destination_impl(
        &self,
        folder_id: Ulid,
        destination_parent_id: Option<Ulid>,
        insertion_index: usize,
        check_name_conflict: bool,
    ) -> Option<TreeMoveAction> {
        let folder_node = self.shell.workspace_tree.node(folder_id)?;
        if folder_node.kind != TreeNodeKind::Folder {
            return None;
        }
        if destination_parent_id == Some(folder_id)
            || destination_parent_id.is_some_and(|id| self.path_has_ancestor_in_tree(id, folder_id))
        {
            return None;
        }
        if check_name_conflict
            && self.has_name_conflict_in_scope(destination_parent_id, folder_id, &folder_node.name)
        {
            return None;
        }

        let (new_parent, known_target_manifest_path) =
            self.folder_parent_input_for_parent_node(destination_parent_id)?;
        Some(TreeMoveAction::MoveFolder(MoveFolderInput {
            folder_id,
            new_parent,
            insertion_index: self.adjust_insertion_index_for_same_container_move(
                destination_parent_id,
                insertion_index,
                folder_id,
            ),
            known_folder_manifest_path: folder_node.manifest_path.clone(),
            known_target_manifest_path,
        }))
    }

    pub(in crate::ui) fn can_accept_tree_drop(
        &self,
        dragged_value: &dyn Any,
        target_id: Ulid,
        placement: TreeDropPlacement,
    ) -> bool {
        if let Some(dragged) = dragged_value.downcast_ref::<DraggedRequest>() {
            return self
                .request_move_action(dragged.request_id, target_id, placement)
                .is_some()
                || self.request_move_blocked_by_name_conflict(
                    dragged.request_id,
                    target_id,
                    placement,
                );
        }
        if let Some(dragged) = dragged_value.downcast_ref::<DraggedFolder>() {
            return self
                .folder_move_action(dragged.folder_id, target_id, placement)
                .is_some()
                || self.folder_move_blocked_by_name_conflict(
                    dragged.folder_id,
                    target_id,
                    placement,
                );
        }
        false
    }

    /// Resolves a slot into `(destination_parent_id, insertion_index)` for the
    /// existing move-action helpers. Container slots map to their container;
    /// item-after slots map to the position just after the anchored child.
    pub(in crate::ui) fn slot_to_destination(
        &self,
        slot: &TreeDropSlot,
    ) -> Option<(Option<Ulid>, usize)> {
        let container_id = slot.container_id;
        let siblings: Vec<Ulid> = if let Some(folder_id) = container_id {
            self.shell
                .workspace_tree
                .node(folder_id)
                .map(|node| node.children.clone())
                .unwrap_or_default()
        } else {
            self.shell.workspace_tree.roots().to_vec()
        };

        let insertion_index = match slot.visual_role {
            crate::tree_dnd::SlotVisualRole::ContainerStart => 0,
            crate::tree_dnd::SlotVisualRole::ContainerEnd => siblings.len(),
            crate::tree_dnd::SlotVisualRole::ItemAfter => {
                let target_id = slot.target_id?;
                siblings.iter().position(|id| *id == target_id)? + 1
            }
        };
        Some((container_id, insertion_index))
    }

    pub(in crate::ui) fn can_accept_tree_drop_slot(
        &self,
        dragged_value: &dyn Any,
        slot: &TreeDropSlot,
    ) -> bool {
        if let Some(dragged) = dragged_value.downcast_ref::<DraggedRequest>() {
            if slot.target_id == Some(dragged.request_id) {
                return false;
            }
            let Some((destination_parent_id, insertion_index)) = self.slot_to_destination(slot)
            else {
                return false;
            };
            return self
                .request_move_action_for_destination(
                    dragged.request_id,
                    destination_parent_id,
                    insertion_index,
                )
                .is_some()
                || self.request_move_slot_blocked_by_name_conflict(dragged.request_id, slot);
        }
        if let Some(dragged) = dragged_value.downcast_ref::<DraggedFolder>() {
            if slot.target_id == Some(dragged.folder_id) {
                return false;
            }
            let Some((destination_parent_id, insertion_index)) = self.slot_to_destination(slot)
            else {
                return false;
            };
            return self
                .folder_move_action_for_destination(
                    dragged.folder_id,
                    destination_parent_id,
                    insertion_index,
                )
                .is_some()
                || self.folder_move_slot_blocked_by_name_conflict(dragged.folder_id, slot);
        }
        false
    }

    /// Slot-based counterpart to [`Self::request_move_blocked_by_name_conflict`],
    /// used when the drop lands on a between-items slot (e.g. an expanded
    /// folder's "insert as first/last child" affordance) instead of a row body.
    pub(in crate::ui) fn request_move_slot_blocked_by_name_conflict(
        &self,
        request_id: Ulid,
        slot: &TreeDropSlot,
    ) -> bool {
        let Some((destination_parent_id, insertion_index)) = self.slot_to_destination(slot) else {
            return false;
        };
        self.request_move_destination_viable(request_id, destination_parent_id, insertion_index)
            && self
                .request_move_action_for_destination(
                    request_id,
                    destination_parent_id,
                    insertion_index,
                )
                .is_none()
    }

    /// Slot-based counterpart to [`Self::folder_move_blocked_by_name_conflict`].
    pub(in crate::ui) fn folder_move_slot_blocked_by_name_conflict(
        &self,
        folder_id: Ulid,
        slot: &TreeDropSlot,
    ) -> bool {
        let Some((destination_parent_id, insertion_index)) = self.slot_to_destination(slot) else {
            return false;
        };
        self.folder_move_destination_viable(folder_id, destination_parent_id, insertion_index)
            && self
                .folder_move_action_for_destination(
                    folder_id,
                    destination_parent_id,
                    insertion_index,
                )
                .is_none()
    }

    pub(in crate::ui) fn tree_row_body_drop_placement(
        target_kind: TreeNodeKind,
    ) -> Option<TreeDropPlacement> {
        match target_kind {
            TreeNodeKind::Folder => Some(TreeDropPlacement::Into),
            TreeNodeKind::Request => None,
        }
    }

    pub(in crate::ui) fn update_tree_drag_hover(
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
            self.clear_tree_drag_row_hover(cx);
            return;
        };

        if !self.can_accept_tree_drop(dragged, target_id, placement) {
            self.clear_tree_drag_row_hover(cx);
            return;
        }

        let new_hover = Some((target_id, placement));
        if self.tree_drag_hover != new_hover {
            self.tree_drag_hover = new_hover;
            cx.notify();
        }
    }

    /// Sets the active drag slot, clearing any row-body hover so the slot takes
    /// visual priority. The row-body hover will be re-asserted by the row's own
    /// `on_drag_move` handler if the cursor is still over a folder body; the
    /// slot still wins because rendering checks `tree_drag_slot_hover` first.
    pub(in crate::ui) fn set_tree_drag_slot_hover(
        &mut self,
        slot: TreeDropSlot,
        cx: &mut Context<Self>,
    ) {
        if self.tree_drag_slot_hover != Some(slot) || self.tree_drag_hover.is_some() {
            self.tree_drag_slot_hover = Some(slot);
            self.tree_drag_hover = None;
            cx.notify();
        }
    }

    /// Clears only the slot hover, leaving row-body hover untouched.
    pub(in crate::ui) fn clear_tree_drag_slot_hover(&mut self, cx: &mut Context<Self>) {
        if self.tree_drag_slot_hover.is_some() {
            self.tree_drag_slot_hover = None;
            cx.notify();
        }
    }

    /// Clears only the row-body hover, leaving slot hover untouched.
    pub(in crate::ui) fn clear_tree_drag_row_hover(&mut self, cx: &mut Context<Self>) {
        if self.tree_drag_hover.is_some() {
            self.tree_drag_hover = None;
            cx.notify();
        }
    }

    /// Clears both slot and row-body hover. Called on drop or drag end.
    pub(in crate::ui) fn clear_tree_drag_hover(&mut self, cx: &mut Context<Self>) {
        self.tree_drag_scroll_task.take();
        if self.tree_drag_hover.is_some() || self.tree_drag_slot_hover.is_some() {
            self.tree_drag_hover = None;
            self.tree_drag_slot_hover = None;
            cx.notify();
        }
    }

    /// Recomputes tree auto-scroll for a drag at `position` within a container whose viewport is
    /// `bounds`. Called from the tree pane's `on_drag_move` handlers so that dragging a row or
    /// folder near the top or bottom edge of the (possibly virtualized/clipped) tree scrolls it,
    /// instead of requiring the user to scroll manually mid-drag.
    pub(in crate::ui) fn update_tree_drag_autoscroll(
        &mut self,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Dropping the old task cancels its loop; a new one is spawned below
        // only if the cursor is still with an edge zone.
        self.tree_drag_scroll_task.take();

        if !bounds.contains(&position) {
            return;
        }

        let distance_from_top = f32::from(position.y - bounds.origin.y);
        let distance_from_bottom = f32::from(bounds.origin.y + bounds.size.height - position.y);
        let delta = if distance_from_top <= TREE_DRAG_SCROLL_FAST_ZONE_PX {
            TREE_DRAG_SCROLL_FAST_STEP_PX
        } else if distance_from_top <= TREE_DRAG_SCROLL_SLOW_ZONE_PX {
            TREE_DRAG_SCROLL_SLOW_STEP_PX
        } else if distance_from_bottom <= TREE_DRAG_SCROLL_FAST_ZONE_PX {
            -TREE_DRAG_SCROLL_FAST_STEP_PX
        } else if distance_from_bottom <= TREE_DRAG_SCROLL_SLOW_ZONE_PX {
            -TREE_DRAG_SCROLL_SLOW_STEP_PX
        } else {
            return;
        };

        let handle = self.collection_scroll_handle.clone();
        self.tree_drag_scroll_task = Some(cx.spawn_in(window, async move |view, cx| {
            loop {
                let updated = view.update(cx, |_, cx| {
                    let offset = handle.offset();
                    let max_offset = handle.max_offset();
                    let new_y = (offset.y + px(delta)).clamp(-max_offset.y, px(0.0));
                    handle.set_offset(point(offset.x, new_y));
                    cx.notify();
                });

                if updated.is_err() {
                    return;
                }
                cx.background_executor().timer(TREE_DRAG_SCROLL_TICK).await;
            }
        }));
    }

    pub(in crate::ui) fn perform_tree_move_action(
        &mut self,
        action: TreeMoveAction,
        _preferred_selected_request_id: Option<Ulid>,
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
            }
            TreeMoveAction::MoveFolder(input) => {
                let command = AppCommand::MoveFolder {
                    input,
                    command_id: next_command_id(),
                };
                if let Err(error) = self.publish_app_command(command) {
                    window.push_notification(error, cx);
                }
            }
        }
    }

    pub(in crate::ui) fn handle_request_tree_drop(
        &mut self,
        request_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.request_move_action(request_id, target_id, placement) else {
            if self.request_move_blocked_by_name_conflict(request_id, target_id, placement) {
                self.notify_tree_move_name_conflict(request_id, window, cx);
            }
            return;
        };
        let expand_target_id = (placement == TreeDropPlacement::Into).then_some(target_id);
        self.perform_tree_move_action(action, Some(request_id), expand_target_id, window, cx);
    }

    pub(in crate::ui) fn handle_folder_tree_drop(
        &mut self,
        folder_id: Ulid,
        target_id: Ulid,
        placement: TreeDropPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.folder_move_action(folder_id, target_id, placement) else {
            if self.folder_move_blocked_by_name_conflict(folder_id, target_id, placement) {
                self.notify_tree_move_name_conflict(folder_id, window, cx);
            }
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

    pub(in crate::ui) fn handle_request_tree_drop_slot(
        &mut self,
        request_id: Ulid,
        slot: &TreeDropSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((destination_parent_id, insertion_index)) = self.slot_to_destination(slot) else {
            return;
        };
        let Some(action) = self.request_move_action_for_destination(
            request_id,
            destination_parent_id,
            insertion_index,
        ) else {
            if self.request_move_slot_blocked_by_name_conflict(request_id, slot) {
                self.notify_tree_move_name_conflict(request_id, window, cx);
            }
            return;
        };
        self.perform_tree_move_action(action, Some(request_id), None, window, cx);
    }

    pub(in crate::ui) fn handle_folder_tree_drop_slot(
        &mut self,
        folder_id: Ulid,
        slot: &TreeDropSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((destination_parent_id, insertion_index)) = self.slot_to_destination(slot) else {
            return;
        };
        let Some(action) = self.folder_move_action_for_destination(
            folder_id,
            destination_parent_id,
            insertion_index,
        ) else {
            if self.folder_move_slot_blocked_by_name_conflict(folder_id, slot) {
                self.notify_tree_move_name_conflict(folder_id, window, cx);
            }
            return;
        };
        let preferred_selected_request_id = self.shell.workspace_tree.selected_request_id();
        self.perform_tree_move_action(action, preferred_selected_request_id, None, window, cx);
    }

    pub(in crate::ui) fn render_tree_drop_slot(
        &self,
        slot: &TreeDropSlot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let depth_inset = tree_depth_inset(slot.depth);
        let slot_copy = *slot;
        let is_active_slot = self.tree_drag_slot_hover == Some(slot_copy);
        let highlight = cx.theme().drag_border;

        div()
            .h(px(SLOT_HIT_HEIGHT_PX))
            .ml(px(depth_inset))
            .mr(px(SLOT_RIGHT_PAD_PX))
            .on_drag_move(
                cx.listener(move |this, drag: &DragMoveEvent<DraggedRequest>, _, cx| {
                    let bounds = drag.bounds;
                    let position = drag.event.position;
                    let in_x = position.x >= bounds.origin.x
                        && position.x <= bounds.origin.x + bounds.size.width;
                    let proximity = px(SLOT_DRAG_PROXIMITY_PX);
                    let in_y = position.y >= bounds.origin.y - proximity
                        && position.y <= bounds.origin.y + bounds.size.height + proximity;
                    if in_x && in_y {
                        let dragged = drag.drag(cx).clone();
                        if this.can_accept_tree_drop_slot(&dragged, &slot_copy) {
                            this.set_tree_drag_slot_hover(slot_copy, cx);
                        } else if this.tree_drag_slot_hover == Some(slot_copy) {
                            this.clear_tree_drag_slot_hover(cx);
                        }
                    } else if this.tree_drag_slot_hover == Some(slot_copy) {
                        this.clear_tree_drag_slot_hover(cx);
                    }
                }),
            )
            .on_drag_move(
                cx.listener(move |this, drag: &DragMoveEvent<DraggedFolder>, _, cx| {
                    let bounds = drag.bounds;
                    let position = drag.event.position;
                    let in_x = position.x >= bounds.origin.x
                        && position.x <= bounds.origin.x + bounds.size.width;
                    let proximity = px(SLOT_DRAG_PROXIMITY_PX);
                    let in_y = position.y >= bounds.origin.y - proximity
                        && position.y <= bounds.origin.y + bounds.size.height + proximity;
                    if in_x && in_y {
                        let dragged = drag.drag(cx).clone();
                        if this.can_accept_tree_drop_slot(&dragged, &slot_copy) {
                            this.set_tree_drag_slot_hover(slot_copy, cx);
                        } else if this.tree_drag_slot_hover == Some(slot_copy) {
                            this.clear_tree_drag_slot_hover(cx);
                        }
                    } else if this.tree_drag_slot_hover == Some(slot_copy) {
                        this.clear_tree_drag_slot_hover(cx);
                    }
                }),
            )
            .child(
                div()
                    .h(px(SLOT_BAR_HEIGHT_PX))
                    .w_full()
                    .rounded(px(SLOT_BAR_HEIGHT_PX / 2.0))
                    .when(is_active_slot, |this| this.bg(highlight)),
            )
            .into_any_element()
    }
}
