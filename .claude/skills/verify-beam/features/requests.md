# Requests

A request is one saved HTTP call. It lives as a `{slug}.request.toml` file in the workspace and shows as a row (`GET <name>`) in the workspace tree.

## Sub-features

- `req-new` creates a request (`Cmd-N`, `File > New Request`, or right-click > `HTTP`).
- `req-rename` renames a request (`F2`, `File > Rename Request`, or right-click > `Rename`). The file is renamed too.
- `req-duplicate` duplicates a request as `<name> (Copy)` (`Cmd-D`, `File > Duplicate Request`, or right-click > `Duplicate`).
- `req-delete` deletes a request after a confirm dialog (`Cmd-Backspace` or right-click > `Delete`).
- `req-focus-url` moves focus to the URL field (`Cmd-L`, `File > Focus URL`).
- `req-ctx-other` right-click also offers `Send Request`, `Copy as cURL` and `Folder` (new folder).

## How to get to it (user POV)

- Menu bar: `File > New Request | Duplicate Request | Rename Request | Focus URL`.
- Right-click a request row in the workspace tree: `Send Request`, `Copy as cURL`, `HTTP`, `Folder`, `Rename`, `Duplicate`, `Delete`. Right-click empty tree space: `HTTP`, `Folder`.
- Keys (Ctrl instead of Cmd on other platforms): `Cmd-N` new, `Cmd-D` duplicate, `F2` rename, `Cmd-Backspace` delete, `Cmd-L` focus URL, `Cmd-Enter` or `Cmd-R` send.

## Driving it with ax.sh

Preconditions:

- `doctor.sh` prints `READY`. `W="$RUN/data/beam/default"`; `ls $W` lists `sample-request.request.toml` and `environments`.
- Dialogs and the tree rows are not reliably in `ax.sh tree`. Read state from `ls $W`, `grep ^name $W/*.request.toml`, and `shot.sh` screenshots.

- **New (key).** `ax.sh key n cmd`, wait 1.2 s. `new-request-1.request.toml` appears and a `New Request 1` row is selected.
- **New (menu).** `ax.sh menu File "New Request"`. Same result; the name increments.
- **Duplicate (key).** Select a row, `ax.sh key d cmd`. `sample-request-copy.request.toml` appears with `name = "Sample Request (Copy)"`.
- **Rename (key).** `ax.sh keycode 120` (F2). A `Rename` dialog appears with the current name preselected. `ax.sh key a cmd`, `ax.sh key "Renamed One"`, `ax.sh keycode 36` (Return). The file becomes `renamed-one.request.toml` with `name = "Renamed One"`; the old file is gone.
- **Delete (key).** `osascript -e 'tell application "System Events" to key code 51 using command down'` after `ax.sh front`. A `Delete Request` dialog asks `Delete request "<name>"?`. Confirm with `ax.sh leftclick 826 230` (the red `Delete` button; coordinates are window-relative points, read from a `shot.sh` image at 2560 px wide divided by 2; recheck if the window size differs). The file disappears from `$W`.
- **Context menu.** `ax.sh rightclick 160 63` on the first tree row opens a native menu (screenshot proves it). Item offsets from the click point (points): `Send Request` +11, `Copy as cURL` +33, `HTTP` +66, `Folder` +88, `Rename` +121, `Duplicate` +143, `Delete` +165 vertically, about +64 horizontally. Example: `ax.sh leftclick 224 184` chose `Rename` and opened the dialog. Native menu items are not clickable by name here; use coordinates and confirm with a screenshot.

Driven 2026-11-07: new (key, menu), duplicate (key), rename (key and context menu to dialog), delete (key), context menu open. Evidence: `evidence/requests/`, `evidence/rename/`.

## Gotchas

- Files land in the active workspace's directory. Read `active_workspace_id` in `workspaces.toml` and match it to `path`.
- Keystrokes act on the selected row. Click or create the target row first; check with a screenshot.
- Rename and delete dialogs may animate: a faint second dialog outline shows in screenshots for a moment. Wait about 1 s before capturing.
- Pressing `Cmd-A` in the Rename dialog selects the name text only if the dialog's field has focus; it did on open.
- Delete is destructive and immediate on confirm. Use a throwaway request, never the only one you need later.
- Not yet driven: `Duplicate`, `Rename`, `Delete` through the context menu to completion, `Copy as cURL`, `Send Request`, `Cmd-L`, folders (`Folder` item).
