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
const MAX_REQUEST_VIEW_HISTORY_ENTRIES: usize = 1_000;

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

/// Tracks the in-memory sequence of requests the user has viewed in one workspace.
#[derive(Clone, Debug, Default)]
pub(super) struct RequestViewHistory {
    entries: Vec<Ulid>,
    cursor: Option<usize>,
    recent: Vec<Ulid>,
}

/// Keeps an independent request-view history and cursor for every visited workspace.
#[derive(Clone, Debug, Default)]
pub(super) struct WorkspaceRequestViewHistories {
    histories: HashMap<Ulid, RequestViewHistory>,
    active_workspace_id: Option<Ulid>,
}

impl WorkspaceRequestViewHistories {
    pub(super) fn set_active_workspace(&mut self, workspace_id: Option<Ulid>) {
        self.active_workspace_id = workspace_id;
    }

    pub(super) fn visit(&mut self, request_id: Ulid) {
        let Some(workspace_id) = self.active_workspace_id else {
            return;
        };
        self.histories
            .entry(workspace_id)
            .or_default()
            .visit(request_id);
    }

    pub(super) fn prune(&mut self, request_id: Ulid) {
        let Some(history) = self
            .active_workspace_id
            .and_then(|workspace_id| self.histories.get_mut(&workspace_id))
        else {
            return;
        };
        history.prune(request_id);
    }

    pub(super) fn prune_workspace(&mut self, workspace_id: Ulid) {
        self.histories.remove(&workspace_id);
    }

    pub(super) fn go_back(&mut self) -> Option<Ulid> {
        self.active_history_mut()?.go_back()
    }

    pub(super) fn go_forward(&mut self) -> Option<Ulid> {
        self.active_history_mut()?.go_forward()
    }

    pub(super) fn recent_request_ids(&self) -> &[Ulid] {
        self.active_workspace_id
            .and_then(|workspace_id| self.histories.get(&workspace_id))
            .map(RequestViewHistory::recent_request_ids)
            .unwrap_or(&[])
    }

    fn active_history_mut(&mut self) -> Option<&mut RequestViewHistory> {
        let workspace_id = self.active_workspace_id?;
        self.histories.get_mut(&workspace_id)
    }
}

impl RequestViewHistory {
    pub(super) fn visit(&mut self, request_id: Ulid) {
        self.touch_recent(request_id);
        if let Some(cursor) = self.cursor
            && self.entries.get(cursor) == Some(&request_id)
        {
            return;
        }
        if let Some(cursor) = self.cursor
            && cursor + 1 < self.entries.len()
        {
            self.entries.truncate(cursor + 1);
        }
        self.entries.push(request_id);
        if self.entries.len() > MAX_REQUEST_VIEW_HISTORY_ENTRIES {
            let excess = self.entries.len() - MAX_REQUEST_VIEW_HISTORY_ENTRIES;
            self.entries.drain(..excess);
        }
        self.cursor = self.entries.len().checked_sub(1);
    }

    pub(super) fn prune(&mut self, request_id: Ulid) {
        self.recent.retain(|id| *id != request_id);
        let Some(cursor) = self.cursor else {
            self.entries.retain(|candidate| *candidate != request_id);
            return;
        };
        let removed_before = self.entries[..cursor]
            .iter()
            .filter(|candidate| **candidate == request_id)
            .count();
        let current_was_removed = self.entries.get(cursor) == Some(&request_id);
        self.entries.retain(|candidate| *candidate != request_id);
        if self.entries.is_empty() {
            self.cursor = None;
            return;
        }
        let new_tip = self.entries.len() - 1;
        let adjusted_cursor = cursor.saturating_sub(removed_before);
        self.cursor = Some(if current_was_removed {
            adjusted_cursor.min(new_tip)
        } else {
            adjusted_cursor
        });
    }

    pub(super) fn go_back(&mut self) -> Option<Ulid> {
        let cursor = self.cursor?;
        if cursor == 0 {
            return None;
        }
        self.cursor = Some(cursor - 1);
        let request_id = self.entries.get(cursor - 1).copied()?;
        self.touch_recent(request_id);
        Some(request_id)
    }

    pub(super) fn go_forward(&mut self) -> Option<Ulid> {
        let cursor = self.cursor?;
        let next = cursor + 1;
        let request_id = self.entries.get(next).copied()?;
        self.cursor = Some(next);
        self.touch_recent(request_id);
        Some(request_id)
    }

    pub(super) fn recent_request_ids(&self) -> &[Ulid] {
        &self.recent
    }

    fn touch_recent(&mut self, request_id: Ulid) {
        self.recent.retain(|id| *id != request_id);
        self.recent.insert(0, request_id);
        self.recent.truncate(MAX_REQUEST_VIEW_HISTORY_ENTRIES);
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
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

#[cfg(test)]
mod tests {
    use super::{
        MAX_REQUEST_VIEW_HISTORY_ENTRIES, RequestViewHistory, WorkspaceRequestViewHistories,
    };
    use ulid::Ulid;

    #[test]
    fn request_view_history_records_and_steps_back_forward() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);

        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.go_back(), Some(r1));
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r2));
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_truncates_forward_on_new_visit_after_back() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let r4 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        assert_eq!(history.go_back(), Some(r2));
        history.visit(r4);

        assert_eq!(history.go_forward(), None);
        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.go_back(), Some(r1));
        assert_eq!(history.go_back(), None);
    }

    #[test]
    fn request_view_history_visit_at_cursor_is_no_op() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let mut history = RequestViewHistory::default();
        history.visit(r1);
        history.visit(r2);
        assert_eq!(history.go_back(), Some(r1));

        history.visit(r1);
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r2));
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn workspace_request_view_histories_keep_independent_entries_and_cursors() {
        let w1 = Ulid::new();
        let w2 = Ulid::new();
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let r4 = Ulid::new();
        let mut histories = WorkspaceRequestViewHistories::default();

        histories.set_active_workspace(Some(w1));
        histories.visit(r1);
        histories.visit(r2);
        histories.set_active_workspace(Some(w2));
        histories.visit(r3);
        histories.visit(r4);

        assert_eq!(histories.go_back(), Some(r3));
        assert_eq!(histories.go_back(), None);

        histories.set_active_workspace(Some(w1));
        assert_eq!(histories.go_back(), Some(r1));
        assert_eq!(histories.go_forward(), Some(r2));

        histories.set_active_workspace(Some(w2));
        assert_eq!(histories.go_forward(), Some(r4));
    }

    #[test]
    fn request_view_history_revisit_existing_entry_records_transition() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let r4 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        history.visit(r4);
        history.visit(r3);

        assert_eq!(history.go_back(), Some(r4));
        assert_eq!(history.go_back(), Some(r3));
        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.go_back(), Some(r1));
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r2));
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.go_forward(), Some(r4));
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_new_visit_after_back_replaces_forward_branch() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let r4 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        assert_eq!(history.go_back(), Some(r2));
        history.visit(r4);

        assert_eq!(history.go_forward(), None);
        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.go_back(), Some(r1));
        assert_eq!(history.go_back(), None);
    }

    #[test]
    fn request_view_history_prune_drops_deleted_request() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        assert_eq!(history.go_back(), Some(r2));
        history.prune(r2);

        assert_eq!(history.go_back(), Some(r1));
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_prune_unselected_request_keeps_cursor() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        history.prune(r1);

        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_prune_tip_only_entry_clears_history() {
        let r1 = Ulid::new();
        let mut history = RequestViewHistory::default();
        history.visit(r1);

        history.prune(r1);
        assert!(history.is_empty());
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), None);
    }

    #[test]
    fn request_view_history_tracks_deduplicated_mru_order() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        history.visit(r1);

        assert_eq!(history.recent_request_ids(), &[r1, r3, r2]);
    }

    #[test]
    fn request_view_history_navigation_updates_mru_order() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r3);
        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.recent_request_ids(), &[r2, r3, r1]);
        assert_eq!(history.go_forward(), Some(r3));
        assert_eq!(history.recent_request_ids(), &[r3, r2, r1]);
    }

    #[test]
    fn request_view_history_prune_removes_all_mru_and_navigation_occurrences() {
        let r1 = Ulid::new();
        let r2 = Ulid::new();
        let r3 = Ulid::new();
        let mut history = RequestViewHistory::default();

        history.visit(r1);
        history.visit(r2);
        history.visit(r1);
        history.visit(r3);
        history.prune(r1);

        assert_eq!(history.recent_request_ids(), &[r3, r2]);
        assert_eq!(history.go_back(), Some(r2));
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(r3));
    }

    #[test]
    fn request_view_history_discards_oldest_entries_over_limit() {
        let request_ids = (0..=MAX_REQUEST_VIEW_HISTORY_ENTRIES)
            .map(|_| Ulid::new())
            .collect::<Vec<_>>();
        let mut history = RequestViewHistory::default();

        for request_id in request_ids.iter().copied() {
            history.visit(request_id);
        }

        for expected in request_ids[1..MAX_REQUEST_VIEW_HISTORY_ENTRIES]
            .iter()
            .rev()
        {
            assert_eq!(history.go_back(), Some(*expected));
        }
        assert_eq!(history.go_back(), None);
    }

    #[test]
    fn request_view_history_limits_recent_requests() {
        let request_ids = (0..=MAX_REQUEST_VIEW_HISTORY_ENTRIES)
            .map(|_| Ulid::new())
            .collect::<Vec<_>>();
        let mut history = RequestViewHistory::default();

        for request_id in request_ids.iter().copied() {
            history.visit(request_id);
        }

        assert_eq!(
            history.recent_request_ids().len(),
            MAX_REQUEST_VIEW_HISTORY_ENTRIES
        );
        assert_eq!(history.recent_request_ids().first(), request_ids.last());
        assert!(!history.recent_request_ids().contains(&request_ids[0]));
    }
}
