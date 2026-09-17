document.documentElement.dataset.ndExtension = "live";
chrome.runtime.onMessage.addListener((message) => {
  if (message && message.ndMenu) document.documentElement.dataset.ndMenu = message.ndMenu;
});
