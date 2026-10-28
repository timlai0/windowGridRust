<#
.SYNOPSIS
  Builds the Rust overlay (release) and copies it to rust\dist as WindowGridRustOverlay.exe.

.DESCRIPTION
  The overlay uses the standard library, so unlike the hook DLLs it needs a working linker
  for the C runtime. This script uses the GNU target, whose Rust toolchain bundles one, and
  puts a directory with binutils' dlltool.exe and as.exe on PATH (the windows crate's
  raw-dylib imports need them). Pass -BinutilsDir if MSYS2 is not in C:\msys64\ucrt64\bin.

  With -Isolated the executable uses test-only mutex, window-message and shared-memory
  names (see rust\README.md) and is written to dist-iso\, so it can never replace or disturb
  a running WindowGridRust overlay.
#>
param(
    [switch]$Isolated,
    [string]$BinutilsDir = 'C:\msys64\ucrt64\bin'
)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

if (-not (Test-Path (Join-Path $BinutilsDir 'dlltool.exe'))) {
    throw "dlltool.exe not found in $BinutilsDir. Install MSYS2 binutils or pass -BinutilsDir."
}
$env:PATH = "$BinutilsDir;$env:PATH"

$features = @()
$targetDir = 'target'
$outDir = 'dist'
if ($Isolated) {
    $features = @('--features', 'isolated')
    $targetDir = 'target-iso'
    $outDir = 'dist-iso'
}

New-Item -ItemType Directory -Force $outDir | Out-Null
cargo build -p wg-overlay --release --target-dir $targetDir @features
if ($LASTEXITCODE -ne 0) { throw 'build failed' }
Copy-Item "$targetDir\release\wg-overlay.exe" "$outDir\WindowGridRustOverlay.exe" -Force
'{0,-24} {1,8:N0} bytes' -f 'WindowGridRustOverlay.exe', (Get-Item "$outDir\WindowGridRustOverlay.exe").Length
