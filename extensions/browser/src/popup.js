const bridgeUrlInput = document.getElementById("bridgeUrl");
const captureEnabledInput = document.getElementById("captureEnabled");
const sendReferrerInput = document.getElementById("sendReferrer");
const sendUserAgentInput = document.getElementById("sendUserAgent");
const sendCookiesInput = document.getElementById("sendCookies");
const saveButton = document.getElementById("saveButton");
const statusText = document.getElementById("statusText");

function setStatus(message) {
  statusText.textContent = message;
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

loadSettings().catch((error) => {
  console.error(error);
  setStatus(`Load failed: ${error}`);
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
