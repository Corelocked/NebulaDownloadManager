param(
    [switch]$TorrentRqbit
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$distRoot = Join-Path $repoRoot "dist"
$packageVariant = if ($TorrentRqbit) { "rqbit" } else { "default" }
$packageName = "NebulaDM-win64-$packageVariant-" + (Get-Date -Format "yyyyMMdd-HHmmss")
$packageRoot = Join-Path $distRoot $packageName
$binaryName = "desktop.exe"
$targetDirName = if ($TorrentRqbit) { "target-release-rqbit-desktop" } else { "target-release-desktop" }
$releaseBinary = Join-Path $repoRoot "$targetDirName\release\$binaryName"
$extensionSource = Join-Path $repoRoot "extensions\browser"
$extensionDest = Join-Path $packageRoot "browser-extension"
$readmeSource = Join-Path $repoRoot "README.md"
$readmeDest = Join-Path $packageRoot "README.md"
$setupDest = Join-Path $packageRoot "SETUP.txt"

New-Item -ItemType Directory -Path $packageRoot | Out-Null

$cargoArgs = @("build", "-p", "desktop", "--release")
if ($TorrentRqbit) {
    $cargoArgs += @("--features", "torrent-rqbit")
}

Write-Host "Building NebulaDM release..."
$env:CARGO_TARGET_DIR = $targetDirName
try {
    & cargo @cargoArgs
}
finally {
    Remove-Item Env:\CARGO_TARGET_DIR -ErrorAction SilentlyContinue
}

if (-not (Test-Path $releaseBinary)) {
    throw "Release binary not found at $releaseBinary"
}

Copy-Item -LiteralPath $releaseBinary -Destination (Join-Path $packageRoot "NebulaDM.exe")
Copy-Item -LiteralPath $readmeSource -Destination $readmeDest
Copy-Item -LiteralPath $extensionSource -Destination $extensionDest -Recurse

$featureLine = if ($TorrentRqbit) {
    "This build includes the real torrent engine feature: torrent-rqbit"
} else {
    "This build uses the default simulated torrent engine path. Re-run with -TorrentRqbit to bundle the real torrent feature build."
}

$setupText = @"
NebulaDM Windows Setup
======================

1. Launch NebulaDM.exe
2. In Chrome or Edge, open the extensions page and enable Developer mode
3. Choose Load unpacked and select:
   $extensionDest
4. Keep NebulaDM running while testing browser capture

Storage
-------
- Queue state is stored in %LOCALAPPDATA%\NebulaDM when available
- Downloads default into your Downloads\NebulaDM folder

Bridge
------
- Browser extension target: http://127.0.0.1:35791
- Chromium browsers normally require unpacked loading or browser-side CRX packaging with a private key

Build Notes
-----------
- $featureLine
"@

Set-Content -LiteralPath $setupDest -Value $setupText

Write-Host ""
Write-Host "Package ready:"
Write-Host "  $packageRoot"
