# ADR 004: Rust App (`wg-app`)

**Date:** 2026-10-14  
**Amended:** 2026-10-20  
**Status:** Accepted

## Context

The legacy C# app (`WindowGrid.exe`) was a 141 MB self-contained WPF program hosting the settings window, tray icon, installer, and supervision of the overlay and hook DLLs. Measured on a running installation: **245 MB working set for the app plus 213 MB for its overlay**, before the two hook hosts.

## Decisions

**One executable that carries everything.** `wg-app` embeds the overlay and the hook DLLs (`build.rs` includes them from `dist/`, or `dist-iso/` for the isolated feature, and the build still works with a warning if one is missing) and extracts them next to itself on start. A component that is mapped into running processes (the hook DLLs) cannot be overwritten, so a file that cannot be written is renamed aside and rewritten; renamed leftovers are deleted at a later start. This replaces the 67 MB zip that was re-extracted on every start.

**UI: Slint, software renderer, Fluent style.** The settings window ports the C# one: live grid preview with the selected theme, columns/rows steppers (− and + buttons plus a slider) and presets, themes, blur, opacity, fill, radial gradient, margin compensation, preserve size, defer resize, anchor, the selection HUD options, the shortcut editor (command, modifier, key), splash, and the two Windows options that apply immediately (Aero Shake, window contents while dragging). Plus the Info & Usage page, the setup dialog and the splash screen. It follows the system's light or dark theme. The options are split over seven pages (Grid, Appearance, Behavior, Selection HUD, Shortcuts, Info & Usage, Under the hood) so that each fits the default window without scrolling; the C# window put them all on one long scrolling page. Slint's royalty-free license requires showing the `AboutSlint` widget on an About screen reachable from the top-level menu: it is on the Info page, which the tray menu opens.

**Behavior is split into testable logic and glue.** `args` (command line), `viewmodel` (clamps, summaries, presets), `config` (the settings file and what is published from it), `installer` (sequencing, against an `InstallPorts` trait) and `supervisor`'s restart policy and process list are plain logic with unit tests. `winops`, `autostart`, `osopts`, `tray`, `dialogs`, `ui` and `app` are the Windows and Slint glue.

**Startup** mirrors the legacy order: log, become the only instance (a newer instance replaces an older one), handle the command line (`/install`, `/replace`, `/silent` or `/s`, `/shortcut`, `/onstartup`, `/help`), load settings (first run asks whether to run setup, as before), extract the components, create and publish the shared-memory sections, start the supervisor, show the tray icon. The settings file keeps the legacy format and is written through a temporary file.

**Supervisor.** The overlay and the hook hosts (x64 Windows: a 32-bit and a 64-bit host; 32-bit Windows: one; ARM64: a native host and a 32-bit host) run in a single job object that kills them when it closes, so they cannot outlive the app even if it is killed. Each has a watcher thread that restarts it, with a crash-loop limit. Unlike the C# supervisor, this never kills other programs' `rundll32` processes (it killed *every* `rundll32` on the machine); stale hosts are asked to quit with the stop message.

**Installer.** Same steps as the original installer: install directory (default `%ProgramFiles%\WindowGridRust`), elevation through a `runas` relaunch when it is not writable, read-and-execute for all users on the directory (via `icacls`), copy, settings, Start Menu shortcut (COM `IShellLink`), and start at logon (the `Run` key plus a scheduled task with the highest available privileges, built from an XML written for this app). `/replace` installs the running copy at a path and starts it.

## Deviations from the C# app (intentional)

- "Show window contents while dragging" is now *persisted* (`SPIF_UPDATEINIFILE`); the old code changed it only until the next sign-in.
- The cell-shape text prints one decimal as intended (`1.8:1 Wide`); the old format string `0.1` was a bug.
- Settings are saved atomically; an unreadable file is reported and defaults are used until the user saves.
- Original icon: a generated grid icon (`assets/`), not the original project's artwork. Window and tray use it.
- A release build uses `panic = "abort"` (workspace setting). A bug that panics ends the app, and the job object then ends its helpers.

## Results

| | C# | Rust |
|---|---|---|
| App executable | 141 MB (self-contained) | 10.5 MB (includes overlay and hooks) |
| App, idle | 245 MB working set | 27 MB (2.9 MB private) |
| Overlay, idle | 213 MB | 9 MB |
| Hook hosts | not measured | 8-9 MB each |

Verified: 47 unit tests; headless renders of every window reviewed as images; an end-to-end suite (`wg-app --test lifecycle`) that runs the real (isolated) executable: first-run extraction, shared-memory publication, overlay and both hook hosts started, a killed overlay restarted, killing the app takes all helpers with it, clean quit, `/install` copying and relaunching the installed copy from its directory, and the settings window opening on the real desktop (rendered with the real winit backend, in the system's dark theme); plus `wg-e2e` (`real_drags_supervised_by_the_app`) verifying all four real-drag scenarios (resize with deferral, move preserving size, Esc cancel, and live resize) under full `wg-app` supervision.

## Open items

- Elevated installs (UAC), the Start Menu shortcut in the all-users folder, and creating the scheduled task require administrator rights and are tested in isolation via unit/component tests (shortcut creation and resolution in a temporary folder, task XML generation, `Run` registry value in a scratch key, and `InstallPorts` sequence verification) rather than unprompted elevation during `cargo test`.
- Tray menu and double-click interactions are verified at the window/callback boundary rather than via synthetic shell tray clicks.
- ARM64: the x64 hook host is not started on ARM64 Windows (how to launch an emulated `rundll32` there is unverified), and ARM64 binaries have not yet been executed on physical ARM64 hardware.
