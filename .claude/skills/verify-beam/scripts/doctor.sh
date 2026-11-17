#!/usr/bin/env bash
# Read-only. Prints READY or the first failed check. Exit 0 = worth driving.
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
BASE="$ROOT/target/verify-beam"
fail() { echo "NOT READY: $*"; exit 1; }
[ -f "$BASE/current" ] || fail "no run; start one with scripts/launch.sh"
RUN="$(cat "$BASE/current")"; PID="$(cat "$RUN/pid" 2>/dev/null)"
[ -n "$PID" ] && kill -0 "$PID" 2>/dev/null || fail "pid ${PID:-?} not running"
[ "$(ps -o comm= -p "$PID")" = "./beam" ] || [ "$(lsof -p "$PID" -Fn 2>/dev/null | grep -c "^n$RUN/beam$")" -ge 1 ] || fail "pid $PID is not $RUN/beam"
[ -d "$RUN/data/beam" ] || fail "no data dir: portable mode inactive, instance may be using ~/beam"
[ "$(stat -f %i "$RUN/beam")" = "$(stat -f %i "$ROOT/target/debug/beam")" ] || echo "WARN: binary differs from current target/debug/beam (rebuilt since launch); relaunch to test new code"
W="$(osascript -e "tell application \"System Events\" to count windows of (first process whose unix id is $PID)" 2>&1)" || fail "System Events refused: grant Accessibility to this terminal ($W)"
[ "$W" -ge 1 ] || fail "no window"
echo "READY pid=$PID run=$RUN windows=$W"
