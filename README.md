# Beam

A HTTP client for developers built with Rust and Iced.

## Features

- 🚀 Fast and lightweight
- 📁 Request collections
- 🔐 Multiple authentication methods (Bearer, Basic, API Key)
- 🌍 Environment variables support
- 📝 Request body formats (JSON, XML, Text)
- 📜 Post-request scripts with JavaScript
- 💾 Persistent storage for requests and collections
- 🎨 Clean, intuitive interface

## Installation

### macOS

```bash
# Extract and move to Applications
tar -xzf beam-macos-*.tar.gz
mv Beam.app /Applications/
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
cargo build --release
```

### Run

```bash
cargo run
```

## License

MIT
