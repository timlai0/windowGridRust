<#
.SYNOPSIS
  Builds the complete Rust WindowGridRust: hook DLLs, overlay, then the app that embeds them.

.DESCRIPTION
  Produces dist\WindowGridRust.exe, a single executable that contains the overlay and the hook
  DLLs and extracts them next to itself on start. (The components are also left in dist\.)

  Like build-overlay.ps1 this needs the GNU target's linker plus binutils (dlltool, as and,
  for the icon, windres) from MSYS2; pass -BinutilsDir if it is not in C:\msys64\ucrt64\bin.

  With -Isolated everything uses test-only names (see rust\README.md) and goes to dist-iso\,
  so it can run next to a real WindowGridRust without interfering with it.
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

$forward = @{}
$features = @()
$targetDir = 'target'
$outDir = 'dist'
if ($Isolated) {
    $forward['Isolated'] = $true
    $features = @('--features', 'isolated')
    $targetDir = 'target-iso'
    $outDir = 'dist-iso'
}

& "$PSScriptRoot\build-hooks.ps1" @forward
& "$PSScriptRoot\build-overlay.ps1" @forward -BinutilsDir $BinutilsDir

# The components changed, so make sure the app picks them up.
(Get-Item "$PSScriptRoot\wg-app\build.rs").LastWriteTime = Get-Date

cargo build -p wg-app --release --target-dir $targetDir @features
if ($LASTEXITCODE -ne 0) { throw 'build failed' }
Copy-Item "$targetDir\release\wg-app.exe" "$outDir\WindowGridRust.exe" -Force
'{0,-24} {1,12:N0} bytes' -f 'WindowGridRust.exe', (Get-Item "$outDir\WindowGridRust.exe").Length
