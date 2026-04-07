# NebulaDM v0.1.3

## Highlights

- Introduces a polished Windows desktop download manager UI with separate direct and torrent workspaces.
- Adds tray-first behavior, startup-in-background support, and a browser-capture confirmation flow with per-download save location selection.
- Ships a privacy-first foundation with Privacy Mode, reduced browser metadata retention, and safer torrent defaults.
- Moves browser video capture into the extension popup, improves YouTube handoff handling, and improves desktop scrolling and browser-capture filename handling.

## Included in this release

- Direct downloads with resume, retry, chunked transfers, and browser handoff support.
- Magnet registration and integrated torrent handling inside the desktop app.
- Native Windows tray icon, close-to-tray behavior, and startup registration support.
- Native Windows toast notifications for download events.
- Real Windows installer packaging through Inno Setup.
- Public release assets for both installer and portable zip distribution.
- Popup-first browser video capture with clearer `Download Video` and `Queue Only` actions.
- Cleaner browser-capture confirmation windows with compact metadata display.
- Fixed browser-captured video jobs that previously failed from invalid long YouTube-style filenames.
- Bundled `ffmpeg` and `yt-dlp` support for smoother YouTube downloads without separate tool installs.
- Main desktop content now scrolls when the page grows taller than the window.
- Fixed restoring the desktop app from the Windows system tray.

## Downloads

- Installer: NebulaDM-Setup.exe
- Portable build: NebulaDM-win64-default-v0.1.3.zip

## Install / Update

1. Download NebulaDM-Setup.exe from this release.
2. Run the installer and complete setup.
3. Launch NebulaDM.
4. Load the unpacked browser extension from the bundled browser-extension folder if you want browser handoff.

## Notes

- The browser extension is currently distributed as unpacked for Chrome/Edge.
- Reload the unpacked browser extension after updating so the popup-only capture flow and YouTube handoff changes take effect.
- Restart the desktop app after updating so the tray restore fix, popup capture behavior, and bundled media-tool support take effect.
- Windows may still show SmartScreen warnings for unsigned builds.
- The updater expects a hosted manifest that points to the uploaded GitHub Release installer asset.
