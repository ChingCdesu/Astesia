[CmdletBinding()]
param(
    [string]$Target = 'x86_64-pc-windows-msvc'
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

if ($Target -ne 'x86_64-pc-windows-msvc') {
    throw "Unsupported Windows target: $Target"
}

$RepositoryRoot = Split-Path -Parent $PSScriptRoot
$Manifest = Join-Path $RepositoryRoot 'src-tauri/Cargo.toml'
$ManifestText = Get-Content -Raw $Manifest
$VersionMatch = [regex]::Match($ManifestText, '(?m)^version = "([^"]+)"')
if (-not $VersionMatch.Success) {
    throw 'Could not read the Astesia package version'
}
$Version = $VersionMatch.Groups[1].Value
$TargetRoot = if ($env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR
} else {
    Join-Path $RepositoryRoot 'src-tauri/target'
}
$ReleaseDirectory = Join-Path $TargetRoot "$Target/release"
$PackageDirectory = Join-Path $TargetRoot 'package'
$Archive = Join-Path $PackageDirectory "astesia-$Version-$Target.zip"
$Stage = Join-Path ([System.IO.Path]::GetTempPath()) "astesia-package-$([guid]::NewGuid())"
$Bundle = Join-Path $Stage 'Astesia'

try {
    rustup target add --toolchain 1.97.1 $Target
    if ($LASTEXITCODE -ne 0) { throw 'Could not install the Rust target' }
    rustup run 1.97.1 cargo build --release --locked --manifest-path $Manifest --target $Target --bin astesia --bin astesia-mcp
    if ($LASTEXITCODE -ne 0) { throw 'Could not build Astesia' }

    New-Item -ItemType Directory -Force -Path $Bundle, $PackageDirectory | Out-Null
    Copy-Item (Join-Path $ReleaseDirectory 'astesia.exe') (Join-Path $Bundle 'astesia.exe')
    Copy-Item (Join-Path $ReleaseDirectory 'astesia-mcp.exe') (Join-Path $Bundle 'astesia-mcp.exe')
    if (Test-Path $Archive) { Remove-Item $Archive -Force }
    Compress-Archive -Path $Bundle -DestinationPath $Archive
    Write-Output $Archive
} finally {
    if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
}
