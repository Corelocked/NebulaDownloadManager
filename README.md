# NebulaDM

NebulaDM is a Rust-first download manager workspace for building an IDM-style desktop app with torrent support, automatic file categorization, and a companion browser extension.

## Product direction

- Native Windows desktop executable with a GUI for active, queued, and completed downloads
- Direct HTTP/HTTPS downloads with resumable and segmented transfer support
- Torrent downloads in the same queue and visualization layer
- Browser extension that hands off captured downloads to the desktop app
- Automatic destination routing into folders like `Downloads/Videos`, `Downloads/Documents`, and `Downloads/Programs`

## Workspace layout

- `apps/desktop`: native GUI executable built in Rust
- `crates/core`: reusable download engine and destination-planning logic
- `crates/shared`: shared types for requests, queue items, and IPC payloads
- `extensions/browser`: Chrome/Edge extension that forwards captured downloads to the desktop app

## Run the desktop app

```powershell
cargo run -p desktop
```

To enable the real `librqbit` torrent engine path:

```powershell
cargo run -p desktop --features torrent-rqbit
```

## Build a Windows release folder

```powershell
pwsh -File .\scripts\build-release.ps1
```

To package the real torrent-enabled build too:

```powershell
pwsh -File .\scripts\build-release.ps1 -TorrentRqbit
```

This produces a timestamped folder like `dist/NebulaDM-win64-default-YYYYMMDD-HHMMSS/` or `dist/NebulaDM-win64-rqbit-YYYYMMDD-HHMMSS/` with:

- `NebulaDM.exe`
- `browser-extension/`
- `README.md`
- `SETUP.txt`

## Build a Windows installer

NebulaDM now includes a real Windows installer packaging path built around Inno Setup.

1. Open the desktop app and use `Setup Center -> Build Windows Installer` to generate `dist/installer/NebulaDM.iss`
2. Build the release payload:

```powershell
pwsh -File .\scripts\build-release.ps1
```

3. Compile the installer:

```powershell
pwsh -File .\scripts\build-installer.ps1
```

If `iscc` is not on your `PATH`, the script will leave the generated `.iss` file in `dist/installer/` and tell you how to compile it manually.

## Auto-update feed

NebulaDM now supports a manifest-driven update check from the desktop app.

- Set an update feed URL in `Setup Center`
- Click `Check For Updates`
- If a newer installer is available, NebulaDM downloads it into `%LOCALAPPDATA%\NebulaDM\updates\` and launches it

Use [update-feed.example.json](/d:/Projects/download-manager/assets/update-feed.example.json) as the schema reference for your hosted update manifest.

## Windows storage paths

- Queue state is stored in the per-user app-data folder for `NebulaDM`
- On Windows this typically resolves under `%LOCALAPPDATA%\\NebulaDM`
- The app falls back to the repo-local `data/` folder if a platform app-data directory is unavailable
- Downloads default into your user Downloads folder under `Downloads\\NebulaDM\\...`

## Current usability

- Direct downloads support resume, retry/backoff, segmented fresh downloads, and browser handoff context
- Torrent downloads can run through the simulated engine by default or the real `librqbit` path with `torrent-rqbit`
- The browser extension forwards the download URL by default and can optionally include referrer, user-agent, and cookies only when the user opts in
- The desktop app now includes a `Privacy Mode` toggle that defaults on and enforces auto-stop on torrent completion, no seeding, reduced local metadata retention, and a delete-history action
- This is still an active prototype, not a finished packaged IDM replacement yet

## First-run checklist

1. Build or package the desktop app.
2. Launch `NebulaDM.exe`.
3. Load `extensions/browser` or the packaged `dist/NebulaDM-win64-*/browser-extension` folder as an unpacked Chrome/Edge extension.
4. Start a direct download from the browser and verify it appears in the desktop queue.
5. For real magnet support, use the `torrent-rqbit` desktop build.

## Recommended implementation phases

1. Build the download engine for direct downloads with pause/resume and persistence.
2. Add a local IPC API so a browser extension can send captured downloads to the app.
3. Integrate torrent support through a Rust BitTorrent library.
4. Persist history, categories, and settings in SQLite.
5. Expand the desktop UI into a full queue, progress, speed, and file browser experience.

## Browser extension note

The main application and backend can be Rust, but Chromium/Firefox extensions still require standard web-extension code (typically TypeScript or JavaScript). The extension can stay very thin and simply forward metadata to the Rust desktop app.

## Real torrent engine path

The workspace currently includes a simulated torrent session flow for UI and queue integration.

- A feature-gated real adapter is prepared in `crates/core` behind `torrent-rqbit`
- The verified upstream API shape is based on `librqbit` session embedding
- When you are ready to wire the real engine, enable the feature and connect the adapter events to the existing torrent queue/session UI
