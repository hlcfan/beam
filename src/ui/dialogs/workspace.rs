use super::super::*;

pub(in crate::ui) enum WorkspaceDialogMode {
    Create,
    Rename,
}

pub(in crate::ui) struct WorkspaceNameDialogView {
    target_view: Entity<BeamView>,
    mode: WorkspaceDialogMode,
    name_input: Entity<InputState>,
}

impl WorkspaceNameDialogView {
    pub(in crate::ui) fn new(
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

    pub(in crate::ui) fn focus_name_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.name_input.update(cx, |state, cx| {
            state.focus(window, cx);
            let cursor_end = state.value().encode_utf16().count() as u32;
            state.set_cursor_position(Position::new(0, cursor_end), window, cx);
        });
    }

    pub(in crate::ui) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

pub(in crate::ui) struct WorkspaceDeleteDialogView {
    target_view: Entity<BeamView>,
    workspace_id: Ulid,
    workspace_name: String,
}

impl WorkspaceDeleteDialogView {
    pub(in crate::ui) fn new(
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

    pub(in crate::ui) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
