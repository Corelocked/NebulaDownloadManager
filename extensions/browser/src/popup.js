const bridgeUrlInput = document.getElementById("bridgeUrl");
const captureEnabledInput = document.getElementById("captureEnabled");
const sendReferrerInput = document.getElementById("sendReferrer");
const sendUserAgentInput = document.getElementById("sendUserAgent");
const sendCookiesInput = document.getElementById("sendCookies");
const videoPanel = document.getElementById("videoPanel");
const videoTitle = document.getElementById("videoTitle");
const videoSource = document.getElementById("videoSource");
const downloadVideoButton = document.getElementById("downloadVideoButton");
const queueVideoButton = document.getElementById("queueVideoButton");
const saveButton = document.getElementById("saveButton");
const statusText = document.getElementById("statusText");

let detectedVideo = null;
let activeTab = null;

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

  const [result] = await chrome.scripting.executeScript({
    target: { tabId: tab.id },
    func: () => {
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
        poster: best.poster || null,
        candidates: []
      };
    }
  });

  renderVideoPanel(result?.result || null);
  if (result?.result?.url) {
    setStatus("Video detected. Download hands it off to NebulaDM, while Queue Only keeps playback in the browser.");
  } else {
    setStatus("No active HTML5 video detected in this tab.");
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
    setStatus("Saving with cookies enabled will trigger a browser permission prompt.");
    return;
  }

  setStatus("Cookie forwarding is off. Authenticated downloads may stay in the browser.");
});
