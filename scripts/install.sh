#!/bin/sh
# Install the latest stable release for the current user.
set -eu

main() {
    case "$(uname -s):$(uname -m)" in
        Linux:x86_64)
            platform=linux
            archive=beam-linux-x86_64.tar.gz
            checksum_tool=sha256sum
            ;;
        Darwin:arm64)
            platform=macos
            archive=beam-macos-aarch64.tar.gz
            checksum_tool=shasum
            command -v xattr >/dev/null 2>&1 || fail 'Required command not found: xattr'
            ;;
        *) fail 'Beam supports Linux x86_64 and macOS Apple Silicon only.' ;;
    esac
    for tool in curl tar "$checksum_tool" install mktemp sed mv; do
        command -v "$tool" >/dev/null 2>&1 || fail "Required command not found: $tool"
    done
    : "${HOME:?HOME must be set}"

    work_dir=$(mktemp -d)
    staged_bin=
    trap 'rm -rf "$work_dir"; rm -f "$staged_bin"' EXIT
    trap 'exit 1' HUP INT TERM
    repo=https://github.com/hlcfan/beam
    # Resolve once so a concurrent release cannot mix archive and checksum versions.
    release_url=$(curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
        -o /dev/null -w '%{url_effective}' "$repo/releases/latest")
    case "$release_url" in
        "$repo"/releases/tag/v*) tag=${release_url##*/} ;;
        *) fail 'Could not resolve the latest stable Beam release.' ;;
    esac
    base_url="$repo/releases/download/$tag"
    printf 'Downloading Beam %s…\n' "$tag"
    download "$base_url/$archive" "$work_dir/$archive"
    download "$base_url/$archive.sha256" "$work_dir/$archive.sha256"
    if [ "$platform" = macos ]; then
        (cd "$work_dir" && shasum -a 256 -c "$archive.sha256")
        install_macos
    else
        (cd "$work_dir" && sha256sum --check "$archive.sha256")
        install_linux
    fi
}

install_macos() {
    tar -xzf "$work_dir/$archive" -C "$work_dir" beam-macos-aarch64/Beam.app
    source_app="$work_dir/beam-macos-aarch64/Beam.app"
    [ -d "$source_app" ] && [ ! -L "$source_app" ] &&
        [ -f "$source_app/Contents/Info.plist" ] &&
        [ -x "$source_app/Contents/MacOS/beam" ] || fail 'Invalid Beam app bundle.'

    # Clear extended attributes on the verified bundle before replacing the installed app.
    xattr -cr "$source_app" || fail 'Could not clear extended attributes from Beam.'

    applications_dir="$HOME/Applications"
    install -d "$applications_dir"
    destination="$applications_dir/Beam.app"
    [ ! -L "$destination" ] || fail 'Refusing to replace a symlink at ~/Applications/Beam.app.'
    # Stage on the destination filesystem and keep the previous app until replacement succeeds.
    app_stage=$(mktemp -d "$applications_dir/.beam-install.XXXXXX")
    trap 'rm -rf "$work_dir"; if [ -e "$app_stage/previous.app" ]; then
        printf "Previous app preserved at %s/previous.app\n" "$app_stage" >&2
    else rm -rf "$app_stage"; fi' EXIT
    if ! mv "$source_app" "$app_stage/Beam.app"; then
        fail 'Could not stage Beam.'
    fi
    if [ -e "$destination" ]; then
        mv "$destination" "$app_stage/previous.app"
    fi
    if ! mv "$app_stage/Beam.app" "$destination"; then
        if [ -e "$app_stage/previous.app" ]; then
            mv "$app_stage/previous.app" "$destination" || {
                trap 'rm -rf "$work_dir"' EXIT
                fail "Could not restore Beam. Previous app preserved at $app_stage/previous.app."
            }
        fi
        fail 'Could not install Beam.'
    fi
    rm -rf "$app_stage/previous.app"
    printf 'Installed Beam to %s. Open Beam from Finder.\n' "$destination"
}

install_linux() {
    bin_dir="$HOME/.local/bin"
    data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
    case "$data_dir" in /*) ;; *) fail 'XDG_DATA_HOME must be an absolute path.' ;; esac

    # Extract only the files we install, never arbitrary archive paths.
    tar -xzf "$work_dir/$archive" -C "$work_dir" \
        beam-linux-x86_64/beam \
        beam-linux-x86_64/com.hlcfan.beam.desktop \
        beam-linux-x86_64/com.hlcfan.beam.metainfo.xml \
        beam-linux-x86_64/com.hlcfan.beam.png 2>/dev/null ||
        fail "Release $tag does not include the desktop files. Download the archive manually from $repo/releases."
    source_dir="$work_dir/beam-linux-x86_64"
    for name in beam com.hlcfan.beam.desktop com.hlcfan.beam.metainfo.xml com.hlcfan.beam.png; do
        [ -f "$source_dir/$name" ] && [ ! -L "$source_dir/$name" ] || fail "Invalid release file: $name"
    done

    # Stage the executable beside its destination, allowing updates while Beam runs.
    install -d "$bin_dir"
    staged_bin=$(mktemp "$bin_dir/.beam-install.XXXXXX")
    install -m 755 "$source_dir/beam" "$staged_bin"
    mv -f "$staged_bin" "$bin_dir/beam"

    # An absolute Exec path lets launchers work even when ~/.local/bin is not in PATH.
    escaped_bin=$(printf '%s' "$bin_dir/beam" | sed 's/\\/\\\\\\\\/g; s/"/\\\\"/g; s/`/\\\\`/g; s/\$/\\\\$/g; s/%/%%/g')
    while IFS= read -r line; do
        case "$line" in
            Exec=*) printf 'Exec="%s"\n' "$escaped_bin" ;;
            *) printf '%s\n' "$line" ;;
        esac
    done < "$source_dir/com.hlcfan.beam.desktop" > "$work_dir/beam.desktop"
    install -Dm644 "$work_dir/beam.desktop" "$data_dir/applications/com.hlcfan.beam.desktop"
    install -Dm644 "$source_dir/com.hlcfan.beam.png" "$data_dir/icons/hicolor/512x512/apps/com.hlcfan.beam.png"
    install -Dm644 "$source_dir/com.hlcfan.beam.metainfo.xml" "$data_dir/metainfo/com.hlcfan.beam.metainfo.xml"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$data_dir/applications" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -f -t "$data_dir/icons/hicolor" >/dev/null 2>&1 || true
    fi
    printf 'Installed Beam to %s. Open Beam from your application launcher.\n' "$bin_dir/beam"
    case ":${PATH:-}:" in
        *":$bin_dir:"*) ;;
        *) printf 'To run beam in a terminal, add %s to your PATH.\n' "$bin_dir" ;;
    esac
}

download() {
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL --retry 3 "$1" -o "$2"
}

fail() {
    printf '%s\n' "$1" >&2
    exit 1
}

main "$@"
