# Dot-source this file from PowerShell before running Cargo or the Tauri CLI.
param(
    [string]$Msys2Prefix = "$env:USERPROFILE\.local\msys64\ucrt64",
    [string]$VcpkgRoot = (Join-Path $PSScriptRoot '../.tools/vcpkg'),
    [string]$Python = ''
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath (Join-Path $Msys2Prefix 'bin/g++.exe'))) {
    throw 'Install MSYS2 UCRT64 dependencies first, or specify -Msys2Prefix.'
}
if (-not (Test-Path -LiteralPath (Join-Path $VcpkgRoot 'installed/x64-windows-static-md/lib/heif.lib'))) {
    throw 'Install libheif[aom]:x64-windows-static-md with vcpkg first, or specify -VcpkgRoot.'
}
$env:OPENROOM_MSYS2_PREFIX = (Resolve-Path -LiteralPath $Msys2Prefix).Path
if (-not $Python) { $Python = Join-Path $Msys2Prefix 'bin/python.exe' }
$env:OPENROOM_PYTHON = (Get-Command $Python -ErrorAction Stop).Source
$env:VCPKG_ROOT = (Resolve-Path -LiteralPath $VcpkgRoot).Path
$env:VCPKGRS_TRIPLET = 'x64-windows-static-md'
$env:PATH = (Join-Path $env:USERPROFILE '.cargo/bin') + [IO.Path]::PathSeparator + $env:PATH
