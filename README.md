# Beam

A HTTP client for developers built with Rust and gpui and gpui-component.

![Beam Screenshot](screenshot.png)

## Features

- Native, local-first, no telemetry
- Multi-workspace organization with folders and requests
- HTTP request with common methods (GET, POST, PUT, DELETE, PATCH, HEAD, QUERY, and OPTIONS)
- Authentication support for Bearer, Basic, and API Key (header or query)
- Request body formats for JSON, XML, Text, GraphQL, Form URL-Encoded, and Multipart
- Global environments with `{{variable}}` interpolation in URLs, params, headers, auth, and bodies
- Post-response JavaScript scripting with console output, tests, and environment updates
- Import Postman collections and environments, plus Insomnia JSON and v5 YAML exports
- Paste cURL commands into the URL field to populate the current request
- Command palette for quickly opening requests, revealing folders, and running common commands
- Response history view in the response pane
- Persistent TOML-based storage for workspaces, requests, environments, local history, and script results on your machine
- Theme support with light/dark mode and a clean desktop interface

## Importing

Open **Import** from the status bar, then select one or more files or a folder. Beam detects and imports:

- Postman collections v2.0 and v2.1
- Postman environments
- Insomnia JSON exports
- Insomnia v5 YAML collections

Multiple files can be imported as one batch. When a batch contains multiple collections, Beam keeps each collection in its own top-level folder. Insomnia workspace exports create a new workspace; collections and environments import into the active workspace.

Multiple workspace exports can be imported together, but they cannot be combined with separate Postman environment files because the target workspace would be ambiguous.

You can also paste a cURL command directly into the request URL field to fill the current request's method, URL, headers, query parameters, authentication, and body.

## Privacy

Beam is local-first. It stores your data in local TOML files on your machine so you can inspect, back up, and manage it yourself.

Beam does not send telemetry or analytics data. There is no account requirement and no cloud dependency for core usage.

## Installation

### macOS (Apple Silicon)

```bash
# Extract and move to Applications
tar -xzf beam-macos-aarch64.tar.gz
mv Beam.app /Applications/

# Since the app is unsigned ad-hoc, macOS Gatekeeper will show a "damaged" error or a warning.
# Run this command to remove the quarantine attribute and allow it to open:
xattr -cr /Applications/Beam.app
```

### Windows

Extract `beam-windows-x86_64.zip` and run `beam.exe`

### Linux

```bash
tar -xzf beam-linux-x86_64.tar.gz
./beam
```

## Development

### Prerequisites

- Rust 1.70+

### Build

```bash
cargo build
```

### Run

```bash
cargo run
```

### Test

```
cargo test
```

## License

GPL-3.0.
