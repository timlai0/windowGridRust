# ADR 004: Defer Window Resize Until Drag Release

**Date:** 2026-10-09  
**Amended:** 2026-10-11  
**Status:** Accepted  

## Context

When resizing windows using WindowGrid, the native hook procedures (`WindowGrid64.dll`, `WindowGrid32.dll`, `WindowGridARM64.dll`) intercept `WM_WINDOWPOSCHANGING` and continuously overwrite `WINDOWPOS.cx` and `WINDOWPOS.cy` while clearing `SWP_NOSIZE`. During mouse drag operations over grid cells, this causes the target window to continuously resize in real time.

For complex and GPU-accelerated applications (such as Chromium-based browsers, Electron apps, VS Code, and Discord), continuous intermediate sizing triggers severe layout reflowing, DOM relayout, and GPU swapchain recreation, resulting in visual thrashing, stuttering, and dropped frames.

## Decisions

1. **Configurable Deferral Setting:** Added `DeferResizeUntilRelease` (default `true`) in `GridSettings`, `GridWindowMemory`, and the Settings UI.
2. **IPC Shared Memory Signaling:** Utilized bit 16 (`0x10000`, `DEFER_FLAG`) of `grid_x` in `SettingsMemory` (`"WindowGridMapping_Hook"`) to communicate the deferral state to the hook DLLs without altering struct alignment or binary ABI. Clean `grid_x` values are copied to the overlay shared memory (`"WindowGridMapping_GridWindow"`) and masked in `wg-hook` so grid cell math remains intact.
3. **Native Hook Live-Resize Suppression (`wg-hook`):**
   - In `WM_MOVING` (`0x216`): When deferral is enabled, bypasses message conversion to `WM_SIZING` (`0x214`) and bypasses copying `grid_pixels` into `lParam`, preventing intermediate window size manipulation while preserving `WindowGridArea` updates for real-time overlay HUD tracking.
   - In `WM_WINDOWPOSCHANGING` (`0x46`): When deferral is enabled, bypasses `WINDOWPOS.cx`/`cy` updates and leaves `SWP_NOSIZE` intact.
4. **Target HWND Tracking and Final Snap Execution (`wg-overlay`):**
   - **Snap trigger is drag end, not hotkey release.** Activating grid mode with Space or Right Mouse Button is a momentary toggle (tap/click): releasing that input does **not** terminate the drag. The overlay stays visible and interactive for the entire drag.
   - `wg-hook` posts `WindowGridShowGrid(0, 0)` (`Mode::End`) when the drag ends, which executes the single-shot `SetWindowPos` snap with DWM margin compensation, anchor alignment, and final grid bounding dimensions.
   - **Safety-net fallback polling:** A 15 ms timer runs during active sessions so the overlay cannot leak if a hook message is lost. It cancels on `VK_ESCAPE`, and snaps only when the primary mouse button has been released for **5 consecutive ticks (~75 ms debounce)**, resolving the primary button via `GetSystemMetrics(SM_SWAPBUTTON)`.

## Consequences

- **Positive:** Eliminates intermediate layout reflow and rendering thrashing during window resizing while preserving fluid visual feedback via the overlay HUD.
- **Negative:** None. Live resizing behavior is preserved when the setting is toggled off.
