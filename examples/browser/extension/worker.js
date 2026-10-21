// Action state lives in memory, so it is set again each time the worker starts.
chrome.action.setBadgeText({ text: "nd" });

const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

async function fill(tabId) {
  const answer = await chrome.runtime.sendNativeMessage("dev.formalsnake.nd_test_host", { ask: "secret" });
  // The page may still be loading: its content script answers once it has run.
  for (let i = 0; i < 100; i++) {
    try {
      return await chrome.tabs.sendMessage(tabId, { fill: answer.secret });
    } catch (e) {
      await sleep(50);
    }
  }
  throw new Error("the page's content script never answered");
}

chrome.runtime.onMessage.addListener((message, sender, reply) => {
  if (message.want !== "fill") return;
  fill(message.tabId ?? sender.tab.id).then((r) => reply({ ok: true, r }), (e) => reply({ ok: false, error: String(e) }));
  return true;
});
