# ADR 000: Window Margin Compensation and Move Size Preservation

**Date:** 2026-08-23  
**Status:** Accepted  

## Context

When snapping or moving windows in Windows 10 and 11:
1. Windows Desktop Window Manager (DWM) applies invisible drop shadow borders (typically 7px on the left, right, and bottom) to modern application windows. Standard Win32 `SetWindowPos` calls treat target coordinates as outer window bounds (including invisible borders), which causes the visible client frame of the snapped window to shrink by 14px horizontally and 7px vertically, leaving unsightly gaps between windows and the screen edge.
2. When performing window Move operations (`Command::MOVE`), treating the single target grid cell as a resize bounding box forced windows to resize down to the dimensions of a single cell instead of preserving their active dimensions.

## Decisions

1. **Configurable DWM Margin Compensation:** Added `CompensateWindowMargins` (default `true`) in `GridSettings`, `GridWindowMemory` (shared IPC memory), and the Settings UI.
2. **Dynamic DWM Frame Measurement:** On snap completion, `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)` is compared with `GetWindowRect` to calculate exact per-window invisible margins (left, top, right, bottom), expanding the `SetWindowPos` bounding box so that the visible window frame precisely matches grid cell boundaries without shrinking.
3. **Move Dimension Preservation:** Added `PreserveSizeOnMove` (default `true`) to distinguish `Command::MOVE` from `Command::RESIZE`, retaining active window width and height when moving a window while aligning its anchor position with the targeted grid cell.

## Consequences

- **Positive:** Eliminates window shrinking and gap anomalies after move and resize operations. Fully configurable via Settings UI and persistent across sessions.
- **Negative:** None. Margin compensation dynamically adapts to custom themed or borderless windows and falls back gracefully if DWM composition is unavailable.
