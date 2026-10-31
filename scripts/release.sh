#!/usr/bin/env bash
# Bump the package version, commit, tag, and push. The tag triggers the Release workflow.
# Usage: scripts/release.sh <version>   e.g. scripts/release.sh 0.10.1
set -euo pipefail

main() {
  local version="${1:-}"
  require_version "$version"
  cd "$(git rev-parse --show-toplevel)"
  require_clean_main
  require_new_tag "v$version"

  bump_cargo_version "$version"
  git add Cargo.toml Cargo.lock
  git commit -m "Release v$version"
  git tag "v$version"
  git push origin main "v$version"
  echo "Pushed v$version. Watch the Release workflow on GitHub."
}

require_version() {
  if [[ ! "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Usage: $0 <major.minor.patch>" >&2
    exit 1
  fi
}

require_clean_main() {
  if [[ "$(git branch --show-current)" != "main" ]]; then
    echo "Run the release from main." >&2
    exit 1
  fi
  if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    echo "Commit or stash tracked changes first." >&2
    exit 1
  fi
  git fetch origin main --quiet
  if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
    echo "main is not in sync with origin/main." >&2
    exit 1
  fi
}

require_new_tag() {
  if git rev-parse -q --verify "refs/tags/$1" >/dev/null; then
    echo "Tag $1 already exists." >&2
    exit 1
  fi
}

bump_cargo_version() {
  # Only the first `version = ` line, which belongs to [package].
  perl -0pi -e "s/^version = \".*\"\$/version = \"$1\"/m" Cargo.toml
  cargo update --workspace --offline --quiet
  if ! grep -q "^version = \"$1\"$" Cargo.toml; then
    echo "Failed to update Cargo.toml." >&2
    exit 1
  fi
}

main "$@"
