# Standard Operating Procedure: Maintenance & Deployment

## 1. Overview

This runbook outlines the operational instructions, build pipeline, testing procedures, and deployment/rollback steps for WindowGridRust (`rust/` workspace).

## 2. Build & Testing Procedures

Toolchain setup, build scripts, test environment variables, and the test suites are documented in the [Rust Workspace Guide](../rust/README.md). Before a release:

1. Build with `build-app.ps1` (outputs `rust\dist\WindowGridRust.exe`).
2. Run the unit, render, and isolated lifecycle tests.
3. Run the real-drag end-to-end suite (`wg-e2e`) with any running WindowGridRust instance closed.

## 3. Deployment & Release Management

### 3.1 Release Artifacts

`rust\dist\WindowGridRust.exe` is a self-contained single-file executable (~10.5 MB) that embeds `WindowGridRustOverlay.exe`, `WindowGridRust64.dll`, `WindowGridRust32.dll`, and `WindowGridRustARM64.dll`.

- On startup, `WindowGridRust.exe` extracts its embedded components next to itself if missing or changed. If a hook DLL is currently mapped into another process and cannot be overwritten directly, `wg-app` renames the in-use file aside (`*.old-<N>`), writes the new DLL, and cleans up `.old-*` files on subsequent startups.
- Before distributing a release, sign `WindowGridRustOverlay.exe` and the hook DLLs prior to running `cargo build -p wg-app --release`, then sign the resulting `WindowGridRust.exe`.
- Do not deploy active runtime directories inside OneDrive-synced folders.

### 3.2 Installation & Rollback

- **Install / Update (CMD):**
  ```cmd
  WindowGridRust.exe /install "C:\Program Files\WindowGridRust" /silent /shortcut /onstartup
  ```
  Or replace an existing binary in place:
  ```cmd
  WindowGridRust.exe /replace "C:\Program Files\WindowGridRust\WindowGridRust.exe"
  ```
- **Rollback Procedure:**
  1. Execute the previous known-good release with `/replace`:
     ```cmd
     WindowGridRust-previous.exe /replace "C:\Program Files\WindowGridRust\WindowGridRust.exe"
     ```
     This stops the active instance, replaces `WindowGridRust.exe`, and automatically extracts matching embedded overlay and hook DLLs on launch.
  2. Verify `WindowGridRust.log` confirms clean startup of the restored version.

### 3.3 Post-Deployment Smoke Test

1. Launch `WindowGridRust.exe` and verify `WindowGridRust.log` reports clean startup of `WindowGridRustOverlay.exe`, `WindowGridRust32.dll`, and `WindowGridRust64.dll`.
2. Drag a window by its title bar, tap **Space** (or click the right mouse button), move across grid cells, and release the primary mouse button. Verify the overlay remains visible throughout the drag, snaps accurately on release (including when dragging a maximized window or releasing near a monitor edge), and logs `Snap: Target=0x...` without `did not settle` warnings.
3. Repeat a drag and press **Esc** before releasing the mouse button; verify the session cancels without snapping.

## 4. Security & Privacy Maintenance

- **Dependency Audits:** Run `cargo audit` regularly in `rust/`.
- **Log Inspection:** Inspect `WindowGridRust.log` in the application directory for unexpected child exits, crash-loop backoff warnings, or `did not settle` snap warnings.
- **Privacy:** Keep WindowGridRust free of telemetry, personal data collection, and outbound network requests.
