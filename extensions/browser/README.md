# NebulaDM Browser Extension

This folder contains a Chromium-compatible extension that captures downloads and forwards them to the local NebulaDM desktop bridge.

## What it does

- Listens to browser download attempts
- Sends the download metadata to `http://127.0.0.1:35791`
- Cancels the browser-managed download when the desktop app accepts the handoff
- Falls back to normal browser behavior if the desktop bridge is unavailable
- Tries to take over downloads as early as possible so NebulaDM becomes the effective download manager while the desktop app is running
- Uses privacy-first defaults: only the file name, source URL, and inferred kind are forwarded unless the user explicitly enables extra browser metadata

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
  "referrer": null,
  "user_agent": null,
  "cookie_header": null
}
```

## Notes

- `kind` becomes `Torrent` automatically for `.torrent` files and `magnet:` links.
- Referrer, user-agent, and cookie forwarding are opt-in from the popup.
- Enabling cookie forwarding triggers an extra browser permission prompt because it needs access to site cookies and origins.
- Authenticated downloads may need those opt-in settings if a site refuses plain URL downloads.
- Keep `captureEnabled` turned on in the popup if you want the browser to hand downloads off to NebulaDM instead of the browser download manager.
- The bridge URL can be changed from the popup if you move the desktop listener.
- Icons are not wired yet, so the unpacked extension can load without image assets first.
