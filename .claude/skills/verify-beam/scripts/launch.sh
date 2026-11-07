#!/usr/bin/env bash
# Build Beam and start one isolated instance. Prints the run dir.
# Optional: SEED_DIR=<dir> copies that dir (mirroring data/: beam/, beam_local/) into data/ first.
# Isolation: Beam uses <dir of exe>/data when that folder exists (src/paths.rs
# portable mode). The exe is HARD-linked into a fresh run dir. A symlink would
# be canonicalized back to target/debug/ and reach the developer's data.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
BASE="$ROOT/target/verify-beam"
cd "$ROOT"
cargo build --bin beam 2>&1 | tail -2
RUN="$BASE/$(date +%Y%m%d-%H%M%S)-$$"
mkdir -p "$RUN/data" "$RUN/evidence"
if [ -n "${SEED_DIR:-}" ]; then cp -R "$SEED_DIR"/. "$RUN/data/"; fi
ln "$ROOT/target/debug/beam" "$RUN/beam" 2>/dev/null || cp "$ROOT/target/debug/beam" "$RUN/beam"
echo "$RUN" > "$BASE/current"
cd "$RUN"
nohup ./beam >"$RUN/stdout.log" 2>&1 &
echo $! >"$RUN/pid"
cd "$ROOT"
PID="$(cat "$RUN/pid")"
for _ in $(seq 1 60); do
  kill -0 "$PID" 2>/dev/null || { echo "beam exited early; see $RUN/stdout.log" >&2; exit 1; }
  W="$(osascript -e "tell application \"System Events\" to count windows of (first process whose unix id is $PID)" 2>/dev/null || echo 0)"
  # Ready = window exists AND the registry is on disk.
  if [ "${W:-0}" -ge 1 ] && [ -f "$RUN/data/beam/workspaces.toml" ]; then
    osascript -e "tell application \"System Events\" to tell (first process whose unix id is $PID) to set frontmost to true" >/dev/null
    echo "$RUN"; exit 0
  fi
  sleep 0.5
done
echo "timeout waiting for Beam window; see $RUN/stdout.log" >&2; exit 1
