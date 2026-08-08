use super::*;

impl BeamView {
    pub(in crate::ui) fn render_tree_row(
        &self,
        row: &TreeRowViewModel,
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
        let indent = px(tree_depth_inset(row.depth));

        let mut row_content = h_flex()
            .w_full()
            .items_center()
            .justify_start()
            .gap_2()
            .text_sm();
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
        let tooltip_label = label.clone();
        row_content = row_content.child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .id(format!("tree-row-label-{}", row.id))
                .tooltip(move |window, cx| Tooltip::new(tooltip_label.clone()).build(window, cx))
                .child(label.clone()),
        );

        let row_data = crate::app_shell::TreeRow {
            id: row.id,
            kind: row.kind,
            depth: row.depth,
            selected: row.selected,
        };
        let row_id = row.id;
        let row_kind = row.kind;
        let body_view = cx.entity();
        let drag_hover = self.tree_drag_hover;
        let drag_slot_hover = self.tree_drag_slot_hover;
        let mut body = div()
            .id(format!("tree-row-body-{}", row_id))
            .cursor_pointer()
            .child(
                ListItem::new(format!("tree-row-{}", row_id))
                    .w_full()
                    .rounded(px(8.0))
                    .py_1()
                    .pr(px(6.0))
                    .pl(indent)
                    .selected(row.selected)
                    .when(
                        drag_hover
                            .is_some_and(|(id, p)| id == row_id && p == TreeDropPlacement::Into)
                            && drag_slot_hover.is_none(),
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
                            this.focus_handle.focus(window, cx);
                            this.select_request(row_id, window, cx);
                            if let Err(error) = this.persist_last_opened_request_id(row_id) {
                                window.push_notification(error, cx);
                            }
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
        if Self::tree_row_body_drop_placement(row_kind).is_some() {
            body = body
                .can_drop(move |dragged_value, _window, app| {
                    body_view.update(app, |this, _| {
                        if this.tree_drag_slot_hover.is_some() {
                            return false;
                        }
                        let Some(placement) = Self::tree_row_body_drop_placement(row_kind) else {
                            return false;
                        };
                        this.can_accept_tree_drop(dragged_value, row_id, placement)
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
                ));
        } else {
            body = body
                .on_drag_move(cx.listener(
                    move |this, drag: &DragMoveEvent<DraggedRequest>, _, cx| {
                        let bounds = drag.bounds;
                        let position = drag.event.position;
                        if position.y >= bounds.origin.y
                            && position.y <= bounds.origin.y + bounds.size.height
                        {
                            this.clear_tree_drag_row_hover(cx);
                        }
                    },
                ))
                .on_drag_move(cx.listener(
                    move |this, drag: &DragMoveEvent<DraggedFolder>, _, cx| {
                        let bounds = drag.bounds;
                        let position = drag.event.position;
                        if position.y >= bounds.origin.y
                            && position.y <= bounds.origin.y + bounds.size.height
                        {
                            this.clear_tree_drag_row_hover(cx);
                        }
                    },
                ));
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

        body.into_any_element()
    }

    pub(in crate::ui) fn build_tree_row_context_menu(
        &self,
        row: crate::app_shell::TreeRow,
    ) -> NativeMenu {
        let row_id = row.id;
        let row_kind = row.kind;
        let menu = NativeMenu::new();
        match row_kind {
            TreeNodeKind::Folder => {
                let menu = self.build_tree_create_context_menu_group(menu, row_id);
                let menu = menu.separator();
                let menu = append_with_image_or_plain(
                    menu,
                    "Rename",
                    "icons/edit.svg",
                    false,
                    Box::new(TreeMenuRename(row_id)),
                );
                let menu = append_with_image_or_plain(
                    menu,
                    "Duplicate",
                    "icons/duplicate.svg",
                    false,
                    Box::new(TreeMenuDuplicateFolder(row_id)),
                );
                append_with_image_or_plain(
                    menu,
                    "Delete",
                    "icons/trash.svg",
                    false,
                    Box::new(TreeMenuDelete(row_id)),
                )
            }
            TreeNodeKind::Request => {
                let menu = append_with_image_or_plain(
                    menu,
                    "Send Request",
                    "icons/send.svg",
                    false,
                    Box::new(TreeMenuSendRequest(row_id)),
                );
                let menu = append_with_image_or_plain(
                    menu,
                    "Copy as cURL",
                    "icons/copy.svg",
                    false,
                    Box::new(TreeMenuCopyAsCurl(row_id)),
                );
                let menu = menu.separator();
                let menu = self.build_tree_create_context_menu_group(menu, row_id);
                let menu = menu.separator();
                let menu = append_with_image_or_plain(
                    menu,
                    "Rename",
                    "icons/edit.svg",
                    false,
                    Box::new(TreeMenuRename(row_id)),
                );
                let menu = append_with_image_or_plain(
                    menu,
                    "Duplicate",
                    "icons/duplicate.svg",
                    false,
                    Box::new(TreeMenuDuplicateRequest(row_id)),
                );
                append_with_image_or_plain(
                    menu,
                    "Delete",
                    "icons/trash.svg",
                    false,
                    Box::new(TreeMenuDelete(row_id)),
                )
            }
        }
    }

    pub(in crate::ui) fn build_tree_create_context_menu_group(
        &self,
        menu: NativeMenu,
        row_id: Ulid,
    ) -> NativeMenu {
        let menu = append_with_image_or_plain(
            menu,
            "HTTP",
            "icons/add.svg",
            false,
            Box::new(TreeMenuAddRequestInFolder(row_id)),
        );
        append_with_image_or_plain(
            menu,
            "Folder",
            "icons/folder-add.svg",
            false,
            Box::new(TreeMenuAddFolderInFolder(row_id)),
        )
    }

    pub(in crate::ui) fn build_empty_space_context_menu(&self) -> NativeMenu {
        let menu = NativeMenu::new();
        let menu = append_with_image_or_plain(
            menu,
            "HTTP",
            "icons/add.svg",
            false,
            Box::new(TreeMenuAddRequestAtRoot),
        );
        append_with_image_or_plain(
            menu,
            "Folder",
            "icons/folder-add.svg",
            false,
            Box::new(TreeMenuAddFolderAtRoot),
        )
    }

    pub(in crate::ui) fn render_workspace_panel(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut panel = v_flex()
            .h_full()
            .w_full()
            .gap(px(2.0))
            .pt_0()
            .px_2()
            .pb_2()
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

        let items = build_tree_render_items(&self.shell.workspace_tree);
        let is_empty = self.shell.workspace_tree.roots().is_empty();
        if is_empty {
            let view = cx.entity();
            let empty_view = view.clone();
            let root_slot = TreeDropSlot {
                depth: 0,
                target_id: None,
                target_kind: None,
                placement: TreeDropPlacement::Before,
                container_id: None,
                visual_role: crate::tree_dnd::SlotVisualRole::ContainerStart,
            };
            panel.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .can_drop(move |_dragged_value, _window, app| {
                        empty_view.update(app, |this, _| this.tree_drag_slot_hover.is_some())
                    })
                    .on_drop(
                        cx.listener(move |this, dragged: &DraggedRequest, window, cx| {
                            if let Some(slot) = this.tree_drag_slot_hover {
                                this.handle_request_tree_drop_slot(
                                    dragged.request_id,
                                    &slot,
                                    window,
                                    cx,
                                );
                            }
                            this.clear_tree_drag_hover(cx);
                        }),
                    )
                    .on_drop(
                        cx.listener(move |this, dragged: &DraggedFolder, window, cx| {
                            if let Some(slot) = this.tree_drag_slot_hover {
                                this.handle_folder_tree_drop_slot(
                                    dragged.folder_id,
                                    &slot,
                                    window,
                                    cx,
                                );
                            }
                            this.clear_tree_drag_hover(cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        move |ev: &MouseDownEvent, window, cx| {
                            view.update(cx, |this, cx| {
                                this.focus_handle.focus(window, cx);
                                let menu = this.build_empty_space_context_menu();
                                let position = Point {
                                    x: ev.position.x + px(4.),
                                    y: ev.position.y,
                                };
                                menu.show(position, window, cx);
                            });
                        },
                    )
                    .child(
                        v_flex()
                            .size_full()
                            .child(self.render_tree_drop_slot(&root_slot, cx))
                            .child(
                                div().flex_1().flex().items_center().justify_center().child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("No requests yet"),
                                ),
                            ),
                    ),
            )
        } else {
            let view = cx.entity();
            let menu_view = view.clone();
            let scroll_view = view.clone();
            let list_view = view.clone();
            // v_virtual_list lays items out at their declared heights, so we
            // must publish a per-item height Vec that matches what the
            // renderer will draw. Rows use the fixed TREE_ROW_HEIGHT_PX; slots
            // are SLOT_HIT_HEIGHT_PX tall, with an extra SLOT_DEPTH_GAP_PX top
            // margin when stacked after a slot at a different depth.
            let item_sizes = Rc::new(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| match item {
                        TreeRenderItem::Row(_) => size(px(0.0), px(TREE_ROW_HEIGHT_PX)),
                        TreeRenderItem::Slot(slot) => {
                            let needs_depth_gap = i > 0
                                && match &items[i - 1] {
                                    TreeRenderItem::Slot(prev) => prev.depth != slot.depth,
                                    TreeRenderItem::Row(_) => false,
                                };
                            size(
                                px(0.0),
                                px(SLOT_HIT_HEIGHT_PX
                                    + if needs_depth_gap {
                                        SLOT_DEPTH_GAP_PX
                                    } else {
                                        0.0
                                    }),
                            )
                        }
                    })
                    .collect::<Vec<_>>(),
            );
            let items_for_list = Rc::new(items);
            panel.child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        v_virtual_list(
                            list_view,
                            "workspace-tree-list",
                            item_sizes,
                            move |this, range: Range<usize>, window, cx| {
                                let mut rendered: Vec<AnyElement> =
                                    Vec::with_capacity(range.end - range.start);
                                let mut prev_slot_depth: Option<usize> = if range.start == 0 {
                                    None
                                } else {
                                    match &items_for_list[range.start - 1] {
                                        TreeRenderItem::Slot(prev) => Some(prev.depth),
                                        TreeRenderItem::Row(_) => None,
                                    }
                                };
                                for idx in range {
                                    match &items_for_list[idx] {
                                        TreeRenderItem::Slot(slot) => {
                                            let needs_depth_gap = prev_slot_depth
                                                .is_some_and(|prev| prev != slot.depth);
                                            let slot_el = this.render_tree_drop_slot(slot, cx);
                                            let el = if needs_depth_gap {
                                                div()
                                                    .mt(px(SLOT_DEPTH_GAP_PX))
                                                    .child(slot_el)
                                                    .into_any_element()
                                            } else {
                                                slot_el
                                            };
                                            prev_slot_depth = Some(slot.depth);
                                            rendered.push(el);
                                        }
                                        TreeRenderItem::Row(row) => {
                                            prev_slot_depth = None;
                                            rendered.push(this.render_tree_row(row, window, cx));
                                        }
                                    }
                                }
                                rendered
                            },
                        )
                        .size_full()
                        .track_scroll(&self.collection_scroll_handle),
                    )
                    .on_drag_move(cx.listener(
                        |this, drag: &DragMoveEvent<DraggedRequest>, window, cx| {
                            this.update_tree_drag_autoscroll(
                                drag.bounds,
                                drag.event.position,
                                window,
                                cx,
                            );
                        },
                    ))
                    .on_drag_move(cx.listener(
                        |this, drag: &DragMoveEvent<DraggedFolder>, window, cx| {
                            this.update_tree_drag_autoscroll(
                                drag.bounds,
                                drag.event.position,
                                window,
                                cx,
                            );
                        },
                    ))
                    .can_drop(move |_dragged_value, _window, app| {
                        scroll_view.update(app, |this, _| {
                            this.tree_drag_slot_hover.is_some() || this.tree_drag_hover.is_some()
                        })
                    })
                    .on_drop(
                        cx.listener(move |this, dragged: &DraggedRequest, window, cx| {
                            if let Some(slot) = this.tree_drag_slot_hover {
                                this.handle_request_tree_drop_slot(
                                    dragged.request_id,
                                    &slot,
                                    window,
                                    cx,
                                );
                            } else if let Some((target_id, TreeDropPlacement::Into)) =
                                this.tree_drag_hover
                            {
                                this.handle_request_tree_drop(
                                    dragged.request_id,
                                    target_id,
                                    TreeDropPlacement::Into,
                                    window,
                                    cx,
                                );
                            }
                            this.clear_tree_drag_hover(cx);
                        }),
                    )
                    .on_drop(
                        cx.listener(move |this, dragged: &DraggedFolder, window, cx| {
                            if let Some(slot) = this.tree_drag_slot_hover {
                                this.handle_folder_tree_drop_slot(
                                    dragged.folder_id,
                                    &slot,
                                    window,
                                    cx,
                                );
                            } else if let Some((target_id, TreeDropPlacement::Into)) =
                                this.tree_drag_hover
                            {
                                this.handle_folder_tree_drop(
                                    dragged.folder_id,
                                    target_id,
                                    TreeDropPlacement::Into,
                                    window,
                                    cx,
                                );
                            }
                            this.clear_tree_drag_hover(cx);
                        }),
                    )
                    .vertical_scrollbar(&self.collection_scroll_handle)
                    .on_mouse_down(MouseButton::Right, {
                        let view = menu_view;
                        move |ev: &MouseDownEvent, window, cx| {
                            view.update(cx, |this, cx| {
                                this.focus_handle.focus(window, cx);
                                let position = Point {
                                    x: ev.position.x + px(4.),
                                    y: ev.position.y,
                                };
                                if let Some(row) = this.collection_context_menu_row.take() {
                                    let menu = this.build_tree_row_context_menu(row);
                                    menu.show(position, window, cx);
                                } else {
                                    let menu = this.build_empty_space_context_menu();
                                    menu.show(position, window, cx);
                                }
                            });
                        }
                    }),
            )
        }
    }
}
