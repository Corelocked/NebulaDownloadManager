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
