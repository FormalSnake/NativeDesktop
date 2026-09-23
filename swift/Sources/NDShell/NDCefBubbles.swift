#if canImport(CCef)
// Features that raise a Chrome bubble from the page itself, anchored to a
// location bar this embedding does not have: the password manager (save and
// update password; password extensions fill without it), autofill saving (save
// card, save address) and translate. Peer of `bubble_prefs` in
// src/cef/engine.zig; BUBBLES.md has the full table.
let ndCefBubblePrefs = [
    "credentials_enable_service", "credentials_enable_autosignin",
    "autofill.profile_enabled", "autofill.credit_card_enabled", "translate.enabled",
]
#endif
