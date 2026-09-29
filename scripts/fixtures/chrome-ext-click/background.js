// No host permissions: executeScript only succeeds on a tab that activeTab was
// granted for, which is what a real toolbar click does. Each click bumps the
// badge so a capture shows how many landed.
let clicks = 0;
chrome.action.setBadgeText({ text: "0" });
chrome.action.setBadgeBackgroundColor({ color: [0, 110, 220, 255] });
chrome.action.onClicked.addListener(async (tab) => {
  clicks += 1;
  chrome.action.setBadgeText({ text: String(clicks) });
  try {
    await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: (id, url, n) => {
        document.documentElement.dataset.ndActionClicked = `${n} ${id} ${url}`;
      },
      args: [tab.id, tab.url ?? "", clicks],
    });
  } catch (error) {
    console.error("ND_ACTION_CLICK_FAIL", String(error));
  }
});
