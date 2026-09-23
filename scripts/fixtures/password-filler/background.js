chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message === "credentials") reply({ user: "nd-user", password: "filled-by-extension" });
});
