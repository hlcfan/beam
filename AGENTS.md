# AGENTS.md

This document provides guidance for AI coding agents working with the Beam HTTP client codebase.

## Project Overview

Beam is a fast, lightweight HTTP client built with Rust and the Iced GUI framework. It provides features similar to Postman or Insomnia, including request collections, environment variables, authentication methods, and post-request scripting.

## Architecture

### Core Technologies

- **Language**: Rust 1.70+
- **GUI Framework**: Iced (reactive GUI framework)
- **HTTP Client**: Custom implementation in `src/http/`
- **Storage**: File-based persistence using TOML format
- **Scripting**: JavaScript execution for post-request scripts (via `boa` engine)

### Project Structure

```
beam/
├── src/
│   ├── main.rs              # Application entry point, main state management
│   ├── http/                # HTTP request handling
│   ├── storage/             # Persistence layer
│   ├── ui/                  # UI components and panels
│   ├── script/              # Post-request script execution
│   └── types.rs             # Core data structures
├── assets/                  # Application assets
├── examples/                # Example configurations
└── Cargo.toml              # Rust dependencies
```

### Key Components

1. **BeamApp** (`src/main.rs`): Main application state and update logic
   - Manages three-panel layout (Collections, Request Config, Response)
   - Handles message routing and state updates
   - Coordinates between UI panels and business logic

2. **Storage Layer** (`src/storage/`):
   - File-based persistence using TOML
   - Manages collections, requests, environments
   - Tracks last opened request for session restoration

3. **UI Panels** (`src/ui/`):
   - `CollectionPanel`: Request collection tree view
   - `RequestPanel`: Request configuration (URL, headers, body, auth)
   - `ResponsePanel`: Response display and formatting
   - `EditorView`: Custom widget wrapper for text editors providing:
     - Line number gutter with automatic width calculation
     - Keyboard shortcut interception (Cmd+Z/Redo/Cmd+F)
     - Search result highlighting with word-wrap support
     - Optimized rendering with viewport culling and caching

4. **HTTP Module** (`src/http/`):
   - Request execution
   - Authentication handling
   - Response parsing

## Development Guidelines

### Making Code Changes

1. **UI Changes**: 
   - UI is built using Iced's reactive pattern
   - Each panel has its own `Message` enum and `update()` function
   - State flows from `BeamApp` down to panels
   - User actions flow up as messages

2. **Adding Features**:
   - Add new message variants to appropriate `Message` enum
   - Implement handler in corresponding `update()` function
   - Update UI in `view()` function
   - Consider persistence requirements

3. **Storage Changes**:
   - All persistence goes through `StorageManager`
   - Use async operations to avoid blocking UI
   - Handle errors gracefully with logging

### Common Patterns

#### Debounced Saves
The app uses a debounce pattern for auto-saving requests:
```rust
// Send to debounce channel
if let Some(tx) = &self.debounce_tx {
    if let Err(_) = tx.try_send(request_config) {
        info!("Debounce channel is full or closed");
    }
}
```

#### Async Storage Operations
Storage operations are non-blocking:
```rust
tokio::spawn(async move {
    if let Ok(storage_manager) = StorageManager::with_default_config() {
        storage_manager.storage().save_collection(&collection);
    }
});
```

#### Environment Variable Resolution
Requests support variable substitution from active environment:
```rust
let resolved_config = self.resolve_request_config_variables(&self.current_request);
```

#### EditorView Widget Pattern
The `EditorView` widget wraps text editors to provide enhanced functionality:

**Keyboard Event Interception**:

- Intercepts Cmd+Z, Cmd+Y, Cmd+Shift+Z, and Cmd+F before the wrapped editor

**Line Number Rendering**:
- Calculates gutter width based on line count digits
- Caches line heights to avoid repeated measurements
- Uses viewport culling to only render visible line numbers
- Handles word-wrapped lines by measuring actual text height

**Search Result Highlighting**:
- Renders semi-transparent overlays on matching text selections
- Uses token-based word-wrap simulation to match TextEditor behavior
- Correctly handles multi-line wrapped selections
- Accounts for text editor padding, borders, and scrollbar width


### Testing

- Run tests: `cargo test`
- Run application: `cargo run`
- Build release: `cargo build --release`
- Build macOS app: `./build_macos_app.sh`

### Code Style

- Follow Rust standard formatting (`cargo fmt`)
- Use `cargo clippy` for linting
- Prefer explicit error handling over `.unwrap()`
- Use logging (`log` crate) for debugging
- Keep functions focused and modular

## Key Data Structures

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

## Common Tasks

### Adding a New Authentication Method

1. Add variant to `AuthType` enum in `types.rs`
2. Add UI fields in `RequestPanel`
3. Implement auth logic in HTTP module
4. Update request serialization/deserialization

### Adding a New Panel

1. Create new module in `src/ui/`
2. Define `Message` enum and `update()` function
3. Add panel to `PaneContent` enum
4. Update pane grid initialization in `BeamApp::default()`
5. Add message routing in `BeamApp::update()`

### Modifying Storage Format

1. Update data structures in `types.rs`
2. Implement migration logic in `storage/` module
3. Update serialization/deserialization
4. Test with existing data files

## Debugging Tips

- Enable debug logging: `RUST_LOG=debug cargo run`
- Check storage location for persisted data
- Use Iced's built-in debugging features
- Monitor async task completion with logging

## Dependencies

Key dependencies to be aware of:
- `iced`: GUI framework
- `reqwest`: HTTP client
- `tokio`: Async runtime
- `serde`: Serialization
- `toml`: Configuration format
- `boa_engine`: JavaScript execution
- `log` / `env_logger`: Logging

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
- Review Iced documentation for UI questions
- Consult Rust documentation for language features
- Check GitHub issues for known problems
