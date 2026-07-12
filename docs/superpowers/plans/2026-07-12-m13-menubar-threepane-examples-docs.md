# M13 — Menu bar, three-pane SplitView, Notes rework, examples, Starlight docs

> Architect-authored design spec for the M13 implementation waves. Implementation agents: read this
> whole file before touching code. The architecture facts referenced here (codegen emit points, vtable
> flow, automation putMeta precedent) were verified against HEAD 99c759c on 2026-07-12.

## Goal

One React app, zero platform-specific code, adopts BOTH 2026 desktop design languages:

- **macOS 26 (Liquid Glass)**: real `NSApp.mainMenu` menu bar with the standard default menus
  (App/File/Edit/View/Window/Help, responder-chain selectors) that apps extend declaratively; SF
  Symbol icons on menu items (`preferredImageVisibility = .visible` — macOS 27 hides symbol images
  by default); `NSSplitViewController` three-pane (`.sidebar` / `.contentList` / detail) with the
  toolbar split by `NSTrackingSeparatorToolbarItem` per divider.
- **GNOME (Adwaita 2026)**: the SAME `<menubar>` declaration renders as a primary (hamburger) menu —
  `GtkMenuButton` + `GMenu` per HIG (GNOME Shell never shows a top menubar; icons in popover menus
  deliberately do not render and are omitted); three-pane = nested `AdwOverlaySplitView`
  (folders / list / content, the Files-style pattern).

Plus: Apple-Notes-style notes example on that new machinery, two new examples, a Starlight docs
site under `docs-site/`, and the `ndshot` TCC-proper screenshot utility (separate agent, already
dispatched, `tools/ndshot/`).

## Hard rules (carry-overs, non-negotiable)

- Schema is **append-only** (positional widget-type IDs): new widgets go at the END of
  `schema/widgets.json`'s `widgets` array; new enum values at the END of their `values` array; new
  props at the END of the widget's `props` array. After any schema/codegen change run
  `bun tools/codegen.ts` and commit the regenerated files; never hand-edit generated files.
- Both backends must build and their drives stay green: GTK runs natively on this Mac via quartz
  (`nix develop -c zig build`), mac leg via `./scripts/mac/mac-m11.sh` (recipe: `zig build libnd
  -Dbackend=abi` → repack → `env -u SDKROOT -u DEVELOPER_DIR swift build`).
- GTK-linked test binaries print spurious `failed command … --listen=-` lines then pass on rerun —
  trust exit codes only.
- Screenshot assertions don't work on mac (offscreen render misses `_NSCoreHostingView`) — assert
  geometry/structure via automation `getTree`; the owner confirms visuals headful.
- No git commits from implementation agents — leave the working tree dirty; the architect commits
  after review. Never push to GitHub; local transport pushes (`~/nd.git`) are architect-only.
- Import hooks from `@nativedesktop/react`, never `react` (HMR contract).

---

## Feature A — Menu bar (`<menubar>` / `<menu>` / `<menuitem>`)

### Schema additions (append after SearchInput)

```json
{
  "name": "Menubar",
  "intrinsic": "menubar",
  "container": { "childModel": "multi" },
  "props": [
    { "name": "defaults", "type": "bool", "default": true, "appliesTo": "create" },
    { "name": "testID", "type": "string", "appliesTo": "meta" }
  ],
  "events": [],
  "automation": { "role": "menubar", "textFrom": null }
},
{
  "name": "Menu",
  "intrinsic": "menu",
  "container": { "childModel": "multi" },
  "props": [
    { "name": "label", "type": "string", "default": "", "appliesTo": "create" },
    { "name": "testID", "type": "string", "appliesTo": "meta" }
  ],
  "events": [],
  "automation": { "role": "menu", "textFrom": "label" }
},
{
  "name": "MenuItem",
  "intrinsic": "menuitem",
  "container": null,
  "props": [
    { "name": "label", "type": "string", "default": "", "appliesTo": "create" },
    { "name": "iconName", "type": "string", "appliesTo": "create" },
    { "name": "accelerator", "type": "string", "appliesTo": "create" },
    { "name": "role", "type": "enum", "values": ["none", "separator", "about", "settings", "quit", "undo", "redo", "cut", "copy", "paste", "delete", "selectAll", "close", "minimize", "zoom", "fullscreen"], "default": "none", "appliesTo": "create" },
    { "name": "enabled", "type": "bool", "default": true, "appliesTo": "createAndUpdate" },
    { "name": "testID", "type": "string", "appliesTo": "meta" }
  ],
  "events": [
    { "name": "selected", "ndpName": "onSelect" }
  ],
  "automation": { "role": "menuitem", "textFrom": "label" }
}
```

JSX shape (Window's structural arm routes by type, like ToolbarView's isA(HeaderBar) precedent —
`<menubar>` is a Window child alongside the content child; Window keeps childModel single? NO:
Window's structural template gains a type branch: a Menubar child registers as the app menu, any
other child is the content. Window's `container.childModel` stays `"single"` in the schema — the
menubar is chrome, not content; enforce leniently (menubar + one content child both accepted).

```tsx
<window title="ND Notes">
  <menubar>
    <menu label="File">
      <menuitem label="New Note" iconName="document-new" accelerator="primary+n" onSelect={createNote} />
    </menu>
    <menu label="Note">
      <menuitem label="Pin" accelerator="primary+p" onSelect={togglePin} />
      <menuitem role="separator" />
      <menuitem label="Delete" iconName="edit-delete" accelerator="primary+backspace" onSelect={deleteNote} />
    </menu>
  </menubar>
  <splitview>…</splitview>
</window>
```

### Semantics (both platforms)

- **Item behavior**: an item has EITHER a `role` (native behavior) OR `onSelect` (custom event). If
  both are set, `onSelect` wins and the role contributes nothing but documentation. `role="separator"`
  renders a native separator; label/icon/accelerator ignored for separators.
- **Defaults**: the default menu chrome exists even when the tree has NO `<menubar>` (installed at
  window create on mac; on GTK "no menubar declared" = no primary menu button, which is legitimate
  GNOME). `<menubar defaults={false}>` opts out of merging (mac: only App menu + declared menus;
  GTK: only declared content + About).
- **Merge rule (mac)**: declared `<menu label>` matching a default top-level title (File, Edit,
  View, Window, Help) → its items are appended to that menu after a separator; any other label →
  new menu inserted after View (i.e. before Window), in declaration order.
- **Accelerator grammar**: `mod+…+key`, mods ⊂ {`primary`, `shift`, `alt`, `ctrl`}, key = one
  printable char or {`enter`, `escape`, `backspace`, `delete`, `space`, `tab`, `f1`…`f12`,
  `left`, `right`, `up`, `down`, `comma`, `period`}. `primary` = ⌘ on mac, `<Primary>` (Ctrl) on GTK.
- **Icons**: `iconName` is a freedesktop name (same convention as Button). mac: resolve via
  `ndSFSymbol(forFreedesktop:)` (extend the map in `swift/Sources/NDShell/Icons.swift` as needed) →
  `NSImage(systemSymbolName:)` → set `item.image` and `preferredImageVisibility = .visible` (guard
  `#available`). GTK: **ignored by design** (GtkPopoverMenu does not render item icons; GNOME HIG
  discourages them) — no warning, documented behavior.
- **enabled**: createAndUpdate. mac: stored flag consulted from `NSMenuItemValidation
  validateMenuItem` (custom items) / GTK: `g_simple_action_set_enabled`.

### macOS implementation

New hand-written `swift/Sources/NDShell/MenuBar.swift` (NDGen+NDShell are one target):

- `NDMenuManager` (process-global, like NDToolbarManager): builds the standard default main menu at
  window create using the canonical programmatic pattern — **first top-level item is always the App
  menu** (title ignored at runtime), then File / Edit / View / Window / Help. Assign
  `NSApp.windowsMenu` and `NSApp.helpMenu`. Standard selectors/key-equivalents:
  - App: About (`orderFrontStandardAboutPanel(_:)`, target NSApp) · sep · Hide `h` (`hide(_:)`) ·
    Hide Others `⌥⌘h` (`hideOtherApplications(_:)`) · Show All (`unhideAllApplications(_:)`) · sep ·
    Quit `q` (`terminate(_:)`).
  - File: Close `w` (`NSWindow.performClose(_:)`).
  - Edit: Undo `z` / Redo `⇧z` (declare an `@objc protocol` for `undo(_:)`/`redo(_:)` — not on
    NSResponder formally) · sep · Cut `x` / Copy `c` / Paste `v` / Delete (`NSText` selectors,
    target nil) · Select All `a`.
  - View: Enter Full Screen `⌃⌘f` (`toggleFullScreen(_:)`).
  - Window: Minimize `m` (`performMiniaturize(_:)`) · Zoom (`performZoom(_:)`) · sep · Bring All to
    Front (`arrangeInFront(_:)`, target NSApp).
  - Help: empty menu assigned to `NSApp.helpMenu`.
- Menubar/Menu/MenuItem create arms (codegen `genSwiftCreateBody`) construct lightweight NDShell
  model objects (e.g. `NDMenuNode` class instances) registered by nodeID; structural arms
  (`SWIFT_STRUCTURAL`) assemble NSMenu/NSMenuItem and hand the finished tree to `NDMenuManager`
  for merge. Rebuild-on-change is fine (full-recreate precedent: NDToolbarManager).
- Custom items: `target = NDMenuDispatcher.shared`, action fires
  `nd_emit_event(gCtx, nodeID, "selected", "{}")` (same path as Events.swift).
- Role items: selector table above, `target = nil` (responder chain) except App-menu roles
  (target NSApp). This makes Edit>Copy etc. work in every text field for free.
- Automation: menu nodes are **putMeta-only** in the retained tree (the registerOverlayNode
  precedent — node surfaces in getTree with `visible:false`, no geometry). `semanticClick` on a
  MenuItem ref must work: custom item → dispatcher fire; role item → `NSApp.sendAction(selector…)`.
  Guard EVERY generic vtable op (apply_style, set_visible, geometry, setText) so a menu nodeID
  no-ops instead of casting to NSView.

### GTK implementation

- Menubar/Menu/MenuItem create arms build `gio.Menu` / `gio.MenuItem` wrappers (all GObjects — use
  `gobject.ext.isA(x, gtk.Widget)` guards in every generic op so menu handles never get cast to
  GtkWidget; same guard requirement as mac).
- Custom items: one `gio.SimpleAction` per item named `nd-menu-<nodeId>`, added to the
  `gtk.Application` action map (`global_app` — `src/gtk/backend.zig` `setApp` singleton), activate
  handler calls the existing `emitEventAdapter` path with `"selected"`. Accelerators via
  `gtk.Application.setAccelsForAction("app.nd-menu-<id>", "<Primary>n")`.
- Roles on GTK: `about` → present an about dialog (AdwAboutDialog if present in the vendored adw1
  bindings, else gtk.AboutDialog) with the window title as program name; `settings` → item labeled
  "Preferences" that emits `selected`; `separator` → GMenu section break; **all other roles are
  dropped** (quit/close/minimize/undo/cut/copy/… duplicate window chrome / system conventions —
  GNOME HIG). A menu whose items all drop is omitted entirely.
- Rendering: `<menubar>` materializes as a **primary menu button** (`GtkMenuButton`,
  `open-menu-symbolic`, tooltip "Main Menu") appended to the END slot of the LAST headerbar in
  document order (by GNOME convention that is the content pane's header — document this v1 rule).
  No headerbar in the tree → fallback `gtk.Application.setMenubar` (in-window strip). Menu model:
  each declared `<menu>` → submenu entry; `role="separator"` items split sections; bottom section
  gets "Preferences" (if a settings role exists anywhere) then "About" (when `defaults` is true).
- `semanticClick` on MenuItem → `g_action_group_activate_action` on its SimpleAction (about/settings
  routed the same way).

### Files touched (per the architecture map)

`schema/widgets.json` · `tools/codegen.ts` (genZigCreateBody/genZigApplyBody/STRUCTURAL +
genSwiftCreateBody/genSwiftApplyBody/SWIFT_STRUCTURAL + SIGNALS/SWIFT_SIGNALS for `selected`) ·
regenerated files (`bun tools/codegen.ts`) · new `swift/Sources/NDShell/MenuBar.swift` ·
`swift/Sources/NDShell/Icons.swift` (map additions) · `src/gtk/backend.zig` (menu registry, guards,
semanticClick arm, primary-button injection) · `src/automation.zig` only if a putMeta-only node
breaks an assumption (map says it won't) · conformance/null-backend: generic, no changes.

**Wire caveat**: menu widgets ride the existing create/append ops — the C ABI (`include/nd.h`,
18 vtable fns) must NOT change. `runtime/ndp-binary.ts` + `widget-types` are regenerated; run
`bun test` in packages/react and runtime to catch positional-encoding drift.

## Feature B — Three-pane SplitView

- Schema: SplitView `slot` enum values become `["sidebar", "content", "list"]` ("list" appended
  LAST); new prop `listWidth` (float, default 0.0, create) appended after `collapsed`.
- GTK structural arm: lazily create an inner `AdwOverlaySplitView` on first `list` child —
  outer.sidebar = sidebar child; outer.content = inner; inner.sidebar = list child; inner.content =
  content child. Two-pane apps (no `list` child) keep today's exact behavior. `listWidth` applies
  to the inner split the same way `sidebarWidth` applies to the outer.
- mac structural arm three-way branch: sidebar → `NSSplitViewItem(sidebarWithViewController:)` at
  index 0; list → `NSSplitViewItem(contentListWithViewController:)` after the sidebar (index = 1 if
  sidebar present else 0), `minimumThickness` 240, `canCollapse` true; content → default item
  appended last. Wrap each in `ndMakePaneViewController` (safe-area precedent). `listWidth` → same
  fraction-constraint mechanism as the existing sidebar width.
- `NDToolbarManager` (`swift/Sources/NDShell/HeaderBar.swift`): three buckets (sidebar/list/content)
  and a `NSTrackingSeparatorToolbarItem` per divider (index 0 and 1) when three panes exist;
  two-pane behavior byte-identical.
- Regression gates: counter/gallery drives + existing notes drive must stay green BEFORE the notes
  rework lands (two-pane path untouched).

## Feature C — Notes example rework (Apple Notes style)

`examples/notes/main.tsx` becomes three-pane + menu bar. Structure:

- `<menubar>`: defaults on; File → New Note (`primary+n`, `document-new`); Note menu → Pin/Unpin
  (`primary+p`), separator, Delete (`primary+backspace`, `edit-delete`).
- Sidebar pane (glass): folders — All Notes / Personal / Work / Trash as `navigation-sidebar`
  flat buttons with `labelAlign="start"`, counts in the label; headerbar with app title.
- List pane: headerbar + `<searchinput>`, scroll list of note-row buttons (title only, flat,
  selection = suggested-action), count caption at bottom.
- Editor pane: headerbar carrying the **floating editing buttons** (icon-only, `flat`, end slot:
  pin, delete, new-note — this is the "floating editor controls" ask; they live in the pane's
  header/toolbar, which on mac is the unified NSToolbar section above the editor pane and on GTK
  the pane's AdwHeaderBar) + title `<textinput>` + `<textarea>` + word-count/saved caption.
- Note model gains `folderId`; keep the pinned sort, the `key={selected.id}` remount workaround,
  and existing testIDs where the widget survives (`note-row-*`, `search-input`, `editor-textarea`,
  `title-input`, `new-note-button`, `delete-note-button`).
- `scripts/notes-drive.ts` updates: keep ND_NOTES_OK, ND_PIN/UNPIN_YORDER_OK, ND_CHROMEGEOM_OK,
  ND_NAVCHROME_OK; add ND_THREEPANE_OK (x-order: sidebar row < list row < editor textarea, all
  three panes ≥ 150pt wide) and ND_MENU_NEWNOTE_OK (getTree finds the File>New Note menuitem,
  semanticClick it, note count increments). Both backends.

## Feature D — New examples

- `examples/tasks` — smallest real app: single pane (toolbarview + headerbar + list), checkbox
  rows, `<searchinput>` filter, progressbar of done/total, menubar with Task menu (New `primary+n`,
  Clear Completed) — shows default menu chrome + adaptive look with zero platform code.
- `examples/settings` — two-pane splitview: sidebar categories (General/Appearance/Advanced),
  content = grouped forms using `boxed-list` cards: checkbox, radio group, select, slider, labeled
  captions — shows Adwaita boxed-lists vs clean mac forms from the same tree.
- Each mirrors notes' package.json/tsconfig (workspace glob `examples/*` picks them up). Follow the
  hooks-import rule. Optional small drive legs (ND_TASKS_OK) welcome but not gating.

## Feature E — Starlight docs site (`docs-site/`, already scaffolded)

Astro 7 + Starlight 0.41, own lockfile (NOT in the bun workspace). Sidebar groups (vercel-native
inspired): Get Started (Introduction · Quick Start · Project Layout) / Core Concepts (App Model ·
State & Hot Reload · Styling & Design Language · Automation-First) / Components (Overview + Widget
Reference sourced from schema/widgets.json — a small gen script or a faithful port of
docs/widgets.md; must not hand-drift from the schema) / Native Platform (Windows & Chrome · Menu
Bar · Split Views · Icons · Platform Support matrix) / Automation & Testing (Automation socket ·
MCP tools · Screenshots/ndshot) / Packaging. Tone: principle-led intros, concrete verified
commands, agent-friendly. Source material: docs/agents/*.md, docs/styling.md, docs/widgets.md,
docs/packaging.md, the design spec (this file). `cd docs-site && bun run build` must pass. Remove
the starter sample content. Menu Bar / Split Views pages land in wave 2 (after features A/B merge).

## Waves

1. **S — three-pane** (Feature B) ∥ **docs wave 1** (Feature E minus menu/split pages) ∥ ndshot
   (dispatched). S and the menu work both edit tools/codegen.ts + regenerate the same outputs —
   NEVER in parallel.
2. **M — menu bar** (Feature A, after S lands) ∥ docs wave 2 (menu/split pages after M).
3. **Notes rework** (C) ∥ **examples** (D) — after A+B.
4. Architect verification: codegen freshness · `zig build test` · GTK drives natively (quartz) ·
   mac leg (`mac-m11.sh` + notes drive with new gates) · counter/gallery regression · docs build ·
   ledger entry · commits (architect only).
