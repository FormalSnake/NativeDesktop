# Chromium bubbles in Chrome style

Chrome style embeds the browser with no toolbar (`CEF_CTT_NONE`), so every
Views bubble Chromium anchors to the location bar, the toolbar or the tab strip
has nothing to anchor to. On macOS such a bubble is a window of its own, which
`NDCefSurfaceWindows` then centres at the top of the web contents; on Linux
Views draws it inside the browser's X window. Either way it lands over the page,
which is what the owner saw with the zoom bubble.

Each bubble gets one of three answers:

1. **Native**: the framework reports the state (an event or an API) and the app
   draws its own UI where it belongs.
2. **Suppressed**: the command that raises it is refused
   (`on_chrome_command`), its page action icon is hidden
   (`is_chrome_page_action_icon_visible`, which also removes the page action
   from the location bar CEF builds), or the profile preference behind it is
   off.
3. **Stopgap**: still Chromium's bubble, in Chromium's place, listed here
   until 1 or 2 is possible.

Code: `src/cef/engine.zig` (sections "Chrome's other bubbles" and "Page zoom
reported to the app"), `swift/Sources/NDShell/NDCefBubbles.swift`,
`swift/Sources/NDShell/NDCefZoom.swift`. Gates: `scripts/headless-cef-zoom.sh`
and `scripts/mac/cef-zoom.sh` (drive `scripts/cef-zoom-drive.ts`),
`scripts/headless-headerfield.sh` and `scripts/mac/mac-headerfield.sh` (the
trailing icon and its popover anchor).

| Bubble | Raised by | Answer | How |
| --- | --- | --- | --- |
| Zoom | Cmd/Ctrl `+` `-` `0` (on macOS Chrome maps them through its main menu, which this embedding lacks, so there they are the app's own View menu accelerators), ctrl+wheel, `setZoom`, an extension's `chrome.tabs.setZoom` | Native | The engine serves `IDC_ZOOM_PLUS/MINUS/NORMAL` itself (Chrome's preset steps) and reports every change as `zoomChanged {factor, source}`. The app shows a magnifier as the address field's trailing icon while zoom is not 100% and a popover under it (minus, value, plus, Reset); a chord or a menu step shows the popover for 1.5s. macOS: the bubble window is closed the moment it appears, before it is placed. **Linux: blocked on our own CEF build**, see below. A trackpad pinch is page scale and raises nothing. |
| Find bar | Cmd/Ctrl+F, F3, Cmd/Ctrl+G | Native | The app's own find popover (`findStart`/`findNext`, `findResult`). `IDC_FIND`, `IDC_FIND_NEXT`, `IDC_FIND_PREVIOUS` are refused, so a chord the app did not bind raises nothing. The `no-escape` branch routes them to the app as `browserCommand` instead. |
| Downloads | a download starting | Native | Owned by the `pages` branch (chrome://downloads and the bubble). The engine already cancels Chrome's download and reports `downloadRequested`. |
| Save / update password | submitting a login form | Suppressed | Profile preferences `credentials_enable_service` and `credentials_enable_autosignin` off (Chrome's password manager; the owner uses an extension). `IDC_MANAGE_PASSWORDS_FOR_PAGE` refused, and the key icon is hidden. |
| Passkey / security key | `navigator.credentials` | Stopgap (correct place) | WebAuthn is a tab-modal dialog, not an anchored bubble: `NDCefSurfaceWindows` (macOS) and the window watch (Linux) put it at the top centre of the web contents, which is where Chrome puts tab-modal sheets. Saving a passkey to Google Password Manager needs a signed-in profile, which this browser never has. |
| Save card / save address | submitting a payment or address form | Suppressed | `autofill.credit_card_enabled` and `autofill.profile_enabled` off. |
| Translate | a page in another language, the context menu, `IDC_SHOW_TRANSLATE` | Suppressed | `translate.enabled` off; `IDC_SHOW_TRANSLATE` and `IDC_CONTENT_CONTEXT_TRANSLATE` refused (the context menu drops the item). |
| Cookie controls | the location bar's eye icon | Suppressed | Page action icon hidden, so there is nothing to click and no anchor. |
| Page info | the location bar's padlock | Native | The app's own site-info popover under its padlock (`leadingIconName` + `anchorSlot="leadingIcon"`). Chrome's page action is hidden. |
| Extension installed | installing from the Web Store | Stopgap | A Views dialog anchored to the extensions button; it arrives as a top-level window and the window watch centres it on the view (`chromeDialog`). The app's extensions toolbar (`extensionsChanged`) is the native half; replacing the dialog needs the install flow to report before Chrome draws it. |
| Extension site access / requests | the toolbar's extensions menu | Suppressed | The toolbar is hidden (`is_chrome_toolbar_button_visible`), so the menu and its requests bubble cannot open. |
| Intent picker | a link a native app handles | Suppressed | Page action icon hidden. |
| PWA install | the install icon, `IDC_INSTALL_PWA` | Suppressed | Icon hidden, command refused. |
| Sharing hub / QR code | the share icon, `IDC_SHARING_HUB`, `IDC_QRCODE_GENERATOR`, the context menu | Suppressed | Icons hidden, commands refused (`IDC_CONTENT_CONTEXT_GENERATE_QR_CODE` too; the context menu drops it). |
| Send tab to self | the icon, `IDC_SEND_TAB_TO_SELF` | Suppressed | Icon hidden, command refused; it also needs a signed-in profile. |
| Reading list | `IDC_READING_LIST_MENU_ADD_TAB` | Suppressed | Command refused. |
| Bookmark star | Cmd/Ctrl+D (`IDC_BOOKMARK_THIS_TAB`), `IDC_BOOKMARK_ALL_TABS` | Suppressed | Commands refused, star hidden. `no-escape` routes Cmd/Ctrl+D to the app as `browserCommand` `bookmarkPage`. |
| Tab search | Cmd/Ctrl+Shift+A (`IDC_TAB_SEARCH`) | Suppressed | Command refused; the app's command palette is the native tab search. |
| Avatar / profile menu | `IDC_SHOW_AVATAR_MENU` | Suppressed | Command refused. |

## Linux zoom bubble: why stock CEF cannot remove it

On Linux Views draws the zoom bubble inside the browser's own X window (no X
top-level appears, so the window watch never sees it), at the view's top right,
for 1.5s. Tried, none of them removes it:

- `is_chrome_page_action_icon_visible(CEF_CPAIT_ZOOM)` false (the shipped
  answer) and true: the bubble does not hang off the icon. `ZoomViewController`
  shows it from `ToolbarView::ZoomChangedForActiveTab(can_show_bubble)`, which
  runs with no location bar at all.
- Preferences: there is no zoom-bubble preference, and Chrome reads the
  per-host zoom preferences only at profile load
  (`ChromeZoomLevelPrefs::InitHostZoomMap`), so a zoom cannot be applied
  through them.
- The engine's own zoom path: `set_zoom_level` and `zoom` both go through
  `ZoomController::SetZoomLevel`, which sets `can_show_bubble` from
  `can_show_bubble_`. That flag defaults to true and only
  `ZoomController::SetShowsNotificationBubble` (C++) changes it.
- Zooming without the controller: the only CDP route is
  `Emulation.setDeviceMetricsOverride`, which emulates a device (layout shrinks
  to a box in the corner of the view), not a page zoom.

The fix is one call in our own CEF build (already planned for
`GetChromeToolbarType`): `ZoomController::FromWebContents(contents)->SetShowsNotificationBubble(false)`
when CEF creates a Chrome-style browser, or a `cef_browser_settings_t` flag
that does it. Until then the bubble shows for 1.5s after a zoom on Linux, and
`scripts/cef-zoom-drive.ts` reports it as `ND_CEF_ZOOM_BUBBLE_IN_VIEW`.

## Password manager off, extensions still fill

`scripts/cef-zoom-drive.ts` loads `scripts/fixtures/password-filler`, an
extension that fills a login form from its service worker the way 1Password's
does, with Chrome's password manager off, and signs in: the form fills and no
save-password bubble comes up (`ND_CEF_EXTENSION_FILL_OK`, both platforms).

## App integration

The app's part lives in `src/ZoomControl.tsx` (nativebrowser):

- `zoomFieldProps(factor, shown, onClick)` goes on whatever address field the
  layout has (`<searchinput>` or `<textinput>`): it sets `trailingIconName`,
  its tooltip and label, and `onTrailingIconClicked`. The name is always
  passed, empty while hidden, because GTK only gives a search field an icon
  slot when it mounts with one.
- `<ZoomPopover anchor={fieldRef} …>` is portalled and anchored with
  `anchorSlot="trailingIcon"`, so it points at the magnifier in either layout.
  `fieldRef` must be a ref object passed as `ref={fieldRef}`, not a callback
  ref: React re-attaches a callback ref on every commit and the popover reads
  the ref mid-commit.
- `useZoomPopover(tabId, notice)` opens it for 1.5s when the tab's
  `zoomNotice` counter moves (a chord inside the page, a menu or palette step)
  and until dismissed when the icon is clicked.

Today both layouts use the header's address field, so the indicator is in the
right place in each. The sidebar's own URL field (Arc layout, `sidebar`
branch) takes the same two lines when it lands.
