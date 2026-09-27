// The gate evaluates globalThis.ndPing() in this worker over CDP.
globalThis.ndPing = () =>
  new Promise((resolve) => {
    chrome.runtime.sendNativeMessage("dev.nativedesktop.echo", { ping: 1 }, (reply) => {
      resolve(chrome.runtime.lastError ? `error: ${chrome.runtime.lastError.message}` : JSON.stringify(reply));
    });
  });
