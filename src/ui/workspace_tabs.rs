use super::*;
use gpui_kit::base::{Tab, Tabs};
use gpui_kit::component::Colorize;

#[cfg(test)]
mod tests;

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
struct SelectWorkspaceTab(usize);

pub(super) fn init_workspace_tab_actions(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys((1..=9).map(|number| {
        KeyBinding::new(
            &format!("{modifier}-{number}"),
            SelectWorkspaceTab(number - 1),
            None,
        )
    }));
    cx.on_action(|action: &SelectWorkspaceTab, cx: &mut App| {
        let index = action.0;
        cx.defer(move |cx| {
            let Some(window_handle) = cx.active_window() else {
                return;
            };
            let Some(root) = window_handle
                .downcast::<Root>()
                .and_then(|handle| handle.read(cx).ok())
            else {
                return;
            };
            let Ok(view) = root.view().clone().downcast::<BeamView>() else {
                return;
            };
            let _ = window_handle.update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.select_workspace_tab(index, window, cx);
                });
            });
        });
    });
}

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
            .gap_1()
            .pl_1()
            .mr_1()
            .py_1()
            .h_full()
            .flex_initial()
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
                .h_full()
                .px_3()
                .text_sm()
                .line_height(relative(1.5))
                .cursor_pointer()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.transparent)
                .text_color(theme.foreground)
                .when(!selected, |tab| {
                    tab.hover(|style| style.bg(theme.secondary))
                        .active(|style| style.bg(theme.secondary_active))
                })
                .styles(|styles| {
                    styles.selected(|style| {
                        style
                            .bg(theme.background)
                            .text_color(theme.foreground)
                            .border_color(theme.border.mix(theme.foreground, 0.95))
                            .when(theme.mode.is_dark(), |style| {
                                style
                                    .bg(theme.background.mix_oklab(theme.foreground, 0.65))
                                    .border_color(theme.transparent)
                                    .font_semibold()
                            })
                            .shadow(
                                theme
                                    .shadow_tokens()
                                    .sm
                                    .into_iter()
                                    .map(|shadow| BoxShadow {
                                        color: shadow.color.opacity(0.65),
                                        // Keep depth below the tab without a halo around its border.
                                        offset: point(shadow.offset.x, shadow.offset.y * 2.),
                                        blur_radius: shadow.blur_radius / 2.,
                                        spread_radius: -shadow.blur_radius / 4.,
                                        ..shadow
                                    })
                                    .collect(),
                            )
                    })
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.switch_workspace_from_tab(workspace_id, window, cx);
                }))
                .capture_any_mouse_down(cx.listener(
                    move |this, event: &MouseDownEvent, window, cx| {
                        if event.button != MouseButton::Right {
                            return;
                        }
                        // Native menu tracking can consume mouse-up. Stop before the tab
                        // records a press that would leave its active background stuck.
                        cx.stop_propagation();
                        this.show_workspace_tab_menu(workspace_id, event.position, window, cx);
                    },
                ))
                .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
                .child(div().truncate().child(workspace.name.clone()))
            }))
    }

    pub(in crate::ui) fn schedule_active_workspace_tab_reveal(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The scroll handle gets its viewport and overflow during the first prepaint.
        // Revealing before that consumes the request without scrolling horizontally.
        let view_handle = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = view_handle.update(cx, |view, cx| {
                view.reveal_active_workspace_tab();
                cx.notify();
            });
        });
    }

    pub(in crate::ui) fn reveal_active_workspace_tab(&self) {
        if let Some(workspace_id) = self.shell.workspace.workspace_id {
            self.reveal_workspace_tab(workspace_id);
        }
    }

    pub(in crate::ui) fn reveal_workspace_tab(&self, workspace_id: Ulid) {
        if let Some(index) = self
            .shell
            .workspace
            .all_workspaces
            .iter()
            .position(|entry| entry.workspace_id == workspace_id)
        {
            self.workspace_tabs_scroll_handle.scroll_to_item(index);
        }
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
        // Native menu actions need Beam's dispatch scope, even before an editor is focused.
        if !self.focus_handle.contains_focused(window, cx) {
            self.focus_handle.focus(window, cx);
        }
        let menu = append_with_image_or_plain(
            NativeMenu::new(),
            "Rename",
            "icons/edit.svg",
            false,
            Box::new(WorkspaceMenuRename(workspace_id)),
        );
        let menu = append_with_image_or_plain(
            menu.separator(),
            "Delete",
            "icons/trash.svg",
            self.shell.workspace.all_workspaces.len() <= 1,
            Box::new(WorkspaceMenuDelete(workspace_id)),
        );
        menu.show(position, window, cx);
    }

    fn select_workspace_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.shell.workspace.all_workspaces.get(index) {
            self.switch_workspace_from_tab(workspace.workspace_id, window, cx);
        }
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
}
