# Remove Collection & Multi-Workspace Support — Implementation Plan

## Overview

This plan has **two phases**:

1. **Phase 1 (V2)** — Remove the `Collection` entity. Flatten hierarchy to `Workspace → Folders → Requests`. Remove collection-scoped environments. Consolidate `beam.workspace.toml` and `manifest.toml` into a single file. This is a **breaking change**.
2. **Phase 2 (V3)** — Add multi-workspace support. The data root (`$HOME/beam/`) becomes a container for multiple self-contained workspace folders. Local state lives under `$HOME/beam_local/`. This is also a **breaking change**.

**Directory conventions:**
- **Data root**: `$HOME/beam/`
- **Local state root**: `$HOME/beam_local/`
- **No dot-prefixed files or directories** — avoid hidden files for better user visibility

---

## Phase 1 (V2): Remove Collections

### Goal

Eliminate the `Collection` entity. The hierarchy becomes:

```
Workspace
├── Requests (root-level)
├── Folders (root-level, nestable)
│   └── Requests & Sub-folders
└── Global Environments only
```

Collection-scoped environments are removed entirely. Only global environments remain.

`beam.workspace.toml` and `manifest.toml` are consolidated into a single `beam.workspace.toml` that contains workspace metadata **and** the ordered list of root items.

### New On-Disk Layout (Single Workspace)

```
$HOME/beam/
├── beam.workspace.toml         # Workspace metadata + root item ordering
├── {request-slug}.request.toml # Root-level request
├── {folder-slug}/              # Root-level folder
│   ├── folder.toml
│   ├── {request-slug}.request.toml
│   └── {subfolder-slug}/
│       ├── folder.toml
│       └── ...
└── environments/
    └── {env-slug}.env.toml

$HOME/beam_local/
├── local-state.toml
├── history/
│   ├── by-request/
│   │   └── {request_id}.history.toml
│   └── responses/
│       └── {execution_id}.response.bin
└── script_results/
    └── {request_id}.toml
```

- Requests use `.request.toml` suffix; folders are directories. No collision risk.
- Root ordering lives inside `beam.workspace.toml` (no separate manifest file).
- Local state is completely separate from data, under `$HOME/beam_local/`.

### Consolidated `beam.workspace.toml` Shape

```toml
[workspace]
workspace_id = "01JX8R4M6D5Q4T1N3Y8K7P2A9B"
name = "My Workspace"
description = "API development workspace"
schema_version = 2
created_at = "2026-04-28T12:00:00Z"
updated_at = "2026-04-28T12:00:00Z"

[[items]]
item_id = "01JX8R9K7A3D2M4P5Q6R7S8T9U"
item_type = "request"
name = "Get User"
order = 10

[[items]]
item_id = "01JX8RB4P6C8N1D2E3F4G5H6J7"
item_type = "folder"
name = "Auth"
order = 20
```

### Data Model Changes

#### Remove

| Type / Field | Action |
|--------------|--------|
| `NodeKind::Collection` | Delete variant. `NodeKind` becomes `{ Folder, Request }` |
| `CollectionFile` | Delete struct |
| `CollectionMeta` | Delete struct |
| `CollectionItemRef` | **Rename** to `ManifestItemRef` and keep (used by `beam.workspace.toml` and folder manifests) |
| `FolderMeta.collection_id` | Delete field |
| `EnvironmentMeta.collection_id` | Delete field |
| `EnvironmentScope::Collection` | Delete variant. Only `Global` remains |
| `LocalStateFile.collection_environment_selection` | Delete field |
| `SchemaKind::Collection` | Delete variant |
| `RootOrderFile` | Delete. Replaced by `items` inside `beam.workspace.toml` |
| `CollectionManifestFile` | Delete. Replaced by `items` inside `beam.workspace.toml` |

#### Modified

| Type | Change |
|------|--------|
| `WorkspaceFile` | Add `#[serde(default)] pub items: Vec<ManifestItemRef>` field to hold root-level folder/request ordering. This replaces both `RootOrderFile` and `CollectionManifestFile` for the workspace root. |
| `SharedStore` | `root_ids` now holds root-level `Folder` and `Request` IDs (was `Collection` IDs). No other structural change. |

#### New

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestItemRef {
    pub item_id: Ulid,
    pub item_type: ItemType,   // Folder | Request
    pub name: String,
    pub order: i32,
}
```

### Disk IO Performance Notes

The current repository layer already follows a **manifest-first, per-operation rewrite** pattern. The new model preserves this pattern with simpler path logic:

| Operation | Files Written |
|-----------|--------------|
| Create root request | `{request}.request.toml` + rewrite `beam.workspace.toml` |
| Create folder request | `{request}.request.toml` + rewrite parent `folder.toml` |
| Rename request | New `{request}.request.toml` + rewrite parent manifest + delete old file |
| Move request | Rewrite source + destination parent manifests + `rename()` request file |
| Rename folder | `rename()` folder dir + rewrite parent manifest + rewrite nested `folder.toml` files if paths changed |
| Delete request | Delete `{request}.request.toml` + rewrite parent manifest |
| Delete folder | `remove_dir_all()` + rewrite parent manifest |
| Reorder root item | Rewrite `beam.workspace.toml` only |
| Save request payload | Rewrite `{request}.request.toml` only |
| Create environment | Write `environments/{name}.env.toml` only |

**Load performance:**
- `beam.workspace.toml` is read first (small, contains only metadata + root item refs).
- For each root folder, `folder.toml` is read (small manifest).
- Request payloads are **lazy-loaded** when a request node is encountered; the tree can be navigated without parsing every `.request.toml`.
- Environments are loaded by scanning `environments/` only (no collection env scan).
- This is equivalent to or better than the current model (one fewer directory layer to scan).

**Key principle preserved:** small manifests are cheap to rewrite; only the changed scope's manifest is rewritten, never the entire workspace tree.

### File-by-File Changes (Phase 1)

#### 1. `src/models.rs`

- [ ] Remove `CollectionFile`, `CollectionMeta`, `CollectionItemRef`.
- [ ] Rename `CollectionItemRef` → `ManifestItemRef` everywhere.
- [ ] Remove `collection_id` from `FolderMeta`.
- [ ] Remove `collection_id` from `EnvironmentMeta`.
- [ ] Remove `EnvironmentScope::Collection`.
- [ ] Remove `collection_environment_selection` from `LocalStateFile`.
- [ ] Add `items: Vec<ManifestItemRef>` to `WorkspaceFile`.
- [ ] Bump default `schema_version` to `SCHEMA_VERSION_V2`.

#### 2. `src/schema.rs`

- [ ] Add `SCHEMA_VERSION_V2` constant.
- [ ] Remove `SchemaKind::Collection`.
- [ ] Update validation to accept V2.

#### 3. `src/paths.rs`

- [ ] Remove `COLLECTION_MANIFEST_FILE_NAME` and `COLLECTION_ROOT_ORDER_FILE_NAME`.
- [ ] Remove `collections_dir` and `collections_root_order_file` from `BeamPaths`.
- [ ] Update `BeamPaths::from_root` to remove `collections/` construction.
- [ ] Ensure `local_dir` points to `$HOME/beam_local` (already the case for `default_user_config`).

#### 4. `src/workspace_tree.rs`

- [ ] Remove `NodeKind::Collection`.
- [ ] Remove `CollectionManifestFile`, `collection_dir_name`, `collection_dir_path`, `collection_manifest_from_store`.
- [ ] Update `folder_dir_path` to support top-level folders (`parent_id: None` → workspace root).
- [ ] Update `request_file_path` to handle root-level requests (`parent_id: None`).
- [ ] Replace `root_order_file` with `workspace_manifest_from_store(store) -> WorkspaceFile` that produces the in-memory shape for serialization.
- [ ] Update `root_collection_id_of` → simplify to walk to `parent_id: None` (or remove entirely).
- [ ] Update `ensure_parent_kind` to only accept `Folder` as valid parent.
- [ ] Update `node_dir_path` to remove `Collection` branch.
- [ ] Update `SharedStore` docs/comments to clarify `root_ids` now holds folders/requests.
- [ ] Update all tests.

#### 5. `src/storage.rs` (input types)

- [ ] Remove `CreateCollectionInput`, `DeleteCollectionInput`, `RenameCollectionInput`, `ReorderCollectionInput`.
- [ ] Update `CreateFolderInput` to remove `collection_id`; `parent_folder_id: Option<Ulid>` is the only parent reference.
- [ ] Update `CreateRequestInput.parent` to remove `collection_id` (only `folder_id: Option<Ulid>`).
- [ ] Update `EnvironmentScope` usage.

#### 6. `src/storage/workspace_repo.rs`

- [ ] **Load logic**: Read `beam.workspace.toml` (now contains root items), iterate to load folders/requests recursively. No `collections/` scanning.
- [ ] **Save logic**: Persist root item ordering inside `beam.workspace.toml`.
- [ ] **Folder creation**: Top-level folders go directly under workspace root.
- [ ] **Request creation**: Root-level requests go directly under workspace root.
- [ ] **Environment creation**: Only `EnvironmentScope::Global` accepted.
- [ ] **Bootstrap sample workspace**: Create a root-level request directly, no collection wrapper.
- [ ] Remove all collection-centric CRUD helpers (`rename_collection`, `delete_collection`, `reorder_collection`).
- [ ] Replace `write_root_order` with `write_workspace_file` that includes root items.
- [ ] Replace `write_collection_manifest` with `write_folder_manifest`.
- [ ] Update `persist_shared_tree` to iterate `root_ids` and write each folder manifest + requests, then write `beam.workspace.toml`.

#### 7. `src/app_shell.rs`

- [ ] Remove `TreeNodeKind::Collection`.
- [ ] Remove `CollectionUpserted`, `CollectionDeleted` events.
- [ ] Remove collection CRUD commands.
- [ ] Remove `active_collection_environment_ids` field.
- [ ] Remove `collection_ancestor_for_node` / `active_collection_id_for_selected_request`.
- [ ] Update environment resolution to only use global environments.
- [ ] Update `save_local_state` to remove `collection_environment_selection`.

#### 8. `src/ui.rs`

- [ ] Remove collection rendering from `render_tree_row`.
- [ ] Remove collection context menu entries.
- [ ] Remove `DraggedCollection`.
- [ ] Update drag-and-drop: valid parents are `Folder` only (or root).
- [ ] Update rename modal to only handle `Folder` and `Request`.
- [ ] Update environment dropdown to only show global environments.

#### 9. `docs/DATA_MODEL_REQUIREMENTS.md` & `docs/FEATURES.md`

- [ ] Remove Collection sections.
- [ ] Update hierarchy: Workspace → Folder → Request.
- [ ] Update environment scope: only Global.
- [ ] Update on-disk layout and Rust type examples.
- [ ] Update directory paths to `$HOME/beam/` and `$HOME/beam_local/`.
- [ ] Document consolidated `beam.workspace.toml` shape.

### UI/UX Changes (Phase 1)

- [ ] **Sidebar Tree**: Root items are folders and requests directly. No collection expand/collapse layer.
- [ ] **Context Menus**:
  - Background: Add Request, Add Folder
  - Folder: Add Request, Add Folder, Rename, Delete
  - Request: Send, Copy as cURL, Rename, Duplicate, Delete
- [ ] **Environments**: Only global environments. No collection environment badge in URL bar.

### Test Updates (Phase 1)

- [ ] Update all `NodeKind::Collection` test data.
- [ ] Update `paths.rs` tests.
- [ ] Update `workspace_repo.rs` CRUD tests.
- [ ] Update `models.rs` serialization tests.
- [ ] Update `app_shell.rs` integration tests.

---

## Phase 2 (V3): Multi-Workspace Support

### Goal

Support multiple self-contained workspaces under a single data root (`$HOME/beam/`). Each workspace has its own requests, folders, and environments. Local state, history, and script results are per-workspace under `$HOME/beam_local/`.

### New On-Disk Layout (Multi-Workspace)

```
$HOME/beam/
├── workspaces.toml            # Registry: list of workspaces + active_workspace_id
├── workspace-a/               # Self-contained workspace folder
│   ├── beam.workspace.toml    # Metadata + root items
│   ├── {request-slug}.request.toml
│   ├── {folder-slug}/
│   │   ├── folder.toml
│   │   ├── {request-slug}.request.toml
│   │   └── {subfolder-slug}/
│   │       └── ...
│   └── environments/
│       └── {env-slug}.env.toml
├── workspace-b/
│   └── ...
└── ...

$HOME/beam_local/
├── workspace-a/               # Local state for workspace-a
│   ├── local-state.toml
│   ├── history/
│   │   ├── by-request/
│   │   └── responses/
│   └── script_results/
├── workspace-b/
│   └── ...
└── ...
```

### Workspace Registry File

Stored at data root (`$HOME/beam/workspaces.toml`):

```toml
[registry]
active_workspace_id = "01JX8R4M6D5Q4T1N3Y8K7P2A9B"
schema_version = 3

[[workspaces]]
workspace_id = "01JX8R4M6D5Q4T1N3Y8K7P2A9B"
name = "Personal"
path = "personal"
created_at = "2026-04-28T12:00:00Z"

[[workspaces]]
workspace_id = "01JX8R9K7A3D2M4P5Q6R7S8T9U"
name = "Work"
path = "work"
created_at = "2026-04-28T12:05:00Z"
```

- `path` is a directory name relative to the data root (slugified from name).
- `workspace_id` is the stable identity; renames only change `name` and possibly `path`.
- On first launch with V3, if `workspaces.toml` is missing, create a default workspace named "default".

### Data Model Changes (Phase 2)

#### New Types

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacesRegistryFile {
    pub schema_version: u32,
    pub registry: WorkspacesRegistry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacesRegistry {
    pub active_workspace_id: Option<Ulid>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub workspace_id: Ulid,
    pub name: String,
    pub path: String,  // directory name relative to data root
    pub created_at: DateTime<Utc>,
}
```

#### Modified Types

- `BeamPaths` split into two levels:
  - `DataRootPaths` — points to `$HOME/beam/`, knows about `workspaces.toml`
  - `WorkspacePaths` — points to a specific workspace folder (`$HOME/beam/{slug}/`), derives all internal paths
- `WorkspaceRepository` is initialized per-workspace (takes a `WorkspacePaths`).
- `SharedStore` — no structural change. It continues to represent **one active workspace** in memory. The app layer swaps `WorkspaceRepository` (and thus `SharedStore`) on workspace switch.

### Disk IO Performance Notes (Phase 2)

- **Workspace switch** = save current local state, drop in-memory `SharedStore`, init new `WorkspaceRepository` for target workspace, load its `beam.workspace.toml` + tree.
- **Registry operations** (create/delete/rename workspace) rewrite `workspaces.toml` only.
- **Per-workspace isolation** means no cross-workspace file scanning. Each workspace loads independently.
- **Local state isolation** means history/script_results never leak across workspaces.

### File-by-File Changes (Phase 2)

#### 1. `src/models.rs`

- [ ] Add `WorkspacesRegistryFile`, `WorkspacesRegistry`, `WorkspaceEntry` structs.
- [ ] Bump `schema_version` to `SCHEMA_VERSION_V3` for registry file.

#### 2. `src/paths.rs`

- [ ] Split `BeamPaths` into two concepts:
  - `DataRootPaths` — points to `$HOME/beam/`, knows about `workspaces.toml`
  - `WorkspacePaths` — points to a specific workspace folder, derives all internal paths
- [ ] `BeamPaths::from_root` becomes `BeamPaths::from_data_root(data_root: PathBuf)` that returns `DataRootPaths`.
- [ ] Add `workspace_paths(data_root: &DataRootPaths, workspace_slug: &str) -> WorkspacePaths`.
- [ ] Ensure local paths derive from `$HOME/beam_local/{workspace_slug}/`.

#### 3. `src/storage/workspace_repo.rs`

- [ ] `WorkspaceRepository::new` now takes a `WorkspacePaths` instead of a generic backend with data-root paths.
- [ ] The app layer creates the repository for the **active workspace** only.
- [ ] Workspace switching = drop old repository, create new one.

#### 4. `src/storage.rs`

- [ ] Add `WorkspacesStorage` trait or extend existing with registry operations:
  - `list_workspaces() -> Vec<WorkspaceEntry>`
  - `create_workspace(name: &str) -> WorkspaceEntry`
  - `delete_workspace(workspace_id: Ulid)`
  - `set_active_workspace(workspace_id: Ulid)`
- [ ] Add input types: `CreateWorkspaceInput`, `DeleteWorkspaceInput`, `RenameWorkspaceInput`.

#### 5. New file: `src/storage/registry_repo.rs`

- [ ] Responsible for loading/saving `workspaces.toml`.
- [ ] Validates registry integrity (duplicate IDs, missing directories).
- [ ] Auto-creates default workspace on first run.

#### 6. `src/app_shell.rs`

- [ ] Hold an `Option<WorkspaceRepository>` (or always the active one).
- [ ] Add `AppEvent::WorkspaceSwitched { workspace_id }`.
- [ ] Add `AppCommand::SwitchWorkspace`, `CreateWorkspace`, `DeleteWorkspace`, `RenameWorkspace`.
- [ ] On startup: load registry, pick active workspace, initialize repository.
- [ ] On switch: save current workspace local state, drop repository, init new one, restore UI.

#### 7. `src/ui.rs`

- [ ] Add workspace picker to the title bar or sidebar header (dropdown showing current workspace name).
- [ ] Dropdown items:
  - List of existing workspaces
  - Divider
  - "New Workspace..."
  - "Delete Workspace" (disabled if only one exists)
  - "Rename Workspace"
- [ ] Show workspace name in window title.

### UI/UX Changes (Phase 2)

- [ ] **Workspace Picker**: Accessible from the top of the sidebar or title bar. Shows active workspace name.
- [ ] **New Workspace Flow**: Prompt for name → slugify → create directory → add to registry → switch to it.
- [ ] **Delete Workspace**: Confirmation modal. Not allowed if it's the only workspace.
- [ ] **Rename Workspace**: Updates `name` in registry; may also rename directory if `path` matches name.
- [ ] **Per-Workspace Isolation**: Environments, local state, history, and script results are fully isolated. Switching workspaces feels like switching projects.

### Test Updates (Phase 2)

- [ ] Add registry serialization/deserialization tests.
- [ ] Add workspace creation/deletion tests.
- [ ] Add path resolution tests for multi-workspace layout.
- [ ] Update `BeamPaths` tests.

---

## `SharedStore` Implications

### V2 Changes

`SharedStore` struct stays structurally the same:

```rust
pub struct SharedStore {
    pub nodes: HashMap<NodeId, Node>,
    pub requests: HashMap<NodeId, RequestFile>,
    pub root_ids: Vec<NodeId>,       // Now holds Folder & Request IDs (was Collection IDs)
    pub name_index: HashMap<String, NodeId>,
    pub environments: HashMap<NodeId, EnvironmentFile>,
}
```

The semantic change is that `root_ids` now represents the **workspace root** items (folders and requests directly under the workspace), not a list of collections. `parent_id: None` in a `Node` now means "root-level item" rather than "collection root".

### V3 Changes

`SharedStore` does **not** gain workspace scoping fields. The app loads **one workspace at a time**, so `SharedStore` always represents the active workspace only. Workspace identity is tracked by:

- `WorkspacesRegistry` (in `registry_repo.rs`) — knows all workspaces + active one.
- `WorkspaceRepository` — bound to a specific workspace's `WorkspacePaths`.
- `app_shell.rs` — holds the active `WorkspaceRepository` and manages switching.

This keeps `SharedStore` lightweight and avoids the complexity of managing multiple workspace trees in memory simultaneously.

---

## Breaking Change Rollout

### V2 (Collection Removal)

- [ ] Bump all `schema_version` defaults to `2`.
- [ ] On startup, ignore V1 files. Show a one-time notice that data must be re-created.
- [ ] Delete old `collections/` handling entirely.
- [ ] Update docs to note V2 data model.

### V3 (Multi-Workspace)

- [ ] Bump registry and workspace file schema versions to `3`.
- [ ] On first V3 launch: if `workspaces.toml` is missing, migrate the existing single-workspace layout into a default workspace folder named "default".
  - Move `beam.workspace.toml`, requests, folders, `environments/` into `$HOME/beam/default/`.
  - Move `local-state.toml`, `history/`, `script_results/` into `$HOME/beam_local/default/`.
  - Create `workspaces.toml` with "default" as the only entry and active workspace.
- [ ] Future launches use the registry to locate and switch workspaces.

---

## Summary of Schema Versions

| Version | Change |
|---------|--------|
| V1 | Original model with Collections |
| V2 | Collections removed; `beam.workspace.toml` contains metadata + root items; only global environments; flat workspace root |
| V3 | Multi-workspace support; data root contains `workspaces.toml` registry and workspace folders; local state per-workspace under `$HOME/beam_local/` |

---

## Open Questions (to resolve during implementation)

1. **Should the sidebar pane be renamed from "Collections" to "Workspace" or "Requests"?**
2. **Should workspace directory names auto-slugify from the workspace name, or use the raw name?**
3. **Should deleting a workspace also delete its directory and all data, or move it to a trash/archive location?**
