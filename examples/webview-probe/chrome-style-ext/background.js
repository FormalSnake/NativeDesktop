// The service worker existing at all is the proof: Alloy style has no extension
// runtime, so there is no chrome-extension:// target to find.
chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: "nd-probe", title: "ND probe item", contexts: ["all"] });
  chrome.contextMenus.create({ id: "nd-probe-sub", parentId: "nd-probe", title: "ND probe child", contexts: ["all"] });
});

chrome.contextMenus.onClicked.addListener((info, tab) => {
  chrome.tabs.sendMessage(tab.id, { ndMenu: info.menuItemId });
});
