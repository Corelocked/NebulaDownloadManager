const bridgeUrlInput = document.getElementById("bridgeUrl");
const captureEnabledInput = document.getElementById("captureEnabled");
const saveButton = document.getElementById("saveButton");
const statusText = document.getElementById("statusText");

function setStatus(message) {
  statusText.textContent = message;
}

async function loadSettings() {
  const response = await chrome.runtime.sendMessage({ type: "nebula:get-settings" });
  bridgeUrlInput.value = response.bridgeUrl;
  captureEnabledInput.checked = response.captureEnabled;
}

saveButton.addEventListener("click", async () => {
  setStatus("Saving...");
  const response = await chrome.runtime.sendMessage({
    type: "nebula:save-settings",
    bridgeUrl: bridgeUrlInput.value.trim(),
    captureEnabled: captureEnabledInput.checked
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
