# NebulaDM v0.1.1

## Highlights

- Introduces a polished Windows desktop download manager UI with separate direct and torrent workspaces.
- Adds tray-first behavior, startup-in-background support, and a browser-capture confirmation popup with per-download save location selection.
- Ships a privacy-first foundation with Privacy Mode, reduced browser metadata retention, and safer torrent defaults.

## Included in this release

- Direct downloads with resume, retry, chunked transfers, and browser handoff support.
- Magnet registration and integrated torrent handling inside the desktop app.
- Native Windows tray icon, close-to-tray behavior, and startup registration support.
- Native Windows toast notifications for download events.
- Real Windows installer packaging through Inno Setup.
- Public release assets for both installer and portable zip distribution.

## Downloads

- Installer: NebulaDM-Setup.exe
- Portable build: NebulaDM-win64-default-v0.1.1.zip

## Install / Update

1. Download NebulaDM-Setup.exe from this release.
2. Run the installer and complete setup.
3. Launch NebulaDM.
4. Load the unpacked browser extension from the bundled rowser-extension folder if you want browser handoff.

## Notes

- The browser extension is currently distributed as unpacked for Chrome/Edge.
- Windows may still show SmartScreen warnings for unsigned builds.
- The updater expects a hosted manifest that points to the uploaded GitHub Release installer asset.

