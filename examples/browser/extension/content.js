document.documentElement.dataset.ndExtension = "1";
chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (typeof message.fill !== "string") return;
  const field = document.getElementById("password");
  if (field) field.value = message.fill;
  document.documentElement.dataset.ndOrigin = message.origin;
  document.title = "filled:" + message.fill;
  reply({ ok: !!field });
});
