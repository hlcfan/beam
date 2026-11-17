# Command palette

A searchable overlay for opening requests, revealing folders and running app commands (`Open Settings`, `Open Key Bindings`, `Open Environment Manager`).

## Sub-features

- `palette-open` opens with Cmd-P and shows `Recent` requests plus `Commands`.
- `palette-search` filters requests, folders and commands as the user types.
- `palette-run-command` runs the selected command with Return.
- `palette-close` dismisses with Escape.

## How to get to it (user POV)

- Press Cmd-P while Beam is focused. `AGENTS.md` still says Cmd-K; the running app responds to Cmd-P, so that doc is stale.

## Driving it with ax.sh

Preconditions:

- `doctor.sh` prints `READY`. The palette is closed.

- **Open.** `ax.sh key p cmd`. The tree gains `AXTextField Search requests, folders, and commands…`. A screenshot shows `RECENT > Sample Request` and `COMMANDS` with `Open Settings`, `Open Key Bindings`, `Open Environment Manager`.
- **Search.** `ax.sh key "Open Sett"`. The list narrows to `Open Settings` (screenshot).
- **Run.** `ax.sh keycode 36` (Return). The palette field disappears and the tree gains `AXPopUpButton Font size` and `AXButton Close`: the Settings dialog opened.
- **Close dialog.** `ax.sh click Close`.
- **Escape.** Reopen with Cmd-P, then `ax.sh keycode 53`. `ax.sh tree | grep -c "Search requests"` prints `0`.
- **Not yet driven:** selecting a request or folder (needs more than one request; create some with `File > New Request` first), and `Open Environment Manager` or `Open Key Bindings`.

Run on 2026-11-07. Evidence layout: `evidence/palette/` (screenshots 0 to 3).

## Gotchas

- Without Beam frontmost the keystroke lands in another app. `ax.sh key` raises Beam first.
- The palette field's AX name is its placeholder, not what was typed. Use a screenshot to see typed text.
- The empty-query view lists up to 10 recent requests; a fresh data folder shows only `Sample Request`.
