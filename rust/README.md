# Rust Workspace Guide

Developer reference for the Cargo workspace in `rust/`. For product overview, user features, CLI options, and `WindowGridRust.settings` configuration, see the root [`README.md`](../README.md).

## Crates & Module Layout

| Crate | Purpose | Key Modules |
|---|---|---|
| `wg-core` | Settings serialization, IPC wire structs, key code tables, log format (`no_std` without default `std` feature). | `settings.rs`, `ipc.rs`, `keycode.rs`, `keycode_table.rs`, `json.rs`, `log.rs`, `names.rs` |
| `wg-hook` | `WindowGridRust64.dll`, `WindowGridRust32.dll`, `WindowGridRustARM64.dll` (`RunDll`, in-context WinEvent hook, input hooks, window subclass; `no_std`, CRT-free). | `grid.rs` (pure math), `session.rs` (`Host` state machine), `entry.rs` (Win32 exports/callbacks), `sys.rs` (`raw-dylib`), `rt.rs` (CRT shims), `log.rs` |
| `wg-overlay` | `WindowGridRustOverlay.exe`: click-through DirectComposition window rendered with Direct2D/DirectWrite, plus final window snap execution and post-snap placement verification. | `snap.rs` (`compute`, `Verify`), `hud.rs`, `state.rs`, `geom.rs`, `theme.rs` (pure logic), `gfx.rs` (Direct2D/DirectWrite), `app.rs` (window, messages, single instance) |
| `wg-app` | `WindowGridRust.exe`: Slint Fluent UI (behind the default `slint-ui` feature), system tray icon, CLI installer, and job-object supervisor; embeds the overlay and hook DLLs. | `args.rs`, `viewmodel.rs`, `config.rs`, `installer.rs` (pure logic), `winops.rs`, `autostart.rs`, `osopts.rs`, `tray.rs`, `ui.rs`, `app.rs`, `ui/app.slint` |
| `wg-app-iced` | `WindowGridRust-iced.exe`: the all-Rust fork of `wg-app` ([ADR 006](../docs/adr/006-iced-ui-fork.md)). Same logic (it uses `wg-app` without the `slint-ui` feature), but the settings, setup and splash windows are [iced](https://docs.rs/iced/latest/iced/) with the software renderer, so there is no `.slint` file. Opt-in: not a default member. | `app.rs` (startup, `iced::daemon`, tray bridge), `settings.rs` (state, update, `Layout`, pages), `install.rs`, `preview.rs` and `backdrop.rs` (canvas), `widgets.rs`, `style.rs`, `assets.rs` |
| `wg-win` | Shared Win32 helpers used by `wg-overlay` and `wg-app`. | `shm.rs`, `logger.rs`, `instance.rs`, `png.rs` |
| `wg-e2e` | Real-drag integration test harness driving physical mouse and keyboard events via `SendInput` (`#[ignore]`d by default). | `tests/drag.rs` |

## Toolchain & Building

Requires the Rust toolchain with three MSVC targets and MSYS2 binutils (`dlltool.exe`, `as.exe`, `windres.exe` in `C:\msys64\ucrt64\bin`, or pass `-BinutilsDir`):

```cmd
rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc aarch64-pc-windows-msvc

powershell -NoProfile -ExecutionPolicy Bypass -File build-hooks.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File build-overlay.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File build-app.ps1
```

Or directly in PowerShell:

```powershell
.\build-hooks.ps1    # Builds dist\WindowGridRust64.dll, WindowGridRust32.dll, WindowGridRustARM64.dll
.\build-overlay.ps1  # Builds dist\WindowGridRustOverlay.exe
.\build-app.ps1      # Builds hooks + overlay, then embeds them into dist\WindowGridRust.exe
.\build-app-iced.ps1 # Same, but the iced fork: dist\WindowGridRust-iced.exe
```

`WindowGridRust-iced.exe` and `WindowGridRust.exe` use the same settings file, log, and IPC names, so only one of them runs at a time: starting one replaces the other.

- **Hook DLLs (`wg-hook`):** Link directly with `rust-lld` and `raw-dylib` imports (`.cargo/config.toml`), requiring no Visual Studio or Windows SDK and importing only `kernel32`, `user32`, and `dwmapi`.
- **Overlay & App (`wg-overlay`, `wg-app`):** Use `std` and link against the C runtime via the GNU target toolchain plus MSYS2 binutils (or MSVC Build Tools if installed).

## ARM64 Windows

An in-context WinEvent hook requires a DLL matching the target process architecture. `wg-hook` builds `WindowGridRustARM64.dll` (`aarch64-pc-windows-msvc`, exports `RunDll`).

Status: builds and lints, not yet tested on ARM64 hardware. On ARM64 Windows, `wg-app` starts the native ARM64 and 32-bit `rundll32.exe` hosts; launching an emulated x64 `rundll32.exe` host on ARM64 and handling ARM64EC/ARM64X targets remain open items.

## Testing

### Unit & Headless Render Tests

```cmd
set "PATH=C:\msys64\ucrt64\bin;%PATH%"
cargo test
```

Offscreen PNG renders for visual inspection are written to `target\tmp\` by `cargo test -p wg-overlay --test render` and `cargo test -p wg-app --test ui_render`.

### Isolated Process Tests (`--features isolated`)

Because WindowGridRust uses global mutexes, shared-memory sections, and broadcast window messages (`WindowGridRustStop`), running standard process tests alongside an active installation would stop or replace the running instance.

Building or testing with `--features isolated` (or `-Isolated` on the build scripts, outputting to `dist-iso\`) switches all IPC identifiers to `WindowGridRustIso*` / `-iso` suffixes. That separates test builds from an installed WindowGridRust, **not from each other**: all isolated instances share one set of names, and a newer one replaces an older one, as in the normal app. Close any isolated copy you started (`dist-iso\*.exe`) before running these tests, and do not run the tests while dragging windows with an isolated copy running.

```cmd
powershell -NoProfile -ExecutionPolicy Bypass -File build-app.ps1 -Isolated
set "PATH=C:\msys64\ucrt64\bin;%PATH%"
cargo test -p wg-overlay --features isolated
cargo test -p wg-app --test lifecycle --features isolated -- --test-threads=1
```

| Environment Variable | Crate | Description |
|---|---|---|
| `WG_TEST_RECT` | `wg-overlay` | Overrides monitor work area (`left,top,right,bottom`). |
| `WG_TEST_TARGET` | `wg-overlay` | Forces the target `HWND` for snap tests. |
| `WG_TEST_NO_TIMER` | `wg-overlay` | Disables the 15 ms safety-net timer in synthetic message tests. |
| `WG_TEST_NO_PROMPT` | `wg-app` | Skips the first-run setup prompt. |
| `WG_TEST_OPEN_SETTINGS` | `wg-app` | Opens the Settings window on launch. |
| `WG_TEST_QUIT_AFTER_MS` | `wg-app`, `wg-app-iced` | Automatically quits the app after `<ms>` milliseconds. |
| `WG_TEST_PAGE` | `wg-app-iced` | With `WG_TEST_OPEN_SETTINGS`, the settings page shown first (`0`-`6`, in navigation order). |
| `WG_TEST_ASK` | `wg-app-iced` | With `WG_TEST_OPEN_SETTINGS`, opens the `exit` or `reset` question over the settings window. |
| `WG_TEST_NO_HELPERS` | `wg-app-iced` | Starts the tray app without the overlay, the hook hosts and the shared memory, for tests of the windows: nothing then loads a hook DLL into another process. |

`WG_TEST_NO_PROMPT`, `WG_TEST_OPEN_SETTINGS` and `WG_TEST_QUIT_AFTER_MS` work in `wg-app-iced` as in `wg-app`. As there, the switches are read in `--features isolated` builds only.

### The iced fork (`wg-app-iced`)

Not a default member, so name it explicitly:

```cmd
set "PATH=C:\msys64\ucrt64\bin;%PATH%"
cargo test -p wg-app-iced
powershell -NoProfile -ExecutionPolicy Bypass -File build-hooks.ps1 -Isolated
powershell -NoProfile -ExecutionPolicy Bypass -File build-overlay.ps1 -Isolated
cargo test -p wg-app-iced --features isolated --test lifecycle -- --test-threads=1
```

The first command runs the unit tests (settings and setup dialog update logic, layout rules, preview geometry, view construction). `lifecycle` runs the real isolated executable: extraction, helper startup, a clean quit that takes the helpers with it, a tray double-click (posted to the app's own tray window) opening the settings window, and captures on the real desktop, written to `target\tmp\iced-*.png`: every settings page at its default size and resized to 816 x 560, 620 x 520 and 380 x 700, the confirmation, and the setup dialog.

`--test frame_cost` is headless (no window, tray or helper process, so it is safe to run any time): it runs the real settings view through iced's layout, drawing, damage tracking and software rasterizer, and checks that a hover repaints only the navigation cells involved and that switching pages is a single repaint. It counts pixels and rectangles instead of timing, so it does not flake. This guards against a slow, laggy UI (once a hover took ~100 ms because canvases were redrawn every frame): `cargo test -p wg-app-iced --release --test frame_cost`, and add `-- --ignored --nocapture` for the timing reports (`frame_cost_report`, `interaction_report`, `resize_report` for the cost of a whole frame at window sizes up to 2560 x 1440, and `first_paint_report -- --exact` for a cold first frame). Always time with `--release`: the release profile builds the crates that draw the iced windows for speed (`[profile.release.package.*]` in `Cargo.toml`, [ADR 006](../docs/adr/006-iced-ui-fork.md)), so figures from a debug build do not describe the real app.

`lifecycle` also has timing reports that are ignored by default: `opening_the_settings_window_report` (process start and tray double-click to a painted window) and `resizing_the_window_report` need nothing from the user; `hovering_the_navigation_shows_up_quickly` moves the real mouse pointer over the navigation and times the repaint, so it needs the window in front. Run them with `-- --ignored --nocapture <name>`.

### Real-Drag End-to-End Tests (`wg-e2e`)

`wg-e2e` spawns a test window with the isolated hook and overlay and drives real mouse and keyboard input (`SendInput`) on the primary monitor for ~15 seconds. **Close any running WindowGridRust instance first**, then run:

```cmd
powershell -NoProfile -ExecutionPolicy Bypass -File build-app.ps1 -Isolated
set "PATH=C:\msys64\ucrt64\bin;%PATH%"
cargo test -p wg-e2e -- --ignored --nocapture --test-threads=1
```

Scenarios verified (with mid-drag screenshots saved to `target\tmp\drag-*.png`):
1. Resize with hold-Space-then-drag (`DeferResizeUntilRelease = true`: no live resize, snaps on mouse release).
2. Move with Ctrl (`PreserveSizeOnMove = true`: visible dimensions preserved).
3. `Esc` cancel (window restored, no snap).
4. Live resize (`DeferResizeUntilRelease = false`).
5. `real_drags_supervised_by_the_app`: full supervised lifecycle driven through `wg-app`.
