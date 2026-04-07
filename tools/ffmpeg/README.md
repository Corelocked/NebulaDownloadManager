Place a redistributed `ffmpeg.exe` binary in this folder as:

`tools/ffmpeg/ffmpeg.exe`

When present:

- `scripts/build-release.ps1` includes it in the portable package at `tools/ffmpeg.exe`
- `scripts/build-installer.ps1` includes it in the Windows installer at `{app}\tools\ffmpeg.exe`
- NebulaDM automatically looks for bundled ffmpeg next to the app, including `tools\ffmpeg.exe`

This keeps adaptive browser video downloads seamless for end users without requiring a separate ffmpeg install.
