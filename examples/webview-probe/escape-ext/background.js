// The gate calls chrome.windows.create, chrome.tabs.create and
// chrome.runtime.openOptionsPage from this worker over the debugging port, which
// is exactly the call an extension makes on its own.
self.ndEscape = true;

// An idle MV3 worker is stopped after 30 s and drops out of the target list; an
// extension API call inside that window restarts the clock.
setInterval(() => chrome.runtime.getPlatformInfo(() => {}), 20000);
