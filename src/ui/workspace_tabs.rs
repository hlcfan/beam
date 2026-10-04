use super::*;
use gpui_kit::base::{Tab, Tabs};

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(in crate::ui) struct WorkspaceMenuRename(Ulid);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(in crate::ui) struct WorkspaceMenuDelete(Ulid);

impl BeamView {
    pub(in crate::ui) fn render_workspace_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let workspaces = &self.shell.workspace.all_workspaces;

        Tabs::new("workspace-tabs")
            .flex()
            .items_center()
            .flex_1()
            .min_w_0()
            .overflow_x_scroll()
            .overflow_y_hidden()
            .track_scroll(&self.workspace_tabs_scroll_handle)
            .children(workspaces.iter().enumerate().map(|(index, workspace)| {
                let workspace_id = workspace.workspace_id;
                let selected = Some(workspace.workspace_id) == self.shell.workspace.workspace_id;
                let name = workspace.name.clone();
                Tab::new(SharedString::from(format!(
                    "workspace-tab-{}",
                    workspace.workspace_id
                )))
                .accessibility_label(name.clone())
                .set_position(index + 1, workspaces.len())
                .selected(selected)
                .flex_shrink_0()
                .max_w(rems(12.0))
                .h_7()
                .px_3()
                .text_sm()
                .cursor_pointer()
                .border_b_2()
                .border_color(theme.transparent)
                .text_color(theme.muted_foreground)
                .hover(|style| style.bg(theme.secondary))
                .styles(|styles| {
                    styles.selected(|style| {
                        style
                            .bg(theme.secondary)
                            .text_color(theme.foreground)
                            .border_color(theme.primary)
                    })
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.switch_workspace_from_tab(workspace_id, window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.focus_handle.focus(window, cx);
                        this.show_workspace_tab_menu(workspace_id, event.position, window, cx);
                    }),
                )
                .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
                .child(div().truncate().child(workspace.name.clone()))
            }))
    }

    pub(in crate::ui) fn on_action_workspace_menu_rename(
        &mut self,
        action: &WorkspaceMenuRename,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_rename_workspace_dialog(action.0, cx);
    }

    pub(in crate::ui) fn on_action_workspace_menu_delete(
        &mut self,
        action: &WorkspaceMenuDelete,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_delete_workspace_dialog(action.0, cx);
    }

    fn show_workspace_tab_menu(
        &self,
        workspace_id: Ulid,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = append_with_image_or_plain(
            NativeMenu::new(),
            "Rename…",
            "icons/edit.svg",
            false,
            Box::new(WorkspaceMenuRename(workspace_id)),
        );
        let menu = append_with_image_or_plain(
            menu.separator(),
            "Delete…",
            "icons/trash.svg",
            self.shell.workspace.all_workspaces.len() <= 1,
            Box::new(WorkspaceMenuDelete(workspace_id)),
        );
        menu.show(position, window, cx);
    }

    pub(in crate::ui) fn switch_workspace_from_tab(
        &mut self,
        workspace_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if Some(workspace_id) == self.shell.workspace.workspace_id {
            return;
        }
        if let Err(error) = self.publish_app_command(AppCommand::SwitchWorkspace {
            workspace_id,
            command_id: next_command_id(),
        }) {
            window.push_notification(error, cx);
        }
    }

    pub(in crate::ui) fn reveal_active_workspace_tab(&self) {
        if let Some(index) = self
            .shell
            .workspace
            .all_workspaces
            .iter()
            .position(|workspace| Some(workspace.workspace_id) == self.shell.workspace.workspace_id)
        {
            self.workspace_tabs_scroll_handle.scroll_to_item(index);
        }
    }
}
