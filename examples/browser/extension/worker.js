// Action state lives in memory, so it is set again each time the worker starts.
chrome.action.setBadgeText({ text: "nd" });

const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

async function fill(tabId) {
  const answer = await chrome.runtime.sendNativeMessage("dev.formalsnake.nd_test_host", { ask: "secret" });
  // A POST from the worker carries an Origin, which the page's server echoes back.
  const tab = await chrome.tabs.get(tabId);
  const origin = await (await fetch(new URL("/origin", tab.url), { method: "POST", body: "nd" })).text();
  // The page may still be loading: its content script answers once it has run.
  for (let i = 0; i < 100; i++) {
    try {
      return await chrome.tabs.sendMessage(tabId, { fill: answer.secret, origin });
    } catch (e) {
      await sleep(50);
    }
  }
  throw new Error("the page's content script never answered");
}

chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message.want !== "fill") return;
  fill(message.tabId).then((r) => reply({ ok: true, r }), (e) => reply({ ok: false, error: String(e) }));
  return true;
});
