# AGENTS.md

This document provides guidance for AI coding agents working with the Beam HTTP client codebase.

Keep all LLM replies concise and accurate. Prefer the short response that fully answers the user's request. Clarify with user if anything no clear.

## Project Overview

Beam is a fast, lightweight HTTP client built with Rust and the gpui GUI framework. It provides features similar to Postman or Insomnia, including request collections, environment variables, authentication methods, and post-request scripting.

## Supported Features

// TODO

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
├── ui.rs                    # GPUI views and rendering — the main GUI layer (panels, editors, menus, etc.)
├── workspace_tree.rs        # Pure domain model — in-memory tree (SharedStore, Node, NodeKind, manifests)
├── models.rs                # Serializable DTOs — RequestFile, EnvironmentFile, WorkspaceFile, LocalStateFile, etc.
├── request_authoring.rs     # Request authoring state — tabs, send-button logic, validation helpers
├── script.rs                # Post-request script execution — QuickJS runtime, console capture, test results
├── schema.rs                # Schema versioning — SCHEMA_VERSION_V1, SchemaKind, version validation
├── paths.rs                 # File-system path definitions — BeamPaths, collection/environment directory layout
├── error.rs                 # Error types — BeamError enum and Result<T> alias
├── assets.rs                # Asset helpers — embedded theme contents, icon paths
└── storage/
    ├── mod.rs               # Storage DTOs + WorkspaceStorage trait — CRUD input structs, BootstrapReport
    ├── io_backend.rs        # StorageIoBackend trait — abstract I/O (read/write TOML, dirs, rename, remove)
    ├── workspace_repo.rs    # WorkspaceRepository — primary repository, all CRUD operations on SharedStore
    └── fs_backend.rs        # FileSystemStorage — concrete std::fs adapter implementing StorageIoBackend
```

### Key Components

| Module / File | Role |
|---|---|
| `workspace_tree.rs` | **Pure domain model**. Holds `SharedStore` (in-memory tree), `Node`/`NodeKind`, manifest structs, and tree-manipulation helpers (name scoping, uniqueness checks, move/reorder logic). No I/O. |
| `models.rs` | **Serializable data structures**. Every TOML-backed entity (requests, environments, workspace, local state) is defined here. Used by both the domain layer and the storage layer. |
| `storage/mod.rs` | **Storage contracts & DTOs**. Defines the `WorkspaceStorage` trait and all input structs consumed by repository methods (`CreateRequestInput`, `MoveFolderInput`, etc.). Also holds `BootstrapReport`. |
| `storage/io_backend.rs` | **I/O abstraction**. The `StorageIoBackend` trait decouples repository logic from the file system so tests can swap in a fake backend. |
| `storage/fs_backend.rs` | **Concrete file-system adapter**. `FileSystemStorage` implements `StorageIoBackend` using `std::fs`. Handles TOML serialization, atomic writes, and path-based operations. |
| `storage/workspace_repo.rs` | **Primary repository**. `WorkspaceRepository<B: StorageIoBackend>` loads the full workspace into `SharedStore`, then performs all CRUD (create, rename, move, delete, duplicate, reorder) while keeping disk and in-memory state in sync. |
| `app_shell.rs` | **Application shell & orchestration**. Owns `AppShellState`, `DataSyncRuntime`, pane-split layout, startup preload logic, and the background command queue that feeds the repository. |
| `ui.rs` | **GPUI front-end**. All view rendering, event handling, context menus, modal dialogs, and user-interaction logic lives here. |
| `request_authoring.rs` | **Request editor state**. Tab enums, send-button states, and validation helpers for the request authoring panel. |
| `script.rs` | **Script engine**. Executes post-request JavaScript via `rquickjs`, captures console output, and returns `ScriptExecutionResult`. |
| `schema.rs` | **Schema compatibility**. Central place for version constants and per-entity schema validation. |
| `paths.rs` | **Path conventions**. `BeamPaths` defines where collections, environments, local state, and workspace files live on disk. |
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
When loading collections from disk, the storage layer skips individual corrupted items and returns warnings rather than failing the entire load:

- **Corrupted request files** (invalid TOML, schema mismatch, duplicate `request_id`) are skipped with a warning.
- **Corrupted folder manifests** (missing `folder.toml`, invalid TOML, schema mismatch, wrong `collection_id`, wrong `parent_folder_id`, or duplicate `folder_id`) are skipped with a warning. The folder and its contents are omitted from the loaded collection, but the rest of the collection continues to load normally.
- **Warnings** are collected in a `Vec<String>` and displayed as red text in the UI collection panel.

This pattern ensures that a single corrupted file on disk never renders the entire workspace unreadable.

#### Environment Variable Resolution
Requests support variable substitution from active environment.
// TODO

#### Editor/Input Context Menu Enablement
When adding a custom context menu for any editor (`Input`/`InputState`) in Beam, do not assume default enable/disable behavior is preserved. Reuse shared helper builders in `src/ui.rs` for all text editing menus instead of hand-rolled menu blocks.

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
- For UI colors, always use theme tokens from `cx.theme()` and avoid hard-coded/custom color values (for example, avoid direct `rgb(...)`/`rgba(...)` color literals in UI styling).

## Key Data Structures

// TODO

### RequestConfig
The core request configuration structure containing:
- HTTP method, URL, headers, params
- Authentication settings
- Request body and format
- Post-request script
- Last response data
- Collection/request indices

### Environment
Environment variables for request configuration:
- Name and description
- Key-value variable pairs
- Active environment tracking

### RequestCollection
Hierarchical organization of requests:
- Collection name and metadata
- List of requests
- Expanded/collapsed state
- Folder name for storage

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
- GraphQL support
- WebSocket support
- Request history
- Import/export functionality
- Collaborative features
- Cloud sync

## Getting Help

- Check existing code patterns in similar features
- Review gpui's documentation for UI questions
- Consult Rust documentation for language features
- Check GitHub issues for known problems
