const OVERLAY_ID = "nebula-video-overlay";
const STATE_ID = "nebula-video-overlay-state";

let activeVideo = null;
let refreshScheduled = false;
let cachedYouTubeCandidates = [];
let cachedYouTubeCandidateUrl = "";
let cachedYouTubeCandidateTime = 0;

function ensureOverlay() {
  let overlay = document.getElementById(OVERLAY_ID);
  if (overlay) {
    return overlay;
  }

  overlay = document.createElement("div");
  overlay.id = OVERLAY_ID;
  overlay.innerHTML = `
    <div class="nebula-card">
      <div class="nebula-title">NebulaDM</div>
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
    if (!button || !activeVideo) {
      return;
    }

    const action = button.getAttribute("data-action");
    const state = document.getElementById(STATE_ID);
    state.textContent =
      action === "download" ? "Starting NebulaDM handoff..." : "Queueing without interrupting playback...";

    try {
      const response = await chrome.runtime.sendMessage({
        type: action === "download" ? "nebula:download-video" : "nebula:capture-video",
        tabId: activeVideo.tabId,
        tabUrl: activeVideo.pageUrl,
        video: activeVideo
      });
      state.textContent = response.ok ? response.message : `Failed: ${response.error}`;
    } catch (error) {
      state.textContent = `Failed: ${error}`;
    }
  });

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

  return new Promise((resolve) => {
    const requestId = `nebula-yt-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const onMessage = (event) => {
      if (event.source !== window || event.data?.type !== requestId) {
        return;
      }

      window.removeEventListener("message", onMessage);
      resolve(Array.isArray(event.data.candidates) ? event.data.candidates : []);
    };

    window.addEventListener("message", onMessage);
    const script = document.createElement("script");
    script.textContent = `
      (() => {
        const response = window.ytInitialPlayerResponse || window.ytplayer?.config?.args?.player_response;
        let playerData = response;
        if (typeof playerData === "string") {
          try { playerData = JSON.parse(playerData); } catch (_error) {}
        }
        const streamingData = playerData?.streamingData || {};
        const formats = Array.isArray(streamingData.formats) ? streamingData.formats : [];
        const adaptiveFormats = Array.isArray(streamingData.adaptiveFormats) ? streamingData.adaptiveFormats : [];
        const candidates = formats
          .map((item) => ({
            url: item.url || null,
            mimeType: item.mimeType || "",
            kind: "youtube-muxed"
          }))
          .concat(
            adaptiveFormats
              .filter((item) => String(item.mimeType || "").startsWith("video/"))
              .map((item) => ({
                url: item.url || null,
                mimeType: item.mimeType || "",
                kind: "youtube-adaptive-video"
              }))
          )
          .filter((item) => item.url);
        window.postMessage({ type: "${requestId}", candidates }, "*");
      })();
    `;
    (document.head || document.documentElement).appendChild(script);
    script.remove();

    window.setTimeout(() => {
      window.removeEventListener("message", onMessage);
      resolve([]);
    }, 700);
  });
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
    candidates: pageCandidates
  };
}

function positionOverlay(video) {
  const overlay = ensureOverlay();
  const rect = video.getBoundingClientRect();
  const overlayWidth = 210;
  overlay.style.left = `${Math.max(12, rect.right - overlayWidth - 10)}px`;
  overlay.style.top = `${Math.max(12, rect.top + 12)}px`;
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
  document.getElementById(STATE_ID).textContent = "Download takes over. Queue keeps playback here.";
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
scheduleRefresh();
