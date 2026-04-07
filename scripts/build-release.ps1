param(
    [switch]$TorrentRqbit
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$distRoot = Join-Path $repoRoot "dist"
$packageVariant = if ($TorrentRqbit) { "rqbit" } else { "default" }
$appVersion = (Get-Content (Join-Path $repoRoot "apps\desktop\Cargo.toml") | Where-Object { $_ -match '^version = ' } | Select-Object -First 1)
$appVersion = ($appVersion -replace 'version = "', '') -replace '"', ''
$packageName = "NebulaDM-win64-$packageVariant-v$appVersion"
$packageRoot = Join-Path $distRoot $packageName
$portableZipPath = Join-Path $distRoot ($packageName + ".zip")
$binaryName = "desktop.exe"
$targetDirName = if ($TorrentRqbit) { "target-release-rqbit-desktop" } else { "target-release-desktop" }
$releaseBinary = Join-Path $repoRoot "$targetDirName\release\$binaryName"
$extensionSource = Join-Path $repoRoot "extensions\browser"
$extensionDest = Join-Path $packageRoot "browser-extension"
$ffmpegBundleSource = Join-Path $repoRoot "tools\ffmpeg\ffmpeg.exe"
$ffmpegBundleDest = Join-Path $packageRoot "tools\ffmpeg.exe"
$ytDlpBundleSource = Join-Path $repoRoot "tools\yt-dlp\yt-dlp.exe"
$ytDlpBundleDest = Join-Path $packageRoot "tools\yt-dlp.exe"
$readmeSource = Join-Path $repoRoot "README.md"
$readmeDest = Join-Path $packageRoot "README.md"
$setupDest = Join-Path $packageRoot "SETUP.txt"
$packagePattern = "NebulaDM-win64-$packageVariant-v*"

Get-ChildItem -Path $distRoot -Filter $packagePattern -Force -ErrorAction SilentlyContinue | ForEach-Object {
    if ($_.PSIsContainer) {
        Remove-Item -LiteralPath $_.FullName -Recurse -Force
    }
    else {
        Remove-Item -LiteralPath $_.FullName -Force
    }
}
New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null

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
if (Test-Path $ffmpegBundleSource) {
    New-Item -ItemType Directory -Path (Split-Path -Parent $ffmpegBundleDest) -Force | Out-Null
    Copy-Item -LiteralPath $ffmpegBundleSource -Destination $ffmpegBundleDest -Force
}
if (Test-Path $ytDlpBundleSource) {
    New-Item -ItemType Directory -Path (Split-Path -Parent $ytDlpBundleDest) -Force | Out-Null
    Copy-Item -LiteralPath $ytDlpBundleSource -Destination $ytDlpBundleDest -Force
}

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
- If present, a bundled ffmpeg binary is placed in tools\ffmpeg.exe for adaptive browser video merging
- If present, a bundled yt-dlp binary is placed in tools\yt-dlp.exe for reliable YouTube downloads
"@

Set-Content -LiteralPath $setupDest -Value $setupText
Compress-Archive -LiteralPath $packageRoot -DestinationPath $portableZipPath

Write-Host ""
Write-Host "Package ready:"
Write-Host "  $packageRoot"
Write-Host "Portable zip:"
Write-Host "  $portableZipPath"
