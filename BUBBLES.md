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
   (`on_chrome_command` lets only the page's own commands through, see
   ESCAPES.md), its page action icon is hidden
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
| Zoom | Cmd/Ctrl `+` `-` `0` (on macOS Chrome maps them through its main menu, which this embedding lacks, so there they are the app's own View menu accelerators), ctrl+wheel, `setZoom`, an extension's `chrome.tabs.setZoom` | Native | The engine serves `IDC_ZOOM_PLUS/MINUS/NORMAL` itself (Chrome's preset steps) and reports every change as `zoomChanged {factor, source}`. The app shows a magnifier while zoom is not 100% (compact: the address field's trailing icon, popover under it; sidebar layout: a glyph at the sidebar's foot, popover above it; sidebar hidden: a toast with the value) and a popover with minus, value, plus, Reset; a chord or a menu step shows it for 1.5s. macOS: the bubble window is closed the moment it appears, before it is placed. Linux: every page browser is created with `chrome_zoom_bubble` disabled, see below. A trackpad pinch is page scale and raises nothing. |
| Find bar | Cmd/Ctrl+F, F3, Cmd/Ctrl+G | Native | The app's own find popover (`findStart`/`findNext`, `findResult`). `IDC_FIND`, `IDC_FIND_NEXT`, `IDC_FIND_PREVIOUS` are refused and reach the app as `browserCommand`. |
| Downloads | a download starting | Native | Owned by the `pages` branch (chrome://downloads and the bubble). The engine already cancels Chrome's download and reports `downloadRequested`. |
| Save / update password | submitting a login form | Suppressed | Profile preferences `credentials_enable_service` and `credentials_enable_autosignin` off (Chrome's password manager; the owner uses an extension). Written at every request context's initialization with the startup preferences. `IDC_MANAGE_PASSWORDS_FOR_PAGE` refused, and the key icon is hidden. |
| Passkey / security key | `navigator.credentials` | Stopgap (correct place) | WebAuthn is a tab-modal dialog, not an anchored bubble: `NDCefSurfaceWindows` (macOS) and the window watch (Linux) put it at the top centre of the web contents, which is where Chrome puts tab-modal sheets. Saving a passkey to Google Password Manager needs a signed-in profile, which this browser never has. |
| Save card / save address | submitting a payment or address form | Suppressed | `autofill.credit_card_enabled` and `autofill.profile_enabled` off. |
| Translate | a page in another language, the context menu, `IDC_SHOW_TRANSLATE` | Suppressed | `translate.enabled` off; `IDC_SHOW_TRANSLATE` and `IDC_CONTENT_CONTEXT_TRANSLATE` refused (the context menu drops the item). |
| Cookie controls | the location bar's eye icon | Suppressed | Page action icon hidden, so there is nothing to click and no anchor. |
| Page info | the location bar's padlock | Native | The app's own site-info popover under its padlock (`leadingIconName` + `anchorSlot="leadingIcon"`). Chrome's page action is hidden. |
| Extension install prompt | "Add to Chrome" on the Web Store | Native | The app asks in its own dialog before Chromium is called; on a yes `acceptExtensionInstall` answers Chromium's "Add <name>?" unseen (AppKit: transparent, pressed through accessibility; GTK: empty shape, Tab and Space over XTest), and the app's toast says the extension was added. See webview.md. |
| Extension installed | after a Web Store install | Stopgap | A Views dialog anchored to the extensions button, not seen on 151.3.23 in either embedding; should one come up it arrives as a top-level window and the window watch centres it on the view (`chromeDialog`). The app's extensions toolbar (`extensionsChanged`) is the native half. |
| Extension site access / requests | the toolbar's extensions menu | Suppressed | The toolbar is hidden (`is_chrome_toolbar_button_visible`), so the menu and its requests bubble cannot open. |
| Intent picker | a link a native app handles | Suppressed | Page action icon hidden. |
| PWA install | the install icon, `IDC_INSTALL_PWA` | Suppressed | Icon hidden, command refused. |
| Sharing hub / QR code | the share icon, `IDC_SHARING_HUB`, `IDC_QRCODE_GENERATOR`, the context menu | Suppressed | Icons hidden, commands refused (`IDC_CONTENT_CONTEXT_GENERATE_QR_CODE` too; the context menu drops it). |
| Send tab to self | the icon, `IDC_SEND_TAB_TO_SELF` | Suppressed | Icon hidden, command refused; it also needs a signed-in profile. |
| Reading list | `IDC_READING_LIST_MENU_ADD_TAB` | Suppressed | Command refused. |
| Bookmark star | Cmd/Ctrl+D (`IDC_BOOKMARK_THIS_TAB`), `IDC_BOOKMARK_ALL_TABS` | Suppressed | Commands refused, star hidden; Cmd/Ctrl+D reaches the app as `browserCommand` `bookmarkPage`. |
| Tab search | Cmd/Ctrl+Shift+A (`IDC_TAB_SEARCH`) | Suppressed | Command refused; the app's command palette is the native tab search. |
| Avatar / profile menu | `IDC_SHOW_AVATAR_MENU` | Suppressed | Command refused. |
| Device chooser (WebUSB, Web Serial, WebHID) | `requestDevice`, `requestPort` | Stopgap (in the window) | Chromium's own chooser. With no location bar to hang from it opens at the page's top left, drawn inside the browser's window. The engine's focus sync used to set the browser's focus again every 500 ms, which took Views' focus off the bubble and closed it before it could be used (the promise rejected with "No device selected"); focus the browser already holds is now left alone. Under openbox a click into the chooser still closes it: the window manager focuses the toplevel on every click, which deactivates the browser's window. Hyprland does not, and a device can be picked there. |
| Sad tab ("Aw, Snap!") | a renderer crash, out of memory, a kill | Native | `renderProcessGone {reason, errorCode, error}` (`on_render_process_terminated`); the app draws the sad tab and reloads the view. |
| Page Unresponsive | a page that stops answering | Native (engine) | Chromium's hang monitor ignores any page with a devtools session attached, which every view here has, so neither CEF's unresponsive callback nor Chrome's dialog ever runs. The engine pings the page on show (`Runtime.evaluate`) every 3 s; one unanswered for 15 s puts up an AdwAlertDialog over the window with Wait and Exit page. Wait asks again 15 s later, an answer closes it, Exit page kills the renderer (found by its `--renderer-client-id`, which each renderer reports) and so ends in `renderProcessGone`. |

## Linux zoom bubble

On Linux Views draws the zoom bubble inside the browser's own X window (no X
top-level appears, so the window watch never sees it), at the view's top right,
for 1.5s. `cef_browser_settings_t.chrome_zoom_bubble` set to `STATE_DISABLED`
removes it: CEF passes it to `ZoomController::SetShowsNotificationBubble` when
it creates the browser, which is the flag every zoom path
(`ZoomController::SetZoomLevel`, the chords, ctrl+wheel, `setZoom`) reads. CEF
applies it in `CefBrowserPlatformDelegateChromeViews::NotifyBrowserCreated`,
which covers both the child-window embedding and the Views-hosted browsers, so
the engine sets it at both create calls.

Routes tried before that setting was found, none of which removes the bubble:
`is_chrome_page_action_icon_visible(CEF_CPAIT_ZOOM)` (the bubble does not hang
off the icon), preferences (there is no zoom-bubble preference), closing the
`LocationBarBubbleDelegateView` early (flickers the page), and
`Emulation.setDeviceMetricsOverride` (emulates a device, not a page zoom).

`scripts/cef-zoom-drive.ts` fails a step whose capture shows the bubble in the
view's top right, and keeps one capture per step as `cef-zoom-<step>.png` in
`XDG_RUNTIME_DIR`. `ND_ACCEPT_LEGS=zoom scripts/headless-app-chrome.sh` drives
the real app the same way on x11, wlr and hypr: the zoom chords, the app's
popover and a reload, with captures `zoom-<rig>-<step>.png`.

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

The sidebar layout has no address field: `<ZoomFootControl>` puts the
magnifier among the glyphs at the sidebar's foot, beside the padlock, with the
popover above it, and with the sidebar hidden a chord or menu step says the
new value in a toast.
