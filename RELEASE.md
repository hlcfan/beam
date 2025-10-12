# Release Process

This project uses GitHub Actions to automatically build and release binaries for multiple platforms.

## Supported Platforms

- **Windows (x86_64)**: `beam-windows-x86_64.zip`
- **macOS Intel (x86_64)**: `beam-macos-x86_64.tar.gz`
- **macOS Apple Silicon (arm64)**: `beam-macos-arm64.tar.gz`
- **Linux (x86_64)**: `beam-linux-x86_64.tar.gz`

## How to Create a Release

### Automatic Release (Recommended)

1. Create and push a new tag:
   ```bash
   git tag v1.0.0
   git push origin v1.0.0
   ```

2. The GitHub Actions workflow will automatically:
   - Build binaries for all supported platforms
   - Create a new GitHub release
   - Upload all platform binaries as release assets

### Manual Release

You can also trigger a release manually from the GitHub Actions tab:

1. Go to the "Actions" tab in your GitHub repository
2. Select the "Release" workflow
3. Click "Run workflow"
4. Enter the desired tag name (e.g., `v1.0.0`)
5. Click "Run workflow"

## Workflow Details

The release workflow (`.github/workflows/release.yml`) includes:

- **Cross-platform builds**: Uses GitHub's hosted runners for Windows, macOS, and Linux
- **Rust toolchain setup**: Automatically installs the correct Rust toolchain for each target
- **Dependency caching**: Caches Cargo dependencies to speed up builds
- **System dependencies**: Installs required system libraries (GTK, WebKit) for Linux builds
- **Artifact creation**: Creates compressed archives for each platform
- **Release automation**: Automatically creates GitHub releases with all binaries

## Build Targets

- `x86_64-pc-windows-msvc` (Windows)
- `x86_64-apple-darwin` (macOS Intel)
- `aarch64-apple-darwin` (macOS Apple Silicon)
- `x86_64-unknown-linux-gnu` (Linux)

## Dependencies

The workflow automatically handles all build dependencies:

- **Linux**: GTK3, WebKit2GTK, AppIndicator, librsvg, patchelf
- **macOS**: No additional dependencies required
- **Windows**: No additional dependencies required

All builds are optimized for release with LTO (Link Time Optimization) enabled.