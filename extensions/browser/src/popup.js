const bridgeUrlInput = document.getElementById("bridgeUrl");
const captureEnabledInput = document.getElementById("captureEnabled");
const sendReferrerInput = document.getElementById("sendReferrer");
const sendUserAgentInput = document.getElementById("sendUserAgent");
const sendCookiesInput = document.getElementById("sendCookies");
const enableCookiesButton = document.getElementById("enableCookiesButton");
const videoPanel = document.getElementById("videoPanel");
const videoTitle = document.getElementById("videoTitle");
const videoSource = document.getElementById("videoSource");
const downloadVideoButton = document.getElementById("downloadVideoButton");
const queueVideoButton = document.getElementById("queueVideoButton");
const saveButton = document.getElementById("saveButton");
const statusText = document.getElementById("statusText");

let detectedVideo = null;
let activeTab = null;

const YT_DLP_SITE_PATTERNS = [
  /(^|\.)youtube\.com$/i,
  /(^|\.)youtu\.be$/i,
  /(^|\.)facebook\.com$/i,
  /(^|\.)fb\.watch$/i,
  /(^|\.)instagram\.com$/i,
  /(^|\.)tiktok\.com$/i,
  /(^|\.)twitter\.com$/i,
  /(^|\.)x\.com$/i,
  /(^|\.)vimeo\.com$/i,
  /(^|\.)dailymotion\.com$/i
];

function setStatus(message) {
  statusText.textContent = message;
}

function truncateMiddle(value, maxLength = 88) {
  if (!value || value.length <= maxLength) {
    return value || "";
  }

  const head = Math.floor((maxLength - 3) / 2);
  const tail = maxLength - 3 - head;
  return `${value.slice(0, head)}...${value.slice(-tail)}`;
}

function inferVideoLabel(video) {
  return video?.title || video?.fileName || "Detected video";
}

function shouldPreferYtDlpForUrl(url) {
  try {
    const parsed = new URL(url);
    return YT_DLP_SITE_PATTERNS.some((pattern) => pattern.test(parsed.hostname) || pattern.test(url));
  } catch (_error) {
    return false;
  }
}

function normalizeDetectedVideo(video, tab) {
  if (!video) {
    return null;
  }

  const normalized = {
    ...video,
    pageUrl: video.pageUrl || tab?.url || "",
    pageHost: video.pageHost || (() => {
      try {
        return new URL(video.pageUrl || tab?.url || "").hostname;
      } catch (_error) {
        return "";
      }
    })()
  };

  if (shouldPreferYtDlpForUrl(normalized.pageUrl || normalized.url || "")) {
    normalized.useYtDlp = true;
    normalized.url = normalized.pageUrl || normalized.url;
  }

  return normalized;
}

function renderVideoPanel(video) {
  if (!video?.url) {
    videoPanel.hidden = true;
    detectedVideo = null;
    return;
  }

  detectedVideo = video;
  videoPanel.hidden = false;
  videoTitle.textContent = inferVideoLabel(video);
  videoSource.textContent = truncateMiddle(video.url);
}

async function detectVideoInActiveTab() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  activeTab = tab || null;

  if (!tab?.id || !tab.url || !/^https?:/i.test(tab.url)) {
    renderVideoPanel(null);
    setStatus("Open a regular web page with a playing video to enable quick capture.");
    return;
  }

  const frameResults = await chrome.scripting.executeScript({
    target: { tabId: tab.id, allFrames: true },
    func: () => {
      const isYouTube = /(^|\.)youtube\.com$/i.test(location.hostname);
      const mapCandidate = (item, kind, hasAudio) => ({
        url: item.url || null,
        mimeType: item.mimeType || "",
        kind,
        hasAudio,
        qualityLabel: item.qualityLabel || item.quality || "",
        contentLength: item.contentLength || null,
        itag: item.itag || null
      });

      const fetchYouTubeCandidates = () => {
        const playerSources = [];
        const playerPatterns = [
          /ytInitialPlayerResponse\s*=\s*(\{.+?\})\s*;\s*(?:var\s+meta|var\s+playerResponse|<\/script>)/s,
          /["']PLAYER_VARS["']\s*:\s*(\{.+?\})\s*,\s*["']PLAYER_JS_URL["']/s,
          /["']player_response["']\s*:\s*"(.+?)"/s
        ];

        for (const script of Array.from(document.scripts)) {
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
              } catch (_ignored) {
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

        const candidateMap = new Map();
        for (const playerData of playerSources.map(decodePlayerSource).filter(Boolean)) {
          const streamingData = playerData.streamingData || {};
          const formats = Array.isArray(streamingData.formats) ? streamingData.formats : [];
          const adaptiveFormats = Array.isArray(streamingData.adaptiveFormats)
            ? streamingData.adaptiveFormats
            : [];
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
        return Array.from(candidateMap.values());
      };

      if (isYouTube) {
        const candidates = fetchYouTubeCandidates();
        return {
          url: window.location.href,
          fileName: "",
          title: document.title || "YouTube Video",
          pageUrl: window.location.href,
          pageHost: window.location.hostname,
          poster: null,
          candidates,
          useYtDlp: true
        };
      }

      const videos = Array.from(document.querySelectorAll("video"));
      const ranked = videos
        .map((video) => {
          const rect = video.getBoundingClientRect();
          return {
            element: video,
            area: Math.max(rect.width, 0) * Math.max(rect.height, 0),
            currentSrc: video.currentSrc || video.src || "",
            poster: video.poster || "",
            paused: video.paused
          };
        })
        .filter((video) => Boolean(video.currentSrc))
        .sort((left, right) => {
          if (left.paused !== right.paused) {
            return left.paused ? 1 : -1;
          }
          return right.area - left.area;
        });

      const best = ranked[0];
      if (!best) {
        return null;
      }

      const sourceUrl = best.currentSrc;
      const fileName = sourceUrl.split("/").pop()?.split("?")[0] || "";
      return {
        url: sourceUrl,
        fileName,
        title: document.title || fileName || "Video",
        pageUrl: window.location.href,
        pageHost: window.location.hostname,
        poster: best.poster || null,
        candidates: [],
        useYtDlp: false
      };
    }
  });

  const scriptCandidates = frameResults
    .map((entry) => entry?.result || null)
    .filter(Boolean)
    .map((entry) => normalizeDetectedVideo(entry, tab));
  const rankedCandidates = scriptCandidates.sort((left, right) => {
    const leftScore = Number(Boolean(left?.useYtDlp)) * 100 + Number((left?.candidates || []).length > 0);
    const rightScore = Number(Boolean(right?.useYtDlp)) * 100 + Number((right?.candidates || []).length > 0);
    return rightScore - leftScore;
  });

  let selectedVideo = rankedCandidates[0] || null;

  if (!selectedVideo) {
    const response = await chrome.runtime.sendMessage({
      type: "nebula:get-media-candidates",
      tabId: tab.id
    });
    const observedCandidates = Array.isArray(response?.candidates) ? response.candidates : [];
    const bestObserved = observedCandidates.find((candidate) => /^https?:/i.test(candidate?.url || ""));
    if (bestObserved) {
      selectedVideo = normalizeDetectedVideo(
        {
          url: bestObserved.url,
          fileName: bestObserved.url.split("/").pop()?.split("?")[0] || "",
          title: tab.title || "Detected video",
          pageUrl: tab.url,
          pageHost: (() => {
            try {
              return new URL(tab.url).hostname;
            } catch (_error) {
              return "";
            }
          })(),
          poster: null,
          candidates: observedCandidates,
          useYtDlp: shouldPreferYtDlpForUrl(tab.url)
        },
        tab
      );
    }
  }

  renderVideoPanel(selectedVideo);
  if (selectedVideo?.url) {
    setStatus("Video detected. Download sends it straight to NebulaDM, while Queue Only saves it to NebulaDM without interrupting playback.");
  } else if (shouldPreferYtDlpForUrl(tab.url)) {
    const fallbackVideo = normalizeDetectedVideo(
      {
        url: tab.url,
        fileName: "",
        title: tab.title || "Detected video",
        pageUrl: tab.url,
        pageHost: (() => {
          try {
            return new URL(tab.url).hostname;
          } catch (_error) {
            return "";
          }
        })(),
        poster: null,
        candidates: [],
        useYtDlp: true
      },
      tab
    );
    renderVideoPanel(fallbackVideo);
    setStatus("This site is being handed to NebulaDM through the page URL so the bundled extractor can try the download.");
  } else {
    setStatus("No video was detected in this tab yet. Start playback first, then reopen the popup.");
  }
}

async function loadSettings() {
  const response = await chrome.runtime.sendMessage({ type: "nebula:get-settings" });
  bridgeUrlInput.value = response.bridgeUrl;
  captureEnabledInput.checked = response.captureEnabled;
  sendReferrerInput.checked = response.sendReferrer;
  sendUserAgentInput.checked = response.sendUserAgent;
  sendCookiesInput.checked = response.sendCookies;
}

saveButton.addEventListener("click", async () => {
  setStatus("Saving...");
  const response = await chrome.runtime.sendMessage({
    type: "nebula:save-settings",
    bridgeUrl: bridgeUrlInput.value.trim(),
    captureEnabled: captureEnabledInput.checked,
    sendReferrer: sendReferrerInput.checked,
    sendUserAgent: sendUserAgentInput.checked,
    sendCookies: sendCookiesInput.checked
  });

  setStatus(response.ok ? "Saved" : `Save failed: ${response.error}`);
});

enableCookiesButton.addEventListener("click", async () => {
  setStatus("Requesting cookie access...");
  const response = await chrome.runtime.sendMessage({ type: "nebula:enable-cookies" });
  if (response.ok) {
    sendCookiesInput.checked = true;
  }
  setStatus(response.ok ? response.message : `Cookie access failed: ${response.error}`);
});

downloadVideoButton.addEventListener("click", async () => {
  if (!detectedVideo?.url) {
    setStatus("No video is currently available to download.");
    return;
  }

  setStatus("Starting browser download so NebulaDM can take over...");
  try {
    const response = await chrome.runtime.sendMessage({
      type: "nebula:download-video",
      video: detectedVideo,
      tabId: activeTab?.id ?? -1,
      tabUrl: activeTab?.url || detectedVideo.pageUrl || null
    });
    setStatus(response.ok ? response.message : `Video download failed: ${response.error}`);
  } catch (error) {
    console.error(error);
    setStatus(`Video download failed: ${error}`);
  }
});

queueVideoButton.addEventListener("click", async () => {
  if (!detectedVideo?.url) {
    setStatus("No video is currently available to queue.");
    return;
  }

  setStatus("Queueing video in NebulaDM without interrupting playback...");
  const response = await chrome.runtime.sendMessage({
    type: "nebula:capture-video",
    video: detectedVideo,
    tabId: activeTab?.id ?? -1,
    tabUrl: activeTab?.url || detectedVideo.pageUrl || null
  });

  setStatus(response.ok ? response.message : `Queue failed: ${response.error}`);
});

loadSettings().catch((error) => {
  console.error(error);
  setStatus(`Load failed: ${error}`);
});

detectVideoInActiveTab().catch((error) => {
  console.error(error);
  setStatus(`Video detection failed: ${error}`);
});

captureEnabledInput.addEventListener("change", () => {
  setStatus(
    captureEnabledInput.checked
      ? "NebulaDM will take over browser downloads when the desktop app is reachable."
      : "Browser downloads will stay in the browser until capture is re-enabled."
  );
});

sendCookiesInput.addEventListener("change", () => {
  if (sendCookiesInput.checked) {
    setStatus("Save, or use Enable YouTube Cookies, to grant cookie access.");
    return;
  }

  setStatus("Cookie forwarding is off. Authenticated downloads may stay in the browser.");
});
