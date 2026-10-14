# NativeDesktop (zig branch) parity checklist

Source: /Users/kyandesutter/Developer/NativeDesktop @ `zig` (HEAD 72ad68f5, release v0.5.10, 992 commits). Paths are relative to that repo.
Tick a box only when the Rust rewrite passes the gate named in the line (or the gate in section 11).

## 1. Widgets (schema/widgets.json, 72 widgets, one codegen template per backend)

Every widget carries the common set, listed once here: props `enabled, tooltip, testID, draggable, dragPayload, dropTarget, contextMenu`; events `dragStarted, dragEnded, dragOver, dropped, contextMenuSelected`. Tables below list only widget-specific items.
Style keys (schema `style`): `background, color, font{fontSize,fontWeight,fontFamily}, padding, margin, hexpand, vexpand, halign, valign, minWidth, minHeight, border{borderWidth,borderColor,borderRadius}`. Unknown key = StyleError. 
cssClasses whitelist (47): suggested-action, destructive-action, flat, raised, circular, pill, linked, toolbar, spacer, title-1..4, heading, document, body, caption-heading, caption, monospace, numeric, accent, success, warning, error, boxed-list, boxed-list-separate, card, activatable, navigation-sidebar, nd-native-sidebar, selection-mode, osd, dimmed, dim-label, background, view, glass, frame, compact, menu, inline, large-title, property, round, opaque, devel, icon-dropshadow, lowres-icon. Unknown class = CssClassError.

- [ ] common widget props/events above on all 72 widgets (schema/widgets.json)
- [ ] style validation + cssClasses validation with errors (packages/react/src/core/style-validate.ts, css-classes-validate.ts)
- [ ] set-replace semantics for style/cssClasses; dropped prop resets to schema default or type zero (ndApplyDroppedDefaults; scripts/propreset-drive.ts)
- [ ] platform-only widgets no-op on other backends + one-time dev warn (`platforms`, widgetPlatforms in schema-meta.ts)

| widget | key props | events | commands | platforms |
|---|---|---|---|---|
| window | title, defaultWidth/Height, tabGroup, toolbarStyle, frameAutosaveName, density, presentation(window/sheet) | alertResult, openFileResult, saveFileResult, newTabRequested, closed, focused, sizeChanged, fullscreenChanged | showAlert, openFile, saveFile, showAbout, showTabOverview, present, setFullscreen | both |
| box | orientation, spacing, windowHandle, tileMinWidth, tileMaxColumns, tileAspect | hoverChanged | | both |
| label | text, ellipsize, ellipsizeMode, variant | | | both |
| button | label, iconName, iconData, labelAlign, ellipsize, prominent, destructive, badge, size | clicked, hoverChanged | focus | both |
| textinput | text, placeholder, leading/trailingIcon{Name,Tooltip,Label}, editable | changed, activate, leadingIconClicked, trailingIconClicked | focus, activateLeadingIcon, activateTrailingIcon | both |
| searchinput | same as textinput minus editable | changed, activate, icon clicks | focus, activate*Icon | both |
| textarea | text, minContentHeight | changed | focus | both |
| checkbox | checked, label | toggled | focus | both |
| radio | group, checked, label | toggled | focus | both |
| select | options, selectedIndex | selectionChanged | focus | both |
| combobox | options, selectedIndex, text, placeholder, editable | selectionChanged, changed | focus | both |
| slider | min, max, step, value, orientation | valueChanged | | both |
| numberinput | value, min, max, step, digits, wraps | valueChanged | focus | both |
| progressbar | fraction | | | both |
| progresscircle | fraction, lineWidth, showLabel | | | both |
| spinner | spinning | | | both |
| skeleton | width, height, radius, animated | | | both |
| image | path, iconName, pixelSize, symbolScale, symbolWeight, symbolRenderingMode | | | both |
| separator | orientation | | | both |
| scrollview | minContentHeight, hscroll | | | both |
| tabview | selectedIndex | selectionChanged | | both |
| grid | (children placement) | | | both |
| paned | orientation, position | positionChanged | | both |
| splitview | sidebarWidth(fraction), collapsed, listWidth, breakpoint, edgeReveal, contentStyle | revealChanged | revealSidebar, concealSidebar | both |
| headerbar | title, subtitle, showTitleButtons, canGoBack, canGoForward | back, forward | | both |
| toolbarview | topBarStyle, bottomBarStyle, extendContentToTopEdge, topBarsAutoHide | | revealTopBars, concealTopBars | both |
| windowcontrols | side | emptyChanged | | both |
| listview | items, selectedIndex, empty{IconName,Title,Description} | rowActivated | | both |
| sourcelist | items, selectedIndex, empty* | selectionChanged, rowActivated | | both |
| sourcetree | nodes, actions, selectedId, actionVisibility(hover/always), indentationPerLevel, empty* | selectionChanged, rowActivated, nodeExpanded, nodeCollapsed, actionClicked, middleClicked | | both |
| table | columns, rows, selectionMode, selectedIndex(es), columnsReorderable, showRowSeparators, empty* | selectionChanged, rowActivated, sortChanged, columnsResized, columnsReordered | | both |
| treeview | nodes(flat), selectedIndex, indentationPerLevel, empty* | selectionChanged, rowActivated, nodeExpanded, nodeCollapsed | | both |
| menubar | defaults | | | both |
| menu | label | | | both |
| menuitem | label, iconName, iconVisible, accelerator, role | selected | | both |
| menubutton | label, iconName | | | both |
| splitbutton | label, iconName | clicked | | both |
| settingsgroup | title, description, separateRows | | | both |
| row | title, subtitle, iconName, iconData, activatable | activated | | both |
| switchrow | title, subtitle, checked | toggled | | both |
| switch | checked | toggled | focus | both |
| togglebutton | label, iconName, active | toggled | | both |
| segmentedcontrol | options, selectedIndex | selectionChanged | | both |
| linkbutton | label, uri, visited, openExternal | activate | | both |
| levelindicator | value, min, max, warningValue, criticalValue, discrete, indicatorStyle | | | both |
| colorpicker | value, supportsAlpha | colorChanged | | both |
| datepicker | value, displayStyle, minDate, maxDate | dateChanged | | both |
| fontpicker | value | fontChanged | | both |
| clamp | maximumSize, tighteningThreshold | | | both |
| overlay | (child + overlay children) | | | both |
| banner | title, buttonLabel, revealed | buttonClicked | | both |
| popover | anchor(wire id), anchorSlot, open, position | closed | | both |
| expander | label, expanded | toggled | | both |
| statuspage | iconName, title, description | | | both |
| toastoverlay | | toastButtonClicked, toastDismissed | showToast, dismissToast | both |
| dialog | open, title, contentWidth/Height, closable | closed | | both |
| sheet | open, edge, size | closed | | both |
| commandpalette | open, placeholder, query, items | queryChanged, activate, submit, cancel | | both |
| avatar | text, imagePath, size, showInitials | | | both |
| badge | label, variant, dot | | | both |
| tag | label, variant, removable | removed | | both |
| kbd | keys | | | both |
| breadcrumb | items, selectedIndex | itemActivated | | both |
| richtext | markdown, selectable | linkActivated | | both |
| chart | type, series, xLabel, yLabel, showLegend, showGrid, animated, minContentHeight | pointSelected | | both |
| codeeditor | text, language, theme, showLineNumbers, readOnly, tabWidth, diagnostics | changed, cursorMoved, diagnosticClicked | focus | both |
| video | src, autoplay, loop, controls | | | both |
| nativeview | viewKind, props (native plugin view host) | nativeEvent | | both |
| trayitem | iconName | | | macOS only |
| sharebutton | label, iconName, items | | | macOS only |
| terminal | command, cwd, fontSize, fontFamily, palette, foreground, background, cols, rows, remote, host, port, sessionId, ticket, restoreScrollback | titleChanged, bell, exited, connectionState, selectionChanged, imagePaste, focusChanged | copy, paste, selectAll, clearSelection, focus | both |
| webview | url, profile, engine, contextMenuMode, adoptPopups, popup | see section 7 | see section 7 | both |

## 2. Layout and styling

- [ ] box main axis: pack at natural size, leftover split equally among expanders (GTK semantics) (swift/Sources/NDShell/Layout.swift NDBoxView)
- [ ] box cross axis fill/start/center/end with per-kind default from `ndSelfSizedKinds` table, not live intrinsic size (tools/codegen.ts nd_self_sized_kinds)
- [ ] Box.spacing default = platform standard (8 AppKit / 6 GTK), schema sentinel -1 (schema/widgets.json)
- [ ] hexpand/vexpand/halign/valign/margin/minWidth/minHeight set-replace; dropped key falls back to kind default (src/gtk/style.zig applyAlignment)
- [ ] natural and minimum size measured and cached separately per child; wrapping label min vs natural
- [ ] invalidate up the box chain only (O(depth)); append/insert O(1) (ndInvalidateBoxChain)
- [ ] tabview measures homogeneously over all pages; scroll-shaped data widgets measure rows x row height capped at 10
- [ ] frames round outward to backing pixel grid (backingAlignedRect)
- [ ] Grid and ScrollView place children by their own mechanism, halign/valign wired through
- [ ] ndBoxPlace sibling placement (Box, StatusPage, per-slot AdwHeaderBar boxes); anchor not packed = append
- [ ] ellipsized button title never sizes its parent; clip scroll view holds no min width
- [ ] spacing scale `Spacing`, `ContentMargin`, `ContentWidth` (packages/react/src/core/metrics.ts)
- [ ] styling-design-language: native look per OS (libadwaita on Linux, AppKit/SwiftUI leaves + macOS 26 glass on mac); fix in backend, never app (CLAUDE.md)
- [ ] `navigation-sidebar` / `nd-native-sidebar` classes carry real sidebar semantics per backend
- [ ] user gtk.css theme must not break view tile, activatable hover, osd load bar (src/gtk/basecss.zig)
- [ ] one CSS provider per distinct style, not per node (perf, src/gtk/style.zig)
- [ ] appearance + accentColor in payload, dark/light change events (system.getAppearance, onAppearanceChange)
- [ ] SF Symbol mapping (~50+ names) on mac, freedesktop icon names on GTK, iconData bytes in one 16pt slot (swift/Sources/NDShell/Icons.swift, src/gtk/icons.zig)
- [ ] fonts: colour emoji at text size, terminal nerd-glyph fit (scripts/headless-fonts.sh)

## 3. Windowing and tabs

- [ ] multi-window: several `<window>` roots, independent OS windows (examples/multiwindow, scripts/multiwindow-drive.ts)
- [ ] window handle pooling by node id; `resolve_window` rebinds on crash/HMR respawn (src/tree.zig)
- [ ] per-window scoping: automation measures against own window, screenshot targets `window`, crash overlay on every window, toolbars attach to owner, `core:window.create` ACL per window id
- [ ] native system tabs via `tabGroup`: macOS NSWindow addTabbedWindow (id `nd.x`), GTK AdwTabOverview>AdwTabView scaffold windows (src/gtk/tabs.zig, swift/Sources/NDShell/WindowTabs.swift)
- [ ] tab events `newTabRequested`, `closed` with close HELD PENDING until app unmount lands; JS unmount closes natively via `window.close` semantic action
- [ ] tab drag-out/in/reorder native; GTK `create-window` spawns sibling scaffold, page transfers with webview state intact; `page-detached` never destroys or emits
- [ ] cross-group tabs never merge; `tabGroup` is create-only; `showTabOverview` command; overview new-tab button disabled on GTK by design
- [ ] cross-window reparent without reload: Portal into pool + `moveNode(ref, slot)` -> `__ndReparent` widgetCommand -> `reparent_child` op (word 21), GTK ref/unref bracket (scripts/multiwindow-drive.ts, scripts/mac/cef-reparent-drive.ts)
- [ ] `createPool`/`Portal` process-lifetime off-window pools (packages/react/src/renderer.ts)
- [ ] window toolbarStyle, frameAutosaveName, density, sheet presentation, setFullscreen, fullscreenChanged, present
- [ ] traffic lights follow toolbar band/sidebar slot; hidden toolbar of only tracking separators takes no clicks (AppKit)
- [ ] HIG chrome: headerbar, toolbarview slots + bar styles, inspector split slot, NSBackgroundExtensionView edge-to-edge, window-controls widget, app-chrome gates (scripts/app-chrome-drive.ts)
- [ ] splitview: sidebarWidth fraction for life of split, breakpoint with 8pt hysteresis (SplitController.applyBreakpoint), edgeReveal peeking sidebar, SplitMotion/SplitReveal (swift/Sources/NDShell/Split*.swift)
- [ ] `<paned>` positionChanged echo must not loop (packages/panes)
- [ ] window dialogs per window: showAlert/openFile/saveFile/showAbout, one pending per window (packages/react/src/core/dialogs.ts)
- [ ] toasts: showToast/dismissToast, id-correlated queue (packages/react/src/core/toast.ts)
- [ ] crash overlay on host while JS dead; non-fatal vs fatal runtimeError overlay text (src/runtime.zig, src/gtk/overlay.zig)
- [ ] menu bar: native accelerators, named keys (plus/minus/brackets), roles, app-menu about/hide/quit with app name, menu order (scripts/menubar-drive.ts, menu-order-drive.ts)
- [ ] context menus: innermost contextMenu wins; contextMenuSelected (src/gtk/contextmenu.zig, ContextMenu.swift)
- [ ] tray item and share button (macOS) (TrayItems.swift, ShareButtons.swift)
- [ ] popovers anchored by `anchorRef` (wire id, 0 = tree parent), stay inside their window, close with removed subtree / Esc (examples/popoveranchor, scripts/popover-anchor-drive.ts)
- [ ] command palette widget with reused rows, idle fill, layout assertions via paletteLayout (scripts/palette-layout-drive.ts)
- [ ] drag and drop between widgets and OS file drop: draggable/dragPayload/dropTarget events, `app.onFileDrop` (src/gtk/dnd.zig, DragDrop.swift)
- [ ] type-ahead in lists (src/gtk/typeahead.zig, TypeAhead.swift)

## 4. Input and automation

RPC (schema/rpc.json, 23 methods, errors -32001 notActionable, -32002 waitForTimeout, -32003 inputUnsupported, -32004 windowGone, -32601/-32602/-32603/-32700):
- [ ] getTree: refs, testIDs, text, logical geometry, role/enabled/focused/value/checked/selected/expanded/placeholder/label/options; optional `window`; children in retained sibling order
- [ ] screenshot: PNG of window (per `window`); snapshotNode: one node PNG
- [ ] click (semantic, NSApp.sendAction ~1.7 ms), doubleClick, rightClick, hover, setValue (kind-dispatched), type (append, never synthetic keysyms), scroll, focus, scrollIntoView
- [ ] waitFor host-side ~50 ms tick: one of textContains|refVisible|testId + state present|gone|visible|enabled|disabled|focused, countAtLeast, valueEquals, valueContains (scripts/waitfor-drive.ts)
- [ ] resolve: rank testID matches (actionable, key window, tree order); windows: key/main/visible/title/tabGroup/geometry; setWindowFrame
- [ ] pointer (down/move/up), drag (whole batch in ONE marshal on mac), keys (chord synthesis); GTK answers -32003 for synthesized input
- [ ] webviewInfo, webviewEval, paletteLayout, menuModel (flattened live native menu)
- [ ] actionability ladder: single match, visible (intersects ancestor clips), enabled, real geometry, stable frame
- [ ] label rejects setValue/type; slider drag maps end point to value for hosted SwiftUI slider; segmented control setValue picks option; one selectionChanged per setValue (sourcetree)
- [ ] ND_AUTOMATION_DIALOG_SCRIPT per-method FIFO JSON answering dialog.* / window.* dialogs (src/automation_dialogs.zig)
- [ ] screenshot capture modes: default in-process, `ND_AUTOMATION_CAPTURE=screencapturekit|region`; composited sheets/menus; window forced front/key (swift/Sources/NDShell/AutomationCapture.swift)
- [ ] `--nd-capture`, `--nd-input` (HID CGEvents), `--nd-grant` (SIP-off TCC rows) helper modes of the host binary (CaptureGrant.swift, InputHelper.swift)
- [ ] accessibility roles schema-declared, per-node `a11y` probe through `semantic_action` (no extra ABI op)
- [ ] automation socket: length-prefixed JSON, `NATIVE_AUTOMATION=1`, stderr markers (docs: automation-testing/automation-socket.md)
- [ ] `ndshot` tool: capture --region, focus via Accessibility, doctor --grant (tools/ndshot)
- [ ] sync keys: keys typed right after menu item / ctrl+T wait for opened field and land in order

## 5. System APIs (packages/react/src/core/system.ts, shell.ts, paths.ts)

Wire: systemRequest/systemResponse/systemEvent frames -> one `system_request` op (#22); ACL: `core:dialog`, `core:notification`, `core:recent`, `core:clipboard.write`, `core:audio` default-granted; `core:clipboard.read`, `core:credentials` need `ND_ACL_GRANTS` (`defaultWindow` or per-window `grants`) (src/acl.zig).
- [ ] dialog.openFile / saveFile / showMessage (app-level, ACL)
- [ ] clipboard.readText / writeText / readImage
- [ ] notifications.show / onClick (data echoed); bundled-app requirement on macOS
- [ ] recentDocuments.add / clear
- [ ] credentials.set / get / delete (Keychain / dlopen'd libsecret)
- [ ] app.isActive (sync, replayed before HelloAck), onActivate, onDeactivate, onOpenUrl, onOpenFile, onFileDrop
- [ ] system.getAppearance / onAppearanceChange (appearance, accentColor)
- [ ] audio.play({path|url, volume, spectrum}) -> handle; pause/resume/stop/seek/setVolume; onState; onSpectrum (32 log bins, ~15 Hz) (Audio.swift, src/gtk/audio.zig GStreamer dlopen)
- [ ] openExternal / openPath / revealPath (pure TS, unsandboxed)
- [ ] getAppDataDir / ensureAppDataDir / getCefProfileDir (macOS ~/Library/Application Support/<name>, Linux $XDG_DATA_HOME/<name>)
- [ ] hasWidget(type) / hasCommand(type, cmd) from helloAck host manifest; Platform.os / Platform.backend
- [ ] file associations + URL schemes injected at package time (Info.plist, .desktop + shared-mime XML)
- [ ] survivable error policy: onUnhandledError, setUnhandledErrorPolicy, rate cap 20/10s, ND_FATAL_REJECTIONS (packages/react/src/core/errors.ts)
- [ ] ACL core (src/acl.zig) and per-window grants
- [ ] native plugins: `nd_plugin.h` ABI v2, pluginCommand/pluginResult, `<nativeview>`, defineNativeComponent, `ND_NATIVE_PACKAGE`, plugins/colorview, plugins/hello (src/plugin.zig, examples/nativeview-demo, scripts/m10-plugin-drive.ts)
- [ ] update system: signed update manifest, verify, update server (src/core/update.zig, tools/update-server.ts, packages/nd/src/package/updates.ts)

## 6. Rendering protocol and runtime

- [ ] two processes: native host owns main/event loop/retained tree; Bun child runs app; JS crash/hang leaves window up (src/runtime.zig)
- [ ] NDP frames (schema/protocol.json): hello, helloAck, error, event, commitBatch, ping, pong, runtimeError, pluginCommand, pluginResult, widgetCommand, systemRequest, systemResponse, systemEvent
- [ ] ops: create, append, insertBefore, remove, setText, update, hide, unhide
- [ ] binary commit encoder, negotiated via helloAck `encodings` (packages/react/src/core/ndp-binary.ts, src/ndp_binary.zig); gates scripts/bench-10k.ts, bench-solid.tsx
- [ ] inbound framing via head/tail offsets, no Buffer.concat
- [ ] host: decode+gate commit on reader thread, UI thread only tree.apply; writer thread for outbound frames; input event holds UI thread up to 8 ms for app reply
- [ ] reconciler (src/tree.zig): generation GC, ownership one strong ref per node, `release_node` word 23, getTree safe against freed GObject
- [ ] ABI vtable append-only, layout assert (`23 * sizeOf(usize)`), header sync (scripts/sync-native-headers.sh, include/nd.h)
- [ ] renderer: Solid 2.0 universal; props set before create fold into create; same-tick updates merge per node; undefined prop = removal; propsEqual deep-by-value (4 levels); batch ships updates after inserts/removals (packages/react/src/renderer.ts)
- [ ] widget commands call session.flush() first; sendCommand/sendNativeCommand/moveNode
- [ ] page events render at default priority (one render per load burst)
- [ ] HMR: Solid refresh, state-preserving, pinned solid-js/@solidjs/*, unpatchable edit remounts (scripts/counter-hmr-drive.ts)
- [ ] `nd-solid-build` ahead-of-time single-bundle compile; register.ts Bun plugin (packages/react/src/register.ts, build.ts)
- [ ] codegen from schemas for Zig, Swift, TS (tools/codegen.ts); stub registry for mid-implementation widgets
- [ ] null backend + conformance suite (src/null_backend.zig, src/conformance.zig)
- [ ] perf tracing (src/gtk/perftrace.zig, scripts/perf-drive.ts, switch-latency-drive.ts)

## 7. Webview and CEF (docs/webview.md)

Engines: WKWebView (mac), WebKitGTK (Linux), CEF (opt-in, loaded at runtime, zero Chromium bytes unless config enables).

Rust status (M8/M9, 2026-10-14): `nd-webview` (WKWebView) and `nd-cef` (CEF 154 off-screen) on macOS pass `gate_m8_webview::parity_*` (every event and command below that WebKit or Chromium can produce, against a local page), `real_overlay_leak_*`, `real_resize_{wkwebview,cef}` and `gate_m9_cef_package::*` (zero Chromium bytes without CEF; bundled CEF app runs the parity scene). Not ported yet: Chrome style (extensions, native messaging, 1Password, install prompts, Chrome dialogs, browserCommand chords), content blocking, tab parking, picture in picture, CEF session save/restore and cookiesChanged, WebKitGTK/WPE and WebView2 (stubs that report the missing engine), CEF on Linux and Windows.
- [ ] props url, profile, engine, contextMenuMode(native|suppress), adoptPopups, popup
- [ ] events navigate, titleChanged, loadingChanged, focusChanged, backAvailable, forwardAvailable, loadProgress, loadFailed, renderProcessGone, newWindow, browserCommand, downloadRequested, downloadUpdated, javaScriptResult, scriptMessage, schemeRequest, cookiesResult, cookiesChanged, faviconChanged, findResult, securityChanged, linkHover, contextMenu, contextMenuItemClicked, sessionSaved, audioStateChanged, extensionsList, extensionActions, extensionsChanged, chromeDialog, permissionRequest, permissionRequestDismissed, zoomChanged, pictureInPicture, fullscreenChanged, contentBlocked, externalProtocol, popup
- [ ] commands goBack, goForward, reload, stop, executeJavaScript, setZoom, setUserAgent, openDevTools, addUserScript, removeUserScript, clearUserScripts, registerScriptMessage, unregisterScriptMessage, respondScheme, respondPermission, resetPermissions, allowPopups, respondDownload, startDownload, pauseDownload, resumeDownload, cancelDownload, getCookies, setCookie, deleteCookie, findStart/Next/Previous/Stop, saveSession, restoreSession, setMuted, setContextMenuItems, listExtensions, watchExtensions, listExtensionActions, readExtensionAction, triggerExtensionAction, installExtension, uninstallExtension, setExtensionEnabled, acceptExtensionInstall, exitFullscreen, requestClose, focus
- [ ] app API wrappers: executeJavaScript, getCookies, saveSession, setContextMenuItems, extension and download APIs, newWindowRequest, openBlockedPopup (packages/react/src/core/webview.ts)
- [ ] context menu merged into engine menu in place, per-context/per-URL filters, radio/checkbox/submenu items
- [ ] newWindow denied by host, app opens a tab; with adoptPopups the popup browser waits for `<webview popup={id}>`; window.open keeps opener; gesture-free popups blocked; blocked popup reopens with opener
- [ ] webviewEngine.active() / cefStyle(); resolveWebViewEngine per platform; CEF styles (alloy vs chrome) (packages/nd/src/config.ts)
- [ ] CEF loader: dlopen, version pin (DEFAULT_CEF_VERSION), bundled dist, nd-cef-helper, libcef strip on Linux (packages/host/src/cef.ts, src/cef/*)
- [ ] CEF Chrome-style: extensions (framework-extension), action popups, native messaging hosts from other browsers, 1Password, install prompt auto-answer, downloads owned by app (no Chrome bubble/animation)
- [ ] CEF dialogs tab-modal (script, leave-site, sign-in, file chooser transient to window); permission decisions stick per origin; screen share picker + portal on Wayland; getDisplayMedia preferCurrentTab
- [ ] CEF page fullscreen/F11, document picture-in-picture as pinned window, devtools docked inspector, zoom bubble off on Linux
- [ ] CEF tab parking: last 3 tabs live 2 min, parked window minimized and keeps last frame, switching moves page instead of resize
- [ ] CEF input: keyboard focus follows last explicit request; parked/hidden/under-32px view never holds input focus; declared accelerators pre-empt page; Chrome chords surface as browserCommand
- [ ] CEF shutdown ordered (hostOf), signals ignored by chromium subprocesses, container destroyed after browser closes
- [ ] CEF profile dir (`ND_CEF_CACHE`), mock keychain for test hosts, navigator.webdriver false, TLS state from visible entry
- [ ] content blocking: nd-adblock Rust crate + cosmetic.js, main world only (crates/nd-adblock, src/adblock.zig, include/nd_adblock.h, webviewEngine.contentBlocking.*)
- [ ] user scripts, scheme handlers, script messages, cookies, find in page, session save/restore, per-profile data, favicon (SVG via gtk_svg fallback, texture cache), load progress bar (osd 2 px line)
- [ ] Hyprland/X11/Wayland window-class + shape handling for chromium popups (src/cef/hyprland.zig, x11.zig, shape.zig)
- [ ] not exposed: request interception (note, still absent)

## 8. Terminal (libghostty-vt, vendor/libghostty-vt, src/core/terminal.zig)

- [ ] `<terminal>` local pty (command, cwd), font size/family, palette, fg/bg, cols/rows, scrollback restore
- [ ] renderers: GTK (src/gtk/terminal.zig, sprite.zig) and AppKit (NDTerminalView.swift, NDTerminalSprites.swift); font metrics measured, baseline draw, nerd glyph scale-to-fit
- [ ] events titleChanged, bell, exited, connectionState, selectionChanged, imagePaste, focusChanged; commands copy, paste, selectAll, clearSelection, focus
- [ ] remote terminal: host/port/sessionId/ticket, byte-lane keepalive + rcvtimeo heartbeat + interruptible dial/backoff (src/core/remote_terminal.zig, include/ndremote.h, ndterm.h)
- [ ] remote session gates: scripts/remote-terminal-drive.ts + fake server

## 9. Data, store, rpc, panes, ui packages

- [ ] @nativedesktop/data: openDatabase -> query/mutate/transaction/close in Bun Worker; SqliteDatabase; types OpenOptions, RunResult, SqlParams, SqlValue, TxStep; SqliteExecutor contract; `./solid` createQuery (packages/data/src)
- [ ] ORM adapters tested: Drizzle sqlite-proxy, Kysely custom Dialect (packages/data/src/adapters.test.ts)
- [ ] createStore / useStoreValue: versioned `${name}.json`, debounced serialized writes, tmp+rename, sync flush on exit/SIGINT/SIGTERM, migrate on every load, corrupt file backed up (packages/react/src/core/store.ts)
- [ ] @nativedesktop/rpc: RpcClient, RpcError, RpcContract, socketTransport, webSocketTransport, ConnectionLadder (backoffDelayMs, RECONNECT_BASE_MS/MAX_MS, STABILITY_WINDOW_MS); `./solid` createRpcStatus; `@nativedesktop/react/core/rpc`
- [ ] @nativedesktop/panes model ops: splitPane, closePane, focusNeighbor, focusPane(At), setPaneRatio, clampPaneRatio, seedPanes, migratePanes, paneLeaves, paneIdAt, samePaneShape, emptyPanes; same-reference-when-unchanged contract
- [ ] PaneTree/createPaneTree; DockView/createDock (addTab, closeTab, activateTab, applyDockDrop, dockDragPayload, serialize/deserialize); TilesView/createTiles (placeTile, moveTile, resizeTile, raiseTile, applyTileDrop, serialize)
- [ ] @nativedesktop/ui: Accordion, ButtonGroup, DescriptionList, Form/FormField, HoverCard, OtpInput, Pagination, SearchableList, StatusBar, Stepper + pure helpers (computePaginationRange, stepState, filterItems, otpCellChanged, nextExpandedIds)
- [ ] @nativedesktop/react exports: ./build, ./core, ./core/rpc, ./jsx-runtime, ./register; render, Portal, createPool, nextCommit, Activity, defineNativeComponent; typed intrinsics
- [ ] app model: single `.tsx` entry, Bun preload `--preload=@nativedesktop/react/register` via BUN_OPTIONS

## 10. CLI, packaging, release

- [ ] `nd dev [entry] [--backend gtk|appkit]` (ND_DEV=1, HMR, crash overlay; ND_BACKEND; ND_HOST_BINARY override)
- [ ] `nd build` (bun run compile); `nd doctor [--json]` toolchain readiness (incl. nix-ld mix warning)
- [ ] `nd package [mac|linux] [--out --entry --version --cwd --no-compile --sign/--no-sign --notarize/--no-notarize --format appimage|appdir]` (packages/nd/src/package/*)
- [ ] mac: .app bundle, Info.plist/entitlements, codesign, notarize, layered Icon Composer icons (iconcomposer.ts, icons.ts, mac.ts)
- [ ] linux: AppImage/AppDir, .desktop, flatpak manifest, bundled gtk4 gsettings schemas, libcef strip (linux.ts, packaging/*)
- [ ] identity: app id, fileAssociations, urlSchemes; modules.ts native module build (buildNativePlugins)
- [ ] config: defineConfig/loadConfig `nativedesktop.config.ts` (app, package.mac/linux, webview engine, cef, native plugins, updates) (packages/nd/src/config.ts)
- [ ] `@nativedesktop/host` resolveHostBinary: prebuilt optionalDeps host-darwin-arm64 / host-linux-x64, in-checkout build, buildCefHelper; hostBinaryCandidates; preload helpers (findAppDir, usesSolid, withPreload)
- [ ] template app (template/), scripts/new-app.sh
- [ ] release workflow: v* tags, macos-26 build, ubuntu:26.04 container (libadwaita 1.7), lockstep version check, skip already-published, raw binaries on GitHub release (.github/workflows/release.yml, scripts/release/check-versions.ts)
- [ ] CPU baselines: darwin apple_m1, linux x86_64_v2
- [ ] version bump committed with `bun install` (workspace:* lock rule)
- [ ] docs/releasing.md procedure; every confirmed fix ships a patch release

## 11. Testing and MCP (packages/test, packages/mcp)

- [ ] launchApp / AppHandle / killAll / connectApp (AttachedApp); stderr marker parsing
- [ ] AutomationClient (ONE copy of wire framing), TimedClient, AutomationRpcError
- [ ] Locator API: getByTestId/getByRole/getByText/getByLabel/getByPlaceholder/locator; lazy re-resolve, one stale-ref retry; actions click/dblclick/fill/type/focus/scrollIntoView, isChecked/isSelected etc.
- [ ] expect(locator) polling matchers (LocatorAssertions, ValueAssertions)
- [ ] selector grammar: `role=button[name="Save"][exact] >> nth=1`, testid=, text=, label=, placeholder=, type= (parseSelector/formatSelector)
- [ ] app.keyboard, app.mouse, app.cursor (real HID cursor), app.window(titleOrIndex), app.setWindowSize, app.screenshot
- [ ] poll, dialogScriptEnv, takeScreenshot, pngSize, renderSnapshot, resolveTarget, findNode(s)
- [ ] LocatorError / StrictModeError / TimeoutError with candidate ranking
- [ ] MCP bridge `nd-mcp`: tools execute({code}), snapshot, reset; env ND_MCP_ENTRY, ND_MCP_BACKEND, ND_AUTOMATION_SOCKET reconnect; launches with region capture
- [ ] gates (drive scripts; 59 *-drive.ts files, ~11k lines): counter, counter-hmr, gestures, keys-menu, locator, waitfor, harness, dialog-script, errors, propreset, popover-anchor, sourcetree, headerfield, command-palette, palette-layout, notes, panes, parity, settings, multiwindow, tabs, webview, cef (m1, zoom, menu, portal, relaunch, chrome), adblock, native-messaging, remote-terminal, m2..m10 milestone drives, menubar, menu-order, app-chrome, app-escape, blur-activate, decoration, region-capture, cursor, perf, switch-latency, autosize, arcchrome, threepane
- [ ] shell gates: scripts/headless-*.sh (Linux headless via compositor), scripts/mac/*.sh (mac-gestures -> MAC_GESTURES_OK, ND_LOCATOR_OK, ND_TABS_OK, ND_REGION_CAPTURE_OK, ND_CURSOR_OK), gettree-soak.sh, kill9-test.sh, browser-gate.sh

## 12. Docs (docs-site/src/content/docs, 53 pages + docs/*.md)

- [ ] index.mdx, packaging.md, packaging/app-icons.md
- [ ] get-started: examples, installation, introduction, monorepo, project-layout, quick-start, tutorial-counter, tutorial-settings, tutorial-terminal
- [ ] core-concepts: app-data-storage, app-model, architecture, automation-first, error-handling, imperative-commands, layout, rpc-client, state-hot-reload, styling-design-language
- [ ] components: charts, code-editor, command-palette, composition-components, data-views, dialogs, display-navigation, drag-and-drop, feedback, form-controls, menus-and-popovers, overlays, overview, pickers, progress-loading, rich-text, terminal, video, webview, widget-reference
- [ ] native-platform: icons, menu-bar, multi-window, native-modules, platform-support, split-views, system-capabilities, tabs, windows-chrome
- [ ] automation-testing: automation-socket, locators, mcp-tools, test-harness
- [ ] repo docs: docs/webview.md, widgets.md, styling.md (codegen-generated), native-components.md, packaging.md, releasing.md, runtime-deps.md, docs/agents/{README,automation,styling}.md
- [ ] widget-reference + docs/styling.md generated from schema (no hand edit)
- [ ] docs screenshots pipeline (scripts/docs-shots.ts, headless-docs-shots.sh)

## 13. Examples (examples/*, one line each)

- [ ] adblock-probe: content-blocking verdict probe
- [ ] arcchrome: Arc-style browser chrome (sidebar tabs, palette)
- [ ] browser: full browser app on `<webview>`
- [ ] cef-probe: CEF engine bring-up probe
- [ ] command-palette: command palette + header probe
- [ ] counter: minimal Solid counter (HMR + autosize gates)
- [ ] dialogs: window and app-level dialogs
- [ ] errors: error policy / crash overlay demo
- [ ] gallery: every widget; packaged by nd package (nativedesktop.config.ts)
- [ ] gestures: pointer/drag/keys targets
- [ ] headerfield: header search field
- [ ] inspector: tree inspector app
- [ ] locators: locator-driven target app
- [ ] multiwindow: multi-window + pool reparent
- [ ] nativeview-demo: native plugin view (colorview)
- [ ] ndwidgets-probe: widget expansion probe
- [ ] notes: notes app (menubar, chrome, menu-order probes)
- [ ] panes: split/dock/tiles
- [ ] parity: GPUI parity matrix app (docs/plans/gpui-parity.md)
- [ ] popoveranchor: popover anchorRef
- [ ] propreset: dropped-prop reset
- [ ] remote-terminal: remote terminal client
- [ ] settings: settings with store + settingsgroup
- [ ] sourcetree: sourcetree sidebar
- [ ] tasks: tasks app with data/SQLite
- [ ] terminal: terminal app
- [ ] webview-probe: cef-chrome, cef-m1, native-messaging, reparent, scheme probes

## 14. Behaviours that were hard-won (must not regress)

Focus and keyboard
- [ ] activate on Return only, never on focus loss (appkit text field)
- [ ] keys typed right after ctrl+T or an app menu item wait for the field it opens, land in order
- [ ] `focus` command hands keyboard to the page's render widget; only the active window takes keyboard back from its page; parked/hidden/tiny pages never hold X focus
- [ ] Esc closes popover from anchor's window; Esc in dialog search field closes it; dialog keeps menu accelerators
- [ ] Chrome chords and declared accelerators (symbols, Tab, nav keys) pre-empt the page, surface as browserCommand
- [ ] real-cursor press types printable keys by character on current layout; fails naming the other app that would take the press
Popovers
- [ ] popover stays inside its window, placed from anchor not last position; waits for unmapped anchor; clipped by popover not window; content does not make anchor expand
- [ ] mac popover panel tinted glass, 4.5:1 label contrast on light glass; fits content when child min size changes; closes with removed subtree
- [ ] page in a popover takes clicks and keys (GTK)
- [ ] update ops ship after inserts/removals so a popover opens over its new content
Drag and tabs
- [ ] tab transfer keeps webview state; page-detached never destroys; close held pending until unmount; cross-group never merges
- [ ] mac drag posts down/dragged/up in ONE marshal (AppKit tracking loop deadlock); hosted SwiftUI slider drag maps to value
- [ ] pane/dock/tile drops return same reference when nothing changed (no native echo loop)
- [ ] reparent moves live widget; removal/recreate would reload webview
Rendering and layout
- [ ] children order: emptied expression refills in own slot, not after neighbours (react renderer)
- [ ] prop set to undefined reaches host as removal; style becomes {}, cssClasses []
- [ ] new button never inherits freed one's icon name; dropped iconData clears image; GTK iconData button toggles icon-only/with-label
- [ ] flat/ellipsize: only ellipsize-marked button gives up title width
- [ ] load bar sits on page top edge, takes no clicks, hides after fade so frame clock idles (GTK)
- [ ] sidebar slot refills without animating from 0 px; traffic lights track toolbar band / peeking sidebar
- [ ] toastoverlay-wrapped splitview installs as contentViewController or sidebar subtree is orphaned
- [ ] scrollview paints no background so glass shows through sidebar
- [ ] right-click opens innermost contextMenu; view contextMenu wins over toolbar menu
Lifetimes and threads
- [ ] backend holds one strong ref per node, released via release_node; GTK handles were floating before and getTree probed freed GObject
- [ ] file dialog paths duped before glib.free; failed webkit eval message serialized before free
- [ ] core allocates with malloc (shared with AppKit host); getpid via libc
- [ ] plugin result validated as JSON before splicing into pluginResult frame
- [ ] runtime replays engine + activation state ahead of helloAck so first render reads them
- [ ] CEF: browsers closed before container destroyed (30 s GPU stall otherwise); ordered shutdown never waits 5 s on unasked close; chromium children ignore SIGTERM/SIGINT
Startup and perf
- [ ] chromium initializes while Bun child loads; GTK icon-theme reload on worker thread; one bundled file instead of ~50 on launch
- [ ] tooltips set without querying display pointer; command palette builds rows on open only
- [ ] menu models rebuild once per main-loop pass; favicon textures cached by bytes
Packaging and release
- [ ] nd-solid-build bundles react + core so packaged app never runs a second solid-js; register preload loads other solid copies as its own
- [ ] release builds target apple_m1 / x86_64_v2 (SIGILL on non-AVX512 otherwise)
- [ ] layered icon groups written front to back
- [ ] nd-cef-helper shipped in darwin host package; libcef.so always stripped on Linux, fail if no stripper
Automation
- [ ] getTree recurses through untracked wrapper widgets; unrooted widget has no bounds; children follow retained sibling order
- [ ] snapshotNode renders through the rect getTree reports; mac screenshot uses fresh bitmap reps (no ghosting)
- [ ] gates tolerate machines with no crash reports; mac gates read system crash dir
