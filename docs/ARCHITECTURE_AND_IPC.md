# Architecture & IPC Data Contracts

## 1. System Components

WindowGrid operates via a multi-process architecture in Rust (`rust/` workspace) to safely inject global hooks into the Windows OS across process bitness boundaries:

- **`wg-app` (`WindowGrid.exe`):** Host application managing the Slint settings UI, system tray icon, CLI installer, shared-memory configuration publication, and child process supervision via a Win32 job object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`). Embeds the compiled overlay and hook binaries and extracts them next to itself on startup.
- **`wg-overlay` (`WindowGridOverlay.exe`):** Standalone DirectComposition / Direct2D / DirectWrite process that renders the click-through grid, selection, HUD, and interaction hints on the GPU, and executes the final window snap (`SetWindowPos`) with DWM extended frame margin compensation.
- **`wg-hook` (`WindowGrid64.dll`, `WindowGrid32.dll`, `WindowGridARM64.dll`):** `no_std`, CRT-free native libraries hosted via `rundll32.exe <dll>,RunDll` (one host per architecture). Each host installs an in-context `SetWinEventHook(EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND)` and per-drag thread-scoped `WH_KEYBOARD` / `WH_MOUSE` hooks plus window procedure subclassing (`GWLP_WNDPROC`).
- **`wg-core`:** Shared library defining the `WindowGrid.settings` JSON model, key code tables, log formatting, and `repr(C, packed)` shared-memory wire contracts.
- **`wg-win`:** Shared Win32 helpers for pagefile-backed shared memory, mutex synchronization, single-instance management, and file logging.

## 2. Inter-Process Communication (IPC)

Because `wg-hook` executes inside the address space of whatever application window the user drags, WindowGrid uses pagefile-backed named shared-memory sections (`CreateFileMappingW` / `OpenFileMappingW`) and registered window messages (`RegisterWindowMessageW`) for zero-copy communication.

### 2.1 Shared Memory Sections (`wg-core::ipc`)

All shared-memory structs are `#[repr(C, packed)]` plain old data read and written at offset 0. Single-byte booleans (`u8`, `0` or `1`) and packed ARGB colors (`0xAARRGGBB` as `i32`) preserve wire compatibility with the legacy format.

#### `WindowGridMapping_Hook` (`SettingsMemory`, 16 bytes)
Written by `wg-app`, updated with `grid_window` by `wg-overlay`, and read by `wg-hook`:

```rust
#[repr(C, packed)]
pub struct SettingsMemory {
    pub grid_window: i32, // Overlay HWND (truncated to 32 bits, sign-extended by hooks)
    pub grid_x: i32,      // Columns, with DEFER_FLAG (0x10000) in bit 16
    pub grid_y: i32,      // Rows
    pub input_size: i32,  // Number of active entries in WindowGridMapping_Input (max 8)
}
```

#### `WindowGridMapping_Input` (`[InputMemory; 8]`, 64 bytes)
Written by `wg-app`, read by `wg-hook`:

```rust
#[repr(C, packed)]
pub struct InputMemory {
    pub input: i32,    // KeyCode (scan code / mouse button ID)
    pub modifier: i16, // Modifier flags
    pub command: i16,  // 1 = Resize, 2 = Move
}
```

#### `WindowGridMapping_GridWindow` (`GridWindowMemory`, 44 bytes)
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

#### `WindowGridMapping_GridWindowHandle` (4 bytes)
Holds the overlay `HWND` as a 32-bit integer (`i32`).

### 2.2 Synchronization & Single-Instance Mutexes

- **Shared-Memory Access:** Each shared-memory section `<Name>` is paired with a named mutex `Global\<Name>` to serialize reads and writes.
- **Single-Instance & Host Guards:**
  - App instance: `fde9b139-746f-4ca9-898f-b6dcff4c8f5e`
  - Overlay instance: `4AB23252-ACFE-487D-BA61-0C32CE3DB81E`
  - 64-bit hook host (`RunDll`): `Global\{58B6584C-820B-4813-97D1-6C923AD1CB42}`
  - 32-bit hook host (`RunDll`): `Global\{C26E1FC8-F719-4044-9122-029960C6C933}`
  - Log writer mutex: `Global\WindowGridLog`
- **Isolated Test Mode (`--features isolated`):** Appends `-iso` or `WindowGridIso*` prefixes so automated tests (`wg-e2e`, `wg-overlay`, `wg-app`) never collide with a running installation.

### 2.3 Registered Window Messages

- **`WindowGridShowGrid`:** Posted by `wg-hook` to the overlay `HWND`. `wParam`: `1` (`Resize`), `2` (`Move`), or `0` (`End` — drag finished, execute snap and hide). `lParam`: packed `(x, y)` center of the target monitor work area.
- **`WindowGridArea`:** Posted by `wg-hook` to the overlay `HWND` when selected cells change. `wParam`: packed `(start_x, start_y)` or `(left, top)`. `lParam`: packed `(cur_x, cur_y)` or `(right, bottom)`.
- **`WindowGridStop`:** Broadcast via `PostMessageW(HWND_BROADCAST, ...)` by `wg-app` to cleanly terminate running overlay and `rundll32` hook hosts.

## 3. Security & Reliability Constraints

- **No PII or Network Activity:** The IPC contract strictly transmits local UI and grid configuration values. The application makes no outbound network requests and collects no telemetry.
- **Process Lifecycle Containment:** All spawned helper processes (`WindowGridOverlay.exe` and `rundll32.exe` hook hosts) are assigned to a Win32 job object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Helpers cannot outlive `WindowGrid.exe`, and unrelated system `rundll32.exe` processes are never terminated.
- **Hardened Execution:** System tools (`rundll32.exe`, `schtasks.exe`, `icacls.exe`) are invoked exclusively via resolved absolute paths (`System32` / `SysWOW64`), and installation directories are ACL-hardened.
