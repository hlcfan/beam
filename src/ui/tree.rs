mod actions;
mod drag_drop;
mod render;

use super::*;

/// Distance (px) from the top/bottom edge of the tree viewport within which a
/// drag triggers auto-scroll. Closer to the edge scrolls faster.
const TREE_DRAG_SCROLL_FAST_ZONE_PX: f32 = 16.0;
const TREE_DRAG_SCROLL_SLOW_ZONE_PX: f32 = 48.0;
const TREE_DRAG_SCROLL_FAST_STEP_PX: f32 = 14.0;
const TREE_DRAG_SCROLL_SLOW_STEP_PX: f32 = 6.0;
const TREE_DRAG_SCROLL_TICK: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TreeNeighborDirection {
    Next,
    Prev,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestViewHistoryDirection {
    Next,
    Prev,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PendingFolderPlacement {
    After {
        parent: FolderParentRef,
        insertion_index: usize,
        known_target_manifest_path: Option<KnownParentManifestPath>,
    },
}

#[derive(Clone, Debug)]
pub(super) struct DraggedFolder {
    folder_id: Ulid,
    label: String,
}

#[derive(Clone, Debug)]
pub(super) struct DraggedRequest {
    request_id: Ulid,
    label: String,
}

#[derive(Clone, Debug)]
pub(super) enum TreeMoveAction {
    MoveRequest(MoveRequestInput),
    MoveFolder(MoveFolderInput),
}

pub(super) struct TreeDragPreview {
    label: String,
    kind: TreeNodeKind,
    position: Point<Pixels>,
}

impl TreeDragPreview {
    fn new(label: String, kind: TreeNodeKind, position: Point<Pixels>) -> Self {
        Self {
            label,
            kind,
            position,
        }
    }
}

impl Render for TreeDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let icon_path = match self.kind {
            TreeNodeKind::Folder => "icons/folder.svg",
            TreeNodeKind::Request => "icons/file.svg",
        };

        div()
            .pl(self.position.x - px(72.0))
            .pt(self.position.y - px(18.0))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .rounded(px(8.0))
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .shadow_md()
                    .child(
                        Icon::default()
                            .path(icon_path)
                            .size(px(14.0))
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .max_w(px(240.0))
                            .truncate()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(self.label.clone()),
                    ),
            )
    }
}
