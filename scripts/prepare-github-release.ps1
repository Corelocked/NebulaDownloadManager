param(
    [Parameter(Mandatory = $true)]
    [string]$Tag,
    [Parameter(Mandatory = $true)]
    [string]$Repo,
    [switch]$TorrentRqbit
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$buildReleaseScript = Join-Path $PSScriptRoot "build-release.ps1"
$buildInstallerScript = Join-Path $PSScriptRoot "build-installer.ps1"
$releaseNotesTemplate = Join-Path $repoRoot "setup\RELEASE_NOTES_TEMPLATE.md"
$releaseNotesPath = Join-Path $repoRoot "setup\RELEASE_NOTES.md"
$manifestPath = Join-Path $repoRoot "setup\update-feed.json"
$setupDir = Join-Path $repoRoot "setup"
$distRoot = Join-Path $repoRoot "dist"
$appVersion = (Get-Content (Join-Path $repoRoot "apps\desktop\Cargo.toml") | Where-Object { $_ -match '^version = ' } | Select-Object -First 1)
$appVersion = ($appVersion -replace 'version = "', '') -replace '"', ''
$normalizedTag = $Tag.Trim()
$tagVersion = $normalizedTag.TrimStart("v")
$packageVariant = if ($TorrentRqbit) { "rqbit" } else { "default" }
$portableZipName = "NebulaDM-win64-$packageVariant-v$appVersion.zip"
$portableZipPath = Join-Path $distRoot $portableZipName
$installerPath = Join-Path $setupDir "NebulaDM-Setup.exe"
$installerUrl = "https://github.com/$Repo/releases/download/$normalizedTag/NebulaDM-Setup.exe"
$notesUrl = "https://github.com/$Repo/releases/tag/$normalizedTag"
$defaultReleaseNotesTemplate = @"
# NebulaDM {{TAG}}

## Highlights

- Introduces a polished Windows desktop download manager UI with separate direct and torrent workspaces.
- Adds tray-first behavior, startup-in-background support, and a browser-capture confirmation popup with per-download save location selection.
- Ships a privacy-first foundation with `Privacy Mode`, reduced browser metadata retention, and safer torrent defaults.

## Included in this release

- Direct downloads with resume, retry, chunked transfers, and browser handoff support.
- Magnet registration and integrated torrent handling inside the desktop app.
- Native Windows tray icon, close-to-tray behavior, and startup registration support.
- Native Windows toast notifications for download events.
- Real Windows installer packaging through Inno Setup.
- Public release assets for both installer and portable zip distribution.

## Downloads

- Installer: `{{INSTALLER}}`
- Portable build: `{{PORTABLE_ZIP}}`

## Install / Update

1. Download `{{INSTALLER}}` from this release.
2. Run the installer and complete setup.
3. Launch NebulaDM.
4. Load the unpacked browser extension from the bundled `browser-extension` folder if you want browser handoff.

## Notes

- The browser extension is currently distributed as unpacked for Chrome/Edge.
- Windows may still show SmartScreen warnings for unsigned builds.
- The updater expects a hosted manifest that points to the uploaded GitHub Release installer asset.
"@

if ($tagVersion -ne $appVersion) {
    throw "Tag version '$normalizedTag' does not match apps/desktop version '$appVersion'."
}

if ($TorrentRqbit) {
    & $buildReleaseScript -TorrentRqbit
    & $buildInstallerScript -TorrentRqbit
}
else {
    & $buildReleaseScript
    & $buildInstallerScript
}

if (-not (Test-Path $installerPath)) {
    throw "Installer not found at $installerPath"
}

if (-not (Test-Path $portableZipPath)) {
    throw "Portable zip not found at $portableZipPath"
}

$manifest = [ordered]@{
    version = $appVersion
    installer_url = $installerUrl
    notes_url = $notesUrl
}
$manifest | ConvertTo-Json | Set-Content -LiteralPath $manifestPath

if (-not (Test-Path $releaseNotesTemplate)) {
    $defaultReleaseNotesTemplate | Set-Content -LiteralPath $releaseNotesTemplate
}

$releaseNotes = Get-Content $releaseNotesTemplate -Raw
$releaseNotes = $releaseNotes.Replace("{{VERSION}}", $appVersion)
$releaseNotes = $releaseNotes.Replace("{{TAG}}", $normalizedTag)
$releaseNotes = $releaseNotes.Replace("{{INSTALLER}}", "NebulaDM-Setup.exe")
$releaseNotes = $releaseNotes.Replace("{{PORTABLE_ZIP}}", $portableZipName)
$releaseNotes | Set-Content -LiteralPath $releaseNotesPath

Write-Host ""
Write-Host "GitHub release assets ready:"
Write-Host "  Installer: $installerPath"
Write-Host "  Portable zip: $portableZipPath"
Write-Host "  Update manifest: $manifestPath"
Write-Host "  Release notes draft: $releaseNotesPath"
Write-Host ""
Write-Host "Release URLs:"
Write-Host "  Installer URL: $installerUrl"
Write-Host "  Notes URL: $notesUrl"
