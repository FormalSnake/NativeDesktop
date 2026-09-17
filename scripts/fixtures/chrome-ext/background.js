// The gate drives this service worker over CDP. Everything it needs is hung off
// globalThis because Runtime.evaluate has no other way to reach extension APIs.
globalThis.ndOpenWindow = () => chrome.windows.create({ url: "about:blank" });
globalThis.ndOpenTab = () => chrome.tabs.create({ url: "about:blank" });
globalThis.ndOpenOptions = () => chrome.runtime.openOptionsPage();
globalThis.ndWrite = (value) => chrome.storage.local.set({ ndGate: value });
globalThis.ndRead = () => chrome.storage.local.get("ndGate").then((r) => r.ndGate ?? "");
