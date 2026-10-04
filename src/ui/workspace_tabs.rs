use super::*;
use gpui_kit::base::{Tab, Tabs};

impl BeamView {
    pub(in crate::ui) fn render_workspace_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let workspaces = &self.shell.workspace.all_workspaces;

        Tabs::new("workspace-tabs")
            .flex()
            .items_center()
            .flex_1()
            .min_w_0()
            .overflow_x_hidden()
            .children(workspaces.iter().enumerate().map(|(index, workspace)| {
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
                .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
                .child(div().truncate().child(workspace.name.clone()))
            }))
    }
}
