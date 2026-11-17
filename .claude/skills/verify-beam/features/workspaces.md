# Workspaces

A workspace is a top-level container with its own folders, requests and environments. The user creates one from the workspace tab strip and each workspace gets its own directory.

## Sub-features

- `ws-create` creates a workspace from a name and makes it active.
- `ws-cancel` dismisses the dialog without creating anything.
- `ws-tabs` shows one tab per workspace and the active one is selected.

## How to get to it (user POV)

- Click the `New workspace` button next to the workspace tabs.

## Driving it with ax.sh

Preconditions:

- `doctor.sh` prints `READY`. The tab strip shows an `AXRadioButton default`.

- **Open.** `ax.sh click "New workspace"`. The tree gains `AXTextField Workspace name`, `AXButton Cancel` and `AXButton Create`.
- **Type.** `ax.sh key "Verify WS"`. The name field has focus on open, so the text lands in it.
- **Create.** `cp "$RUN/data/beam/workspaces.toml" before.toml`, then `ax.sh click Create`. The tree gains `AXRadioButton Verify WS`; the dialog controls disappear.
- **Disk proof.** `diff before.toml "$RUN/data/beam/workspaces.toml"` shows a new `[[registry.workspaces]]` with `name = "Verify WS"`, `path = "verify-ws"`, and `active_workspace_id` changed to its id. `$RUN/data/beam/verify-ws/beam.workspace.toml` exists.
- **Cancel.** Open the dialog, then `ax.sh click Cancel`. No new `AXRadioButton` and no new directory under `$RUN/data/beam/`.

Run once on 2026-11-07 (create path). Evidence layout: `evidence/create-workspace/`.

## Gotchas

- The text field's AX name is its label (`Workspace name`), not its value. Prove the value from `workspaces.toml`.
- Names are slugified for the directory (`Verify WS` becomes `verify-ws`). Duplicate names are rejected by the storage layer; the dialog's error text has not been mapped.
- `ax.sh key` types into whatever is focused. If focus did not land in the field, the text goes nowhere; check the diff, not the keystroke.
