// Chromium's download bubble and download-started animation are drawn against
// a toolbar the embedding does not have, and no switch or pref turns the
// animation off. setUiOptions does, for as long as this extension is loaded.
const off = () => chrome.downloads.setUiOptions({ enabled: false }).catch(() => {});
off();
chrome.runtime.onStartup.addListener(off);
chrome.runtime.onInstalled.addListener(off);
