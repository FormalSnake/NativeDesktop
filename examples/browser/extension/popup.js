(async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const answer = await chrome.runtime.sendMessage({ want: "fill", tabId: tab.id });
  document.getElementById("state").textContent = answer.ok ? "filled" : answer.error;
  if (answer.ok) window.close();
})();
