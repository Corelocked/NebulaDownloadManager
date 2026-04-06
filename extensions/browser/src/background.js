const DEFAULT_SETTINGS = {
  bridgeUrl: "http://127.0.0.1:35791",
  captureEnabled: true,
  sendReferrer: false,
  sendUserAgent: false,
  sendCookies: false
};

const pendingCaptures = new Map();

async function getSettings() {
  const stored = await chrome.storage.local.get(DEFAULT_SETTINGS);
  return {
    bridgeUrl: stored.bridgeUrl || DEFAULT_SETTINGS.bridgeUrl,
    captureEnabled: stored.captureEnabled !== false,
    sendReferrer: stored.sendReferrer === true,
    sendUserAgent: stored.sendUserAgent === true,
    sendCookies: stored.sendCookies === true
  };
}

async function getCookieHeader(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    if (!cookies.length) {
      return null;
    }
    return cookies.map((cookie) => `${cookie.name}=${cookie.value}`).join("; ");
  } catch (error) {
    console.warn("NebulaDM cookie lookup failed", error);
    return null;
  }
}

async function ensureCookiePermissions() {
  const granted = await chrome.permissions.request({
    permissions: ["cookies"],
    origins: ["<all_urls>"]
  });

  return granted;
}

function inferKind(filename, url) {
  const lowerName = (filename || "").toLowerCase();
  const lowerUrl = (url || "").toLowerCase();
  if (lowerName.endsWith(".torrent") || lowerUrl.startsWith("magnet:")) {
    return "Torrent";
  }
  return "Direct";
}

function inferFileName(item) {
  const source = item.finalUrl || item.url || "";
  const urlTail = source.split("/").pop();
  return item.filename || urlTail || "download.bin";
}

async function buildPayload(item, settings) {
  const source = item.finalUrl || item.url;
  return {
    file_name: inferFileName(item),
    source,
    kind: inferKind(item.filename, item.url),
    referrer: settings.sendReferrer ? item.referrer || null : null,
    user_agent: settings.sendUserAgent ? navigator.userAgent : null,
    cookie_header:
      settings.sendCookies && source.startsWith("http") ? await getCookieHeader(source) : null
  };
}

async function notifyCapture(message) {
  await chrome.notifications.create({
    type: "basic",
    title: "NebulaDM",
    message
  });
}

async function captureDownload(item, downloadId) {
  const existingCapture = pendingCaptures.get(downloadId);
  if (existingCapture) {
    return existingCapture;
  }

  const capturePromise = (async () => {
    const settings = await getSettings();
    const payload = await buildPayload(item, settings);
    const result = await postCapture(payload);
    if (result.ok) {
      try {
        await chrome.downloads.cancel(downloadId);
      } catch (error) {
        console.warn("NebulaDM could not cancel browser download", error);
      }

      try {
        await chrome.downloads.erase({ id: downloadId });
      } catch (error) {
        console.warn("NebulaDM could not erase browser download entry", error);
      }

      await notifyCapture(`Sent ${payload.file_name} to the desktop app`);
    }

    return result;
  })();

  pendingCaptures.set(downloadId, capturePromise);
  try {
    return await capturePromise;
  } finally {
    pendingCaptures.delete(downloadId);
  }
}

async function postCapture(payload) {
  const settings = await getSettings();
  if (!settings.captureEnabled) {
    return { ok: false, skipped: true, reason: "capture disabled" };
  }

  const response = await fetch(settings.bridgeUrl, {
    method: "POST",
    headers: {
      "Content-Type": "application/json"
    },
    body: JSON.stringify(payload)
  });

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Bridge returned ${response.status}: ${text}`);
  }

  return { ok: true };
}

chrome.runtime.onInstalled.addListener(async () => {
  await chrome.storage.local.set(DEFAULT_SETTINGS);
});

chrome.downloads.onCreated.addListener((item) => {
  (async () => {
    try {
      await captureDownload(item, item.id);
    } catch (error) {
      console.error("NebulaDM onCreated capture failed", error);
    }
  })();
});

chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  (async () => {
    try {
      const result = await captureDownload(item, item.id);

      if (result.ok) {
        suggest({ cancel: true });
        return;
      }

      suggest();
    } catch (error) {
      console.error("NebulaDM bridge failed", error);
      suggest();
    }
  })();

  return true;
});

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === "nebula:get-settings") {
    getSettings().then(sendResponse);
    return true;
  }

  if (message?.type === "nebula:save-settings") {
    (async () => {
      const nextSettings = {
        bridgeUrl: message.bridgeUrl,
        captureEnabled: Boolean(message.captureEnabled),
        sendReferrer: Boolean(message.sendReferrer),
        sendUserAgent: Boolean(message.sendUserAgent),
        sendCookies: Boolean(message.sendCookies)
      };

      if (nextSettings.sendCookies) {
        const granted = await ensureCookiePermissions();
        if (!granted) {
          sendResponse({
            ok: false,
            error: "Cookie access was not granted. Cookies remain disabled."
          });
          return;
        }
      }

      await chrome.storage.local.set(nextSettings);
      sendResponse({ ok: true });
    })()
      .catch((error) => sendResponse({ ok: false, error: String(error) }));
    return true;
  }

  return false;
});
