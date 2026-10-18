# Standard Operating Procedure: Maintenance & Deployment

## 1. Overview

This runbook outlines the operational instructions, build pipeline, testing procedures, and deployment steps for WindowGrid (Rust workspace in `rust/`).

## 2. Build & Testing Procedures

### 2.1 Prerequisites

- Windows 10 or Windows 11
- Rust toolchain (`rustup`) with targets:
  ```powershell
  rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc aarch64-pc-windows-msvc
  ```
- MSYS2 binutils (`dlltool.exe`, `as.exe`, `windres.exe`) in `C:\msys64\ucrt64\bin` (or pass `-BinutilsDir <path>` to the build scripts) when linking with the GNU toolchain.

### 2.2 Build Pipeline

All build scripts are located in `rust/` and output release binaries to `rust/dist/`:

```powershell
cd rust
.\build-hooks.ps1    # Builds WindowGrid64.dll, WindowGrid32.dll, WindowGridARM64.dll into rust\dist\
.\build-overlay.ps1  # Builds WindowGridOverlay.exe into rust\dist\
.\build-app.ps1      # Builds hooks + overlay, then embeds them into rust\dist\WindowGrid.exe
```

To build isolated test binaries (`WindowGridIso*` IPC names in `rust/dist-iso/` that do not disturb a running WindowGrid installation):

```powershell
cd rust
.\build-app.ps1 -Isolated
```

### 2.3 Automated Verification & Quality Assurance

1. **Unit, Render, and Lifecycle Tests:**
   ```powershell
   cd rust
   $env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
   cargo test --features isolated
   ```
2. **Real-Drag End-to-End Suite (`wg-e2e`):**
   Close any active WindowGrid instance before running, as `wg-e2e` injects physical mouse/keyboard events via `SendInput`:
   ```powershell
   cd rust
   .\build-app.ps1 -Isolated
   $env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
   cargo test -p wg-e2e -- --ignored --nocapture --test-threads=1
   ```

## 3. Deployment & Release Management

### 3.1 Release Artifacts

`rust\dist\WindowGrid.exe` is a self-contained single-file executable (~10.5 MB) that embeds `WindowGridOverlay.exe`, `WindowGrid64.dll`, `WindowGrid32.dll`, and `WindowGridARM64.dll`.

- On startup, `WindowGrid.exe` extracts its embedded components next to itself if missing or changed. If a hook DLL is currently mapped into another process and cannot be overwritten directly, `wg-app` renames the in-use file aside (`*.old-<N>`), writes the new DLL, and cleans up `.old-*` files on subsequent startups.
- Before distributing a release, sign `WindowGridOverlay.exe` and the hook DLLs prior to running `cargo build -p wg-app --release`, then sign the resulting `WindowGrid.exe`.
- Do not deploy active runtime directories inside OneDrive-synced folders.

### 3.2 Installation & Rollback

- **Install / Update:** Run `WindowGrid.exe /install [path] [/silent|/s] [/shortcut] [/onstartup]` (defaults to `%ProgramFiles%\WindowGrid`) or `/replace <path>`.
- **Rollback Procedure:**
  1. Revoke the problematic release artifact from the distribution channel.
  2. Replace `WindowGrid.exe` with the previous known-good release (`WindowGrid.exe /replace "<installed_path>"`), which automatically extracts matching embedded components on launch.

### 3.3 Post-Deployment Smoke Test

1. Launch `WindowGrid.exe` and verify `WindowGrid.log` reports clean startup of `WindowGridOverlay.exe`, `WindowGrid32.dll`, and `WindowGrid64.dll`.
2. Drag a window by its title bar, tap **Space** (or click the right mouse button), move across grid cells, and release the primary mouse button. Verify the overlay remains visible throughout the drag and snaps accurately on release, and that pressing `Esc` cancels without snapping.

## 4. Security & Privacy Maintenance

- **Dependency Audits:** Run `cargo audit` regularly in `rust/`.
- **Log Inspection:** Inspect `WindowGrid.log` in the application directory for unexpected child exits or crash-loop backoff warnings.
- **Privacy:** Keep WindowGrid free of telemetry, personal data collection, and outbound network requests.
