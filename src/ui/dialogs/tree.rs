use super::super::*;

pub(in crate::ui) struct TreeRenameDialogView {
    target_view: Entity<BeamView>,
    node_id: Ulid,
    node_kind: TreeNodeKind,
    name_input: Entity<InputState>,
}

impl TreeRenameDialogView {
    pub(in crate::ui) fn new(
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

    pub(in crate::ui) fn focus_name_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.name_input.update(cx, |state, cx| {
            state.focus(window, cx);
            let cursor_end = state.value().encode_utf16().count() as u32;
            state.set_cursor_position(Position::new(0, cursor_end), window, cx);
        });
    }

    pub(in crate::ui) fn submit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

pub(in crate::ui) struct TreeNodeDeleteDialogView {
    target_view: Entity<BeamView>,
    node_id: Ulid,
    node_kind: TreeNodeKind,
    node_name: String,
}

impl TreeNodeDeleteDialogView {
    pub(in crate::ui) fn new(
        target_view: Entity<BeamView>,
        node_id: Ulid,
        node_kind: TreeNodeKind,
        node_name: String,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            target_view,
            node_id,
            node_kind,
            node_name,
        }
    }

    pub(in crate::ui) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let node_id = self.node_id;
        let node_kind = self.node_kind;
        let _ = self.target_view.update(cx, |this, cx| {
            match node_kind {
                TreeNodeKind::Folder => this.delete_folder_from_tree_node(node_id, window, cx),
                TreeNodeKind::Request => this.delete_request_from_tree_node(node_id, window, cx),
            }
            window.close_dialog(cx);
        });
    }
}

impl Render for TreeNodeDeleteDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let node_name = self.node_name.clone();
        let (kind_label, warning) = match self.node_kind {
            TreeNodeKind::Folder => (
                "folder",
                "This deletes the folder and all of its contents from disk.",
            ),
            TreeNodeKind::Request => ("request", "This deletes the request from disk."),
        };

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
                            .child(format!("Delete {kind_label} \"{node_name}\"?")),
                    )
                    .child(div().text_sm().font_semibold().child(warning)),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("delete-tree-node-cancel")
                            .small()
                            .ghost()
                            .cursor_pointer()
                            .label("Cancel")
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("delete-tree-node-submit")
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
