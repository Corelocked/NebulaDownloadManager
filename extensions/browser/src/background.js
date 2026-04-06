const DEFAULT_SETTINGS = {
  bridgeUrl: "http://127.0.0.1:35791",
  captureEnabled: true,
  sendReferrer: false,
  sendUserAgent: false,
  sendCookies: false
};

const pendingCaptures = new Map();
const VIDEO_EXTENSIONS = [".mp4", ".m4v", ".webm", ".mov", ".mkv", ".avi"];
const mediaRequestsByTab = new Map();
const MEDIA_REQUEST_MAX_AGE_MS = 3 * 60 * 1000;
const MEDIA_REQUEST_LIMIT = 24;

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

function inferVideoFileName(video) {
  const candidates = [
    video?.downloadName,
    video?.fileName,
    video?.title,
    video?.url ? video.url.split("/").pop()?.split("?")[0] : null
  ];

  for (const candidate of candidates) {
    const value = (candidate || "").trim();
    if (!value) {
      continue;
    }

    const hasKnownExtension = VIDEO_EXTENSIONS.some((extension) =>
      value.toLowerCase().endsWith(extension)
    );
    if (hasKnownExtension) {
      return value;
    }
  }

  const sanitizedTitle = (video?.title || "video")
    .replace(/[<>:"/\\|?*\u0000-\u001f]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();

  return `${sanitizedTitle || "video"}.mp4`;
}

function isDownloadableVideoUrl(url) {
  return /^https?:/i.test(url || "");
}

function isLikelyYouTubeVideo(video, tabUrl) {
  const pageHost = String(video?.pageHost || "");
  const sourceUrl = String(video?.url || "");
  const tab = String(tabUrl || video?.pageUrl || "");
  return (
    /(^|\.)youtube\.com$/i.test(pageHost) ||
    /youtube\.com/i.test(tab) ||
    /googlevideo\.com/i.test(sourceUrl)
  );
}

function rememberMediaRequest(details) {
  if (details.tabId < 0 || !isDownloadableVideoUrl(details.url)) {
    return;
  }

  const mimeType = details.url.match(/[?&]mime=([^&]+)/i)?.[1] || "";
  const decodedMime = decodeURIComponent(mimeType || "");
  const candidate = {
    url: details.url,
    type: details.type || "",
    tabId: details.tabId,
    timeStamp: Date.now(),
    initiator: details.initiator || "",
    mimeType: decodedMime
  };

  const existing = mediaRequestsByTab.get(details.tabId) || [];
  const filtered = existing.filter((entry) => entry.url !== candidate.url);
  filtered.unshift(candidate);
  mediaRequestsByTab.set(details.tabId, filtered.slice(0, MEDIA_REQUEST_LIMIT));
}

function scoreVideoCandidate(candidate, tabUrl) {
  let score = 0;
  const url = candidate.url || "";
  const lowerUrl = url.toLowerCase();
  const mimeType = (candidate.mimeType || "").toLowerCase();
  const kind = candidate.kind || "";
  const hasAudio = candidate.hasAudio !== false;
  const contentLength = Number(candidate.contentLength || 0);

  if (!isDownloadableVideoUrl(url)) {
    return -1000;
  }

  score += 20;

  if (VIDEO_EXTENSIONS.some((extension) => lowerUrl.includes(extension))) {
    score += 25;
  }

  if (mimeType.includes("video/")) {
    score += 35;
  }

  if (mimeType.includes("audio/")) {
    score -= 50;
  }

  if (hasAudio) {
    score += 45;
  } else {
    score -= 80;
  }

  if (lowerUrl.includes("googlevideo.com")) {
    score += 25;
  }

  if (lowerUrl.includes("videoplayback")) {
    score += 30;
  }

  if (kind === "youtube-muxed") {
    score += 140;
  } else if (kind === "youtube-adaptive-video") {
    score -= 120;
  } else if (kind === "video-element") {
    score += 30;
  } else if (kind === "network-observed") {
    score += 20;
  }

  if (contentLength > 1024 * 1024) {
    score += 10;
  } else if (contentLength > 0 && contentLength < 128 * 1024) {
    score -= 80;
  }

  if (tabUrl && candidate.initiator && tabUrl.startsWith(candidate.initiator)) {
    score += 10;
  }

  const ageMs = Date.now() - (candidate.timeStamp || Date.now());
  score -= Math.min(30, ageMs / 10_000);

  return score;
}

function pickBestVideoCandidate(video, tabId, tabUrl) {
  const directCandidates = (video?.candidates || [])
    .filter((candidate) => isDownloadableVideoUrl(candidate?.url))
    .map((candidate) => ({
      url: candidate.url,
      mimeType: candidate.mimeType || "",
      kind: candidate.kind || "video-element",
      hasAudio: candidate.hasAudio !== false,
      contentLength: candidate.contentLength || null,
      initiator: tabUrl || "",
      timeStamp: Date.now()
    }));

  if (isDownloadableVideoUrl(video?.url)) {
    directCandidates.unshift({
      url: video.url,
      mimeType: video.mimeType || "",
      kind: "video-element",
      hasAudio: true,
      contentLength: null,
      initiator: tabUrl || "",
      timeStamp: Date.now()
    });
  }

  const observedCandidates = (mediaRequestsByTab.get(tabId) || [])
    .filter((candidate) => Date.now() - candidate.timeStamp <= MEDIA_REQUEST_MAX_AGE_MS)
    .map((candidate) => ({ ...candidate, kind: "network-observed" }));

  const combined = [...directCandidates, ...observedCandidates];
  if (!combined.length) {
    return null;
  }

  const ranked = combined
    .map((candidate) => ({
      ...candidate,
      score: scoreVideoCandidate(candidate, tabUrl)
    }))
    .sort((left, right) => right.score - left.score);

  if (isLikelyYouTubeVideo(video, tabUrl)) {
    const muxedCandidate = ranked.find((candidate) => candidate.kind === "youtube-muxed");
    if (muxedCandidate) {
      return muxedCandidate;
    }

    const safeCandidate = ranked.find(
      (candidate) => candidate.hasAudio !== false && candidate.score > 0
    );
    return safeCandidate || null;
  }

  return ranked[0];
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

async function resolveVideoDownload(video, tabId, tabUrl) {
  const bestCandidate = pickBestVideoCandidate(video, tabId, tabUrl);
  if (!bestCandidate) {
    throw new Error(
      "No downloadable video source was found. This page may be using protected or browser-only streaming."
    );
  }

  return {
    file_name: inferVideoFileName(video),
    source: bestCandidate.url,
    referrer: tabUrl || video.pageUrl || null
  };
}

async function captureVideoFromTab(video, tabId, tabUrl) {
  const settings = await getSettings();
  const resolved = await resolveVideoDownload(video, tabId, tabUrl);
  const sourceUrl = resolved.source || "";
  const requiresBrowserContext =
    /googlevideo\.com/i.test(sourceUrl) || /videoplayback/i.test(sourceUrl);
  const payload = {
    file_name: resolved.file_name,
    source: resolved.source,
    kind: "Direct",
    referrer:
      settings.sendReferrer || requiresBrowserContext ? resolved.referrer : null,
    user_agent:
      settings.sendUserAgent || requiresBrowserContext ? navigator.userAgent : null,
    cookie_header:
      settings.sendCookies && resolved.source.startsWith("http")
        ? await getCookieHeader(resolved.source)
        : null
  };

  return postCapture(payload);
}

async function triggerBrowserVideoDownload(video, tabId, tabUrl) {
  const resolved = await resolveVideoDownload(video, tabId, tabUrl);
  return chrome.downloads.download({
    url: resolved.source,
    filename: resolved.file_name,
    saveAs: false,
    conflictAction: "uniquify"
  });
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

chrome.webRequest.onBeforeRequest.addListener(
  (details) => {
    const lowerUrl = (details.url || "").toLowerCase();
    const looksLikeMedia =
      details.type === "media" ||
      lowerUrl.includes("mime=video") ||
      lowerUrl.includes("mime=audio") ||
      lowerUrl.includes("videoplayback") ||
      VIDEO_EXTENSIONS.some((extension) => lowerUrl.includes(extension));

    if (looksLikeMedia) {
      rememberMediaRequest(details);
    }
  },
  { urls: ["<all_urls>"] }
);

chrome.tabs.onRemoved.addListener((tabId) => {
  mediaRequestsByTab.delete(tabId);
});

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
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

  if (message?.type === "nebula:capture-video") {
    captureVideoFromTab(message.video, message.tabId ?? sender.tab?.id ?? -1, message.tabUrl)
      .then((result) => {
        if (!result.ok) {
          sendResponse({
            ok: false,
            error:
              result.reason === "capture disabled"
                ? "Browser capture is disabled in the extension settings."
                : "NebulaDM could not queue this video."
          });
          return;
        }

        sendResponse({
          ok: true,
          message: `Queued ${inferVideoFileName(message.video)} in NebulaDM without interrupting playback.`
        });
      })
      .catch((error) => sendResponse({ ok: false, error: String(error) }));
    return true;
  }

  if (message?.type === "nebula:download-video") {
    triggerBrowserVideoDownload(
      message.video,
      message.tabId ?? sender.tab?.id ?? -1,
      message.tabUrl
    )
      .then(() =>
        sendResponse({
          ok: true,
          message: `Started browser handoff for ${inferVideoFileName(message.video)}.`
        })
      )
      .catch((error) => sendResponse({ ok: false, error: String(error) }));
    return true;
  }

  return false;
});
