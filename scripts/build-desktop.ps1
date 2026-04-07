param(
    [switch]$Release,
    [switch]$TorrentRqbit
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$targetDirName = if ($Release) {
    if ($TorrentRqbit) { "target-release-rqbit-desktop" } else { "target-release-desktop" }
} else {
    "target"
}
$profileDir = if ($Release) { "release" } else { "debug" }
$binaryPath = Join-Path $repoRoot "$targetDirName\$profileDir\desktop.exe"
$runtimeToolsDir = Join-Path $repoRoot "$targetDirName\$profileDir\tools"
$ffmpegSource = Join-Path $repoRoot "tools\ffmpeg\ffmpeg.exe"
$ytDlpSource = Join-Path $repoRoot "tools\yt-dlp\yt-dlp.exe"

$cargoArgs = @("build", "-p", "desktop")
if ($Release) {
    $cargoArgs += "--release"
}
if ($TorrentRqbit) {
    $cargoArgs += @("--features", "torrent-rqbit")
}

Write-Host "Building NebulaDM desktop app..."
$env:CARGO_TARGET_DIR = $targetDirName
$buildSucceeded = $false
try {
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
    $buildSucceeded = $true
}
finally {
    Remove-Item Env:\CARGO_TARGET_DIR -ErrorAction SilentlyContinue
}

if (-not $buildSucceeded) {
    exit 1
}

if (-not (Test-Path $binaryPath)) {
    throw "Desktop binary not found at $binaryPath"
}

New-Item -ItemType Directory -Path $runtimeToolsDir -Force | Out-Null

if (Test-Path $ffmpegSource) {
    Copy-Item -LiteralPath $ffmpegSource -Destination (Join-Path $runtimeToolsDir "ffmpeg.exe") -Force
    Write-Host "Bundled ffmpeg copied to $runtimeToolsDir"
}
else {
    Write-Warning "Bundled ffmpeg was not found at $ffmpegSource"
}

if (Test-Path $ytDlpSource) {
    Copy-Item -LiteralPath $ytDlpSource -Destination (Join-Path $runtimeToolsDir "yt-dlp.exe") -Force
    Write-Host "Bundled yt-dlp copied to $runtimeToolsDir"
}
else {
    Write-Warning "Bundled yt-dlp was not found at $ytDlpSource"
}

Write-Host ""
Write-Host "Desktop build ready:"
Write-Host "  $binaryPath"
