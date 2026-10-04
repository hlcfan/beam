use super::*;
use gpui_kit::base::{Tab, Tabs};

#[cfg(test)]
mod tests;

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(in crate::ui) struct WorkspaceMenuRename(Ulid);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(in crate::ui) struct WorkspaceMenuDelete(Ulid);

actions!(
    beam,
    [
        FocusNextWorkspaceTab,
        FocusPreviousWorkspaceTab,
        FocusFirstWorkspaceTab,
        FocusLastWorkspaceTab,
        ActivateWorkspaceTab,
        OpenWorkspaceTabMenu
    ]
);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("right", FocusNextWorkspaceTab, Some("WorkspaceTabs")),
        KeyBinding::new("left", FocusPreviousWorkspaceTab, Some("WorkspaceTabs")),
        KeyBinding::new("home", FocusFirstWorkspaceTab, Some("WorkspaceTabs")),
        KeyBinding::new("end", FocusLastWorkspaceTab, Some("WorkspaceTabs")),
        KeyBinding::new("enter", ActivateWorkspaceTab, Some("WorkspaceTabs")),
        KeyBinding::new("space", ActivateWorkspaceTab, Some("WorkspaceTabs")),
        KeyBinding::new("shift-f10", OpenWorkspaceTabMenu, Some("WorkspaceTabs")),
    ]);
}

#[derive(Clone, Copy)]
enum WorkspaceTabNavigation {
    Next,
    Previous,
    First,
    Last,
}

impl BeamView {
    pub(in crate::ui) fn render_workspace_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let workspaces = &self.shell.workspace.all_workspaces;

        Tabs::new("workspace-tabs")
            .track_focus(&self.workspace_tabs_focus_handle)
            .tab_group()
            .key_context("WorkspaceTabs")
            .on_action(cx.listener(|this, _: &FocusNextWorkspaceTab, window, cx| {
                this.navigate_workspace_tabs(WorkspaceTabNavigation::Next, window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &FocusPreviousWorkspaceTab, window, cx| {
                    this.navigate_workspace_tabs(WorkspaceTabNavigation::Previous, window, cx);
                }),
            )
            .on_action(cx.listener(|this, _: &FocusFirstWorkspaceTab, window, cx| {
                this.navigate_workspace_tabs(WorkspaceTabNavigation::First, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusLastWorkspaceTab, window, cx| {
                this.navigate_workspace_tabs(WorkspaceTabNavigation::Last, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateWorkspaceTab, window, cx| {
                if let Some(workspace_id) = this.focused_workspace_tab(window) {
                    this.switch_workspace_from_tab(workspace_id, window, cx);
                }
            }))
            .on_action(cx.listener(Self::on_action_open_workspace_tab_menu))
            .flex()
            .items_center()
            .gap_1()
            .px_1()
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
                .when_some(
                    self.workspace_tab_focus_handles.get(&workspace_id),
                    |tab, handle| {
                        tab.track_focus(handle)
                            .tab_index(0)
                            .tab_stop(self.workspace_tab_stop == Some(workspace_id))
                    },
                )
                .flex_shrink_0()
                .max_w(rems(12.0))
                .h_full()
                .px_3()
                .text_sm()
                .cursor_pointer()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.transparent)
                .text_color(theme.muted_foreground)
                .when(!selected, |tab| {
                    tab.hover(|style| style.bg(theme.secondary))
                })
                .active(|style| style.bg(theme.secondary_active))
                .focus_visible(|style| style.border_color(theme.primary))
                .styles(|styles| {
                    styles.selected(|style| {
                        style
                            .bg(theme.background)
                            .text_color(theme.foreground)
                            .border_color(theme.border)
                            .shadow(
                                theme
                                    .shadow_tokens()
                                    .sm
                                    .into_iter()
                                    .map(|shadow| BoxShadow {
                                        color: shadow.color.opacity(0.5),
                                        blur_radius: shadow.blur_radius / 2.,
                                        ..shadow
                                    })
                                    .collect(),
                            )
                    })
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.focus_workspace_tab(workspace_id, window, cx);
                    this.switch_workspace_from_tab(workspace_id, window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.focus_workspace_tab(workspace_id, window, cx);
                        this.show_workspace_tab_menu(workspace_id, event.position, window, cx);
                    }),
                )
                .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
                .child(div().truncate().child(workspace.name.clone()))
            }))
    }

    pub(in crate::ui) fn sync_workspace_tab_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self.focused_workspace_tab(window);
        let workspaces = &self.shell.workspace.all_workspaces;
        self.workspace_tab_focus_handles
            .retain(|id, _| workspaces.iter().any(|entry| entry.workspace_id == *id));
        for entry in workspaces {
            self.workspace_tab_focus_handles
                .entry(entry.workspace_id)
                .or_insert_with(|| cx.focus_handle());
        }
        if !self
            .workspace_tabs_focus_handle
            .contains_focused(window, cx)
            || !self
                .workspace_tab_stop
                .is_some_and(|id| self.workspace_tab_focus_handles.contains_key(&id))
        {
            self.workspace_tab_stop = self
                .shell
                .workspace
                .workspace_id
                .filter(|id| self.workspace_tab_focus_handles.contains_key(id))
                .or_else(|| workspaces.first().map(|entry| entry.workspace_id));
        }
        if focused.is_some_and(|id| !self.workspace_tab_focus_handles.contains_key(&id))
            && let Some(id) = self.workspace_tab_stop
        {
            self.focus_workspace_tab(id, window, cx);
        }
    }

    pub(in crate::ui) fn focused_workspace_tab(&self, window: &Window) -> Option<Ulid> {
        self.workspace_tab_focus_handles
            .iter()
            .find_map(|(id, handle)| handle.is_focused(window).then_some(*id))
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

    fn focus_workspace_tab(
        &mut self,
        workspace_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(handle) = self.workspace_tab_focus_handles.get(&workspace_id) {
            self.workspace_tab_stop = Some(workspace_id);
            handle.focus(window, cx);
            self.reveal_workspace_tab(workspace_id);
            cx.notify();
        }
    }

    fn navigate_workspace_tabs(
        &mut self,
        navigation: WorkspaceTabNavigation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self.focused_workspace_tab(window);
        let workspaces = &self.shell.workspace.all_workspaces;
        let current = workspaces
            .iter()
            .position(|entry| Some(entry.workspace_id) == focused)
            .unwrap_or(0);
        if let Some(index) = workspace_tab_index(workspaces.len(), current, navigation) {
            self.focus_workspace_tab(workspaces[index].workspace_id, window, cx);
        }
    }

    fn on_action_open_workspace_tab_menu(
        &mut self,
        _: &OpenWorkspaceTabMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace_id) = self.focused_workspace_tab(window) else {
            return;
        };
        let Some(index) = self
            .shell
            .workspace
            .all_workspaces
            .iter()
            .position(|entry| entry.workspace_id == workspace_id)
        else {
            return;
        };
        if let Some(bounds) = self.workspace_tabs_scroll_handle.bounds_for_item(index) {
            let position = point(
                bounds.left() + self.workspace_tabs_scroll_handle.offset().x,
                bounds.bottom(),
            );
            self.show_workspace_tab_menu(workspace_id, position, window, cx);
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
        if let Some(workspace_id) = self.shell.workspace.workspace_id {
            self.reveal_workspace_tab(workspace_id);
        }
    }
}

fn workspace_tab_index(
    count: usize,
    current: usize,
    navigation: WorkspaceTabNavigation,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let current = current.min(count - 1);
    Some(match navigation {
        WorkspaceTabNavigation::Next => (current + 1) % count,
        WorkspaceTabNavigation::Previous => {
            if current == 0 {
                count - 1
            } else {
                current - 1
            }
        }
        WorkspaceTabNavigation::First => 0,
        WorkspaceTabNavigation::Last => count - 1,
    })
}
