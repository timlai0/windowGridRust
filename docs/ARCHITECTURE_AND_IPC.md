# Architecture & IPC Data Contracts

## 1. System Components

WindowGridRust operates via a multi-process architecture in Rust (`rust/` workspace) to safely inject global hooks into the Windows OS across process bitness boundaries:

- **`wg-app` (`WindowGridRust.exe`):** Host application managing the Slint settings UI, system tray icon, CLI installer, shared-memory configuration publication, and child process supervision via a Win32 job object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`). Embeds the compiled overlay and hook binaries and extracts them next to itself on startup.
- **`wg-overlay` (`WindowGridRustOverlay.exe`):** Standalone DirectComposition / Direct2D / DirectWrite process that renders the click-through grid, selection, HUD, and interaction hints on the GPU, executes the final window snap (`SetWindowPos` after `ShowWindowAsync(SW_RESTORE)` when maximized) with DWM extended frame margin compensation, and verifies post-snap placement ([ADR 005](adr/005-snap-target-and-placement-verification.md)).
- **`wg-hook` (`WindowGridRust64.dll`, `WindowGridRust32.dll`, `WindowGridRustARM64.dll`):** `no_std`, CRT-free native libraries hosted via `rundll32.exe <dll>,RunDll` (one host per architecture). Each host installs an in-context `SetWinEventHook(EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND)` and per-drag thread-scoped `WH_KEYBOARD` / `WH_MOUSE` hooks plus window procedure subclassing (`GWLP_WNDPROC`).
- **`wg-core`:** Shared library defining the `WindowGridRust.settings` JSON model, key code tables, log formatting, and `repr(C, packed)` shared-memory wire contracts.
- **`wg-win`:** Shared Win32 helpers for pagefile-backed shared memory, mutex synchronization, single-instance management, and file logging.

## 2. Inter-Process Communication (IPC)

Because `wg-hook` executes inside the address space of whatever application window the user drags, WindowGridRust uses pagefile-backed named shared-memory sections (`CreateFileMappingW` / `OpenFileMappingW`) and registered window messages (`RegisterWindowMessageW`) for zero-copy communication.

### 2.1 Shared Memory Sections (`wg-core::ipc`)

All shared-memory structs are `#[repr(C, packed)]` plain old data read and written at offset 0. Single-byte booleans (`u8`, `0` or `1`) and packed ARGB colors (`0xAARRGGBB` as `i32`) match the layout of the original C# WindowGrid byte for byte; only the section and message names differ.

#### `WindowGridRustMapping_Hook` (`SettingsMemory`, 16 bytes)

Written by `wg-app`, updated with `grid_window` by `wg-overlay`, and read by `wg-hook`:

```rust
#[repr(C, packed)]
pub struct SettingsMemory {
    pub grid_window: i32, // Overlay HWND (truncated to 32 bits, sign-extended by hooks)
    pub grid_x: i32,      // Columns, with DEFER_FLAG (0x10000) in bit 16
    pub grid_y: i32,      // Rows
    pub input_size: i32,  // Number of active entries in WindowGridRustMapping_Input (max 8)
}
```

| Byte Offset | Field | Rust Type | Size | Description |
|---|---|---|---|---|
| `0x00` | `grid_window` | `i32` | 4 B | Overlay `HWND` (32-bit handle, sign-extended on 64-bit targets). |
| `0x04` | `grid_x` | `i32` | 4 B | Grid column count (`& 0xFFFF`) ORed with `DEFER_FLAG` (`0x10000`). |
| `0x08` | `grid_y` | `i32` | 4 B | Grid row count. |
| `0x0C` | `input_size` | `i32` | 4 B | Active binding count in `WindowGridRustMapping_Input` (`0..=8`). |

#### `WindowGridRustMapping_Input` (`[InputMemory; 8]`, 64 bytes)

Written by `wg-app`, read by `wg-hook`:

```rust
#[repr(C, packed)]
pub struct InputMemory {
    pub input: i32,    // KeyCode (scan code / mouse button ID)
    pub modifier: i16, // Modifier flags
    pub command: i16,  // 1 = Resize, 2 = Move
}
```

| Byte Offset | Field | Rust Type | Size | Description |
|---|---|---|---|---|
| `0x00` | `input` | `i32` | 4 B | Hardware scan code (`0x01..0xFF`) or mouse button code (`0x101..0x105`). |
| `0x04` | `modifier` | `i16` | 2 B | Modifier bitmask (`0` = None, `1` = Alt, `2` = Ctrl, `4` = Shift, `8` = Win). |
| `0x06` | `command` | `i16` | 2 B | Action command (`1` = `Resize`, `2` = `Move`). |

#### `WindowGridRustMapping_GridWindow` (`GridWindowMemory`, 44 bytes)

Written by `wg-app`, read by `wg-overlay`:

```rust
#[repr(C, packed)]
pub struct GridWindowMemory {
    pub grid_x: i32,
    pub grid_y: i32,
    pub radial_gradient: u8,
    pub blur_radius: f32,
    pub fill_window: u8,
    pub opacity: f32,
    pub color_line: i32,
    pub color_start: i32,
    pub color_end: i32,
    pub show_selection_hud: u8,
    pub show_dimensions: u8,
    pub show_cell_span: u8,
    pub show_screen_coverage: u8,
    pub show_aspect_ratio: u8,
    pub show_mode_indicator: u8,
    pub show_interaction_hints: u8,
    pub anchor: i32, // 0 = TopLeft, 1 = Center, 2 = GrabPoint
    pub compensate_window_margins: u8,
    pub preserve_size_on_move: u8,
    pub defer_resize_until_release: u8,
}
```

| Byte Offset | Field | Rust Type | Size | Description |
|---|---|---|---|---|
| `0x00` | `grid_x` | `i32` | 4 B | Grid columns (`1..=64`). |
| `0x04` | `grid_y` | `i32` | 4 B | Grid rows (`1..=64`). |
| `0x08` | `radial_gradient` | `u8` | 1 B | `1` = radial fill gradient, `0` = diagonal linear gradient. |
| `0x09` | `blur_radius` | `f32` | 4 B | Selection shadow blur factor (`6.0 * blur_radius` DIPs). |
| `0x0D` | `fill_window` | `u8` | 1 B | `1` = filled selection rectangle, `0` = outline-only with outer glow. |
| `0x0E` | `opacity` | `f32` | 4 B | Scene master opacity (`0.05..=1.0`). |
| `0x12` | `color_line` | `i32` | 4 B | Packed `0xAARRGGBB` grid line and border color. |
| `0x16` | `color_start` | `i32` | 4 B | Packed `0xAARRGGBB` gradient start color. |
| `0x1A` | `color_end` | `i32` | 4 B | Packed `0xAARRGGBB` gradient end color. |
| `0x1E` | `show_selection_hud` | `u8` | 1 B | Master toggle for the selection HUD pill. |
| `0x1F` | `show_dimensions` | `u8` | 1 B | Show pixel width and height in HUD. |
| `0x20` | `show_cell_span` | `u8` | 1 B | Show selected cell span (`cols x rows`) in HUD. |
| `0x21` | `show_screen_coverage` | `u8` | 1 B | Show percentage of monitor work area covered. |
| `0x22` | `show_aspect_ratio` | `u8` | 1 B | Show aspect ratio readout in HUD. |
| `0x23` | `show_mode_indicator` | `u8` | 1 B | Show `RESIZE` / `MOVE` badge in HUD. |
| `0x24` | `show_interaction_hints` | `u8` | 1 B | Show bottom interaction key hint bar. |
| `0x25` | `anchor` | `i32` | 4 B | Oversized move alignment (`0` = `TopLeft`, `1` = `Center`, `2` = `GrabPoint`). |
| `0x29` | `compensate_window_margins` | `u8` | 1 B | Expand snap bounds by DWM invisible frame margins. |
| `0x2A` | `preserve_size_on_move` | `u8` | 1 B | Retain visible window dimensions during `Move` commands. |
| `0x2B` | `defer_resize_until_release` | `u8` | 1 B | Suppress live window resizing until drag release. |

#### `WindowGridRustMapping_GridWindowHandle` (4 bytes)

Holds the overlay `HWND` as a 32-bit signed integer (`i32`).

### 2.2 Synchronization & Single-Instance Mutexes

| Resource | Production Identifier | Isolated Identifier (`--features isolated`) | Purpose |
|---|---|---|---|
| Shared-Memory Lock | `Global\<SectionName>` | `Global\<IsoSectionName>` | Serializes reads and writes to the paired shared-memory section. |
| App Instance | `643cf41b-7290-4b37-8eb4-82ca50576665` | `643cf41b-7290-4b37-8eb4-82ca50576665-iso` | Single-instance guard for `WindowGridRust.exe` (paired with `*-pid` section). |
| Overlay Instance | `C28BB0DC-B9EA-4472-9BBC-F0BC63EA5D16` | `C28BB0DC-B9EA-4472-9BBC-F0BC63EA5D16-iso` | Single-instance guard for `WindowGridRustOverlay.exe` (paired with `*-pid` section). |
| 64-bit Hook Host | `Global\{D82B85E1-BAA7-419F-AD0E-C1540EF8BFAB}` | `Global\{D82B85E1-BAA7-419F-AD0E-C1540EF8BFAB}-iso` | Single-instance guard for 64-bit `rundll32.exe` `RunDll` host. |
| 32-bit Hook Host | `Global\{FD439CB8-8CFC-4209-BB19-DF8AA54B6E56}` | `Global\{FD439CB8-8CFC-4209-BB19-DF8AA54B6E56}-iso` | Single-instance guard for 32-bit `rundll32.exe` `RunDll` host. |
| Log Mutex | `Global\WindowGridRustLog` | `Global\WindowGridRustIsoLog` | Serializes cross-process appends to `WindowGridRust.log`. |

### 2.3 Registered Window Messages

| Message Name | Isolated Name | Direction | `wParam` | `lParam` | Description |
|---|---|---|---|---|---|
| `WindowGridRustTarget` | `WindowGridRustIsoTarget` | `wg-hook` -> `wg-overlay` | Dragged `HWND` (`u32`) | `0` | Posted once per drag before the first `WindowGridRustShowGrid` so the overlay targets the exact window ([ADR 005](adr/005-snap-target-and-placement-verification.md)). |
| `WindowGridRustShowGrid` | `WindowGridRustIsoShowGrid` | `wg-hook` -> `wg-overlay` | `1` (`Resize`), `2` (`Move`), `0` (`End`) | Packed `(x, y)` monitor work-area center (`0` on `End`) | Starts/switches grid mode on the target monitor, or ends the session (`0`) and triggers the final window snap. |
| `WindowGridRustArea` | `WindowGridRustIsoArea` | `wg-hook` -> `wg-overlay` | Packed `(start_x, start_y)` or `(left, top)` | Packed `(cur_x, cur_y)` or `(right, bottom)` | Updates the active grid cell selection during the drag. |
| `WindowGridRustStop` | `WindowGridRustIsoStop` | `wg-app` -> `HWND_BROADCAST` | `0` | `0` | Cleanly terminates running `wg-overlay` and `rundll32` hook hosts. |

## 3. Configuration Schema (`WindowGridRust.settings`)

Persisted as UTF-8 JSON (CRLF line endings, no BOM) in the application directory:

| JSON Path | Type | Default | Description |
|---|---|---|---|
| `App.Portable` | `bool` | `true` | Whether the application runs in portable mode. |
| `App.SplashScreen` | `bool` | `false` | Whether to display the splash screen on startup. |
| `Grid.GridX` | `i32` | `12` | Horizontal grid columns (`1..=64`). |
| `Grid.GridY` | `i32` | `6` | Vertical grid rows (`1..=64`). |
| `Grid.BlurRadius` | `f32` | `3.0` | Selection drop-shadow blur factor (`0.0..=100.0`). |
| `Grid.FillWindow` | `bool` | `true` | Fill selection area (`true`) or render outline only (`false`). |
| `Grid.Opacity` | `f32` | `0.75` | Overlay master opacity (`0.05..=1.0`). |
| `Grid.RadialGradient` | `bool` | `true` | Radial gradient (`true`) or diagonal linear gradient (`false`). |
| `Grid.ShowSelectionHUD` | `bool` | `true` | Show floating HUD pill next to selection. |
| `Grid.ShowDimensions` | `bool` | `true` | Show pixel width and height in HUD. |
| `Grid.ShowCellSpan` | `bool` | `true` | Show cell span (`cols x rows`) in HUD. |
| `Grid.ShowScreenCoverage` | `bool` | `true` | Show monitor work-area coverage percentage in HUD. |
| `Grid.ShowAspectRatio` | `bool` | `true` | Show aspect ratio in HUD. |
| `Grid.ShowModeIndicator` | `bool` | `true` | Show `RESIZE` / `MOVE` mode badge in HUD. |
| `Grid.ShowInteractionHints` | `bool` | `true` | Show bottom shortcut hint bar. |
| `Grid.Anchor` | `i32` | `0` | Oversized window anchor (`0` = `TopLeft`, `1` = `Center`, `2` = `GrabPoint`). |
| `Grid.CompensateWindowMargins` | `bool` | `true` | Compensate for DWM invisible resize borders ([ADR 000](adr/000-window-margin-compensation.md)). |
| `Grid.PreserveSizeOnMove` | `bool` | `true` | Preserve visible window dimensions on `Move` ([ADR 000](adr/000-window-margin-compensation.md)). |
| `Grid.DeferResizeUntilRelease` | `bool` | `true` | Defer window resize until drag release ([ADR 001](adr/001-defer-resize-until-release.md)). |
| `Grid.ThemeName` | `string` | `"default"` | Active theme name (`default`, `dark`, `light`, `red`, `green`, `blue`, or custom). |
| `Grid.Themes` | `array` | 6 built-in themes | Array of `{ Name, ColorLine, ColorStart, ColorEnd }` (`#AARRGGBB`). |
| `Input` | `array` | 4 bindings | Array of `{ Key, Modifier, Command }` shortcut bindings. |

## 4. Security & Reliability Constraints

- **No Personal Data or Network Activity:** All shared-memory sections (`wg-core::ipc`) and `WindowGridRust.settings` contain only local UI geometry and hotkey codes. No outbound network sockets or HTTP clients exist in the dependency tree, and no telemetry is collected.
- **Process Lifecycle Containment:** Child processes (`WindowGridRustOverlay.exe` and `rundll32.exe` hook hosts) run inside a Win32 job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so they cannot outlive `WindowGridRust.exe`. Unrelated `rundll32.exe` instances are never terminated.
- **Hardened Execution:** System binaries (`rundll32.exe`, `schtasks.exe`, `icacls.exe`) are invoked exclusively via resolved absolute paths (`System32` / `SysWOW64`) to avoid DLL hijacking and path injection, and installation directories are ACL-hardened via `icacls`.
