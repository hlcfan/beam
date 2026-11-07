# Appearance and settings

The user picks a theme, a light or dark mode, and editor options such as font size. Choices persist in `app-settings.toml`.

## Sub-features

- `theme-select` applies a named theme from the menu bar.
- `settings-open` opens the Settings dialog, which has three sections: `Appearance`, `Editor`, `Update`.
- `settings-font-size` changes font size (`Appearance`; `font_size` = `small | medium | large`).
- `settings-soft-wrap` toggles `Editor soft wrap` (`Editor`; `wrap_body_editor` = `true | false`).
- `settings-wrapping-indent` picks `Same` or `None` (`Editor`; `wrapping_indent` = `same | none`).
- `settings-auto-format` toggles `Auto format response` (`Editor`; `auto_format_response`, default `true`).
- `settings-html-response` picks `HTML code` or the webview (`Editor`; `html_response_view` = `code | webview`).
- `settings-update-check` shows the installed version, the `Release channel` (`Normal release` or `Nightly`; `update_channel` = `release | nightly`), the available version, and `Update` and `Check for updates` buttons (`Update`).

## How to get to it (user POV)

- Menu bar: `beam > Theme > <name>`, `beam > Appearance > Light|Dark`, `beam > Settings`.
- Key: `Cmd-,` (`Ctrl-,` on other platforms).
- Palette: Cmd-P, type `Open Settings`, Return.
- Status bar `Settings` button (bottom left).

## Driving it with ax.sh

Preconditions:

- `doctor.sh` prints `READY`. `grep theme_name "$RUN/data/beam_local/app-settings.toml"` prints nothing (default theme).

- **Theme.** `cp "$RUN/data/beam_local/app-settings.toml" before.toml`, then `ax.sh menu beam Theme "Tokyo Night"`. Wait 1 s. `app-settings.toml` now contains `theme_name = "Tokyo Night"` (confirmed 2026-11-07).
- **Settings open (key).** `ax.sh key "," cmd`, wait 1 s, then `shot.sh`. The screenshot shows a `Settings` dialog with `Appearance`, `Editor`, `Update` sections, a `Theme` dropdown and a `Font size` dropdown (confirmed 2026-11-07).
- **Settings open (menu).** `ax.sh menu beam Settings`. Same dialog.
- **Close.** `ax.sh keycode 53` (Escape). The screenshot afterward shows no dialog (confirmed 2026-11-07). The `x` at the dialog's top right also closes it.
- **Sections.** With the dialog open, click `Editor` with `ax.sh leftclick 247 202` and `Update` with `ax.sh leftclick 253 240` (window-relative points; sections are not in the AX tree). Confirm each with `shot.sh`.
- **Soft wrap.** On `Editor`, `ax.sh leftclick 1039 161` (the switch at the top right). `diff` of `app-settings.toml` shows `wrap_body_editor = true` and a new `updated_at` (confirmed 2026-11-07).
- **Auto format response.** The switch is at about `1039 353`. A single click at that point showed a focus ring but did not change `auto_format_response` in the file. Unconfirmed; retry with a pause after the previous click, or focus the switch and press Space.
- **Update tab (read only).** `Update` shows `Installed version: 0.10.1`, `Available version 0.10.1 (up to date)`, a disabled `Update` button, and the note `This build cannot verify updates. Install an official release to enable updates.` (confirmed 2026-11-07 on a debug build). Opening the tab triggers a network check.
- **Not yet driven:** `Font size` options, `Wrapping indent`, `HTML response` selects, `Check for updates`, and the `Release channel` select. Clicking the pop up button did not expose its options in the AX tree; try `ax.sh keycode 125` (Down) then `ax.sh keycode 36` (Return) and diff `font_size` in `app-settings.toml`.

## Gotchas

- The settings file is under `beam_local/`, which is local state, not `beam/`.
- Theme names containing a comma would break `menu-items` output. Use the names printed by `ax.sh menu-items beam Theme`.
- `ax.sh tree` sometimes lists `AXPopUpButton Font size` and `AXButton Close` for this dialog and sometimes returns nothing (observed empty right after `Cmd-,`). Prove the dialog with a screenshot, not the tree, and do not rely on `ax.sh click Close`.
- **Never press `Update` on a build you care about.** It downloads and installs a release over the running binary. This debug build cannot verify updates, so the button is disabled; a build with `BEAM_UPDATE_PUBLIC_KEY` set could install. Verify installs only on a disposable copy.
- Selects (`Wrapping indent`, `HTML response`, `Font size`, `Release channel`) are dropdowns whose options are not in the AX tree. Open one with a click and read options from a screenshot.
- A macOS system overlay (such as the brightness HUD) can appear in a screenshot. It is not Beam.
- A theme change is visible in a `shot.sh` screenshot, but the file is the proof.
