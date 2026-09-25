# Routes to Chromium's own UI (Chrome style), and where each one ends

Owner report: "There are ways to escape to the full Chromium UI." Every route
below was driven for real in the nativebrowser app (the `~/Developer/nativebrowser`
checkout of 2026-09-23) on the host built from this branch, and on a `main`
build of the same day for the baseline column.

- macOS: `scripts/mac/app-escape.sh` (marker `ND_APP_ESCAPE_MAC_OK`), real HID
  cursor (`app.cursor`, now with modifier keys) and real keystrokes, one route
  group per lock hold: `ND_ESCAPE_GROUPS=key|page|menu|webui|ext`.
- Linux: `ND_ACCEPT_DRIVE=scripts/app-escape-drive.ts scripts/headless-app-chrome.sh`
  on g815, rigs x11, wlr and hypr (`ND_ACCEPT_RIGS`). The drive navigates with
  CDP `Page.navigate` on the active tab, since under XWayland keys go to the
  child window under the pointer and typing into the address field fails.

Each route asserts: no new window of the host process that is not the app's
(window server census on mac, X toplevels on Linux), the app's window and tab
counts are what the app would do, the page viewport did not shrink (a Chromium
toolbar, bookmark bar or omnibox inside the view), and for menus the menu's own
shape (no duplicate label, no leading, trailing or adjacent separators, no empty
submenu, no item that opens Chromium UI). Captures of every surface seen are
written to `$ND_ESCAPE_SHOTS` (default `/tmp/nd-escape-shots`); the ones cited
below are kept under `.dev/` in this worktree (not committed).

Last runs on the branch (2026-09-25, app branch `escape`): mac key 41 of 41
(cmd+shift+S known, crash branch), menu 13 of 15; Linux x11 96 of 96, hypr 96
of 96, wlr 91 of 96 in one run and the five reds green alone. Earlier mac runs:
page 18 of 18 (plus two known bubble routes), ext 7 of 7, webui 11 of 11.

## The fixes

| Seam | Change |
|---|---|
| `on_chrome_command` (both) | Allowlist instead of a deny list: only back, forward, reload, stop, `IDC_CLOSE_FIND_OR_STOP` (Escape), zoom, cut/copy/paste (and on GTK the docked devtools ids) run. Everything else is refused. |
| new webview event `browserCommand` | Refused commands an app has a meaning for reach it by name: `newWindow`, `newPrivateWindow`, `newTab`, `reopenClosedTab`, `closeTab`, `closeWindow`, `nextTab`, `previousTab`, `history`, `downloads`, `bookmarks`, `bookmarkPage`, `settings`, `extensions`, `clearBrowsingData`, `print`, `savePage`, `viewSource`, `openFile`, `find`, `findNext`, `findPrevious`, `focusAddress`, `fullscreen`, `home`, `taskManager`, `quit`. Same names on AppKit and GTK. |
| `get_default_client` (AppKit, new) | Browsers Chrome creates on its own (`chrome.windows.create`, `chrome.tabs.create` into such a window, `runtime.openOptionsPage`) get a sink client: destination reported as `newWindow`, browser closed; the one Chrome keeps is hidden (`BrowserNativeWidgetWindow`, hidden on activation and on every surface sweep). GTK already had this. |
| context menu (both) | Dropped: Open Link in Incognito Window (it was rerouted to `newWindow`, which opened the link in an ordinary tab under a label promising privacy), Import passwords (opened chrome://password-manager), Use enhanced spell check (a Chromium dialog). |
| `app.cursor` | `modifiers` on click/down/up, so a cmd-click and a shift-click are real HID events. |

## macOS route table

| Route | main | no-escape |
|---|---|---|
| F7 | "Turn on caret browsing?" Chromium dialog window (448x186) | refused, nothing |
| `chrome.tabs.create` / `windows.create` (normal, popup, empty, incognito) / `runtime.openOptionsPage` / `tabs.create chrome://settings` | full "Chromium" window with tab strip and omnibox (the "two omniboxes" report), capture `.dev/ext-5231.png` | app tab each, no window |
| cmd+N | app window | app window |
| cmd+T, cmd+shift+T | app tab | app tab |
| cmd+comma | app Settings window | app Settings window |
| cmd+shift+N | nothing (Chromium refused it) | app private window, through `browserCommand newPrivateWindow` |
| cmd+shift+B/A/M/D/O/L/K/H/W, cmd+Y, cmd+D, cmd+S, cmd+O, cmd+E, cmd+opt+B/L/U/N, cmd+opt+shift+I, cmd+1/9, F1, ctrl+F2, shift+Esc, cmd+shift+Delete, cmd+opt+Right | nothing | nothing |
| cmd+P, cmd+opt+P | page zoom goes to 110% (the app menu's "primary+plus" became the key equivalent "p") | macOS print panel, through `browserCommand print` (capture `.dev/r2/shots-j/key.cmdP.png`) |
| cmd+shift+J | nothing | the app's own Downloads popover |
| cmd+shift+S | app's layout toggle (crash agent owns the save dialog report) | same |
| ctrl+cmd+F | AppKit full screen (menu bar strip only, app's own) | same |
| window.open (gesture, popup features, noopener, no gesture), target=_blank, middle click, cmd-click, shift-click | app tab | app tab |
| window.open("about:blank") then navigate | nothing (refused by design, `onBeforePopup`) | same |
| mailto:, unknown protocol | no Chromium surface; the app opens a `mailto:` tab of its own (app-side) | handed to the system (NSWorkspace), the page stays |
| window.print() | macOS print panel (native, allowed) | same |
| navigator.share | macOS share sheet (native, allowed) | same |
| document PiP | refused, nothing | same |
| password form submit | nothing | same |
| geolocation, notification | Chromium permission bubble at the top right of the screen | same: bubbles branch |
| page link status bubble (hover URL) | Chromium's 22 pt strip inside the view | same, reported and allowed |
| menu: Open Link in New Tab / New Window | app tab | app tab |
| menu: Open Link in Incognito Window | app tab (no privacy) | not offered |
| menu: Save As… | macOS save panel (native) | same |
| chrome://settings, /people Customize profile, /manageProfile, profile-picker, history, downloads, extensions + Details, bookmarks, signin intercept | in the app tab, no window | same |

## Linux (x11 rig) route table

| Route | main | no-escape |
|---|---|---|
| F7 | "Turn on caret browsing?" Chromium window | nothing |
| Shift+Esc | "Task Manager - Chromium" window | nothing, `browserCommand taskManager` |
| F11 | Chromium fullscreen bubble | nothing, `browserCommand fullscreen` |
| F1 | Chromium help page in an app tab | nothing |
| menu: Use enhanced spell check | Chromium dialog window | not offered |
| menu: Import passwords | chrome://password-manager app tab | not offered |
| menu: Open link in incognito window | ordinary app tab | not offered |

Clean on both builds: Ctrl+N (app window), Ctrl+T and Ctrl+Shift+T (app tab),
every other chord in the brief (Ctrl+Shift+Q included), every window.open form,
target=_blank, middle/ctrl/shift click, the open-link menu items, all extension
calls (GTK had its sink already), mailto and unknown protocols, window.print,
document PiP, the password form, the chrome:// pages, DevTools undock, and
geolocation/notification (the app's own site-info popover on GTK).

## Context menus (mac captures in `.dev/shots-menu/`)

| Context | Items | Finding |
|---|---|---|
| link, link on image | Open Link in New Tab, Open Link in New Window, Save Link As…, Copy Link Address, Copy, Copy Link to Highlight, Search Google for, ND probe item, Inspect, Open Link in New Tab, Speech | "Open Link in New Tab" twice: Chromium's plus the app's own item. App-side, fixed on the app's `escape` branch. |
| image | Save Image As…, Copy Image, Copy Image Address, probe, Inspect, Save Image | the app added "Save Image" next to Chromium's "Save Image As…"; dropped on the app's `escape` branch |
| page | Back, Forward, Reload, Save As…, probe, Inspect | clean |
| editable empty / with text / misspelled | Emoji & Symbols, Undo…Select All, Search Google for, Language Settings, Writing Direction, Speech | clean; "Search Google" although the app's engine is DuckDuckGo (Chromium default search engine, app-side setting) |
| audio / video (wav fixture) | Loop, Show All Controls, Save Audio As…, Copy Audio Address, probe, Inspect | clean |
| DevTools | not traced on GTK; no menu window appeared | open |

## Still open

- mac menu.linkImage and menu.video: the right-click on the linked image and on
  the video opens no menu, on the branch and on a `main` baseline alike (both
  were green on 2026-09-23). A drive or machine problem, not an escape.
- Linux wlr: in the full run page.windowOpenNoopener, page.externalProtocol,
  menu.openLinkNewTab and two ctx routes can miss (viewport 613 px wide there);
  all five pass alone.
- Linux ctrl+P reaches no `on_chrome_command` at all and shows nothing, like
  window.print() on Linux (print preview is disabled).
- Permission bubbles, zoom bubble, find bar, downloads and password bubbles:
  owned by the `bubbles` branch.
- The kept Chrome-created browser on mac can reach the screen for one sweep
  tick (200 ms) if it is shown without being activated; activation is hidden
  synchronously.
- Chrome Web Store and Clear browsing data links in chrome:// pages were not
  found by text, so those two clicks are untested.
