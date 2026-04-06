# NebulaDM Browser Extension

This folder contains a Chromium-compatible extension that captures downloads and forwards them to the local NebulaDM desktop bridge.

## What it does

- Listens to browser download attempts
- Sends the download metadata to `http://127.0.0.1:35791`
- Cancels the browser-managed download when the desktop app accepts the handoff
- Falls back to normal browser behavior if the desktop bridge is unavailable
- Tries to take over downloads as early as possible so NebulaDM becomes the effective download manager while the desktop app is running

## Load it in Chrome or Edge

1. Open `chrome://extensions` or `edge://extensions`.
2. Enable `Developer mode`.
3. Click `Load unpacked`.
4. Select the `extensions/browser` folder.

## Expected bridge payload

The extension posts JSON like this to the desktop app:

```json
{
  "file_name": "ubuntu.iso",
  "source": "https://example.com/ubuntu.iso",
  "kind": "Direct",
  "referrer": "https://example.com/downloads",
  "user_agent": "Mozilla/5.0 ...",
  "cookie_header": "session=abc123; download_token=xyz"
}
```

## Notes

- `kind` becomes `Torrent` automatically for `.torrent` files and `magnet:` links.
- The extension now forwards browser context headers so authenticated downloads have a better chance of working in the desktop app.
- Keep `captureEnabled` turned on in the popup if you want the browser to hand downloads off to NebulaDM instead of the browser download manager.
- The bridge URL can be changed from the popup if you move the desktop listener.
- Icons are not wired yet, so the unpacked extension can load without image assets first.
