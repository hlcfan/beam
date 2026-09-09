#!/bin/bash

# Build script for Beam macOS app
# This script builds the macOS app bundle for distribution

set -e

echo "🚀 Building Beam macOS App..."

# Clean previous builds
echo "🧹 Cleaning previous builds..."
cargo clean --release

# Build the release binary
echo "🔨 Building release binary..."
cargo build --release --locked --bin beam

# Create the macOS app bundle
echo "📦 Creating macOS app bundle..."
cargo bundle --release

# Verify the app bundle was created
if [ -d "target/release/bundle/osx/Beam.app" ]; then
    echo "✅ Successfully created Beam.app"
    echo "📍 Location: $(pwd)/target/release/bundle/osx/Beam.app"
    echo ""
    echo "🎉 Build complete! You can now:"
    echo "   • Run the app: open target/release/bundle/osx/Beam.app"
    echo "   • Copy to Applications: cp -r target/release/bundle/osx/Beam.app /Applications/"
    echo "   • Create a DMG for distribution"
else
    echo "❌ Failed to create app bundle"
    exit 1
fi
