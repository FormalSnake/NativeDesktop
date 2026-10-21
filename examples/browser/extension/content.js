document.documentElement.dataset.ndExtension = "1";
chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (typeof message.fill !== "string") return;
  const field = document.getElementById("password");
  if (field) field.value = message.fill;
  document.title = "filled:" + message.fill;
  reply({ ok: !!field });
});
// The page asks for a fill itself where the app has no toolbar button to click.
window.addEventListener("message", (e) => {
  if (e.source === window && e.data === "nd-fill") chrome.runtime.sendMessage({ want: "fill" });
});
