---
name: verify-beam
description: Launch and drive the Beam desktop HTTP client (Rust + gpui, macOS) in an isolated data folder and prove a feature works, using macOS Accessibility (System Events) plus on-disk TOML checks. Use when you must confirm a change to Beam's UI, workspaces, requests, settings or storage in the real app, not just in cargo test.
---

# verify-beam

Beam is a native gpui window app. There is no web surface, CLI or API. Proof comes from two sources:

1. The accessibility tree that gpui publishes (read with `scripts/ax.sh tree`), plus menu-bar clicks.
2. The TOML files Beam writes (a read-only second view of every mutation).

Read `features/README.md` for the feature map before driving. Run everything from the repo root.

## Constraints (read first)

- **macOS only.** The terminal running the agent needs Accessibility permission (System Settings > Privacy & Security). `doctor.sh` fails with a clear message when it is missing.
- **Screenshots need Screen Recording permission** for the terminal. With it, `scripts/shot.sh <out.png>` works (region capture of the window bounds). `screencapture -l <window id>` fails even then; do not use it. Without the permission, captures show only the wallpaper or other apps. Check each image: if it does not show Beam, the proof is invalid. Fall back to AX trees.
- **Keystrokes and clicks go to the real desktop.** The user's screen is in use while you drive. `ax.sh key` and `keycode` raise Beam first, because a keystroke goes to the frontmost app. After changing macOS permissions, relaunch: a running instance may have died.
- **Real user data is out of scope.** `launch.sh` always starts with an empty `data/` folder. A plain `cargo run` now reads `~/beam`; this skill never does, so runs cannot damage it. Test against real data only if the user asks.
- **One instance per run.** Each run has its own data folder, so parallel runs do not share state. Do not drive a Beam you did not start: the user's own Beam uses `~/beam` or `target/debug/data`.
- **Network at launch.** Beam checks api.github.com for updates on start. That is expected; it does not affect the data folder.

## Launch

```bash
.claude/skills/verify-beam/scripts/launch.sh     # prints the run dir
```

With `SEED_DIR=<dir>`, the dir's contents are copied into `data/` first (see Proof standards). It runs `cargo build --bin beam`, hard-links the binary into `target/verify-beam/<run>/`, creates an empty `data/` beside it, starts it, and waits until a window exists and `data/beam/workspaces.toml` is written. That empty `data/` folder switches Beam into portable mode (`src/paths.rs`), so all state lives in `<run>/data/{beam,beam_local,logs}`.

Use a hard link, not a symlink or `cargo run`: Beam canonicalizes its exe path, so a symlink resolves to `target/debug/`, where `target/debug/data` already holds the developer's real data.

The run dir is recorded in `target/verify-beam/current`. A rebuild after launch does not change the running instance; relaunch to test new code.

## Doctor

```bash
.claude/skills/verify-beam/scripts/doctor.sh
```

Read-only. Prints `READY pid=... run=... windows=1`, or `NOT READY: <reason>`: no run, pid dead, wrong binary, no portable data dir, Accessibility denied, no window. A `WARN` means the binary was rebuilt after launch. Run it first whenever anything looks off.

## Drive

All driving goes through `scripts/ax.sh`. Run it with no arguments to see its usage header.

| Need | Command |
| --- | --- |
| See the UI | `ax.sh tree` (one `<AXRole> <name>` line per element; unnamed ones are dropped) |
| Click a control | `ax.sh click "<name>"` (button, pop up button, text field, radio, checkbox; first match, any depth) |
| Menu bar | `ax.sh menus`, `ax.sh menu-items <Menu> [<Sub>]`, `ax.sh menu <Menu> [<Sub>] <Item>` |
| Type text | `ax.sh key "text"` goes to the focused field |
| Shortcut | `ax.sh key k cmd`, `ax.sh keycode 53` (Escape), `ax.sh keycode 36` (Return) |
| Right/left click at a point | `ax.sh rightclick <x> <y>`, `ax.sh leftclick <x> <y>` (window-relative points = screenshot px / 2). Needed for tree context menus and for dialog buttons missing from the tree |
| Raise Beam | `ax.sh front` |
| Screenshot | `shot.sh <out.png>`, then read the image to confirm it shows Beam |

Beam's app menu is titled `beam` (it runs as a bare binary). Its items: `Settings`, `Appearance > Light|Dark`, `Theme > <name>`. Others: `File > New Request|Duplicate Request|Rename Request|Focus URL`.

Tips:

- Prefer menu items and named controls. Keystroke shortcuts work (Cmd-P opens the palette, confirmed in the AX tree), but only when Beam is frontmost.
- Poll `ax.sh tree` for the expected element instead of sleeping. A 1 to 1.5 s pause after a click is enough for file writes.
- Element names containing commas can confuse `menu-items` output. The `tree` command does not have this problem.
- Not everything is in the AX tree: sidebar rows, some dialog buttons and native context-menu items are missing or unclickable by name. Use a screenshot plus `leftclick`/`rightclick`, and prove the result from files.
- Do not click macOS menus by index. Menu bar item 1 is the Apple menu.

## Evidence

Write to `<run>/evidence/<feature-id>/` (`RUN=$(cat target/verify-beam/current)`). `target/` is git-ignored and cleanup never touches `evidence/`.

For each proof keep:

- `ax.sh tree` before the action and after it.
- A `shot.sh` screenshot before and after when Screen Recording is granted.
- A copy of the file the action should change, before and after (`cp`, then `diff`).
- The user-path command you ran, in a `commands.txt`.

Proof standards:

- Default to the real user path: menu items, buttons, typed text. Do not call internal functions or test-only hooks.
- Seed a fixture only when the case is hard to reach by hand: a very large response body, a long history, a corrupted file, an unusual image. Edit the request TOML, response or history TOML, or the binary body file in a copy of the data folder, then pass it to `SEED_DIR=<dir> scripts/launch.sh`. The seed dir mirrors `data/` (`beam/...`, `beam_local/...`). Beam keeps data in memory, so edit before launch, not while it runs. The fixture only creates the precondition; trigger and observe the behavior through the UI.
- Record every seeded file in `evidence/<feature-id>/fixture.md` (path, what was changed, why the UI could not produce it). A result that depends on a fixture is reported as "verified on seeded data", not as an ordinary pass.
- Never edit TOML in `data/` after launch to fake a result, and never seed the expected output itself.
- A mutation counts only when the on-disk file confirms it, not just the tree.
- State the feature id and entry point you used. If a mapped entry point cannot be reached, report the failing command and unmet precondition; do not substitute another path.
- Do not touch `~/beam`, `~/beam_local` or `target/debug/data`.

## Cleanup

```bash
.claude/skills/verify-beam/scripts/cleanup.sh
```

Kills the pid this skill started (never by name), copies `beam.log` and `stdout.log` into `evidence/`, then deletes the run's `beam` link and `data/`. Run it after a failed attempt too. Evidence stays in `target/verify-beam/<run>/evidence/`.

## Helpers

| Script | Invocation | Purpose |
| --- | --- | --- |
| `scripts/launch.sh` | `launch.sh` | build, isolate, start, wait for ready |
| `scripts/doctor.sh` | `doctor.sh` | read-only health check |
| `scripts/ax.sh` | `ax.sh <cmd> [args]` | AX tree, clicks, menus, keys |
| `scripts/shot.sh` | `shot.sh <out.png>` | window screenshot |
| `scripts/cleanup.sh` | `cleanup.sh` | stop run, keep evidence |
