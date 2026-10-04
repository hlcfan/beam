use super::super::*;

#[derive(Clone, Copy)]
pub(in crate::ui) enum WorkspaceDialogMode {
    Create,
    Rename { workspace_id: Ulid },
}

impl WorkspaceDialogMode {
    fn command(self, name: &str) -> Result<AppCommand, String> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("Workspace name cannot be empty.".to_string());
        }
        let command_id = next_command_id();
        Ok(match self {
            Self::Create => AppCommand::CreateWorkspace { name, command_id },
            Self::Rename { workspace_id } => AppCommand::RenameWorkspace {
                workspace_id,
                new_name: name,
                command_id,
            },
        })
    }
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
        let command = match self.mode.command(&self.name_input.read(cx).value()) {
            Ok(command) => command,
            Err(error) => {
                window.push_notification(error, cx);
                return;
            }
        };
        let _ = self.target_view.update(cx, |this, cx| {
            if let Err(error) = this.publish_app_command(command) {
                window.push_notification(error, cx);
                return;
            }
            window.close_dialog(cx);
        });
    }
}

impl Render for WorkspaceNameDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit(window, cx);
                            })),
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

#[cfg(test)]
mod tests {
    use super::{AppCommand, Ulid, WorkspaceDialogMode};

    #[test]
    fn rename_command_keeps_the_dialog_workspace_target() {
        let workspace_id = Ulid::new();
        let mode = WorkspaceDialogMode::Rename { workspace_id };
        let command = mode.command("  Renamed workspace  ").expect("valid name");
        assert!(matches!(command, AppCommand::RenameWorkspace {
            workspace_id: target,
            new_name,
            ..
        } if target == workspace_id && new_name == "Renamed workspace"));
    }

    #[test]
    fn workspace_dialog_rejects_blank_names_for_both_modes() {
        for mode in [
            WorkspaceDialogMode::Create,
            WorkspaceDialogMode::Rename {
                workspace_id: Ulid::new(),
            },
        ] {
            assert!(mode.command(" \t\n ").is_err());
        }
    }

    #[test]
    fn create_command_trims_the_workspace_name() {
        let command = WorkspaceDialogMode::Create
            .command("  New workspace  ")
            .expect("valid name");
        assert!(
            matches!(command, AppCommand::CreateWorkspace { name, .. } if name == "New workspace")
        );
    }
}
