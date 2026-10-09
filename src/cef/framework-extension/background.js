// Chromium's download bubble and download-started animation are drawn against
// a toolbar the embedding does not have, and no switch or pref turns the
// animation off. setUiOptions does, for as long as this extension is loaded.
const off = () => chrome.downloads.setUiOptions({ enabled: false }).catch(() => {});
off();
chrome.runtime.onStartup.addListener(off);
chrome.runtime.onInstalled.addListener(off);

// Chrome's audible and muted state per tab, which CEF reports nowhere. The host
// adds the binding to this worker over its browser pipe; a tab is named by its
// page target, the id the host knows each view by.
const reportAudio = async (tab) => {
  if (typeof globalThis.__ndTabAudio !== "function") return;
  const targets = await chrome.debugger.getTargets();
  const page = targets.find((t) => t.tabId === tab.id && t.type === "page");
  if (!page) return;
  globalThis.__ndTabAudio(JSON.stringify({ target: page.id, audible: tab.audible === true, muted: tab.mutedInfo?.muted === true }));
};
chrome.tabs.onUpdated.addListener((_, change, tab) => {
  if ("audible" in change || "mutedInfo" in change) reportAudio(tab).catch(() => {});
});
