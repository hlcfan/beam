# AGENTS.md

This document provides guidance for AI coding agents working with the Beam HTTP client codebase.

Keep all LLM replies concise and accurate. Prefer the short response that fully answers the user's request. Clarify with user if anything no clear.

## Project Overview

Beam is a fast, lightweight HTTP client built with Rust and the gpui GUI framework. It provides features similar to Postman or Insomnia, including multi-workspace support, environment variables, authentication methods, and post-request scripting.

The hierarchy is: **Workspace → Folders → Requests**. Collections have been removed. Only global environments exist (no collection-scoped environments).

## Supported Features

- HTTP methods: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS, QUERY
- Auth: Bearer, Basic, API Key (header or query)
- Body types: JSON, XML, Text, GraphQL, Form URL-Encoded, Multipart
- Global environments with `{{variable}}` interpolation
- Post-response JavaScript scripting
- Multi-workspace organization with folders
- Import Postman collections/environments and Insomnia JSON or v5 YAML exports
- Paste cURL commands into the URL field to populate the current request
- Response history view (next to Headers in response pane)
- Theme support (light/dark), font size settings, response auto-format

## Architecture

### Core Technologies

- **Language**: Rust 1.70+
- **GPUI Framework**: gpui
- **HTTP Client**:
- **Storage**: File-based persistence using TOML format
- **Scripting**: JavaScript execution for post-request scripts (via `boa` engine)

### Project Structure

```
src/
├── main.rs                  # Application entry point — bootstraps storage, initializes workspace, launches UI
├── lib.rs                   # Crate root — declares all public modules
├── app_shell.rs             # App-level state management, data-sync worker, layout state, startup preload
├── ui.rs                    # UI module root — declares submodules and exposes run_app
├── ui/
│   ├── actions.rs           # GPUI actions, application menus, and action registration
│   ├── app_events.rs        # Data-sync polling and application event handling
│   ├── beam_view.rs         # BeamView state, initialization, focus, and top-level rendering
│   ├── environment.rs       # Active environment selection, refresh, and interpolation hover UI
│   ├── persistence.rs       # UI-state persistence not owned by request/response modules
│   ├── text_edit_menu.rs    # Shared input and editor context-menu builders
│   ├── theme.rs             # Theme registry and theme/font/format setting application
│   ├── dialogs.rs           # Dialog module root and shared dialog helpers
│   ├── dialogs/             # Import, environment, settings, key binding, tree, and workspace dialogs
│   ├── request.rs           # Request UI module root and shared request behavior
│   ├── request/             # Body, cURL, editor, execution, and request persistence
│   ├── response.rs          # Response UI module root and shared response behavior
│   ├── response/            # History, persistence, and response rendering
│   ├── tree.rs              # Workspace tree UI module root and shared tree behavior
│   └── tree/                # Tree actions, drag/drop, and rendering
├── workspace_tree.rs        # Pure domain model — in-memory tree (SharedStore, Node, NodeKind, manifests)
├── tree_dnd.rs              # Pure tree drag/drop destination calculations
├── models.rs                # Serializable DTOs — RequestFile, EnvironmentFile, WorkspaceFile, WorkspacesRegistryFile, LocalStateFile, etc.
├── request_authoring.rs     # Request authoring state — tabs, send-button logic, validation helpers
├── importers.rs             # Format detection, shared ImportPlan IR, batch-plan merging
├── importers/
│   ├── curl.rs              # cURL parser — updates the currently open request in place
│   ├── insomnia.rs          # Insomnia JSON and v5 YAML detection/parsing
│   ├── postman.rs           # Postman collection v2.0/v2.1 and environment parsing
│   └── scanner.rs           # Bounded recursive folder scanning for import batches
├── script.rs                # Post-request script execution — QuickJS runtime, console capture, test results
├── schema.rs                # Schema versioning — SCHEMA_VERSION_V1/V3, SchemaKind, version validation
├── paths.rs                 # File-system path definitions — DataRootPaths, BeamPaths, slugify
├── error.rs                 # Error types — BeamError enum and Result<T> alias
├── assets.rs                # Asset helpers — embedded theme contents, icon paths
├── storage.rs               # Storage DTOs + WorkspaceStorage trait — CRUD input structs, BootstrapReport
└── storage/
    ├── io_backend.rs        # StorageIoBackend trait — abstract I/O (read/write TOML, dirs, rename, remove)
    ├── workspace_repo.rs    # WorkspaceRepository — primary repository, all CRUD operations on SharedStore
    ├── registry_repo.rs     # RegistryRepository — loads/saves workspaces.toml, bootstraps default workspace
    └── fs_backend.rs        # FileSystemStorage — concrete std::fs adapter implementing StorageIoBackend
```

### Key Components

| Module / File | Role |
|---|---|
| `workspace_tree.rs` | **Pure domain model**. Holds `SharedStore` (in-memory tree), `Node`/`NodeKind` (`Folder` \| `Request`), manifest structs, and tree-manipulation helpers (name scoping, uniqueness checks, move/reorder logic). No I/O. |
| `models.rs` | **Serializable data structures**. Every TOML-backed entity (requests, environments, workspace, workspaces registry, local state) is defined here. Used by both the domain layer and the storage layer. |
| `storage.rs` | **Storage contracts & DTOs**. Defines the `WorkspaceStorage` trait and all input structs consumed by repository methods (`CreateRequestInput`, `MoveFolderInput`, etc.). Also holds `BootstrapReport`. Parent refs (`RequestParentRef`, `FolderParentRef`) use `folder_id: Option<Ulid>` — `None` means workspace root. |
| `storage/io_backend.rs` | **I/O abstraction**. The `StorageIoBackend` trait decouples repository logic from the file system so tests can swap in a fake backend. |
| `storage/fs_backend.rs` | **Concrete file-system adapter**. `FileSystemStorage` implements `StorageIoBackend` using `std::fs`. Handles TOML serialization, atomic writes, and path-based operations. |
| `storage/workspace_repo.rs` | **Primary repository**. `WorkspaceRepository<B: StorageIoBackend>` loads the full workspace into `SharedStore`, then performs all CRUD (create, rename, move, delete, duplicate, reorder) while keeping disk and in-memory state in sync. |
| `storage/registry_repo.rs` | **Workspace registry**. `RegistryRepository` loads/saves `workspaces.toml`, bootstraps a default workspace on first run, and manages multi-workspace CRUD (create, delete, rename, switch). |
| `app_shell.rs` | **Application shell & orchestration**. Owns `AppShellState`, `DataSyncRuntime`, pane-split layout, startup preload logic, and the background command queue that feeds the repository. Manages workspace switching. |
| `ui.rs` / `ui/` | **GPUI front-end**. `ui.rs` is the module root and public entry point. Cohesive submodules own `BeamView`, actions, application events, dialogs, environments, request authoring/execution/persistence, response history/rendering/persistence, themes, text-edit menus, and workspace-tree interactions. Keep new UI code in the narrowest owning module rather than growing `ui.rs`. |
| `tree_dnd.rs` | **Pure tree drag/drop logic**. Calculates drop destinations independently of GPUI and `BeamView`; UI drag state and rendering remain under `ui/tree/`. |
| `request_authoring.rs` | **Request editor state**. Tab enums, send-button states, and validation helpers for the request authoring panel. |
| `importers.rs` / `importers/` | **Import pipeline**. Detects Postman and Insomnia exports, parses them into the format-neutral `ImportPlan`, merges multi-file batches, and provides bounded folder scanning. The cURL parser uses a separate `CurlPlan` because it updates the open request instead of materializing a workspace tree. |
| `script.rs` | **Script engine**. Executes post-request JavaScript via `rquickjs`, captures console output, and returns `ScriptExecutionResult`. |
| `schema.rs` | **Schema compatibility**. Version constants (`SCHEMA_VERSION_V1` for workspace/request/environment/local-state, `SCHEMA_VERSION_V3` for workspaces registry) and per-entity schema validation. |
| `paths.rs` | **Path conventions**. `DataRootPaths` points to `$HOME/beam/` and knows `workspaces.toml`. `BeamPaths` is per-workspace and derives all internal paths. `slugify` converts names to directory-safe slugs. |
| `error.rs` | **Error taxonomy**. `BeamError` covers I/O, TOML encode/decode, schema mismatch, not-found, and validation errors. |

## Development Guidelines

### Naming convention [!important]

For the variable naming, dont mention `legacy` and `compatible`, treat it as neutral.

### Making Code Changes

// TODO

### Common Patterns

#### Debounced Saves
The app uses a debounce pattern for auto-saving requests.
// TODO


#### Async Storage Operations
// TODO

#### Graceful Loading Degradation
When loading a workspace from disk, the storage layer skips individual corrupted items and returns warnings rather than failing the entire load:

- **Corrupted request files** (invalid TOML, schema mismatch, duplicate `request_id`) are skipped with a warning.
- **Corrupted folder manifests** (missing `folder.toml`, invalid TOML, schema mismatch, wrong `parent_folder_id`, or duplicate `folder_id`) are skipped with a warning. The folder and its contents are omitted, but the rest of the workspace continues to load normally.
- **Warnings** are collected in a `Vec<String>` and displayed as red text in the UI sidebar.

This pattern ensures that a single corrupted file on disk never renders the entire workspace unreadable.

#### Environment Variable Resolution
Requests support variable substitution from active environment.
// TODO

#### Import Pipeline
File imports are split into detection/parsing and materialization:

- `src/importers.rs` owns the format-neutral `ImportPlan`, `PlannedFolder`, `PlannedRequest`, and `PlannedEnvironment` types.
- Detectors and parsers in `src/importers/` convert Postman collections v2.0/v2.1, Postman environments, and Insomnia JSON or v5 YAML exports into an `ImportPlan`.
- `merge_file_plans` combines a batch. If multiple files contain request trees, each tree is wrapped in a top-level folder so file boundaries remain visible.
- A batch containing an Insomnia workspace creates a new workspace. Collection-only, environment-only, and other non-workspace batches import into the active workspace.
- `AppCommand::RunImport` sends materialization to the data-sync worker. The runner in `app_shell.rs` is format-agnostic and emits `AppEvent::ImportResult`.
- Cancellation is cooperative. A failure after creating a new workspace may leave a partial workspace on disk, which the UI reports to the user.
- Folder scanning is bounded by `MAX_SCAN_DEPTH` and `MAX_FILES_PER_SCAN`, skips hidden/VCS/package directories, and never follows symlinks.
- cURL is intentionally separate: `CurlPlan` populates the currently open request when a cURL command is pasted into the URL input.

When adding a file format, add its detector/parser and register the detector in `DETECTORS`; keep source-specific types out of the materialization runner.

#### Editor/Input Context Menu Enablement
When adding a custom context menu for any editor (`Input`/`InputState`) in Beam, do not assume default enable/disable behavior is preserved. Reuse shared helper builders in `src/ui/text_edit_menu.rs` for all text editing menus instead of hand-rolled menu blocks.

- Explicitly gate `Cut` and `Copy` by selection state (`!selected_range().is_empty()`).
- Disable `Cut`/`Copy` menu items when there is no selected text.
- Keep menu item set consistent unless a feature intentionally differs.
- Editor context menu items (code/multiline editor): `Format`, `Find`, `Cut`, `Copy`, `Paste`, `Select All`.
- Input context menu items (single-line/default input): `Cut`, `Copy`, `Paste`, `Select All`.
- Context menu items should show the icons and keyboard shortcuts.
- `context_menu_item_row(label, icon_path, shortcut, muted_color)`: shared row renderer (icon + label + shortcut + pointer cursor).
- `context_menu_action_item(label, icon_path, shortcut, muted_color, action, disabled)`: wraps a row into a `PopupMenuItem` with action + disabled state.
- `build_text_edit_context_menu(menu, has_selection, muted_color)`: standard text-edit menu for `Input` with `Cut`, `Copy`, `Paste`, `Select All`.
- `build_text_edit_context_menu_with_find(menu, has_selection, muted_color)`: editor variant that prepends `Find` and then uses `build_text_edit_context_menu(...)`.
- Compute selection state before menu build (`let has_selection = !input.read(cx).selected_range().is_empty();`) and pass it to helper builders.
- For rich/code editors, add feature-specific items like `Format` first, then chain to `build_text_edit_context_menu_with_find(...)` to keep the shared behavior consistent.

Current icon paths in shared helper menus:
- Find: `icons/search.svg`
- Cut: `icons/cut.svg`
- Copy: `icons/copy.svg`
- Paste: `icons/clipboard-paste.svg`
- Select All: `icons/square-dashed-text.svg`

#### Button Cursor Behavior
When adding clickable buttons in Beam, set `.cursor_pointer()` consistently for all interactive button states, not only prominent actions. This includes secondary actions like `Cancel`, ghost buttons, title bar buttons, toolbar buttons, and dialog or modal actions such as `Create`, `Rename`, or `Delete`.


### Testing

Beam uses a tiered testing strategy:

1. **Unit Tests**: Test core logic in isolation.
   - Run all tests: `cargo test`

### Code Style

- Follow Rust standard formatting (`cargo fmt`)
- Use `cargo clippy` for linting
- Prefer explicit error handling over `.unwrap()`
- Use logging (`log` crate) for debugging
- Keep functions focused and modular
- Follow top-down function ordering (the step-down rule): place public and higher-level functions before private helpers, and place called functions below the functions that call them.
- For UI colors, always use theme tokens from `cx.theme()` and avoid hard-coded/custom color values (for example, avoid direct `rgb(...)`/`rgba(...)` color literals in UI styling).

## Key Data Structures

### On-Disk Layout

```
$HOME/beam/
├── workspaces.toml                  # Registry: workspace list + active_workspace_id (schema_version = 3)
├── my-workspace/                    # Per-workspace directory (slugified name)
│   ├── beam.workspace.toml          # Workspace metadata + root item ordering (schema_version = 1)
│   ├── {request-slug}.request.toml  # Root-level request
│   ├── {folder-slug}/               # Root-level folder
│   │   ├── folder.toml              # Folder manifest + child ordering
│   │   ├── {request-slug}.request.toml
│   │   └── {subfolder-slug}/
│   │       ├── folder.toml
│   │       └── ...
│   └── environments/
│       └── {env-slug}.env.toml
└── ...

$HOME/beam_local/
└── my-workspace/                    # Per-workspace local state (non-synced)
    ├── local-state.toml
    ├── history/
    │   ├── by-request/
    │   └── responses/
    └── script_results/
```

### Core Rust Types

| Type | Location | Purpose |
|------|----------|---------|
| `WorkspacesRegistryFile` | `models.rs` | Top-level `workspaces.toml`; contains `WorkspacesRegistry` + `schema_version = 3` |
| `WorkspaceEntry` | `models.rs` | One workspace in the registry: `workspace_id`, `name`, `path` (slug), `created_at` |
| `WorkspaceFile` | `models.rs` | `beam.workspace.toml`; workspace metadata + `items: Vec<ManifestItemRef>` for root ordering |
| `ManifestItemRef` | `models.rs` | Ordering entry in workspace or folder manifest: `item_id`, `item_type` (`folder`\|`request`), `name`, `order` |
| `FolderFile` | `models.rs` | `folder.toml`; folder metadata + `items: Vec<ManifestItemRef>` for child ordering |
| `RequestFile` | `models.rs` | `{slug}.request.toml`; full request payload including `meta`, `request`, `auth`, `body`, `scripts` |
| `EnvironmentFile` | `models.rs` | `{slug}.env.toml`; global environment with `variables: Vec<EnvironmentVariable>` |
| `LocalStateFile` | `models.rs` | `local-state.toml`; active env, last opened request, theme, expanded tree nodes |
| `SharedStore` | `workspace_tree.rs` | In-memory workspace tree: `nodes`, `requests`, `root_ids`, `name_index`, `environments` |
| `Node` / `NodeKind` | `workspace_tree.rs` | Tree node; `NodeKind` is `Folder` or `Request` (no Collection) |
| `DataRootPaths` | `paths.rs` | Paths for `$HOME/beam/` + `workspaces.toml` + `$HOME/beam_local/` |
| `BeamPaths` | `paths.rs` | Per-workspace paths: `root`, `workspace_file`, `environments_dir`, `local_dir`, `local_state_file` |
| `RegistryRepository` | `storage/registry_repo.rs` | Loads/saves `workspaces.toml`; bootstraps default workspace on first run |
| `WorkspaceRepository` | `storage/workspace_repo.rs` | All CRUD on a single workspace's `SharedStore` |
| `ImportPlan` | `importers.rs` | Format-neutral batch of planned folders, requests, environments, and warnings |
| `CurlPlan` | `importers/curl.rs` | Parsed cURL request applied directly to the active request editor |
| `ImportJob` / `ImportResult` | `app_shell.rs` | Cancellable worker input and materialization outcome for file imports |

### Environment
Environments are **global only** (no collection-scoped environments).
- Stored under `environments/{slug}.env.toml` inside the workspace directory.
- `EnvironmentScope` has a single variant: `Global`.

## Debugging Tips

- Enable debug logging: `RUST_LOG=debug cargo run`
- Check storage location for persisted data

## Dependencies

Key dependencies to be aware of:
// TODO

## Performance Considerations

- UI updates should be fast and non-blocking
- Use async operations for I/O (network, file system)
- Debounce frequent operations (auto-save)
- Consider response size when formatting/displaying
- Lazy load large collections if needed

## Security Notes

- Credentials stored in plain text TOML files
- Post-request scripts execute in sandboxed environment
- Be cautious with script execution permissions
- Consider encryption for sensitive data in future

## Future Enhancement Areas

See `TODO.md` for planned features. Common enhancement areas:
- Additional authentication methods
- GraphQL subscriptions
- WebSocket support
- Request history (currently only response history is stored)
- Export functionality
- Collaborative features
- Cloud sync

## Getting Help

- Check existing code patterns in similar features
- Review gpui's documentation for UI questions
- Consult Rust documentation for language features
- Check GitHub issues for known problems
