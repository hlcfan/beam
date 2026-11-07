# Beam verification map

Maintained source for verifying Beam's user-facing behavior. Read this index, then the feature file that matches the change.

## Baseline preconditions

- Start with `scripts/launch.sh`. It gives an empty data folder, which Beam seeds with a `default` workspace holding `Sample Request` and a `default` environment.
- Run `scripts/doctor.sh` and require `READY`.
- Never drive an instance this run did not start.

## Driving conventions

- Harness: `scripts/ax.sh` (accessibility tree, named clicks, menu bar, keystrokes). Paths are relative to the repo root.
- Prefer menu items and named controls over keystrokes or coordinates.
- Prove each mutation with a before/after copy of the TOML file it should change (`$RUN/data/beam/...` or `$RUN/data/beam_local/...`).
- Restore nothing: each run has throwaway data. Start a new run for a clean baseline.

## Proof and skip reporting

- Evidence: AX tree and screenshot before and after, plus before/after file copies, in `<run>/evidence/<feature-id>/`.
- Screenshots come from `scripts/shot.sh`. Read each one: it must show Beam. Without Screen Recording permission, say so rather than claiming one.
- Report an unreachable path with the attempted command and the unmet precondition. Do not report it verified through a different path.

## Features

- [Workspaces](./workspaces.md) covers creating and switching workspaces. Driven end to end.
- [Requests](./requests.md) covers new, rename, duplicate and delete through keys, the File menu and the tree context menu. Driven end to end.
- [Appearance and settings](./settings.md) covers theme selection and the Settings dialog. Driven end to end.
- [Command palette](./command-palette.md) covers Cmd-P search and commands. Driven end to end.
