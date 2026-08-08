//! Workspace tree drag-and-drop slot model.
//!
//! This module owns the pure render-item model and the recursive builder that
//! turns a [`WorkspaceTreeState`] into a flat list of [`TreeRenderItem`]s
//! (rows interleaved with drop slots). The UI layer only has to match on the
//! resulting items to render them — no slot inference happens during drawing.

use ulid::Ulid;

use crate::app_shell::{TreeNodeKind, WorkspaceTreeState};

/// Visual indent per tree depth level.
pub const TREE_INDENT_PX: f32 = 14.0;
/// Left content inset shared by rows and slots.
pub const TREE_CONTENT_INSET_PX: f32 = 6.0;
/// Declared height of a rendered tree row, used by the virtual list to lay
/// out rows before they are measured. Set deliberately larger than the row's
/// intrinsic content height (text-sm + `py_1` padding) so the leftover space
/// shows up as visible spacing between adjacent rows. Tune here if rows ever
/// appear clipped or too tight/too loose inside the virtualized tree.
pub const TREE_ROW_HEIGHT_PX: f32 = 32.0;
/// Height of the visible slot bar.
pub const SLOT_BAR_HEIGHT_PX: f32 = 2.0;
/// Height of the slot hit area. Matches the bar height so the hover highlight
/// is a thin line rather than a thick band.
pub const SLOT_HIT_HEIGHT_PX: f32 = SLOT_BAR_HEIGHT_PX;
/// Right padding for slot hit areas.
pub const SLOT_RIGHT_PAD_PX: f32 = 6.0;
/// Extra vertical gap between consecutive slots whose depths differ.
pub const SLOT_DEPTH_GAP_PX: f32 = 2.0;
/// Proximity threshold (in pixels) above and below a slot bar within which the
/// slot is considered "active" during a tree drag. The slot's layout box stays
/// [`SLOT_BAR_HEIGHT_PX`] tall (so the tree never shifts), but while a drag is
/// in progress, `on_drag_move` on each slot checks whether the cursor is
/// within this many pixels of the bar's top or bottom edge. If so, the slot is
/// marked active in view state and its bar is highlighted. This gives a
/// `(2 + 2 * PROXIMITY)` px effective hit zone without any layout change.
pub const SLOT_DRAG_PROXIMITY_PX: f32 = 4.0;

/// Left inset (in pixels) shared by rows and slots at `depth`.
pub fn tree_depth_inset(depth: usize) -> f32 {
    depth as f32 * TREE_INDENT_PX + TREE_CONTENT_INSET_PX
}

/// Semantic move type expressed by a drop target. Kept stable so the existing
/// drop/move handling can continue to consume it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeDropPlacement {
    Before,
    Into,
    After,
}

/// Rendering-only role for a slot. Useful for spacing, hit-area, and hover
/// copy decisions. The builder labels the closing slot of a container as
/// [`SlotVisualRole::ContainerEnd`] (it is still a single slot, never an extra
/// one stacked on top of the last child's after-slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotVisualRole {
    ContainerStart,
    ItemAfter,
    ContainerEnd,
}

/// A single drop target between or around tree rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeDropSlot {
    /// Visual depth where the slot bar is drawn.
    pub depth: usize,
    /// Child the slot is anchored to, if any. `None` for an empty container's
    /// start slot.
    pub target_id: Option<Ulid>,
    /// Kind of [`target_id`](Self::target_id), if known.
    pub target_kind: Option<TreeNodeKind>,
    /// Semantic move type used by drop handling.
    pub placement: TreeDropPlacement,
    /// Container the slot belongs to. `None` means the workspace root.
    pub container_id: Option<Ulid>,
    /// Rendering-only role.
    pub visual_role: SlotVisualRole,
}

/// View-model for a single tree row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeRowViewModel {
    pub id: Ulid,
    pub kind: TreeNodeKind,
    pub depth: usize,
    pub selected: bool,
}

/// A flat render item: either a drop slot or a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeRenderItem {
    Slot(TreeDropSlot),
    Row(TreeRowViewModel),
}

impl TreeRenderItem {
    pub fn slot_depth(&self) -> Option<usize> {
        match self {
            TreeRenderItem::Slot(slot) => Some(slot.depth),
            TreeRenderItem::Row(_) => None,
        }
    }
}

/// A container reference for the recursive traversal. `None` means the
/// workspace root container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContainerRef {
    id: Option<Ulid>,
}

/// Builds the full flat list of render items for the visible workspace tree.
///
/// The traversal emits, for every container (root or expanded folder):
/// 1. a container-start slot (skipped for an empty folder — the folder body
///    is the drop target there; the root container always emits one so an
///    empty tree still has a single drop target),
/// 2. each child row followed by its after-slot (recursing into expanded
///    folders between the row and its after-slot).
///
/// The last child's after-slot is labelled [`SlotVisualRole::ContainerEnd`].
pub fn build_tree_render_items(tree: &WorkspaceTreeState) -> Vec<TreeRenderItem> {
    let mut out = Vec::new();
    build_container_render_items(tree, ContainerRef { id: None }, 0, &mut out);
    out
}

fn build_container_render_items(
    tree: &WorkspaceTreeState,
    container: ContainerRef,
    depth: usize,
    out: &mut Vec<TreeRenderItem>,
) {
    let children: Vec<Ulid> = match container.id {
        Some(folder_id) => tree
            .node(folder_id)
            .map(|node| node.children.clone())
            .unwrap_or_default(),
        None => tree.roots().to_vec(),
    };

    // Skip the container-start slot for an empty folder — the folder body is
    // the drop target. An empty root container still emits one so an empty
    // tree has a single drop target.
    let is_empty_folder = container.id.is_some() && children.is_empty();
    if !is_empty_folder {
        let first_child = children.first().copied();
        out.push(TreeRenderItem::Slot(TreeDropSlot {
            depth,
            target_id: first_child,
            target_kind: first_child.and_then(|id| tree.node(id).map(|node| node.kind)),
            placement: TreeDropPlacement::Before,
            container_id: container.id,
            visual_role: SlotVisualRole::ContainerStart,
        }));
    }

    let last_index = match children.len() {
        0 => 0,
        n => n - 1,
    };
    for (index, child_id) in children.iter().enumerate() {
        let Some(node) = tree.node(*child_id) else {
            continue;
        };

        out.push(TreeRenderItem::Row(TreeRowViewModel {
            id: node.id,
            kind: node.kind,
            depth,
            selected: Some(node.id) == tree.selected_node_id(),
        }));

        if node.kind != TreeNodeKind::Request && tree.is_expanded(node.id) {
            build_container_render_items(tree, ContainerRef { id: Some(node.id) }, depth + 1, out);
        }

        let visual_role = if index == last_index {
            SlotVisualRole::ContainerEnd
        } else {
            SlotVisualRole::ItemAfter
        };
        out.push(TreeRenderItem::Slot(TreeDropSlot {
            depth,
            target_id: Some(node.id),
            target_kind: Some(node.kind),
            placement: TreeDropPlacement::After,
            container_id: container.id,
            visual_role,
        }));
    }
}
