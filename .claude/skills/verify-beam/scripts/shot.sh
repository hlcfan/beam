#!/usr/bin/env bash
# Screenshot of Beam's window region -> the path given. Needs Screen Recording
# permission for the terminal. Beam must be visible (not covered): raises it first.
# Usage: shot.sh <out.png>
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
RUN="$(cat "$ROOT/target/verify-beam/current")"; PID="$(cat "$RUN/pid")"
"$(dirname "$0")/ax.sh" front
B="$(osascript -e "tell application \"System Events\" to tell (first process whose unix id is $PID) to get {position, size} of window 1" | tr -d ' ')"
mkdir -p "$(dirname "$1")"
screencapture -x -R"$B" "$1"
echo "$1"
