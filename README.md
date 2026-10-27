# WindowGrid

WindowGrid is a native Windows desktop utility written in Rust that allows users to quickly snap, resize, and align windows to a customizable grid during mouse, touch, or pen drag operations.

## Features

- **Intuitive Snapping:** Snap windows to a customizable grid while dragging by tapping or holding configurable keyboard or mouse shortcuts.
- **Deferred Resizing:** Optionally defer window resizing until the mouse button is released (`DeferResizeUntilRelease`, enabled by default) to avoid layout reflow and visual stuttering in complex apps.
- **Verified Window Placement:** Tracks the exact dragged window handle (`WindowGridTarget`), restores maximized windows prior to snapping, and verifies post-snap placement to prevent displacement by Aero Snap, cross-monitor DPI changes, or application exit-move handlers.
- **Window Margin Compensation:** Compensates for DWM invisible resize borders (`CompensateWindowMargins`) so the visible window frame matches grid boundaries without shrinking.
- **Size-Preserving Moves:** Retains the window's visible dimensions when moving it across grid cells (`PreserveSizeOnMove`).
- **Configurable Alignment Anchors:** Choose how oversized windows align relative to the selected grid cells (`TopLeft`, `Center`, `GrabPoint`).
- **Customizable Appearance & HUD:** Configure column and row counts, color themes (`default`, `dark`, `light`, `red`, `green`, `blue`), outline or filled selection (`FillWindow`), shadow blur (`BlurRadius`), radial or linear gradient, and HUD metrics (dimensions, cell span, screen coverage, aspect ratio, mode indicator, interaction hints).
- **Minimal Footprint:** Single ~10.5 MB executable (`WindowGrid.exe`) embedding the DirectComposition/Direct2D overlay (~447 KB, ~9 MB idle working set) and `no_std` CRT-free hook DLLs (~17–20 KB each), with ~27 MB idle working set for the host app.

## Default Input Bindings

While dragging a target window (holding the primary mouse button):

| Input | Mode | Behavior |
|---|---|---|
| **Space** / **Right Mouse Button** | `Resize` | Tapping activates the grid and locks the starting cell on key/button release; holding moves the starting cell with the cursor until released. |
| **Left Control** / **Middle Mouse Button** | `Move` | Moves the window to the selected grid cell while preserving its visible dimensions. |
| **Release Primary Mouse Button** | Apply | Snaps the dragged window to the selected grid cells. |
| **Esc** | Cancel | Cancels the active grid session without snapping. |

Input bindings can be customized in the Settings window via the system tray icon.

## Installation & CLI Usage

`WindowGrid.exe` can run portably from any writable directory or install itself to the local machine:

```cmd
WindowGrid.exe
WindowGrid.exe /install [path] [/silent|/s] [/shortcut] [/onstartup]
WindowGrid.exe /replace <path>
WindowGrid.exe /help
```

| Flag | Description |
|---|---|
| *(none)* | Run directly (extracts embedded `WindowGridOverlay.exe` and hook DLLs next to itself). |
| `/install [path]` | Install to target directory (defaults to `%ProgramFiles%\WindowGrid`). |
| `/silent` or `/s` | Suppress installer UI dialogs. |
| `/shortcut` | Create Start Menu shortcut. |
| `/onstartup` | Register logon autostart (Registry `Run` value and elevated Task Scheduler entry). |
| `/replace <path>` | Stop running instance, copy current executable to `<path>`, and launch it. |
| `/help` or `/?` | Show CLI usage. |

## Configuration

Settings are stored in `WindowGrid.settings` (JSON) next to `WindowGrid.exe`:

| Category | Setting Keys | Description |
|---|---|---|
| **Application** | `Portable`, `SplashScreen` | Portable execution flag and startup splash screen toggle. |
| **Grid Dimensions** | `GridX`, `GridY` | Column and row counts (`1..=64`, default `12` x `6`; transposed automatically on portrait monitors). |
| **Appearance** | `Opacity`, `BlurRadius`, `FillWindow`, `RadialGradient`, `ThemeName`, `Themes` | Overlay transparency, shadow blur (`6 * BlurRadius` DIPs), filled vs. outline selection, gradient style, and `#AARRGGBB` color themes. |
| **Selection HUD** | `ShowSelectionHUD`, `ShowDimensions`, `ShowCellSpan`, `ShowScreenCoverage`, `ShowAspectRatio`, `ShowModeIndicator`, `ShowInteractionHints` | Individual toggles for the floating HUD readout pill and bottom interaction hint bar. |
| **Behavior** | `Anchor` (`0` = `TopLeft`, `1` = `Center`, `2` = `GrabPoint`), `CompensateWindowMargins`, `PreserveSizeOnMove`, `DeferResizeUntilRelease` | Oversized move alignment, DWM invisible border compensation, move size retention, and deferred resize on release. |
| **Input** | `Input` (`[{ Key, Modifier, Command }]`) | Array of up to 8 shortcut bindings mapping scan codes or mouse buttons to `Resize` or `Move`. |

## Documentation & Development

- [Rust Workspace & Build Guide](rust/README.md) — crate architecture, toolchain setup, build scripts, and isolated/e2e test suites.
- [Architecture & IPC Contracts](docs/ARCHITECTURE_AND_IPC.md)
- [Maintenance Standard Operating Procedures](docs/MAINTENANCE_SOP.md)
- [ADR 003: Window Margin Compensation and Move Size Preservation](docs/adr/003-window-margin-compensation.md)
- [ADR 004: Defer Window Resize Until Drag Release](docs/adr/004-defer-resize-until-release.md)
- [ADR 005: Behavior Specification for the Rust Hook (`wg-hook`)](docs/adr/005-rust-hook-behavior-spec.md)
- [ADR 006: Rust Overlay (`wg-overlay`)](docs/adr/006-rust-overlay.md)
- [ADR 007: Rust App (`wg-app`)](docs/adr/007-rust-app.md)
- [ADR 008: Target Window Identification and Post-Snap Placement Verification](docs/adr/008-snap-target-and-placement-verification.md)
- [Changelog](CHANGELOG.md)

## License

WindowGrid is released under the [MIT License](LICENSE).
