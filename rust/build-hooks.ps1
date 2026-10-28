<#
.SYNOPSIS
  Builds the Rust hook DLLs for x64, x86 and ARM64 and copies them to rust\dist.

.DESCRIPTION
  Produces dist\WindowGridRust64.dll and dist\WindowGridRust32.dll (the names the app expects)
  and dist\WindowGridRustARM64.dll. With -Isolated the DLLs use test-only mutex,
  window-message and shared-memory names (see rust\README.md) so they can never interfere
  with a running WindowGridRust; those go to dist-iso\.

  Needs only the Rust toolchain with the three MSVC targets (rustup target add
  x86_64-pc-windows-msvc i686-pc-windows-msvc aarch64-pc-windows-msvc). No Visual Studio
  or Windows SDK.
#>
param([switch]$Isolated)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$features = @()
$targetDir = 'target'
$outDir = 'dist'
if ($Isolated) {
    $features = @('--features', 'isolated')
    $targetDir = 'target-iso'
    $outDir = 'dist-iso'
}

New-Item -ItemType Directory -Force $outDir | Out-Null

$builds = @(
    @{ Target = 'x86_64-pc-windows-msvc'; Name = 'WindowGridRust64.dll' },
    @{ Target = 'i686-pc-windows-msvc';   Name = 'WindowGridRust32.dll' },
    @{ Target = 'aarch64-pc-windows-msvc'; Name = 'WindowGridRustARM64.dll' }
)

foreach ($b in $builds) {
    cargo build -p wg-hook --release --target $b.Target --target-dir $targetDir @features
    if ($LASTEXITCODE -ne 0) { throw "build failed for $($b.Target)" }
    Copy-Item "$targetDir\$($b.Target)\release\wg_hook.dll" "$outDir\$($b.Name)" -Force
    '{0,-18} {1,8:N0} bytes' -f $b.Name, (Get-Item "$outDir\$($b.Name)").Length
}
