#!/usr/bin/env bash
# Stop the instance this skill started (by pid) and delete its scratch binary
# and data. Evidence in <run>/evidence/ is kept; logs are copied into it first.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
BASE="$ROOT/target/verify-beam"
[ -f "$BASE/current" ] || { echo "nothing to clean"; exit 0; }
RUN="$(cat "$BASE/current")"; PID="$(cat "$RUN/pid" 2>/dev/null || true)"
[ -n "$PID" ] && kill -0 "$PID" 2>/dev/null && kill "$PID" && sleep 1
[ -n "$PID" ] && kill -0 "$PID" 2>/dev/null && kill -9 "$PID"
mkdir -p "$RUN/evidence"
cp "$RUN/data/logs/beam.log" "$RUN/evidence/beam.log" 2>/dev/null || true
cp "$RUN/stdout.log" "$RUN/evidence/stdout.log" 2>/dev/null || true
rm -rf "$RUN/data" "$RUN/beam"
rm -f "$BASE/current"
echo "stopped ${PID:-?}; evidence kept in $RUN/evidence"
