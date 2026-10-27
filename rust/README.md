# Rust Workspace Guide

Developer reference for the Cargo workspace in `rust/`. For product overview, user features, CLI options, and `WindowGrid.settings` configuration, see the root [`README.md`](../README.md).

## Crates & Module Layout

| Crate | Purpose | Key Modules |
|---|---|---|
| `wg-core` | Settings serialization, IPC wire structs, key code tables, log format (`no_std` without default `std` feature). | `settings.rs`, `ipc.rs`, `keycode.rs`, `keycode_table.rs`, `json.rs`, `log.rs`, `names.rs` |
| `wg-hook` | `WindowGrid64.dll`, `WindowGrid32.dll`, `WindowGridARM64.dll` (`RunDll`, in-context WinEvent hook, input hooks, window subclass; `no_std`, CRT-free). | `grid.rs` (pure math), `session.rs` (`Host` state machine), `entry.rs` (Win32 exports/callbacks), `sys.rs` (`raw-dylib`), `rt.rs` (CRT shims), `log.rs` |
| `wg-overlay` | `WindowGridOverlay.exe`: click-through DirectComposition window rendered with Direct2D/DirectWrite, plus final window snap execution and post-snap placement verification. | `snap.rs` (`compute`, `Verify`), `hud.rs`, `state.rs`, `geom.rs`, `theme.rs` (pure logic), `gfx.rs` (Direct2D/DirectWrite), `app.rs` (window, messages, single instance) |
| `wg-app` | `WindowGrid.exe`: Slint Fluent UI, system tray icon, CLI installer, and job-object supervisor; embeds the overlay and hook DLLs. | `args.rs`, `viewmodel.rs`, `config.rs`, `installer.rs` (pure logic), `winops.rs`, `autostart.rs`, `osopts.rs`, `tray.rs`, `ui.rs`, `app.rs`, `ui/app.slint` |
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
.\build-hooks.ps1    # Builds dist\WindowGrid64.dll, WindowGrid32.dll, WindowGridARM64.dll
.\build-overlay.ps1  # Builds dist\WindowGridOverlay.exe
.\build-app.ps1      # Builds hooks + overlay, then embeds them into dist\WindowGrid.exe
```

- **Hook DLLs (`wg-hook`):** Link directly with `rust-lld` and `raw-dylib` imports (`.cargo/config.toml`), requiring no Visual Studio or Windows SDK and importing only `kernel32`, `user32`, and `dwmapi`.
- **Overlay & App (`wg-overlay`, `wg-app`):** Use `std` and link against the C runtime via the GNU target toolchain plus MSYS2 binutils (or MSVC Build Tools if installed).

## ARM64 Windows

An in-context WinEvent hook requires a DLL matching the target process architecture. `wg-hook` builds `WindowGridARM64.dll` (`aarch64-pc-windows-msvc`, exports `RunDll`).

Status: builds and lints, not yet tested on ARM64 hardware. On ARM64 Windows, `wg-app` starts the native ARM64 and 32-bit `rundll32.exe` hosts; launching an emulated x64 `rundll32.exe` host on ARM64 and handling ARM64EC/ARM64X targets remain open items.

## Testing

### Unit & Headless Render Tests

```cmd
set "PATH=C:\msys64\ucrt64\bin;%PATH%"
cargo test
```

Offscreen PNG renders for visual inspection are written to `target\tmp\` by `cargo test -p wg-overlay --test render` and `cargo test -p wg-app --test ui_render`.

### Isolated Process Tests (`--features isolated`)

Because WindowGrid uses global mutexes, shared-memory sections, and broadcast window messages (`WindowGridStop`), running standard process tests alongside an active installation would stop or replace the running instance.

Building or testing with `--features isolated` (or `-Isolated` on the build scripts, outputting to `dist-iso\`) switches all IPC identifiers to `WindowGridIso*` / `-iso` suffixes:

```cmd
powershell -NoProfile -ExecutionPolicy Bypass -File build-app.ps1 -Isolated
set "PATH=C:\msys64\ucrt64\bin;%PATH%"
cargo test -p wg-overlay --features isolated
cargo test -p wg-app --test lifecycle --features isolated
```

| Environment Variable | Crate | Description |
|---|---|---|
| `WG_TEST_RECT` | `wg-overlay` | Overrides monitor work area (`left,top,right,bottom`). |
| `WG_TEST_TARGET` | `wg-overlay` | Forces the target `HWND` for snap tests. |
| `WG_TEST_NO_TIMER` | `wg-overlay` | Disables the 15 ms safety-net timer in synthetic message tests. |
| `WG_TEST_NO_PROMPT` | `wg-app` | Skips the first-run setup prompt. |
| `WG_TEST_OPEN_SETTINGS` | `wg-app` | Opens the Settings window on launch. |
| `WG_TEST_QUIT_AFTER_MS` | `wg-app` | Automatically quits the app after `<ms>` milliseconds. |

### Real-Drag End-to-End Tests (`wg-e2e`)

`wg-e2e` spawns a test window with the isolated hook and overlay and drives real mouse and keyboard input (`SendInput`) on the primary monitor for ~15 seconds. **Close any running WindowGrid instance first**, then run:

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
