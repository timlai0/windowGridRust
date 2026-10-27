# Standard Operating Procedure: Maintenance & Deployment

## 1. Overview

This runbook outlines the operational instructions, build pipeline, testing procedures, and deployment/rollback steps for WindowGrid (`rust/` workspace).

## 2. Build & Testing Procedures

### 2.1 Prerequisites

- Windows 10 or Windows 11
- Rust toolchain (`rustup`) with targets:
  ```cmd
  rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc aarch64-pc-windows-msvc
  ```
- MSYS2 binutils (`dlltool.exe`, `as.exe`, `windres.exe`) in `C:\msys64\ucrt64\bin` (or pass `-BinutilsDir <path>` to the build scripts) when linking with the GNU toolchain.

### 2.2 Build Pipeline

All build scripts are located in `rust/` and output release binaries to `rust\dist\`:

**Windows CMD:**
```cmd
cd rust
powershell -NoProfile -ExecutionPolicy Bypass -File build-hooks.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File build-overlay.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File build-app.ps1
```

**PowerShell:**
```powershell
cd rust
.\build-hooks.ps1    # Builds WindowGrid64.dll, WindowGrid32.dll, WindowGridARM64.dll into rust\dist\
.\build-overlay.ps1  # Builds WindowGridOverlay.exe into rust\dist\
.\build-app.ps1      # Builds hooks + overlay, then embeds them into rust\dist\WindowGrid.exe
```

To build isolated test binaries (`WindowGridIso*` IPC names in `rust\dist-iso\` that do not disturb a running WindowGrid installation):

```cmd
cd rust
powershell -NoProfile -ExecutionPolicy Bypass -File build-app.ps1 -Isolated
```

### 2.3 Test Environment Variables (`--features isolated`)

Isolated test builds (`--features isolated`) honor the following environment variables for deterministic automated testing (ignored in production builds):

| Environment Variable | Component | Format / Values | Description |
|---|---|---|---|
| `WG_TEST_RECT` | `wg-overlay` | `left,top,right,bottom` | Overrides monitor work area with a fixed bounding box. |
| `WG_TEST_TARGET` | `wg-overlay` | `<hwnd_usize>` | Forces the snap target window handle instead of querying pointer coordinates. |
| `WG_TEST_NO_TIMER` | `wg-overlay` | `1` | Disables the 15 ms safety-net timer when testing without physical mouse input. |
| `WG_TEST_NO_PROMPT` | `wg-app` | `1` | Suppresses the first-run setup dialog prompt. |
| `WG_TEST_OPEN_SETTINGS` | `wg-app` | `1` | Opens the Settings window immediately on startup for UI verification. |
| `WG_TEST_QUIT_AFTER_MS` | `wg-app` | `<milliseconds>` | Automatically exits the event loop after the specified duration. |

### 2.4 Automated Verification & Quality Assurance

1. **Unit, Render, and Lifecycle Tests (CMD):**
   ```cmd
   cd rust
   set "PATH=C:\msys64\ucrt64\bin;%PATH%"
   cargo test --features isolated
   ```
2. **Real-Drag End-to-End Suite (`wg-e2e`):**
   Close any active WindowGrid instance before running, as `wg-e2e` injects physical mouse/keyboard events via `SendInput`:
   ```cmd
   cd rust
   powershell -NoProfile -ExecutionPolicy Bypass -File build-app.ps1 -Isolated
   set "PATH=C:\msys64\ucrt64\bin;%PATH%"
   cargo test -p wg-e2e -- --ignored --nocapture --test-threads=1
   ```

## 3. Deployment & Release Management

### 3.1 Release Artifacts

`rust\dist\WindowGrid.exe` is a self-contained single-file executable (~10.5 MB) that embeds `WindowGridOverlay.exe`, `WindowGrid64.dll`, `WindowGrid32.dll`, and `WindowGridARM64.dll`.

- On startup, `WindowGrid.exe` extracts its embedded components next to itself if missing or changed. If a hook DLL is currently mapped into another process and cannot be overwritten directly, `wg-app` renames the in-use file aside (`*.old-<N>`), writes the new DLL, and cleans up `.old-*` files on subsequent startups.
- Before distributing a release, sign `WindowGridOverlay.exe` and the hook DLLs prior to running `cargo build -p wg-app --release`, then sign the resulting `WindowGrid.exe`.
- Do not deploy active runtime directories inside OneDrive-synced folders.

### 3.2 Installation & Rollback

- **Install / Update (CMD):**
  ```cmd
  WindowGrid.exe /install "C:\Program Files\WindowGrid" /silent /shortcut /onstartup
  ```
  Or replace an existing binary in place:
  ```cmd
  WindowGrid.exe /replace "C:\Program Files\WindowGrid\WindowGrid.exe"
  ```
- **Rollback Procedure:**
  1. Revoke the affected release artifact from the distribution channel.
  2. Execute the previous known-good release with `/replace`:
     ```cmd
     WindowGrid-previous.exe /replace "C:\Program Files\WindowGrid\WindowGrid.exe"
     ```
     This stops the active instance, replaces `WindowGrid.exe`, and automatically extracts matching embedded overlay and hook DLLs on launch.
  3. Verify `WindowGrid.log` confirms clean startup of the restored version.

### 3.3 Post-Deployment Smoke Test

1. Launch `WindowGrid.exe` and verify `WindowGrid.log` reports clean startup of `WindowGridOverlay.exe`, `WindowGrid32.dll`, and `WindowGrid64.dll`.
2. Drag a window by its title bar, tap **Space** (or click the right mouse button), move across grid cells, and release the primary mouse button. Verify the overlay remains visible throughout the drag, snaps accurately on release (including when dragging a maximized window or releasing near a monitor edge), and logs `Snap: Target=0x...` without `did not settle` warnings.
3. Repeat a drag and press **Esc** before releasing the mouse button; verify the session cancels without snapping.

## 4. Security & Privacy Maintenance

- **Dependency Audits:** Run `cargo audit` regularly in `rust/`.
- **Log Inspection:** Inspect `WindowGrid.log` in the application directory for unexpected child exits, crash-loop backoff warnings, or `did not settle` snap warnings.
- **Privacy:** Keep WindowGrid free of telemetry, personal data collection, and outbound network requests.
