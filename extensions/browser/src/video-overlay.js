const OVERLAY_ID = "nebula-video-overlay";
const STATE_ID = "nebula-video-overlay-state";
const POSITION_STORAGE_KEY = "nebulaOverlayPosition";

let activeVideo = null;
let refreshScheduled = false;
let cachedYouTubeCandidates = [];
let cachedYouTubeCandidateUrl = "";
let cachedYouTubeCandidateTime = 0;
let overlayPosition = null;
let overlayPositionLoaded = false;
let dragState = null;
let lastActionAt = 0;

function clampOverlayPosition(position) {
  const maxLeft = Math.max(12, window.innerWidth - 222);
  const maxTop = Math.max(12, window.innerHeight - 140);
  return {
    left: Math.min(Math.max(12, Math.round(position.left || 12)), maxLeft),
    top: Math.min(Math.max(12, Math.round(position.top || 12)), maxTop),
    pinned: true
  };
}

async function loadOverlayPosition() {
  if (overlayPositionLoaded) {
    return overlayPosition;
  }

  overlayPositionLoaded = true;
  try {
    const stored = await chrome.storage.local.get(POSITION_STORAGE_KEY);
    if (stored?.[POSITION_STORAGE_KEY]) {
      overlayPosition = clampOverlayPosition(stored[POSITION_STORAGE_KEY]);
    }
  } catch (error) {
    console.warn("NebulaDM could not load overlay position", error);
  }

  return overlayPosition;
}

function saveOverlayPosition() {
  if (!overlayPosition?.pinned) {
    return;
  }

  chrome.storage.local
    .set({ [POSITION_STORAGE_KEY]: clampOverlayPosition(overlayPosition) })
    .catch((error) => console.warn("NebulaDM could not save overlay position", error));
}

function ensureOverlay() {
  let overlay = document.getElementById(OVERLAY_ID);
  if (overlay) {
    return overlay;
  }

  overlay = document.createElement("div");
  overlay.id = OVERLAY_ID;
  overlay.innerHTML = `
    <div class="nebula-card">
      <div class="nebula-title" data-drag-handle="true">NebulaDM</div>
      <div class="nebula-actions">
        <button type="button" data-action="download">Download</button>
        <button type="button" data-action="queue" class="secondary">Queue Only</button>
      </div>
      <div id="${STATE_ID}" class="nebula-state">Queue keeps playback in the browser.</div>
    </div>
  `;

  const style = document.createElement("style");
  style.textContent = `
    #${OVERLAY_ID} {
      position: fixed;
      z-index: 2147483647;
      display: none;
      pointer-events: auto;
      font-family: "Segoe UI", sans-serif;
    }
    #${OVERLAY_ID} .nebula-card {
      min-width: 180px;
      max-width: 210px;
      padding: 8px;
      border-radius: 12px;
      background: rgba(12, 18, 28, 0.92);
      color: #f8fafc;
      box-shadow: 0 12px 32px rgba(0, 0, 0, 0.3);
      border: 1px solid rgba(45, 212, 191, 0.22);
      backdrop-filter: blur(10px);
    }
    #${OVERLAY_ID} .nebula-title {
      font-size: 11px;
      font-weight: 700;
      letter-spacing: 0.06em;
      text-transform: uppercase;
      color: #7dd3fc;
      cursor: move;
      user-select: none;
      touch-action: none;
      margin-bottom: 6px;
    }
    #${OVERLAY_ID} .nebula-actions {
      display: flex;
      gap: 6px;
    }
    #${OVERLAY_ID} button {
      flex: 1;
      border: 0;
      border-radius: 9px;
      padding: 8px 9px;
      font-size: 11px;
      font-weight: 700;
      cursor: pointer;
      background: #22d3ee;
      color: #082f49;
    }
    #${OVERLAY_ID} button.secondary {
      background: #dbeafe;
      color: #1e3a8a;
    }
    #${OVERLAY_ID} .nebula-state {
      margin-top: 6px;
      font-size: 10px;
      color: #cbd5e1;
      line-height: 1.35;
    }
  `;

  overlay.appendChild(style);
  document.documentElement.appendChild(overlay);

  overlay.addEventListener("click", async (event) => {
    const button = event.target.closest("button[data-action]");
    const state = document.getElementById(STATE_ID);
    if (!button) {
      return;
    }

    if (!activeVideo) {
      state.textContent = "No active video payload is ready yet. Reload the page and try again.";
      return;
    }

    const action = button.getAttribute("data-action");
    lastActionAt = Date.now();
    state.textContent =
      action === "download" ? "Starting NebulaDM handoff..." : "Queueing without interrupting playback...";

    try {
      const response = await chrome.runtime.sendMessage({
        type: action === "download" ? "nebula:download-video" : "nebula:capture-video",
        tabId: activeVideo.tabId,
        tabUrl: activeVideo.pageUrl,
        video: activeVideo
      });
      if (!response) {
        state.textContent = "No response from extension background worker. Reload the extension and this page.";
        return;
      }
      state.textContent = response.ok ? response.message : `Failed: ${response.error}`;
    } catch (error) {
      state.textContent = `Failed: ${error}`;
    }
  });

  overlay.addEventListener("pointerdown", (event) => {
    const handle = event.target.closest("[data-drag-handle]");
    if (!handle) {
      return;
    }

    dragState = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      originLeft: overlayPosition?.left ?? (parseFloat(overlay.style.left) || 12),
      originTop: overlayPosition?.top ?? (parseFloat(overlay.style.top) || 12)
    };
    overlay.setPointerCapture(event.pointerId);
    event.preventDefault();
  });

  overlay.addEventListener("pointermove", (event) => {
    if (!dragState || dragState.pointerId !== event.pointerId) {
      return;
    }

    overlayPosition = clampOverlayPosition({
      left: dragState.originLeft + (event.clientX - dragState.startX),
      top: dragState.originTop + (event.clientY - dragState.startY)
    });
    overlay.style.left = `${overlayPosition.left}px`;
    overlay.style.top = `${overlayPosition.top}px`;
  });

  const stopDragging = (event) => {
    if (!dragState || dragState.pointerId !== event.pointerId) {
      return;
    }

    if (overlay.hasPointerCapture(event.pointerId)) {
      overlay.releasePointerCapture(event.pointerId);
    }
    dragState = null;
    saveOverlayPosition();
  };

  overlay.addEventListener("pointerup", stopDragging);
  overlay.addEventListener("pointercancel", stopDragging);

  return overlay;
}

function isVisibleVideo(video) {
  const rect = video.getBoundingClientRect();
  const style = window.getComputedStyle(video);
  return (
    rect.width >= 240 &&
    rect.height >= 135 &&
    rect.bottom > 0 &&
    rect.right > 0 &&
    rect.top < window.innerHeight &&
    rect.left < window.innerWidth &&
    style.visibility !== "hidden" &&
    style.display !== "none"
  );
}

function inferVideoFileNameFromUrl(url) {
  return url.split("/").pop()?.split("?")[0] || "";
}

function fetchYouTubeCandidates() {
  if (!/youtube\.com$/i.test(location.hostname) && !/youtube\.com$/i.test(location.hostname.replace(/^www\./, ""))) {
    return Promise.resolve([]);
  }

  const playerSources = [];
  const scripts = Array.from(document.scripts);
  const playerPatterns = [
    /ytInitialPlayerResponse\s*=\s*(\{.+?\})\s*;\s*(?:var\s+meta|var\s+playerResponse|<\/script>)/s,
    /["']PLAYER_VARS["']\s*:\s*(\{.+?\})\s*,\s*["']PLAYER_JS_URL["']/s,
    /["']player_response["']\s*:\s*"(.+?)"/s
  ];

  for (const script of scripts) {
    const text = script.textContent || "";
    if (!text.includes("ytInitialPlayerResponse") && !text.includes("player_response")) {
      continue;
    }

    for (const pattern of playerPatterns) {
      const match = text.match(pattern);
      if (match?.[1]) {
        playerSources.push(match[1]);
      }
    }
  }

  const decodePlayerSource = (value) => {
    if (!value) {
      return null;
    }

    let parsed = value;
    if (typeof parsed === "string") {
      try {
        parsed = JSON.parse(parsed);
      } catch (_error) {
        try {
          parsed = JSON.parse(parsed.replace(/\\"/g, '"').replace(/\\\\/g, "\\"));
        } catch (_secondError) {
          return null;
        }
      }
    }

    if (parsed?.streamingData) {
      return parsed;
    }

    if (parsed?.player_response) {
      return decodePlayerSource(parsed.player_response);
    }

    return null;
  };

    const mapCandidate = (item, kind, hasAudio) => ({
      url: item.url || null,
      mimeType: item.mimeType || "",
      kind,
    hasAudio,
    qualityLabel: item.qualityLabel || item.quality || "",
    contentLength: item.contentLength || null,
    itag: item.itag || null
  });

  const parsedPlayers = playerSources
    .map(decodePlayerSource)
    .filter((player) => player?.streamingData);

  const candidateMap = new Map();
  for (const playerData of parsedPlayers) {
    const streamingData = playerData.streamingData || {};
    const formats = Array.isArray(streamingData.formats) ? streamingData.formats : [];
    const adaptiveFormats = Array.isArray(streamingData.adaptiveFormats) ? streamingData.adaptiveFormats : [];
    for (const item of formats.map((entry) => mapCandidate(entry, "youtube-muxed", true))) {
      if (item.url) {
        candidateMap.set(item.url, item);
      }
    }
    for (const item of adaptiveFormats
      .filter((entry) => String(entry.mimeType || "").startsWith("video/"))
      .map((entry) => mapCandidate(entry, "youtube-adaptive-video", false))) {
      if (item.url && !candidateMap.has(item.url)) {
        candidateMap.set(item.url, item);
      }
    }
    for (const item of adaptiveFormats
      .filter((entry) => String(entry.mimeType || "").startsWith("audio/"))
      .map((entry) => mapCandidate(entry, "youtube-adaptive-audio", true))) {
      if (item.url && !candidateMap.has(item.url)) {
        candidateMap.set(item.url, item);
      }
    }
  }

  return Promise.resolve(Array.from(candidateMap.values()));
}

async function buildVideoPayload(video) {
  const currentSrc = video.currentSrc || video.src || "";
  const needsYouTubeCandidates =
    !/^https?:/i.test(currentSrc) && /(^|\.)youtube\.com$/i.test(location.hostname);
  let pageCandidates = [];

  if (needsYouTubeCandidates) {
    const now = Date.now();
    if (cachedYouTubeCandidateUrl === location.href && now - cachedYouTubeCandidateTime < 5_000) {
      pageCandidates = cachedYouTubeCandidates;
    } else {
      pageCandidates = await fetchYouTubeCandidates();
      cachedYouTubeCandidates = pageCandidates;
      cachedYouTubeCandidateUrl = location.href;
      cachedYouTubeCandidateTime = now;
    }
  }

  return {
    url: currentSrc,
    fileName: inferVideoFileNameFromUrl(currentSrc),
    title: document.title || "Video",
    pageUrl: location.href,
    mimeType: video.getAttribute("type") || "",
    pageHost: location.hostname,
    candidates: pageCandidates
  };
}

function positionOverlay(video) {
  const overlay = ensureOverlay();
  if (overlayPosition?.pinned) {
    const position = clampOverlayPosition(overlayPosition);
    overlay.style.left = `${position.left}px`;
    overlay.style.top = `${position.top}px`;
  } else {
    const rect = video.getBoundingClientRect();
    const overlayWidth = 210;
    overlay.style.left = `${Math.max(12, rect.right - overlayWidth - 10)}px`;
    overlay.style.top = `${Math.max(12, rect.top + 12)}px`;
  }
  overlay.style.display = "block";
}

async function refreshOverlay() {
  refreshScheduled = false;

  const overlay = ensureOverlay();
  const videos = Array.from(document.querySelectorAll("video")).filter(isVisibleVideo);
  if (!videos.length) {
    overlay.style.display = "none";
    activeVideo = null;
    return;
  }

  const bestVideo = videos
    .map((video) => ({
      video,
      area: video.getBoundingClientRect().width * video.getBoundingClientRect().height,
      paused: video.paused
    }))
    .sort((left, right) => {
      if (left.paused !== right.paused) {
        return left.paused ? 1 : -1;
      }
      return right.area - left.area;
    })[0]?.video;

  if (!bestVideo) {
    overlay.style.display = "none";
    activeVideo = null;
    return;
  }

  positionOverlay(bestVideo);
  const payload = await buildVideoPayload(bestVideo);
  activeVideo = {
    ...payload,
    pageUrl: location.href
  };
  const state = document.getElementById(STATE_ID);
  if (Date.now() - lastActionAt > 1500) {
    state.textContent = "Download takes over. Queue keeps playback here.";
  }
}

function scheduleRefresh() {
  if (refreshScheduled) {
    return;
  }

  refreshScheduled = true;
  window.requestAnimationFrame(() => {
    refreshOverlay().catch((error) => {
      const overlay = ensureOverlay();
      overlay.style.display = "none";
      activeVideo = null;
      console.error("NebulaDM overlay refresh failed", error);
    });
  });
}

new MutationObserver(scheduleRefresh).observe(document.documentElement, {
  childList: true,
  subtree: true,
  attributes: true,
  attributeFilter: ["src", "style", "class"]
});

window.addEventListener("scroll", scheduleRefresh, true);
window.addEventListener("resize", scheduleRefresh);
window.setInterval(scheduleRefresh, 1500);
loadOverlayPosition().finally(scheduleRefresh);
