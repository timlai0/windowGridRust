# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- The final window size and position were sometimes not applied when a drag ended.
  - The hook now tells the overlay which window is being dragged (`WindowGridTarget`) instead of the overlay guessing from the pointer position.
  - The overlay now checks that the snapped window stayed put and re-applies the rectangle (restoring maximized windows first) when something moved it back, such as an Aero Snap at the screen edge, a DPI change between monitors or the app's own exit-move handling. It logs when a window refuses.
  - The safety-net fallback no longer snaps while the target is still inside its move/size loop, which raced the loop's own final positioning (touch and pen drags in particular).

## [2.0.0] - 2026-10-18

### Added
- Native Rust workspace in [`rust/`](rust/README.md):
  - `wg-app` ([ADR 007](docs/adr/007-rust-app.md)): single ~10.5 MB executable hosting the Slint Fluent settings UI, system tray icon, CLI installer, and job-object process supervisor (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), embedding the overlay and hook DLLs (~27 MB idle working set).
  - `wg-overlay` ([ADR 006](docs/adr/006-rust-overlay.md)): click-through DirectComposition window rendered with Direct2D and DirectWrite (~447 KB executable, ~9 MB idle working set) that displays the grid, selection, HUD, and interaction hints and performs the final window snap.
  - `wg-hook` ([ADR 005](docs/adr/005-rust-hook-behavior-spec.md)): `no_std`, CRT-free WinEvent and input hook DLLs (`WindowGrid64.dll`, `WindowGrid32.dll`, `WindowGridARM64.dll`, ~17–20 KB each).
  - `wg-core`: `WindowGrid.settings` JSON model, key code tables, log formatting, and `repr(C, packed)` shared-memory IPC contracts.
  - `wg-win`: shared Win32 primitives for shared memory, mutex synchronization, single-instance lock, and file logging.
  - `wg-e2e`: end-to-end real-drag test suite driving physical mouse and keyboard input via `SendInput`.
- Configurable **Defer Resize Until Release** setting (`DeferResizeUntilRelease`, [ADR 004](docs/adr/004-defer-resize-until-release.md)) to suppress window layout reflow during resizing until the mouse drag is released.
- Configurable **Compensate Window Margins** setting (`CompensateWindowMargins`, [ADR 003](docs/adr/003-window-margin-compensation.md)) to compensate for DWM invisible frame borders and prevent window shrinking.
- Configurable **Preserve Size on Move** setting (`PreserveSizeOnMove`) to retain active visible window dimensions when moving across grid cells.
- Configurable **Window Alignment Anchor** setting (`TopLeft`, `Center`, `GrabPoint`) for windows exceeding selected grid cell dimensions.
- Support for `FillWindow` (outline-only selection mode) and `BlurRadius` shadow scaling in the Direct2D overlay.
