const password = document.querySelector("input[type=password]");
if (password) {
  chrome.runtime.sendMessage("credentials", (found) => {
    const user = document.querySelector("input[autocomplete=username]");
    if (user) user.value = found.user;
    password.value = found.password;
    password.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
