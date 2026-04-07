Place a redistributed `yt-dlp.exe` binary in this folder as:

`tools/yt-dlp/yt-dlp.exe`

When present:

- `scripts/build-release.ps1` includes it in the portable package at `tools/yt-dlp.exe`
- `scripts/build-installer.ps1` includes it in the Windows installer at `{app}\tools\yt-dlp.exe`
- NebulaDM automatically looks for bundled yt-dlp next to the app, including `tools\yt-dlp.exe`

NebulaDM uses bundled yt-dlp for popup-triggered YouTube downloads so users do not need to install yt-dlp separately.
