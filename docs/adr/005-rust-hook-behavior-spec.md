# ADR 005: Behavior Specification for the Rust Hook (`wg-hook`)

**Date:** 2026-10-14  
**Amended:** 2026-10-20  
**Status:** Accepted

## Context

Prior to `v2.0.0`, `WindowGrid32.dll` and `WindowGrid64.dll` were shipped as precompiled 2016 MSVC binaries plus hand-patched code regions (ADR 003/004). To replace them with maintainable Rust source (`rust/wg-hook`), their observable behavior was recorded in this specification by analysing the binaries' imports, strings, and machine code and by calling their pure helper functions; the Rust implementation is written from this specification rather than translated from disassembly.

Corrections to earlier documentation:

- The DLLs do **not** use `WH_MOUSE_LL` / `WH_KEYBOARD_LL`. They use an in-context `SetWinEventHook` plus per-drag `WH_KEYBOARD` / `WH_MOUSE` hooks and a subclassed window procedure.
- The overlay's HUD data comes from `WindowGridArea` messages, not from a polled hook state.

## Process model

- The app starts `rundll32.exe <dll>,RunDll` once per bitness: `System32\rundll32.exe` + `WindowGrid64.dll`, and `SysWOW64\rundll32.exe` + `WindowGrid32.dll`.
- `RunDll(HWND, HINSTANCE, LPSTR, int)`:
  1. Creates a mutex (`Global\{58B6584C-820B-4813-97D1-6C923AD1CB42}` for 64-bit, `Global\{C26E1FC8-F719-4044-9122-029960C6C933}` for 32-bit) and returns at once if it already exists (`ERROR_ALREADY_EXISTS`).
  2. Registers the messages `WindowGridShowGrid`, `WindowGridArea`, `WindowGridTarget`, and `WindowGridStop`, and calls `ChangeWindowMessageFilterEx(hwnd, WindowGridStop, MSGFLT_ALLOW)` on the rundll32 window.
  3. Installs `SetWinEventHook(EVENT_SYSTEM_MOVESIZESTART 0x0A, EVENT_SYSTEM_MOVESIZEEND 0x0B, hDll, callback, 0, 0, WINEVENT_INCONTEXT | WINEVENT_SKIPOWNPROCESS = 6)`.
  4. Runs a `GetMessage` loop and exits when a message equal to `WindowGridStop` arrives (the app broadcasts it with `PostMessage(HWND_BROADCAST, ...)`), then `UnhookWinEvent`.
- Because the event hook is in-context, Windows loads the DLL into every process that owns a window being moved or sized; the callback then runs inside that process, on that thread. Both bitnesses are required.

## WinEvent callback (runs in the dragged window's process)

`callback(hook, event, hwnd, idObject, idChild, idEventThread, time)`:

- Ignore if `hwnd == NULL` or `idEventThread != GetCurrentThreadId()`.
- `MOVESIZESTART` -> begin session for `hwnd`. `MOVESIZEEND` -> end session for `hwnd`.

### Begin session
1. Open the shared sections read-only and refresh the cached settings: overlay window (`Hook.GridWindow`), `GridX`/`GridY` (`Hook.GridX` has the deferral flag in bit 16; mask it), and the input table. Failure to map = abort the session.
2. Initialise the window info: drag-full-windows (`SystemParametersInfoW(SPI_GETDRAGFULLWINDOWS)`), DWM composition (`DwmIsCompositionEnabled`).
3. Reset state: previous start/current cell = (-1,-1) twice; `resize_active = move_mode = false`; `prev_mode = -1`; `moving_msg = 0`; `session_active (armed) = false`; `cancelled = false`.
4. Subclass `hwnd` (`GWLP_WNDPROC`), remembering the original procedure.
5. Install `WH_MOUSE` and `WH_KEYBOARD` hooks (the original passed thread id 0, i.e. global hooks; the Rust version passes the current thread id, since the modal drag loop pumps messages on that thread and nothing else needs them).

### End session
Triggered by `MOVESIZEEND`, `WM_NCDESTROY`, or failure to subclass. Unhook both input hooks. If `hwnd` is the subclassed window: `PostMessage(overlay, WindowGridShowGrid, 0, 0)` (mode 0 = drag ended), `moving_msg = -1`, restore the original window procedure.

## Input hooks

Both hook procedures only observe: they always call `CallNextHookEx` and never swallow input.

**Keyboard (`WH_KEYBOARD`)**: ignore `code < 0` and `code == HC_NOREMOVE (3)`. Key id = scan code from `lParam` bits 16-23; if 0 use `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)`; `VK_PAUSE (0x13)` -> `0xC5`; extended-key bit (`lParam` bit 24) sets `0x80`. Transition from `lParam` bits 30-31: `0` = pressed (first down), `3` = released; `1` (auto-repeat) is ignored. `VK_ESCAPE` sets `cancelled = true` instead of being dispatched.

**Mouse (`WH_MOUSE`)**, `code == HC_ACTION` only. Button messages map to ids: left `0x101`, right `0x102`, middle `0x103`, X button `0x104` or `0x105` (`HIWORD(MOUSEHOOKSTRUCTEX.mouseData) == 1` -> `0x104`). Down/up messages give pressed/released.

**Dispatch (`on_input(key, pressed, released)`)**: do nothing when `pressed == released`. Look the key up in the input table (`Modifier` is **not** consulted). Command 1 (Resize): press = `resize_press`, release = `release`. Command 2 (Move): press = `move_press`, release = `release`. Handlers:

- `resize_press`: `move_mode = false; activate()`
- `move_press`: `move_mode = true; activate()`
- `release`: `resize_active = false` (only; `move_mode` and `armed` stay set)
- `activate()`: `resize_active = true`; if not `armed`: `armed = true; monitor = NONE`. Then if the dragged window is not in drag-full-windows mode: toggle the sign of a `+1/-1` nudge and `SetCursorPos(x, y + nudge)` to provoke a `WM_MOVING`. If drag-full-windows is on: `ShowWindow(hwnd, SW_SHOWNORMAL)` and `BringWindowToTop(hwnd)`, which provokes a `WM_WINDOWPOSCHANGING`.

Consequence: activation is momentary. After releasing the key the session stays armed until the drag ends. In Resize mode the *start* cell follows the cursor while the key is held and is frozen on release; the end cell then follows the cursor.

## Subclassed window procedure

Messages are intercepted only while `hwnd == subclassed window` and `!cancelled`; otherwise (or after interception) the original procedure is called. `WM_EXITSIZEMOVE (0x232)` sets `cancelled = true` after the original procedure returns. `WM_GETMINMAXINFO (0x24)`: after the original procedure returns, capture `ptMinTrackSize` and `ptMaxTrackSize` (components `<= 0` become `INT_MAX`).

| Message | Behavior |
|---|---|
| `WM_MOVING (0x216)` | `moving_msg = 0x216` unless it is -1. If armed and `moving_msg == 0x216`: change the message id to `WM_SIZING (0x214)` **unless deferring**; then, **only when not drag-full-windows**, call `update_grid()` and, **unless deferring**, copy `grid_pixels` into the `RECT` in `lParam`. |
| `WM_WINDOWPOSCHANGING (0x46)` | Only if armed and `moving_msg == 0x216`: `update_grid()`; then, **unless deferring**, `rect = grid_pixels + margins`; write `x, y = left, top`, `cx, cy = right-left, bottom-top` into `WINDOWPOS` and clear `SWP_NOSIZE\|SWP_NOMOVE`. |
| `WM_CAPTURECHANGED (0x215)` | If not armed: `moving_msg = 0`. |
| `WM_NCDESTROY (0x82)` | End session. |

`margins` = window rect minus `DWMWA_EXTENDED_FRAME_BOUNDS` rect (left/top <= 0, right/bottom >= 0), recomputed on every `update_grid()` (empty when DWM is off or the query fails). Frame bounds are converted with `PhysicalToLogicalPointForPerMonitorDPI` when available. The window's original size is captured once per session on the first refresh while not maximised, and used as the "current size" below.

## `update_grid()`

1. `pt = GetCursorPos()`. If `!resize_active && !move_mode && monitor != NONE`: `refresh = false`, `monitor_changed = false`. Otherwise `refresh = true`; `hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)`; `monitor_changed = (hmon != monitor)`, and if it changed read `GetMonitorInfoW` (`rcWork`, `rcMonitor`, primary flag).
2. Refresh window info (window rect, margins, original size).
3. Compute the selection (below) with `reset_start = refresh`.
4. If the overlay window is known: post messages (below).

**Effective grid:** `landscape = (rcMonitor.width >= rcMonitor.height)`. Landscape: `cols = GridX, rows = GridY`; portrait: `cols = GridY, rows = GridX`. Work area `(x0, y0, W, H)` from `rcWork`.

**Selection** (`C` = `cols`, `R` = `rows`, integer division truncates):

1. `cur = (clamp((pt.x-x0)*C/W, 0, C-1), clamp((pt.y-y0)*R/H, 0, R-1))`.
2. If `reset_start`: `start = cur`.
3. Sizes in cells (floating point, divide first): `min = ceil(minTrack / W * C)`, `max = floor(maxTrack / W * C)`, `size = round_half_away(origSize / W * C)`, and likewise vertically with `H`, `R`.
4. **Move mode only:** `min = max(min, size)` and `max = min(max, size)` per axis (selection is forced to the window's own size).
5. `left, right = sorted(start.x, cur.x)`; `top, bottom = sorted(start.y, cur.y)`.
6. Per axis, with `span = right-left`: `target = min-1` if `span < min-1`, else `max-1` if `span > max-1`, else unchanged. `delta = target-span`; `half = delta/2` (truncating); `left -= half`; `right += delta - half`.
7. Per axis, clamp into the grid: if `span > C-1`: `left = 0, right = C-1`; else if `right > C-1`: `right = C-1, left = C-1-span`; else if `left < 0`: `left = 0, right = span`.
8. `cells = (left, top, right, bottom)`; `grid_pixels = (left*W/C + x0, top*H/R + y0, (right+1)*W/C + x0, (bottom+1)*H/R + y0)`.

**Messages** (all `PostMessage` to the overlay window, `HWND` read from `Hook.GridWindow`, sign-extended):

- Once per drag session, immediately before the first `WindowGridShowGrid`: `WindowGridTarget`, `wParam = hwnd as u32`, `lParam = 0` ([ADR 008](008-snap-target-and-placement-verification.md)).
- If the mode (`move_mode`) differs from `prev_mode` (initially -1), or `monitor_changed`: `WindowGridShowGrid`, `wParam = 1` (Resize) or `2` (Move), `lParam = pack(monitor work-area centre)`.
- If the start/current cell pairs differ from the previously posted ones (or the mode just changed): `WindowGridArea`. Move mode: `wParam = pack(left, top)`, `lParam = pack(right, bottom)` of `cells`. Resize mode: `wParam = pack(start)`, `lParam = pack(cur)`.
- `pack(x, y) = (y << 16) | (x & 0xFFFF)` as 16-bit values.

## Rust-side deviations from the original binaries (intentional)

- `WH_KEYBOARD`/`WH_MOUSE` hooks are thread-scoped instead of global.
- No dependence on the C runtime (the DLLs are `no_std`, linked with `rust-lld`, imports via `raw-dylib`).
- Posts `WindowGridTarget` once per drag ahead of `WindowGridShowGrid` so the overlay targets the exact dragged `HWND` ([ADR 008](008-snap-target-and-placement-verification.md)).
- `WM_NCDESTROY` is forwarded to the original window procedure (the legacy binary dropped it after restoring the procedure), and the session is torn down on `WM_NCDESTROY` even after Esc cancelled the grid.
- Shares the same shared-memory layouts and message names as `wg-core::ipc`.

## Implementation status

`rust/wg-hook` implements this specification: `grid.rs` (selection maths) and `session.rs` (state machine behind a `Host` trait) are unit-tested on the host; `entry.rs` is the Windows glue. Verified: all unit tests; x64, x86, and ARM64 builds link and export only `RunDll` / `DllMain`; an isolated `RunDll` start / single-instance / stop cycle under `rundll32`; and **real drags of a test window on a 3840x2160 monitor** (`rust/wg-e2e`) with the isolated Rust overlay and supervisor (`wg-app`): hold Space on a cell, release, drag to a second cell, release the mouse snaps the window exactly onto the cells (deferral on, no live resize; deferral off, the window follows live); Ctrl moves the window onto a cell keeping its size; Esc restores the window and snaps nothing. Not drag-tested on physical hardware: the x86 and ARM64 hooks.

## Open items

- Hotkey `Modifier` values are written into the input table but ignored by the original hook; keep ignoring them unless modifier-filtered hotkeys are explicitly required.
