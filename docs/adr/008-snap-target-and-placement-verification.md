# ADR 008: Target Window Identification and Post-Snap Placement Verification

**Date:** 2026-10-20  
**Status:** Accepted  

## Context

When a window drag ends and `wg-overlay` executes the final snap via `SetWindowPos(..., SWP_ASYNCWINDOWPOS)`, three race conditions could prevent the final window rectangle from taking effect:

1. **Ambiguous Target Detection:** Previously, `wg-overlay` inferred the dragged window by inspecting the window under the cursor when the grid first appeared (`detect_target`). Fast cursor movement, layered/topmost helper windows, or elevated/cross-process ordering could cause the overlay to pick the wrong `HWND` or miss the dragged window.
2. **Post-Snap Displacement:** Because `SetWindowPos` is dispatched asynchronously (`SWP_ASYNCWINDOWPOS`) as the target exits its modal drag loop, subsequent OS or application actions can overwrite the snapped coordinates:
   - Windows Aero Snap or edge-maximize triggered when releasing the cursor near a monitor edge.
   - `WM_DPICHANGED` resizing when dragging across monitors with different DPI scale factors.
   - Custom application `WM_EXITSIZEMOVE` handlers restoring cached window bounds.
   - Maximized (`WS_MAXIMIZE`) state remaining active if a window is snapped without first being restored.
3. **Premature Safety-Net Triggering:** The 15 ms safety-net timer in `wg-overlay` previously checked only whether the primary mouse button was released (`GetAsyncKeyState`). During touch or pen drags (which do not assert the mouse button in `GetAsyncKeyState`), or when a thread remained inside its modal move/size loop slightly after button release, the safety-net timer could fire before the modal loop ended, causing the loop's final positioning to overwrite the snap.

## Decisions

1. **Explicit Target Announcement (`WindowGridTarget`):**
   - Registered a fourth window message, `WindowGridTarget` (`WindowGridIsoTarget` in isolated builds), in `wg-core::ipc` and `wg-core::names`.
   - `wg-hook` posts `WindowGridTarget` (`wParam = hwnd as u32`, `lParam = 0`) to the overlay once per drag session immediately before the first `WindowGridShowGrid` message.
   - `wg-overlay` records `hook_target` and prefers it over pointer-based `detect_target()` whenever `IsWindow(hook_target)` holds.

2. **Modal Loop Guard on Safety-Net Timer (`GUI_INMOVESIZE`):**
   - In `wg-overlay`, the 15 ms safety-net timer now queries `GetGUIThreadInfo` on the target window's thread and checks `GUI_INMOVESIZE`.
   - The drag is treated as active while either the primary mouse button is held or `GUI_INMOVESIZE` is set, preventing premature snaps during touch/pen drags or slow modal loop teardown.

3. **Maximized Window Restoration and Post-Snap Verification (`Verify`):**
   - Before calling `SetWindowPos`, `wg-overlay` checks `IsZoomed(target)` and calls `ShowWindowAsync(target, SW_RESTORE)` if the window is maximized.
   - After applying the initial `SnapRect`, `wg-overlay` starts a verification timer (`VERIFY_TIMER_ID`, `VERIFY_TICK_MS = 40` ms) backed by the pure `snap::Verify` state machine:
     - **Success (`VerifyStep::Done`):** Once `GetWindowRect` matches `SnapRect` within `VERIFY_TOLERANCE` (1 px) and `!IsZoomed` for `VERIFY_STABLE_TICKS` (3) consecutive ticks, verification stops.
     - **Re-application (`VerifyStep::Reapply`):** If the window is still maximized or its bounds deviate from `SnapRect`, `wg-overlay` re-applies `ShowWindowAsync` / `SetWindowPos` (up to `VERIFY_MAX_REAPPLY = 3` times, with a 1-tick cooldown after each re-application).
     - **Abort (`VerifyStep::GiveUp`):** Verification aborts immediately if the user starts a new drag (`GUI_INMOVESIZE` becomes true or a new session begins) or after `VERIFY_MAX_TICKS` (14 ticks, ~560 ms) for windows that enforce larger minimum track sizes.

## Consequences

- **Positive:** Eliminates missed snaps caused by Aero Snap edge conflicts, cross-monitor DPI changes, maximized state retention, touch/pen input, and pointer target misidentification.
- **Negative:** None. Backward compatibility with older hook binaries is preserved via the pointer-detection fallback when `WindowGridTarget` is not received.
