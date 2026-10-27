# ADR 006: Rust Overlay (`wg-overlay`)

**Date:** 2026-10-14  
**Amended:** 2026-10-20  
**Status:** Accepted

## Context

The legacy C# overlay (`WindowGridOverlay.exe`) was a WPF `AllowsTransparency` window. WPF renders such a window on the GPU, copies each frame back to system memory, and hands it to Windows as a layered-window bitmap; it also carries the .NET runtime. Measured on a running installation: 71 MB executable, **219 MB working set (267 MB private)**.

## Decisions

**Rendering.** A click-through window (`WS_EX_NOREDIRECTIONBITMAP | LAYERED | TRANSPARENT | TOPMOST | TOOLWINDOW | NOACTIVATE`, `WM_NCHITTEST -> HTTRANSPARENT`) shows a DirectComposition visual whose content is a premultiplied-alpha DXGI swap chain. Direct2D draws into the swap-chain buffer and DirectWrite draws the text, so frames never visit the CPU. The overlay is shown only while a session is active (a frame is drawn before the window is shown, so no stale frame appears) and hidden afterwards, instead of staying permanently visible at opacity 0 as the WPF version did. Per-monitor-v2 DPI awareness is set at startup; `WM_DPICHANGED` is ignored so the system never resizes the overlay when it moves between monitors.

**Look.** Reproduces the WPF overlay: 1 px frame and 2 px grid lines (crisp, not antialiased), selection with a 2 px border, 6 px corner radius, a radial gradient (centre `start * 0.7` in linear colour space with alpha scaled, as WPF's `Color * float` does) or a diagonal linear gradient, drop shadows, the HUD pill and the hint bar with the same text, colours, paddings and placement rules, and group opacity for the whole scene. Shadows are stacked translucent rounded rectangles rather than an effect graph (no intermediate bitmaps).

**Structure.** All decisions are in pure, unit-tested modules: `snap` (final window rectangle), `hud` (text, aspect ratio, placement), `state` (message and timer state machine returning effects), `geom`, `theme`. `gfx` (Direct2D), `app` (window, message handlers, effect execution), `shm` and `logger` are the Windows glue. The IPC contract is `wg-core::ipc`, verified byte for byte against a running C# app (see `wg-core/tests/live_capture.rs`).

**Settings that the C# overlay accepted but ignored are now real** (ADR 005 left their meaning to this decision):

- `FillWindow = false` draws the selection as an outline only: no fill, and the shadow becomes an outer glow so the interior stays clear.
- `BlurRadius` is the selection shadow's blur in device-independent pixels divided by 6, i.e. shadow blur = `6 x BlurRadius` (the default 3 reproduces the legacy fixed 18 px; 0 disables the shadow).
- `Anchor = GrabPoint`: when the snapped window is larger than the selection (and the drag was a move), it extends *away from the pointer*: with the pointer at the left/top edge of the selection the extra size goes right/down, at the right/bottom edge left/up, proportionally in between. `Center` and `TopLeft` are unchanged. Anchors still do not apply in Resize mode, as before.

**Safety net & target resolution.** The 15 ms timer runs in both Resize and Move mode (it only ran in Resize before). Esc cancels either kind of drag, and a drag end with Esc held does not snap. Releasing the primary mouse button (resolved through `SM_SWAPBUTTON`) and exiting the modal move/size loop (`GetGUIThreadInfo` / `GUI_INMOVESIZE`) for five consecutive ticks snaps as a fallback. The overlay resolves the target window from the `WindowGridTarget` message posted by `wg-hook` (falling back to pointer detection only when absent), restores maximized windows (`ShowWindowAsync(SW_RESTORE)`), and runs a 40 ms post-snap verification loop (`snap::Verify`) that re-applies the rectangle up to 3 times if displaced by Aero Snap, DPI scaling, or `WM_EXITSIZEMOVE` handling ([ADR 008](008-snap-target-and-placement-verification.md)).

**Size preservation fix.** The legacy snap preserved a moved window's *rectangle* size and then added the invisible DWM resize borders on top, so every Move snap grew the window's visible size by about 14 x 7 px. Sizes are now measured on the visible frame whenever margin compensation applies (also for the `Center`/`GrabPoint` oversize comparison); a unit test and the real-drag test pin this.

**Single instance.** Like the C# overlay: a `Global\{GUID}` mutex and a PID section; a newer overlay terminates an older one.

**Test seams (isolated builds only, `--features isolated`).** Test-only mutex, message and section names (`WindowGridIso*`) so tests can never touch a running WindowGrid, plus `WG_TEST_RECT` (use a small rectangle instead of the monitor's work area), `WG_TEST_TARGET` (snap this window instead of detecting one) and `WG_TEST_NO_TIMER=1` (no button is held in a test, which the safety net would read as a finished drag). None of these exist in normal builds.

## Results

| | C# overlay | Rust overlay |
|---|---|---|
| Executable | 71 MB (self-contained .NET) | 447 KB |
| Idle working set | 219 MB (267 MB private) | 9 MB (1.5 MB private) |

Verified: 35 unit tests, offscreen renders reviewed as PNGs (themes, radial/linear, outline, 1.5x DPI), and an end-to-end test that runs the real process: it publishes its window handle to the shared sections, draws on screen (screen capture checked), is click-through, snaps a test window so that its *visible* frame equals the selected cells (DWM margin compensation included), hides afterwards, stops on the stop message, and replaces a previous instance.

## Consequences and open items

- **Not measured:** frame latency/pacing during a real drag, and memory while a full-screen (4K) session is active.
- Driven by the real Rust hook in real drags (`rust/wg-e2e`, see ADR 005): resize, move, Esc and live-resize scenarios pass on a 4K monitor. Not tested: other DPI scales or several monitors, and the C# hook DLLs.
- **Toolchain:** `std` code needs the CRT, so unlike `wg-hook` the overlay cannot be linked with `rust-lld` alone. It builds with the GNU target; the `windows` crate's `raw-dylib` imports additionally need `dlltool` and `as` (MSYS2 binutils on `PATH` here). With Visual Studio Build Tools installed, the MSVC target works too and the ARM64 overlay becomes buildable. Until then an ARM64 Windows machine runs the x64 overlay under emulation, which is fine (only the hook must match the target process's architecture).
- Device loss (GPU reset) is handled by dropping the GPU resources and rebuilding them at the next session; it has not been exercised.
