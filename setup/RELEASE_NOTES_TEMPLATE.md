# NebulaDM {{TAG}}

## Highlights

- Introduces a polished Windows desktop download manager UI with separate direct and torrent workspaces.
- Adds tray-first behavior, startup-in-background support, and a browser-capture confirmation flow with per-download save location selection.
- Ships a privacy-first foundation with Privacy Mode, reduced browser metadata retention, and safer torrent defaults.
- Tightens the browser video overlay, clarifies the capture actions, and improves desktop scrolling and browser-capture filename handling.

## Included in this release

- Direct downloads with resume, retry, chunked transfers, and browser handoff support.
- Magnet registration and integrated torrent handling inside the desktop app.
- Native Windows tray icon, close-to-tray behavior, and startup registration support.
- Native Windows toast notifications for download events.
- Real Windows installer packaging through Inno Setup.
- Public release assets for both installer and portable zip distribution.
- Smaller in-page video capture UI with clearer `Download` and `Queue Only` actions.
- Cleaner browser-capture confirmation windows with compact metadata display.
- Fixed browser-captured video jobs that previously failed from invalid long YouTube-style filenames.
- Main desktop content now scrolls when the page grows taller than the window.
- Fixed restoring the desktop app from the Windows system tray.

## Downloads

- Installer: {{INSTALLER}}
- Portable build: {{PORTABLE_ZIP}}

## Install / Update

1. Download {{INSTALLER}} from this release.
2. Run the installer and complete setup.
3. Launch NebulaDM.
4. Load the unpacked browser extension from the bundled rowser-extension folder if you want browser handoff.

## Notes

- The browser extension is currently distributed as unpacked for Chrome/Edge.
- Reload the unpacked browser extension after updating if you want the refreshed overlay labels and sizing.
- Restart the desktop app after updating so the browser-capture filename fix, tray restore fix, and scrollable main view take effect.
- Windows may still show SmartScreen warnings for unsigned builds.
- The updater expects a hosted manifest that points to the uploaded GitHub Release installer asset.
