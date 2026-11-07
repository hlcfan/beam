#!/usr/bin/env bash
# Drive the current Beam instance through macOS Accessibility (System Events).
#   ax.sh front                         bring Beam to the front
#   ax.sh tree                          one line per element: "<role> <name>"
#   ax.sh menus                         top-level menu bar names
#   ax.sh menu-items <Menu> [<Sub>]     list items of a menu or submenu
#   ax.sh menu <Menu> <Item>            click Menu > Item
#   ax.sh menu <Menu> <Sub> <Item>      click Menu > Sub > Item
#   ax.sh click <name>                  click first button/pop up button/text field named <name> (window 1, any depth)
#   (key and keycode raise Beam first, so input cannot land in another app)
#   ax.sh key <char> [cmd|shift|opt|ctrl ...]   keystroke to the frontmost app
#   ax.sh rightclick <x> <y>            right-click at window-relative point coordinates (opens native context menus)
#   ax.sh leftclick <x> <y>             left-click at window-relative point coordinates (use for native context-menu items; read positions off a shot.sh screenshot, px/2)
#   ax.sh keycode <n>                   key code (53 = Escape, 36 = Return)
# Beam's menu bar title is "beam" (a bare binary, not a bundle).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
RUN="$(cat "$ROOT/target/verify-beam/current")"; PID="$(cat "$RUN/pid")"
cmd="${1:?usage: see header}"; shift || true
SE='tell application "System Events"'
P="tell (first process whose unix id is $PID)"
case "$cmd" in
  front) osascript -e "$SE" -e "$P" -e 'set frontmost to true' -e 'end tell' -e 'end tell'; sleep 0.3 ;;
  tree) osascript <<OSA | grep -v ' missing value$'
$SE
$P
set out to ""
set els to entire contents of window 1
repeat with e in els
try
set out to out & (role of e) & " " & (name of e as text) & linefeed
end try
end repeat
return out
end tell
end tell
OSA
    ;;
  menus) osascript -e "$SE" -e "$P" -e 'return name of every menu bar item of menu bar 1' -e 'end tell' -e 'end tell' ;;
  menu-items)
    if [ $# -eq 1 ]; then
      osascript - "$1" <<OSA
on run argv
$SE
$P
return name of every menu item of menu 1 of menu bar item (item 1 of argv) of menu bar 1
end tell
end tell
end run
OSA
    else
      osascript - "$1" "$2" <<OSA
on run argv
$SE
$P
return name of every menu item of menu 1 of menu item (item 2 of argv) of menu 1 of menu bar item (item 1 of argv) of menu bar 1
end tell
end tell
end run
OSA
    fi ;;
  menu)
    if [ $# -eq 2 ]; then
      osascript - "$1" "$2" <<OSA
on run argv
$SE
$P
click menu item (item 2 of argv) of menu 1 of menu bar item (item 1 of argv) of menu bar 1
end tell
end tell
end run
OSA
    else
      osascript - "$1" "$2" "$3" <<OSA
on run argv
$SE
$P
click menu item (item 3 of argv) of menu 1 of menu item (item 2 of argv) of menu 1 of menu bar item (item 1 of argv) of menu bar 1
end tell
end tell
end run
OSA
    fi ;;
  click)
    osascript - "$1" <<OSA
on run argv
$SE
$P
set els to entire contents of window 1
repeat with e in els
try
if (name of e as text) is (item 1 of argv) and role of e is in {"AXButton", "AXPopUpButton", "AXTextField", "AXRadioButton", "AXCheckBox"} then
click e
return "clicked " & (item 1 of argv)
end if
end try
end repeat
error "no clickable element named " & (item 1 of argv)
end tell
end tell
end run
OSA
    ;;
  key)
    "$0" front
    ch="$1"; shift; mods=""
    for m in "$@"; do case "$m" in cmd) mods="$mods, command down";; shift) mods="$mods, shift down";; opt) mods="$mods, option down";; ctrl) mods="$mods, control down";; esac; done
    mods="${mods#, }"
    if [ -n "$mods" ]; then osascript -e "$SE to keystroke \"$ch\" using {$mods}"; else osascript -e "$SE to keystroke \"$ch\""; fi ;;
  rightclick|leftclick)
    "$0" front
    B="$(osascript -e "$SE" -e "$P" -e 'return (item 1 of (get position of window 1) as text) & "," & (item 2 of (get position of window 1) as text)' -e 'end tell' -e 'end tell' | tr -d ' ')"
    X=$(( ${B%,*} + $1 )); Y=$(( ${B#*,} + $2 ))
    if [ "$cmd" = rightclick ]; then DN=kCGEventRightMouseDown; UP=kCGEventRightMouseUp; BTN=1; else DN=kCGEventLeftMouseDown; UP=kCGEventLeftMouseUp; BTN=0; fi
    osascript -l JavaScript -e 'ObjC.import("CoreGraphics");
function ev(t,b){var e=$.CGEventCreateMouseEvent(null,t,$.CGPointMake('"$X,$Y"'),b);$.CGEventPost($.kCGHIDEventTap,e);}
ev($.kCGEventMouseMoved,0);delay(0.2);ev($.'"$DN"','"$BTN"');ev($.'"$UP"','"$BTN"');' ; sleep 0.7 ;;
  keycode) "$0" front; osascript -e "$SE to key code $1" ;;
  *) echo "unknown command $cmd" >&2; exit 2 ;;
esac
