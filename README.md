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
- Response history view in the response pane
- Persistent TOML-based storage for workspaces, requests, environments, local history, and script results on your machine
- Theme support with light/dark mode and a clean desktop interface

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
