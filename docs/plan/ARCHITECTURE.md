# NativeDesktop (Rust): architecture

Status: draft for adversarial review, 2026-10-13. Inputs: `docs/plan/PROMPT.md` (requirements and
kickoff decisions, including the 2026-10-13 correction), `docs/research/*.md` (evidence),
`docs/parity/zig.md` and `docs/parity/apps.md` (what must survive). Example apps that use the API
defined here live in `docs/plan/examples/`. "Old repo" means `~/Developer/NativeDesktop`, branch `zig`.

## 1. Goals and non-goals

| Prompt requirement | Goal in this design | Where |
| --- | --- | --- |
| "extremely performant", e1504g "fast and snappy" | Idle at 0% CPU, updates cost the size of the dirty view, per-display pacing, measured on e1504g from M2 | 3.7, 5.3, 9 |
| AppKit on mac, GPUI on Linux, WinUI or GPUI on Windows | Real AppKit controls on macOS; GPUI on Linux; GPUI with a Fluent skin on Windows, WinUI addable later behind the same seam | 2, 3.8 |
| "GPUI style syntax", no Bun required | One GPUI-shaped Rust vocabulary on every backend; no JS runtime; one process | 3 |
| Same look, own patterns, override possible | One token set; intents mapped per OS; overrides at every layer | 3.6, 4 |
| Remove repeated patterns from messages/youtubemusic | Theme watcher, chrome, menus, list, single instance, IPC, tray, MPRIS, config, demo mode become framework crates | 2, 14 |
| matugen on Linux | `theme.json` watcher, framework template, token map | 4 |
| CEF | `cef` crate, OSR zero copy, opt-in per app | 8 |
| Nix, home-manager, TOML config | `mkApp` flake helper; one settings schema generates HM module and settings UI | 11 |
| External drive for storage | Build dir and sccache on `/Volumes/Kyan Drive/dev-cache.noindex` | 10.5 |
| Resizes, gestures, no input leak, embedded views that belong to the window | Each is a mechanism with a gate | 5, 6, 8 |
| Web target | GPUI backend on wasm; docs examples run live in the browser | 12 |
| No feature loss, docs remade in the framework | Parity map with named replacements; docs app built with `nd` | 13, 14 |
| Native virtualization | One list/table/grid/tree API; NSTableView family on mac, own engine on GPUI | 7 |
| Multi-monitor smoothness (60, 240+144, 120 Hz) | Per-window display clock on every OS | 5.3 |
| Profiling, logs with cleanup, know what happened | Always-on flight recorder, incident bundles, tracy on demand | 9 |
| Auto update, branded Windows installer | Velopack + ed25519-signed feed, installer branded from `nativedesktop.toml` | 11 |
| ndshell remade | Three test tiers, automation socket over one semantic tree, frame recorder, no VMs | 10 |
| Window controls, drag areas, traffic lights by coordinates or embedded | `window_controls()` and `drag_region()` elements | 5.4 |
| Standard trackpad gestures, haptics | Gesture modifiers; native recognizers on mac, framework recognizers on GPUI | 6.1 |
| Easy to extend (Discord RPC), own widgets | `Plugin` trait; app-defined native widgets on the same public trait the framework uses | 3.4, 3.5 |
| CLAUDE.md, agent docs, bootstrap, fun README | `nd new`, agent docs, `nd check` gates | 15 |

Non-goals: mobile targets; a JS or TS app API; shipping Chromium bytes in apps that did not opt
in; app-level pixel tuning to fake a native look (the old "fix UI at the framework level" rule
stands); any OS look on the web (it renders plain GPUI).

## 2. Crate graph

Apps depend on one crate, `nd`, and write `use nd::prelude::*;`. Subsystems are cargo features
re-exported under `nd::` (`nd::webview`, `nd::cef`, `nd::data`, `nd::ipc`, `nd::terminal`,
`nd::update`). No crate except `nd-gpui` names `gpui`, and no app names objc2 except inside a
native widget's macOS module (checked by `nd check`, 15.4).

| Crate | One responsibility | May depend on |
| --- | --- | --- |
| `nd` | Facade: `Application`, prelude, features, backend selection by target | all below that ship in apps |
| `nd-macros` | `#[derive(Settings)]`, `nd::manifest!()`, `#[nd::test]`, `#[derive(IntoElement)]` re-export | syn, quote |
| `nd-core` | GPUI-free basics: paths (`ND_APP_DATA_DIR` root), clock trait, semantic tree schema, error policy, ids | std, serde, tracing |
| `nd-gpui` | Only GPUI dependency. Pins `gpui-pre =0.3.9` (matches `gpui-kit`), feature `fast` selects `gpui-fast`, carries the patch queue (2.1) | gpui-pre or gpui-fast, gpui-base |
| `nd-ui` | The app vocabulary: `App`, `Entity`, `Context`, `Window`, `Render`, elements, styling, actions, focus, gestures API, `Painter`, retained tree, reconciler, Taffy layout for retained backends, semantic tree, `NativeWidget` trait | nd-core, nd-gpui, taffy |
| `nd-theme` | Tokens, presets, skins, matugen watcher, appearance and accent | nd-core, nd-ui |
| `nd-platform` | Traits for OS services each backend provides: display clock, chrome, materials, haptics, dialogs, menus | nd-core, nd-ui |
| `nd-backend-appkit` | macOS: NSApplication run loop, NSWindow, retained tree to NSViews, primitive views (layer, text, canvas), GPU surface host, NSToolbar, NSMenu, NSPopover, sheets, display links, haptics; contains the `NDSwift` SwiftPM package for Swift-only and SwiftUI-hosted pieces (3.9) | nd-ui, nd-platform, nd-theme, objc2 + framework crates (pinned git rev, `docs/research/macos-and-cef.md` section 1), Swift toolchain at build time |
| `nd-backend-gpui` | Linux, Windows, web: renders the retained tree through GPUI, overlay stack, drawn chrome | nd-ui, nd-platform, nd-theme, nd-gpui |
| `nd-os-linux` | Wayland/X11 chrome negotiation, portals, `wp_presentation`, MPRIS, SNI tray, libsecret | nd-platform, wayland-client, zbus, ashpd |
| `nd-os-windows` | Mica/Acrylic, caption hit-testing, per-monitor pacing, `WM_POINTER`, SMTC, toasts, credentials | nd-platform, windows |
| `nd-os-web` | Browser services: file input/download, async Clipboard API, localStorage, history | nd-platform, web-sys |
| `nd-virtual` | Virtualization model: keyed diff, anchor, follow, measurement cache, selection; presenters per backend | nd-ui |
| `nd-components` | Built-in widgets, each a `NativeWidget` with an AppKit impl and a drawn impl (3.5); chrome intents (`scaffold`, `toolbar`, `sidebar`), overlays, panes model | nd-ui, nd-theme, nd-virtual, nd-icons; objc2 on macOS |
| `nd-icons` | Lucide set as assets, SF Symbol and Segoe Fluent name maps | nd-ui |
| `nd-webview` | WKWebView, WebView2 composition, WPE WebKit, iframe on web | nd-components, nd-platform |
| `nd-cef` | CEF via `cef` crate, OSR default, windowed Chrome-style opt-in | nd-webview, cef |
| `nd-adblock` | Content blocking (port of the old repo's Rust crate) | adblock |
| `nd-terminal` | libghostty-vt terminal widget | nd-components |
| `nd-config` | `nativedesktop.toml` schema; read by CLI and at build time | nd-core, serde, schemars, toml |
| `nd-settings` | Layered TOML settings, watch, `toml_edit` writes, Nix-managed detection, `Store<T>`, `settings_form()` | nd-config, nd-ui, nd-components |
| `nd-system` | Dialogs, clipboard, notifications, recents, credentials, audio + spectrum, tray, media session, open/reveal, single instance | nd-platform |
| `nd-ipc` | JSON lines over unix socket / named pipe, typed contracts, reconnect ladder | nd-core, tokio |
| `nd-data` | SQLite on its own thread, async `query/mutate/transaction` | nd-core, rusqlite |
| `nd-diag` | tracing facade, flight recorder, incident bundles, retention, crash helper, profiler HUD | nd-core, nd-ui, crash-handler, minidumper, tracy-client |
| `nd-automation` | In-app automation server over the semantic tree: input, waitFor, dialog scripts, capture | nd-ui, nd-diag, nd-system |
| `nd-update` | Velopack with our signed feed source; compiled out on Nix and web | nd-config, velopack, ed25519-dalek |
| `nd-test` | Test harness, tiers 1-3, locators | nd, nd-automation, ndshell |
| `ndshell` | Test-only OS helpers: TCC grant, HID input, composited capture, virtual display, headless Hyprland | never linked by apps |
| `nd-mcp` | MCP bridge: `execute`, `snapshot`, `reset` (`rmcp`, JS via `rquickjs`) | nd-test |
| `nd-cli` | `nd new/dev/check/test/build/package/release/doctor/diagnose/gen` | nd-config, nd-test, nd-update |
| `nd-docs` | Static site generator and content loader | nd-config, pulldown-cmark |
| `apps/docs` | Docs app, built with `nd` | nd |

### 2.1 GPUI channel and patch queue
`nd-gpui` pins exact versions, bumped weekly in one PR gated by tiers 1-2 (`docs/research/gpui.md`
recommendation 1). Missing features live as a small patch series in a fork (`nativedesktop/zed`,
branch `nd/<snapshot>`) consumed with `[patch.crates-io]`: embedded-surface `PlatformWindow` for
macOS (3.8), `momentumPhase` forwarding, frame clock injection for animations, Windows per-monitor
pacing, Acrylic, X11 refresh re-query, wasm fixes found in M12. Each patch is filed upstream. M0
checks whether `gpui-pre 0.3.9` has Linux headless rendering (zed PR #64718,
`docs/research/testing-profiling.md` 1.2); if not, it pins that zed rev.

## 3. App programming model

### 3.1 One vocabulary
Apps write GPUI-shaped code against `nd-ui`: entities hold state, `cx.notify()` marks a view
dirty, `impl Render` returns elements built with `div()`-style fluent methods, `actions!` plus
`KeyBinding` drive commands. `App`, `Entity<T>`, `Context<T>`, `AsyncApp` and the executors are
thin newtypes over GPUI's application core (entity store, effect queue, keymap, focus handles, test
dispatcher), so the semantics are GPUI's and the surface stays ours across GPUI bumps. Elements
are ours, because one element must become an `NSButton` on macOS and a drawn Fluent button on
Windows. An app never learns which backend an element resolves to.

### 3.2 A full screen
Sidebar, toolbar with the traffic light slot, a virtual list with custom-drawn rows, a context
menu, swipe and pinch, and a settings sheet, in one file. This is a condensed form of
`docs/plan/examples/notes/`, which splits the same screen into sidebar, list, editor and settings
views and adds the command palette and tests.
```rust
use nd::prelude::*;
use crate::{actions::*, editor::Editor, model::{Library, NoteId}, settings::{NotesSettings, SettingsSheet}};

pub struct Notes {
    library: Entity<Library>,
    list: ListState<NoteId>,
    search: Entity<TextFieldState>,
    editor: Entity<Editor>,
    zoom: f32,
}

impl Notes {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let library = Library::open(cx);
        let list = ListState::new(cx).follow(Follow::None);
        cx.observe(&library, |this, library, cx| {
            this.list.set_items(library.read(cx).visible_ids(), cx); // keyed: scroll never resets
        }).detach();
        Self {
            search: cx.new(|cx| TextFieldState::new(window, cx).placeholder("Search")),
            editor: cx.new(|cx| Editor::new(library.clone(), window, cx)),
            library, list, zoom: 1.0,
        }
    }

    fn row(&mut self, id: &NoteId, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let note = self.library.read(cx).note(id);
        let theme = cx.theme();
        let id = *id;
        h_flex()
            .id(id)
            .gap_3().px_3().py_2()
            .child(canvas(move |bounds, paint, _cx| {                  // custom-drawn colour tag
                paint.fill_circle(bounds.center(), bounds.size.width / 2., note.tag_color);
            }).size(px(10.)))
            .child(v_flex().flex_1().min_w_0()
                .child(text(note.title.clone()).font_weight(FontWeight::SEMIBOLD).truncate())
                .child(text(note.preview.clone()).text_color(theme.text.secondary).line_clamp(2)))
            .context_menu(move |menu, _window, _cx| {
                menu.item("Pin", PinNote(id)).item("Duplicate", DuplicateNote(id))
                    .separator().item_destructive("Delete", DeleteNote(id))
            })
            .swipe_actions(SwipeActions::trailing()
                .action(swipe_action("Delete", Icon::Trash).destructive().dispatch(DeleteNote(id))))
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        window.open_sheet(cx, |window, cx| cx.new(|cx| SettingsSheet::new(window, cx)));
    }
}

impl Render for Notes {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = cx.settings::<NotesSettings>().compact_rows;
        scaffold()
            .key_context("Notes")
            .on_action(cx.listener(Self::open_settings))
            .toolbar(toolbar()
                .leading(window_controls().embedded())
                .item(icon_button("new", Icon::SquarePen).tooltip("New Note").dispatch(NewNote))
                .flexible_space()
                .item(search_field(&self.search)))
            .sidebar(sidebar().min_w(px(220.)).child(
                list(&self.list, cx.processor(Self::row))
                    .estimated_row_height(if compact { px(36.) } else { px(56.) })
                    .selection(Selection::single())
                    .on_select(cx.listener(|this, e: &SelectEvent<NoteId>, _w, cx| {
                        this.editor.update(cx, |editor, cx| editor.open(e.key, cx));
                    }))))
            .content(div().size_full()
                .on_pinch(cx.listener(|this, e: &PinchEvent, _w, cx| {
                    this.zoom = (this.zoom * e.scale).clamp(0.5, 3.0);
                    cx.notify();
                }))
                .child(self.editor.clone()))
    }
}
```
Nothing here names AppKit, GPUI or a platform. On macOS `scaffold()` is an `NSSplitViewController`
window with a glass sidebar, the toolbar is an `NSToolbar`, `list()` is a view-based `NSTableView`
whose cells hold the row subtree, swipe actions are native row actions and the context menu is an
`NSMenu`. On Linux and Windows the same tree draws through GPUI with the matugen or Fluent skin.

### 3.3 State, actions, async, errors
- State: `Entity<T>`, `cx.observe`, `cx.subscribe`, `cx.notify()`; views are not bound to a window,
  so a view moved to another window keeps its state (replaces the old Portal/pool/moveNode).
- Actions and keys: `actions!` and `KeyBinding::new("secondary-n", NewNote, Some("Notes"))`;
  `secondary` is cmd on macOS and ctrl elsewhere. `nd::keymap::standard()` adds OS conventions
  (Preferences, Quit, Close, tab cycling). The menu bar is built from the same actions, so menus
  and shortcuts never disagree.
- Async: `cx.spawn(async move |this, cx| ..)`, `cx.background_spawn(..)`; `nd::runtime::tokio()` is
  one shared 2-thread runtime for network crates. Timers go through the executor so tests stay
  deterministic.
- Errors: a task's `Err` reports non-fatal (rate cap 20 per 10 s); a background panic is caught at
  the task boundary; a foreground panic writes an incident bundle (9.3) and the crash helper
  relaunches the app with window state restored and a toast naming the bundle.

### 3.4 Plugins
```rust
pub trait Plugin: 'static {
    fn name(&self) -> &'static str;
    fn init(&self, cx: &mut App);
    fn settings(&self) -> Option<SettingsSection> { None }   // merged into schema, settings UI, HM module
    fn commands(&self) -> Vec<PaletteCommand> { Vec::new() }
    fn tray_items(&self) -> Vec<MenuItem> { Vec::new() }
}
```
A Discord RPC crate implements `init` (spawns its client on `nd::runtime`, observes an app global)
and `settings` (a `discord` section), and gets a settings page and a
`programs.<app>.settings.discord` HM option without the app writing either.

### 3.5 Widgets: composed and native, one trait
Two ways to build a widget, both identical at the call site to built-ins:
- Composed: a function or `RenderOnce` struct returning elements from the vocabulary
  (`div`, `text`, `canvas`, built-ins). Renders on every backend.
- Native: implement `NativeWidget`. The framework lays it out (Taffy, using `measure`), focuses it,
  routes input, includes it in the semantic tree from `semantics`, applies overrides and
  z-orders it against overlays. Every built-in control (button, text field, switch, slider,
  segmented, select, list, table, toolbar item, sidebar) is implemented on this same public trait
  in `nd-components`, with an `appkit` module and a `drawn` implementation.
```rust
pub trait NativeWidget: Sized + 'static {
    type Props: Clone + PartialEq + 'static;
    type Event: 'static;
    fn semantics(props: &Self::Props) -> Semantics;
    /// Composed implementation: GPUI backends, the web, and anywhere `.drawn()` is set.
    fn drawn(props: &Self::Props, emit: Emit<Self::Event>, window: &mut Window, cx: &mut App) -> AnyElement;
    fn backends() -> Backends<Self> { Backends::drawn_only() } // `.appkit(..)` now, `.winui(..)` later
}

#[cfg(target_os = "macos")]
pub trait AppKitWidget<W: NativeWidget>: 'static {
    type View: objc2::Message;
    fn create(&self, props: &W::Props, cx: &mut AppKitCx<W>) -> Retained<Self::View>;
    fn update(&self, view: &Self::View, old: &W::Props, new: &W::Props, cx: &mut AppKitCx<W>);
    fn measure(&self, view: &Self::View, props: &W::Props, available: AvailableSize) -> Size<Pixels>;
    fn value(&self, _view: &Self::View) -> Option<SemanticValue> { None }
}
```
An element type implements `NativeElement` (`type Widget`, `base_mut()`, `into_parts()`), and
blanket impls give it `Styled`, `InteractiveElement` and the override modifiers. `AppKitCx` gives
the main thread marker, `emitter()`, `request_layout()`, `keep_alive(obj)` (retain a delegate for
the node's lifetime) and the objc2 re-exports under `nd::native::appkit`. A widget written in
Swift registers with `Backends::swift(..)` instead (3.9). Worked example:
`docs/plan/examples/native-widget/` (an `MKMapView` through objc2 and a SwiftUI `Gauge` through
Swift on macOS, drawn fallbacks elsewhere).

### 3.6 Native patterns and overrides
Apps declare intent; the backend maps it.

| Intent | macOS (AppKit) | Windows (GPUI, Fluent skin) | Linux (GPUI) |
| --- | --- | --- | --- |
| `scaffold().sidebar()` | `NSSplitViewItem(sidebarWithViewController:)` hosting a source-list `NSOutlineView`, `NSBackgroundExtensionView` | Nav pane on Mica | Sidebar on `surface_container_low` |
| `scaffold().inspector()` | `NSSplitViewItem(inspectorWithViewController:)` | Right pane on Mica | Right pane |
| `toolbar()` | `NSToolbar` with system items (`toggleSidebar`, `sidebarTrackingSeparator`, `NSSearchToolbarItem`) and native item views | Title row in the DWM-extended frame | Header row; no controls under server decorations |
| `window_controls()` | Real traffic lights in the slot | System DWM caption buttons (Snap Layouts included) | Drawn only when client-decorated |
| `.context_menu()` | `NSMenu` | Fluent menu in the overlay stack | Menu in `xdg_popup` with grab |
| `window.open_sheet()` | Window sheet | Content dialog with scrim | Dialog with scrim, libadwaita metrics |
| settings window | Toolbar-tab window with SwiftUI `Form` (`.grouped`) rows, `cmd-,` | Separate window, nav list | Window or dialog per preset |
| menu bar | Global `NSMenu`, app menu roles | Folded into title-row overflow and palette | Same as Windows |
| alerts, file pickers | `NSAlert`, `NSOpenPanel` | Task dialog, `IFileDialog` | Portal (`ashpd`) |

Overrides, every layer, no fork needed:
- Per element style: `.bg()`, `.rounded()`, `.p_*()`, `.font()`, `.material(Material::Glass)`
  apply to native controls where the control can express them (`bezelColor`, `contentTintColor`,
  font, insets on macOS). A property the native control cannot express is ignored with one
  dev-build warning that names `.drawn()` as the fix.
- Per platform: `.on_macos(|b| b.bezel(Bezel::Glass))`, `.on_windows(..)`, `.on_linux(..)`,
  `.on_web(..)`; the closure runs only on that platform, so app code needs no `cfg`.
  Platform hint methods such as `bezel` exist on every target and are ignored off macOS.
- Mapping: `.drawn()` forces the composed implementation on any backend; `.native(Mapping::..)`
  picks another native mapping (`list(..).native(Mapping::CollectionView)`).
- Theme: `Application::theme(..)`, `window.set_theme(..)`, subtree `.theme(|t| t.accent(..))`.
- Patterns: `.pattern(Pattern::Drawn)` per element, `Application::patterns(..)` per app,
  `[patterns]` in `nativedesktop.toml`, and window chrome such as
  `WindowOptions::titlebar(Titlebar::Custom)` (custom titlebar on macOS too, traffic lights in
  the slot). See `docs/plan/examples/overrides/`.

### 3.7 How backends satisfy the model (and stay fast)
Invalidation is per view, as in GPUI. `cx.notify()` on a view re-runs that view's `render` only;
child views (`Entity` children) are boundaries and are not re-rendered unless notified themselves.
`nd-ui` reconciles the new element description of that one view against its retained subtree by
position and `.id()` key, compares typed props with `PartialEq`, and emits only changed setters to
the backend. Event handlers are stored on the retained node and swapped without a backend call,
since a fresh closure each render is normal. Cost is the size of the dirty view, never the window.
The profiler HUD flags a view whose render produces more than 500 nodes or takes over 1 ms.

- AppKit backend: retained nodes own NSViews. Primitives map to cheap views: a styled `div` is a
  layer-backed `NDLayerView` (background, radius, border, shadow, opacity on its CALayer), `text`
  is a CoreText-drawing view with an NSAccessibility role, `canvas` draws through `Painter` on
  CoreGraphics, `img` sets layer contents. Native widgets are their AppKit views.
- GPUI backend: each `nd` view is a cached GPUI view rendering its retained subtree into GPUI
  elements; only dirty views re-render. Under `nd-gpui/fast` the retained-mode fork skips
  unchanged layout and paint as well.

Hard parts and how they are resolved:
1. Layout authority. Taffy is the authority everywhere: inside GPUI on GPUI backends, and an
   `nd-ui` Taffy tree on AppKit with identical style mapping. Native leaves supply `measure`,
   cached by props and constraint (text measured with CoreText in Rust, not by asking NSTextField).
   Frames are applied top-down in one `CATransaction` with implicit animations off, at the end of
   the frame; Auto Layout and NSStackView are never used (the old repo replaced NSStackView for
   the same reason, `NDShell/Layout.swift`). Native containers that position their own children
   (NSTableView rows, NSSplitView panes, NSToolbar items, NSScrollView documents) are layout
   boundaries: the container gives the child its size, `nd-ui` runs Taffy inside it, and the
   child reports its height or intrinsic size back (`heightOfRow`, `intrinsicContentSize`).
2. Custom-drawn content inside native containers. Composed rows in an NSTableView are layer
   views and CoreGraphics canvases, so a thousand custom rows cost no extra GPU surfaces. A GPU
   surface on macOS appears only where content is GPU-only (3.8).
3. Text metrics differ between CoreText, DirectWrite and cosmic-text; goldens are per platform.
4. Animations: `.with_animation` on transform and opacity becomes Core Animation on macOS and
   frame-clock tweens on GPUI; both read the same timing tokens.
5. Focus: `nd-ui` focus order drives AppKit's key view loop; the first responder and the nd
   focused node are synced both ways.
6. Accessibility: native controls carry NSAccessibility; drawn nodes on macOS get
   `NSAccessibilityElement` proxies from `semantics`; GPUI backends use AccessKit.

### 3.8 Backends per OS
- macOS: `nd-backend-appkit`, real AppKit controls (the user's correction: nativebrowser's
  AppKit buttons have effects and feel GPUI cannot replicate). AppKit is an Objective-C framework,
  so Rust calls it directly through objc2: the controls on screen are AppKit's own classes, not a
  Rust imitation of them. Swift appears only for Swift-only and SwiftUI APIs (3.9). GPUI runs on macOS only for
  GPU-only content: `canvas(..).gpu()` for heavy custom drawing, the code editor, charts with
  large series, CEF frames when a CALayer cannot take the IOSurface directly. Such a subtree is
  hosted in a `CAMetalLayer` view inside the AppKit tree through an embedded-surface
  `PlatformWindow` (patch queue, 2.1). Rules for that case: it is an ordinary NSView, so sibling
  order decides z-order among native views; overlays never draw inside it (menus, popovers,
  tooltips from inside it go through the native overlay path, 6.3), so nothing is clipped or
  covered; AppKit hit-testing delivers events to it and it forwards them to GPUI dispatch; it
  becomes first responder and implements `NSTextInputClient` for IME; its frame is set in the
  same transaction as its siblings and it presents with the transaction during live resize.
- Windows: decision is GPUI with a Fluent 2 skin. Evidence: `windows-reactor` is a preview
  (0.100.0, 2026-09-03, 3.5k downloads), its internals were rewritten on 2026-10-01 with weekly
  follow-ups, it needs the Windows App SDK runtime per app, and embedded surfaces re-hit airspace
  (`docs/research/windows-packaging-updates.md` sections 0 and 1). The skin reproduces WinUI 3:
  Fluent 2 tokens, control templates and states, two-ring focus visuals, WinUI motion curves,
  Segoe UI Variable and Segoe Fluent Icons, Mica/MicaAlt/Acrylic, Snap Layouts, accent and high
  contrast, UIA through AccessKit. Gate: pixel comparison against WinUI Gallery references per
  control. Seam: `Backends::winui(..)` is reserved on `NativeWidget`; a later
  `nd-backend-winui` registers WinUI impls for built-ins and apps change nothing.
- Linux: `nd-backend-gpui` with the GPUI look and matugen palette, Wayland first, X11 supported.
- Web: `nd-backend-gpui` on wasm with the plain GPUI look (section 12).

### 3.9 Swift inside the AppKit backend
Rust calls AppKit directly through objc2, which covers most of AppKit because AppKit is an
Objective-C framework; no Swift is involved there. Swift is used only where an API is Swift-only or
SwiftUI-only, or where a SwiftUI-hosted control is what Apple restyles: SwiftUI-hosted controls in
`NSHostingView`, Swift Charts, grouped `Form` settings rows, `#available` branches for the newest
glass, toolbar and sidebar APIs. The old repo hosted Charts, Banner, LinkButton, Slider, LoadBar,
StatusPage and SettingsGroup/Row in SwiftUI because "Apple owns the chrome: a `LabeledContent` row
picks up System Settings metrics, accent response and light/dark for free, and keeps picking them up
as the design language moves" (`NDShell/SwiftUILeaves.swift:1-20`). Its other lesson carries over:
universal props (enabled, tooltip, focus) and the semantic value probe must be threaded into
SwiftUI state, because a hosting view is not the control the user sees (`NDLeafState`).

Bridge:
- `nd-backend-appkit/swift/NDSwift` is a SwiftPM package. `build.rs` runs `swift build -c release`
  for the target triple and deployment target, links `libNDSwift.a` statically, adds the Swift
  runtime search path (`/usr/lib/swift` rpath, toolchain `lib/swift/macosx` for compatibility
  libs) and clears `SDKROOT` for the Swift step (the old repo's `env -u SDKROOT` quirk). Command
  Line Tools are enough; there is no Xcode project, and `cargo build` is the only command.
  `nd doctor` checks the toolchain; the Nix dev shell provides it on darwin.
- Seam: a narrow C ABI. Swift exports `@_cdecl` functions per hosted kind (`create`, `update`,
  `measure`, `destroy`, `value`); Rust declares them `extern "C"`. Framework hot paths pass
  `#[repr(C)]` prop structs; events come back through a C callback carrying the node id. No
  Swift type is visible above `nd-backend-appkit`.
- App Swift views: an app or crate adds a SwiftPM package (for example `swift/Widgets`) and a
  one-line build script, `nd::build::swift_package("swift/Widgets")`, with `nd` (feature `build`)
  as the build dependency. In Swift the view conforms to `NDSwiftWidget` (from the `NDKit`
  product the framework ships: `static func make(props: Props, emit: NDEmitter<Event>) -> NSView`,
  `update`, `measure`, `value`, with `Props` and `Event` `Codable`) and is exported with the
`#ndExport(..)` macro, which emits the C registration symbol the build script links. In Rust the widget registers
  `Backends::swift(SwiftWidget::named("Gauge"))`; props and events cross as JSON, which is fine
  because a leaf's props cross only when they change. Layout, focus, input, overlays, semantics
  and overrides then work exactly as for an objc2 widget. On other targets the build script is a
  no-op and the drawn implementation is used.

### 3.10 Rule: the OS draws its own structure
Big structural elements use the highest-level system component the OS offers, never a pixel
replica, so a new OS release restyles apps with no framework change.
- macOS: sidebar is `NSSplitViewItem(sidebarWithViewController:)` with a source-list
  `NSOutlineView` (or the SwiftUI equivalent where that is what Apple restyles); inspector is the
  inspector split item; toolbar is `NSToolbar` with system items; window chrome and traffic lights
  are the system's; sheets, popovers, menus and alerts are system classes; the settings window
  follows the system settings pattern with SwiftUI `Form` rows. The AppKit mapping holds no
  system metrics, colours, radii or materials as constants: it reads them from the OS
  (`NSColor` semantic colours resolved at draw time, `NSFont.preferredFont(forTextStyle:)`, control
  sizes from `fittingSize`) or lets the control draw itself. The macOS preset for composed content
  is computed at launch and on appearance change from those same sources.
- Windows: Mica, Acrylic, accent, high contrast, text scale, DWM corners and shadow, and caption
  buttons come from the OS: the title row lives in a DWM-extended frame
  (`DwmExtendFrameIntoClientArea` with `DwmDefWindowProc` hit-testing), so the system draws the
  caption buttons and the Snap Layouts flyout. Drawn caption buttons exist only as an app
  override. The Fluent skin for leaf controls is a replica by necessity until a `winui` backend
  exists; it reads every token it can from the OS.
- Linux: the look is GPUI's by the user's choice; colours come from matugen and the portal,
  fonts, scale and reduced motion from the portal settings.
- Enforced by the OS-release diff job (10.6) and by agent docs (15.1).

### 3.11 Built-in components
`nd-components` ships the parity widgets (`docs/parity/zig.md` section 1) and the ranked
hand-rolled components from `docs/parity/apps.md` section 5, each a `NativeWidget` with an AppKit
(or Swift) impl where native feel matters and a drawn impl: buttons and icon buttons, text and
search fields, switch, checkbox, radio, slider, segmented, select and combobox, number, date,
colour and font pickers, progress and spinners, `glass()`, source list and sidebar, list, table,
grid, tree (section 7), tabs and panes (split, dock, tiles), toolbar items, menus and popovers,
dialogs, sheets and forms, `settings_form`, toast and banner, status page, badges, tags, kbd,
avatar, breadcrumb, rich text, charts (Swift Charts on macOS), code editor, video, image pipeline
with display-size decode and LRU, media transport (player bar, seek clock, queue), synced lyrics
view, carousel/shelf, composer, inspector panel, and the command palette below.

Command palette, `command_palette(id)`. On macOS it is exactly the zig branch's Spotlight-style
palette (`NDShell/CommandPalette.swift`, metrics `NDPaletteMetrics` in `NDShell/Metrics.swift`),
ported to objc2: a dimmed full-window backdrop over the window's frame view (toolbar and sidebar
included, black at 0.15), a Liquid Glass card 640 pt wide with its shadow on an outer host view
(the glass clips to its corner radius), top edge fixed at 18% of the window height with a 20 pt
margin so the field never moves while results change, a borderless search field over an inset
`NSTableView` of one-line 40 pt rows, list capped at 400 pt and collapsing with its separator when
empty, the highlighted row drawn with AppKit's emphasized selection (focus stays in the field),
cells with a 16 pt icon slot that holds the column even when empty, title, secondary subtitle that
yields first, and a right-aligned hint that never truncates. Behaviour carried over unchanged:
- Controlled: the app owns `query` and `items` and does all filtering and ordering; each keystroke
  fires `on_query_change`; every present starts from the app's last `query` with the whole text
  selected; rows reload only when their content fingerprint changes, so a fresh `items` vector
  each render keeps the highlight.
- Items: `palette_item(id, title)` with `.subtitle()`, `.icon()` (or icon bytes), `.hint()`,
  `.completion()`.
- Keys: Up/Down/Home/End clamp the highlight; fresh results highlight the top row; Return fires
  `on_activate(id)`; Return with no highlight, or cmd/ctrl-Return always, fires `on_submit(text)`
  so a typed value that matches no row can be accepted; Esc and a click outside fire `on_cancel`;
  an app-driven close does not echo `on_cancel`.
- Inline completion: the first row's `completion`, when it extends the typed text and the last
  edit inserted text, is shown selected after the caret; Tab or Right accepts it, Tab never leaves
  the field, Backspace over a completion does not bring it straight back, and an unaccepted
  completion never reaches `on_query_change`; Return on a first row carrying a completion means
  the completed text.
- Presents over the app's active window; focus returns to the previous responder on close; an
  unmount while open tears the backdrop down.
- Automation drives the same paths (`setValue`, `type`, `click` routed to the real field and
  table) and `paletteLayout` reports the drawn geometry.
On GPUI backends the drawn impl has the same structure and behaviour with the skin's material.

## 4. Theming and design tokens

One token set in `nd-theme`, read with `cx.theme()`: `surface.{base, sidebar, raised, raised_hover,
overlay}`, `border.{subtle, strong}`, `separator`, `text.{primary, secondary, tertiary, on_accent}`,
`accent`, `accent.container`, `selection`, `danger`, `success`, `warning`, `focus_ring`, `scrim`,
plus metrics (spacing scale `Spacing`/`ContentMargin`/`ContentWidth`, radii, control heights, type
scale with tabular figures, motion durations and springs) and materials. Derived tokens (hover,
pressed, disabled) are computed from about 15 base keys, as messages does.

How tokens reach controls:
- macOS: native controls draw themselves with system colours unless a token is overridden;
  overridden accent maps to `bezelColor`/`contentTintColor`, fonts to `NSFont`, materials to
  `NSGlassEffectView` (`Regular`, `Clear`) on 26+ and `NSVisualEffectView` below. On macOS the
  token values themselves are resolved from `NSColor` semantic colours and text styles (3.10),
  so composed content follows the OS too.
- Windows: the Fluent skin reads Fluent 2 token values, with accent, Mica and high contrast from
  the OS (`UISettings`, DWM).
- Linux: the GPUI preset with libadwaita metrics, Noto Sans bundled, palette from matugen.

`glass()` is a first-class container element (`glass().rounded_lg().p_3().child(..)`) with
`.variant(GlassVariant::Regular | Clear)`, `.tint(color)`, `.interactive()` and `glass_group()`
for merging nearby glass where the OS offers it. It maps to the system material, never a replica
(3.10): macOS 26+ `NSGlassEffectView` (corner radius, tint, style; `NSGlassEffectContainerView`
for groups), `NSVisualEffectView` on 14-15; Windows a Mica or Acrylic system backdrop region;
Linux compositor blur where the compositor offers it (KDE blur protocol; on Hyprland a transparent
surface the compositor blurs by rule) else a translucent matugen surface; web plain GPUI
translucency. It takes every override like any other element.

Palette sources, first hit wins: app or user `theme.toml`; on Linux the matugen `theme.json` at
`~/.config/<app>/theme.json` (watched with `notify` on the parent directory, 50 ms debounce, 1 s
mtime fallback, half-written files keep the last palette; the app never spawns matugen; token map
in `docs/research/linux-nix-config.md` section 1, with `success`/`warning` from matugen
`custom_colors`); OS appearance and accent (`NSAppearance`, `UISettings`, portal `color-scheme` and
`accent-color`); baked-in light and dark palettes. Materials: `Glass`, `Vibrancy`, `Mica`,
`MicaAlt`, `Acrylic` (patch), `Transparent` (Hyprland blurs by its own rules; the HM module ships a
rule snippet), `KdeBlur`, `Opaque`. Overrides apply at app, window and subtree level (3.6). A
light/dark flip or accent change re-resolves tokens and redraws once.

## 5. Windowing

### 5.1 Host windows
macOS: our `NSWindow` with full-size content view and real chrome. Windows: GPUI HWND with
DirectComposition and PerMonitorV2 DPI. Linux: GPUI Wayland toplevel (fractional scale with
viewporter) or X11. Web: one canvas (section 12).

### 5.2 Resize correctness
Every presented frame matches the window size the OS shows, and every native view moves in that
frame. macOS: Taffy runs inside `viewWillDraw`/live-resize layout and frames commit in one
transaction; GPU surface views present with the transaction. Windows: render synchronously in
`WM_SIZING`/`WM_WINDOWPOSCHANGED`, one DirectComposition commit for content and sibling visuals.
Wayland: ack `configure` only with a buffer of that size; subsurfaces commit in sync mode. X11:
`_NET_WM_SYNC_REQUEST`. Gate on all OSes: a scripted edge drag recorded by the frame recorder
(9.2); each frame's content size equals the reported window size and no native rect trails.

### 5.3 Multi-window, multi-monitor, pacing
Each window owns a display clock at the refresh of the display it is on; `cx.now()` reads it (the
test clock in tests). macOS: `NSView.displayLink(target:selector:)` per window (macOS 14), with a
`preferredFrameRateRange` up to 120 while animating. Windows: per-window pacing from the window's
monitor (frame-latency waitable swap chain or `DCompositionWaitForCompositorClock`) instead of
GPUI's single `DwmFlush` clock, claimed only after a g815 trace at 240 + 144 Hz. Wayland:
`wl_surface.frame` plus `wp_presentation` for measured present times. X11: re-query randr on output
change. Idle windows request no frames; inactive ones cap at 30 fps. Budgets per display interval
(4.17 ms at 240 Hz to 16.6 ms at 60 Hz) feed the profiler. Window frames persist by
`WindowOptions` id and are clamped onto a live display when the saved monitor is gone.

### 5.4 Window controls, drag regions, traffic lights
- `drag_region()`: pointer-down starts the OS move (`performDrag(with:)`, `xdg_toplevel.move`,
  `HTCAPTION`); double-click follows `AppleActionOnDoubleClick` on macOS and maximizes elsewhere;
  interactive children opt out by hit-test, so title-row buttons occlude the caption by default.
- `window_controls()`: `.at(point(px(16.), px(18.)))` pins traffic lights at window coordinates;
  `.embedded()` makes it a layout slot and the backend moves the real buttons onto the slot's rect
  each layout pass, keeping hover group, menu and accessibility (port of the old
  `WindowControls.swift`, including riding a sliding sidebar). On Windows the system draws the caption
  buttons in the DWM-extended frame and the slot reserves their space (an app override can draw
  Fluent buttons with `WindowControlArea` hit-testing instead); on Linux it draws buttons only when decorations are
  client-side and `wm_capabilities` lists the action; on Hyprland it renders nothing and takes no
  space. Drawn variants take tokens so they match the app.

### 5.5 Tabs and tear-off
`tab_group()` intent: NSWindow tabbing on macOS, a framework tab strip elsewhere. A tab's view is
an entity, so dragging a tab to another window moves state with it; native surfaces in it are
reparented. Tear-off follows the pointer live where `xdg-toplevel-drag` exists (KWin, Mutter); on
Hyprland a drag preview follows the pointer and a new window opens on release
(`docs/research/linux-nix-config.md` section 2). Cross-group tabs never merge; a close is held
until the app drops the tab's view.

## 6. Input

### 6.1 Gestures
App surface: `.on_pinch`, `.on_rotate`, `.on_swipe_page`, `.on_force_click`, `.on_long_press`,
`.swipe_actions(..)`, `.on_drag(..)`, scroll views with momentum and rubber band, and
`cx.haptic(Haptic::Alignment | LevelChange | Generic)`.
- macOS: native recognition. Scroll views are `NSScrollView` (native momentum, elastic overscroll),
  pinch/rotate/smart zoom/force click come from `NSResponder` events, page swipes use
  `trackSwipeEvent`, row swipe actions are NSTableView row actions, haptics are
  `NSHapticFeedbackManager` with `DrawCompleted` timing.
- GPUI backends: `nd-ui` recognizers in an arena (inner scroller claims its axis, row swipes win
  after a direction lock), framework momentum where the OS sends none (Wayland has no kinetic
  scrolling, zed#14473), rubber band springs on the frame clock, DirectManipulation pan and pinch
  on Windows. Rotate is macOS only; compositor-owned finger counts (Hyprland `gesture =`) are
  never promised. Haptics are a no-op off macOS.

### 6.2 Focus and keyboard
Return activates and focus loss never does; keys typed right after an action that opens a field
are queued until it focuses; Esc closes the innermost overlay; menu accelerators work in dialogs;
declared accelerators pre-empt embedded pages. IME: NSTextField/NSTextView natively on macOS,
GPUI's `text-input-v3`/XIM/TSF on GPUI backends, CEF OSR through the same input handler.

### 6.3 Overlays: no leaks, always on top
Every floating element (popover, menu, tooltip, toast, command palette, sheet, dropdown) opens
through `cx.overlay(spec)` or a built-in that uses it, with modality `Modal`, `LightDismiss` or
`Passive`. macOS: overlays are native (`NSPopover`, `NSMenu`, child `NSPanel` for custom panels,
sheets), so they are separate windows above every view in the window and AppKit's tracking owns
their input; toasts are a top sibling view. GPUI backends: one overlay stack per window; the
input router checks it before content (outside press on `LightDismiss` closes and is consumed);
overlays intersecting a registered native surface (WebView2, WPE, iframe) are promoted to GPUI's
overlay composition surface under `nd-gpui/fast` (zed PR #62379) or to a child popup window;
menus that do not fit use `xdg_popup` with grab or owned popups. Overlays close when their anchor
subtree goes, on Esc, or on key-window loss. Gate: a tier 3 test opens a popover over a live
webview, clicks inside and outside and scrolls over it, and asserts the webview saw zero events,
on all three OSes.

## 7. Virtualization

One API: `list(&state, render_row)`, `table(&state, columns, render_cell)`, `grid(..)`,
`tree(..)` with `ListState<K>` holding keys and options. `nd-virtual` holds the backend-neutral
model: keyed diff, scroll anchor (key + offset), follow mode, measured-height cache keyed by row
key and width, selection, type-ahead. Presenters:
- macOS: view-based `NSTableView` (lists, tables), `NSOutlineView` (trees, source lists),
  `NSCollectionView` (grids). Cells are reused through `makeView(withIdentifier:)` and the row
  subtree is reconciled into the reused cell (the old `SourceTrees.swift` reused items by id so
  open branches survived updates). Heights come from Taffy measurement of the row at table width
  through `heightOfRow`; a row whose size changes calls `noteHeightOfRowsWithIndexesChanged`.
  Keyed diffs become `insertRows`/`removeRows`/`moveRow` with native animation. Anchoring on
  prepend and tail follow adjust the clip view origin in the same transaction.
- GPUI backends: our own element on GPUI's `Element` trait, not GPUI's `ListState`, because its
  contract ("items outside the viewport must not change height unless you call `splice`",
  `gpui/src/elements/list.rs:1-4`) is the source of "last rows disappearing".

Requirements from `docs/parity/apps.md` section 3, structural on both presenters:

| Requirement | Mechanism |
| --- | --- |
| (e) keyed updates never reset scroll | `set_items(keys)` diffs by key; no public splice/reset |
| (a) heights change after first layout | Rows report measured size; the height tree updates in the same frame |
| (b) prepend keeps the view still | Position is an anchor (key + offset), never raw pixels |
| (c) tail follow stops on user scroll | `Follow::Tail` disengages on scroll away, re-engages near the end |
| (d) jump to item with offset | `state.scroll_to(key, Align::Center, px(-96.))`, animated on the frame clock |
| (f) last rows always reachable | Estimates, idle-time measurement near the viewport, synchronous tail measure before painting the end |
| (g) clean rows do not repaint | Row subtrees are views; a row animation touches one row |

Also: bottom alignment for chat, sticky headers, `on_near_start/end`, selection modes, column
resize/reorder/sort events (sorting stays app-owned), drag reorder with edge auto-scroll, empty
states, wheel hand-off between nested scrollers. One conformance suite (randomized
insert/remove/resize sequences asserting the anchor row does not move, tail visibility, 100k rows
within frame budget on e1504g) runs against both presenters.

## 8. Webview and CEF

`NativeSurface` in `nd-ui` is how a platform view joins a window: rect per layout pass applied in
the frame's transaction, visibility, focus hand-off, `reparent`, and occlusion info for overlays.
- `webview()` element: WKWebView on macOS (an NSView sibling; native overlays sit above it);
  WebView2 in composition mode on Windows (a DirectComposition sibling visual, no airspace);
  WPE WebKit on Linux, rendering to dmabuf imported as a texture, because WebKitGTK cannot embed
  in a non-GTK Wayland window (gpui-webview: Wayland unsupported). If the M8 spike fails, Linux
  uses CEF OSR or WebKitGTK under XWayland. Web: an iframe (section 12).
- CEF (`nd-cef`, feature `cef`): OSR with `accelerated_osr` by default. macOS shows the IOSurface
  as CALayer contents in an NSView (no GPU surface needed); GPUI backends import it as a texture.
  `external_begin_frame_enabled` is driven by the window's display clock. OSR input, IME,
  `<select>` popups as overlays, context menus through our menus, drag and drop, cursor, screen
  info and the accessibility bridge are gated one by one. A windowed Chrome-style mode
  (`[webview.cef] style = "chrome"`) keeps extensions, 1Password and native messaging for
  nativebrowser on macOS, Windows and X11. Runtime loading, version pin, signed helper bundles,
  stripped `libcef.so`, zero Chromium bytes unless enabled. Ported behaviours: ordered shutdown,
  chromium children ignore SIGTERM/SIGINT, tab parking, per-origin permissions, app-owned
  downloads, popups adopted as tabs, content blocking.

## 9. Diagnostics, profiling, logging

### 9.1 Profiling
`nd::trace!`/`span!` compile to nothing unless built with `--cfg nd_trace` (zed's `ztracing`
pattern). Always on: info+ events into the flight recorder ring, GPUI's journal and hang detector
on GPUI backends, a main-thread hang detector on AppKit, per-window frame histograms against the
display interval. On demand: tracy (`ondemand`, `only-localhost`), GPU timestamps,
`ND_TRACE_STARTUP=1` chrome traces. Dev builds show a HUD (`secondary-alt-shift-p`): Hz, budget,
misses, slowest view renders.

### 9.2 Frame recorder
A ring of the last frames per window: Metal drawable copies and `CALayer` snapshots of the window's
backing on macOS, DX11 staging copies and wgpu readbacks on GPUI backends, read back only on
incident or test request. Composited capture (sheets, menus, glass) comes from `ndshell`.

### 9.3 Incident bundles
On panic, native crash (crash-handler + minidumper server, zed's `crashes` pattern), hang, or
`nd::report_issue()`, written to `<data>/diagnostics/<timestamp>-<kind>/`: dump or `panic.txt` or
`hang.json`; `log.ndjson` (last 5000 events + log tail); `tree.json` (semantic tree, windows,
displays with Hz, focus, open overlays; the automation `getTree` schema); `input.ndjson` (last 200
inputs, text redacted in release); `frames/` (last 5 per window); `profile.json`; `meta.json`
(version, sha, OS, GPU, backend, config hash). A memory-mapped snapshot refreshed every few
seconds supplies tree and inputs after a native crash. Optional Sentry upload per app.

### 9.4 Retention
Logs roll daily or at 10 MB, 7 kept, gzipped. Bundles: newest 20, none older than 14 days, 200 MB
total. Cleanup runs on a background thread after first frame. `nd diagnose --last` prints a
summary for agents; `--zip` packs it. On by default in every app; no app code needed.

## 10. Testing architecture (ndshell successor)

### 10.1 One semantic tree, three tiers
The retained `nd-ui` tree is the semantic tree on every backend: role, name, value, test id,
bounds, enabled, focused, checked, selected, expanded. It feeds AccessKit on GPUI backends,
NSAccessibility proxies on AppKit, the automation socket and incident bundles. Native controls
report values through `AppKitWidget::value`, so a locator never asks the OS accessibility API
(that is only a cross-check test).
- Tier 1, logic: `#[nd::test]` with the `TestBackend` (records backend ops, no OS) on GPUI's test
  dispatcher: deterministic clock, simulated input, resize, scale, refresh and display changes.
  Parallel, no window server, on every OS.
- Tier 2, pixels: GPUI headless rendering for the Linux, Fluent and web looks (Metal headless on
  macOS, wgpu headless on Linux with lavapipe in CI); offscreen AppKit windows for the AppKit
  backend in `harness = false` binaries, one process per test so they run in parallel. Glass is
  composited by the window server and only appears in tier 3 captures. Animation capture is
  `advance_clock(1/Hz)` + snapshot in a loop.
- Tier 3, real windows: the app as its own process with `NATIVE_AUTOMATION=1`; `nd-test` talks to
  it over the automation socket. For OS-owned behaviour only.

Locators are the same in every tier: `app.get_by_role(Role::Button).name("Save")`,
`get_by_test_id`, `get_by_text`, `get_by_label`, `locator("role=button[name=\"Save\"] >> nth=1")`,
the actionability ladder, `expect(..).to_be_visible().await`. Tier 3 adds `app.cursor`,
`app.capture` and `app.record`. Native AppKit controls are actioned semantically through their
target/action (about 1.7 ms per click in the old repo).

### 10.2 Real use and recording
macOS: input posted as `NSEvent`s into the app's queue (no TCC, window may be in the background);
`--real` uses the `ndshell` HID helper; gestures synthesised at the WindowServer layout fields;
`ndshell grant` writes TCC rows with SIP off keyed to the signing identifier and real path;
`CGVirtualDisplay` gives 120/144/240 Hz displays without hardware. Linux: one
`HYPRLAND_HEADLESS_ONLY=1` Hyprland per test with virtual pointer and keyboard,
`ext-image-copy-capture`, `wp_presentation`; gestures injected at the platform event layer.
Windows: `SendInput` and Windows.Graphics.Capture, serialized, over `ssh windows`. Private APIs
live in `ndshell` only.

### 10.3 Isolation
Every path (socket, data, settings, logs, diagnostics, single-instance lock) resolves from a
per-launch root set via `ND_APP_DATA_DIR`; sockets sit in a per-test temp dir and the child reports
the path on stderr. nextest runs each test in its own process; tier 3 on macOS and Windows runs in
a `max-threads = 1` group, Linux in parallel. Two agents testing two apps never collide.

### 10.4 Speed targets
| Loop | Target |
| --- | --- |
| Logic edit to test result (incremental, filtered) | under 3 s |
| One tier 2 snapshot after build | under 1 s |
| Full tier 1 + 2, parallel, macbook or g815 | under 60 s |
| One tier 3 test on Linux including Hyprland boot | under 5 s |
| VMs anywhere | none |

### 10.5 Build speed
nextest; one integration test binary per crate; deps at `opt-level = 2` in dev; Cranelift for
`nd dev` builds (LLVM for macOS test profiles, since Cranelift aborts on panic there); wild or mold
on Linux. Cargo `build-dir` on `/Volumes/Kyan Drive/dev-cache.noindex/` per workspace, `target-dir`
per worktree, sccache for third-party crates; a `just` guard fails readably when the drive is
missing; Spotlight off on it. g815 builds on local NVMe; e1504g never builds.

### 10.6 OS-release diff job
Per OS release (macOS major and betas, Windows 11 feature updates, GNOME/Hyprland releases for
reference), CI captures the gallery and `apps/reference-*` (minimal pure AppKit/SwiftUI and WinUI
apps with the same structure) on the old and the new OS. For each element mapped to a system
component (sidebar, toolbar, traffic lights, caption buttons, sheets, menus, controls), it diffs
the element's pixels by its semantic-tree bounds. An element that stayed identical while the
reference app's equivalent changed is flagged as a hand-drawn replica and fails the job.
`nd test os-release --from 26 --to 27` runs it.

### 10.7 MCP
`nd-mcp` keeps `execute({code})` (JS in `rquickjs` with the locator API bound), `snapshot`
(compact tree with `[ref=eN]`), `reset` (relaunch or reconnect via `ND_AUTOMATION_SOCKET`).

## 11. Packaging, signing, updates, Nix, config

### 11.1 `nativedesktop.toml`
One file per app beside `Cargo.toml`; name and version come from Cargo. The CLI emits a JSON Schema
for editors. Full examples: `docs/plan/examples/*/nativedesktop.toml`. Tables: `[app]` (id,
display name, icon, categories, URL schemes, `[[app.file_associations]]`), `[patterns]`,
`[webview]` and `[webview.cef]` (`enabled`, `style`), `[updates]` (feed, channels, ed25519 public
key), `[diagnostics]` (retention overrides, Sentry DSN), `[macos]` (minimum version, signing
identity, notarize, entitlements), `[windows]` (publisher, installer splash, accent, shortcuts,
signing), `[linux]` (formats), `[web]` (base path).

### 11.2 Commands
`nd package [mac|win|linux|web]`: macOS `.app` + Velopack `.pkg` + `.dmg`, codesigned inside-out
(CEF helpers with JIT entitlements) and notarized; Windows a branded Velopack `Setup.exe` (name,
icon, splash, publisher, accent from the file; MSI on request) signed with Azure Artifact Signing;
Linux a Velopack AppImage plus optional deb and Flatpak manifest; web a static bundle (section 12).
Release binaries target `apple_m1` and `x86_64_v2`. File associations and URL schemes go into
Info.plist, `.desktop` plus shared-mime XML, and the installer's registry entries.
`nd release --channel beta` packs deltas, signs `releases.<channel>.json` with ed25519, uploads.

### 11.3 Updates and settings at runtime
`nd-update` wraps Velopack's `UpdateManager` with an `UpdateSource` that verifies the feed
signature first (Velopack alone trusts feed hashes, `docs/research/windows-packaging-updates.md`
section 4); background check, apply on quit, a "what's new" view. Off on Nix and web.

Settings layers: defaults < Nix-managed file < user `config.toml` < `config.local.toml` (writable
overlay when the main file is a `/nix/store` symlink) < env `<APP>_*` < CLI flags. `toml` reads,
`toml_edit` writes so comments survive; own writes are tagged so the watcher ignores them; each
field carries `reload = live | restart`; Nix-managed keys show "Managed by Nix" with their option
path and "Copy as Nix". `settings_form::<T>()` renders the schema as native settings rows.
`Store<T>` keeps versioned machine state (debounced tmp+rename writes, migrate on load, flush on
exit). On web both live in localStorage.

### 11.4 Nix
`nativedesktop.lib.mkApp { src, pname; }` (crane) returns `packages.default` (runtime libs wrapped),
`devShells.default`, `checks` (clippy, nextest, fmt, HM module eval, settings schema in sync) and
`homeModules.default` (freeform `settings` typed by `pkgs.formats.toml`, typed options from the
committed `settings.schema.json`, matugen `theme` template, `autostart`, `environmentFile`). One
shared `cargoArtifacts`; attic or cachix for it.

## 12. Web backend

A real backend: `nd-backend-gpui` compiled to `wasm32-unknown-unknown` on `gpui_web` (WebGPU,
WebGL2 fallback), with `nd-os-web` services. The example crates in `examples/` build for wasm
unchanged; app code has no `cfg`.
- Look: the web renders the plain default GPUI look and imitates no OS ("native on desktop, on
  web it's just GPUI"). The web bundle ships no Linux or Fluent skin. The same app code stays
  native on macOS, Windows and Linux with zero changes; `.on_macos/.on_windows/.on_linux`
  closures do not run on the web, `.on_web(..)` does.
- Gaps filled: menus and context menus are drawn overlays; file open uses a hidden
  `<input type=file>` and save uses a download; clipboard uses the async Clipboard API (reads need
  a user gesture); settings and `Store<T>` live in localStorage; extra windows open as in-canvas
  windows or tabs inside the one canvas (`gpui_web` has one window,
  `docs/research/gpui.md` section 1); `webview()` is an iframe positioned over the canvas with
  overlays intersecting it promoted to a canvas stacked above; CEF, tray, notifications beyond the
  Notification API, credentials, global menus and native widgets without a drawn impl are
  unavailable.
- Capability checks: `cx.supports(Feature::MultiWindow)`, `Feature::FileSystem`, `Feature::Cef`,
  and so on, at runtime (the old `hasWidget`/`hasCommand` idea). APIs that cannot work on the web
  return `Err(Unsupported)` and are marked `#[nd::unavailable(web)]`, which `nd check --target web`
  turns into warnings at each call site that lacks a `supports` check.
- `nd build --target web` produces the bundle (wasm, JS loader, assets). Not in the update or
  packaging paths beyond `nd package web` producing a static folder.

## 13. Docs site and docs app

- The design-target apps are in `docs/plan/examples/`: `hello` (smallest app), `notes` (multi-file
  app with sidebar, toolbar, virtual list, palette, gestures, settings, tests), `native-widget`
  (objc2 MapKit view and a Swift-hosted SwiftUI gauge with drawn fallbacks), `overrides` (every
  override layer).
- Every docs page carries runnable example code with "view source" linking the full example app
  it comes from. Examples live in the repo's `examples/` directory (the `docs/plan/examples/` apps
  move there in M1) and are compiled for every target in CI, so docs code cannot rot; page
  snippets are extracted from those files by anchor comments, never pasted.
- `nd-docs` generates static HTML from Markdown, styled from `nd-theme` tokens. Built in CI and
  hosted on Cloudflare Pages. Every page also ships as raw Markdown and in `llms.txt`.
- Each example embed has four tabs next to its source. Web: the live canvas (section 12, plain
  GPUI look). macOS, Linux, Windows: screenshots, plus short frame recordings where motion matters,
  captured by the test harness on the real OS (macbook, g815 Hyprland, Windows PC) and labelled
  "captured on <OS>". Light and dark variants for each.
- Captures are regenerated by `nd docs shots` whenever an example's sources change (CI keys them
  by a hash of the example crate plus framework version), so no tab shows a stale screen. The
  macOS and Windows captures run in the release job on those machines; a missing or outdated
  capture fails the docs build.
- A gallery page lists the example apps with the same per-OS captures.
- `apps/docs` renders the same content natively with live examples in-process: the docs in our
  framework.

## 14. Parity map

Sections refer to `docs/parity/zig.md`; boxes there are ticked when the named gate passes.

| Old area | Lands in | Notes |
| --- | --- | --- |
| 1. 72 widgets + common props/events | `nd-components` built-ins on `NativeWidget`; drag/drop, tooltip, test id, context menu are modifiers on every element | AppKit impls on macOS like the old NDShell, drawn impls elsewhere |
| 1. style keys, cssClasses whitelist | Typed style methods and variant enums | Unknown keys become compile errors, stricter than runtime StyleError |
| 1. dropped-prop reset, set-replace | Reconciler | A modifier absent from a render resets that property on the retained node |
| 1. platform-only widgets | Tray in `nd-system` on all OSes; share button macOS with drawn fallback | One dev warning per unsupported mapping |
| 2. layout semantics, spacing defaults, pixel rounding, measure caching | Taffy + preset spacing; measure cache; frames rounded outward on macOS | Lessons of `Layout.swift` kept |
| 2. design language, sidebar classes, SF Symbols, icons, fonts, emoji | `nd-theme`, `nd-icons`, presets | Emoji and nerd glyph fixes become font setup |
| 3. multi-window, per-window scoping | `nd-ui`, `nd-automation` | Kept |
| 3. window pooling, `resolve_window`, crash/HMR respawn | Dropped | Single process; crash relaunch with state restore (3.3) |
| 3. native tabs, tear-off, cross-group rules | `tab_group()` (5.5) | Hyprland uses drop-to-new-window |
| 3. Portal, createPool, moveNode | Dropped | Entities move between windows; native surfaces reparent |
| 3. toolbar styles, autosave, density, sheets, fullscreen, HIG chrome, split views | `WindowOptions`, `scaffold`, `toolbar`, `split_view` | Breakpoint with 8 pt hysteresis, edge reveal kept |
| 3. dialogs, toasts, crash overlay | `nd-system`, toast built-in | Crash overlay replaced by relaunch + toast |
| 3. menu bar, context menus, popovers, palette, DnD, type-ahead | Intents, overlays, `nd-virtual` | Kept |
| 4. automation RPC, error codes, waitFor, dialog scripts, capture modes, helpers | `nd-automation`, `nd-test`, `ndshell` | Same method names and codes |
| 4. GTK `-32003 inputUnsupported` | Fixed | Linux tier 3 has real input |
| 5. system APIs | `nd-system`, `nd-config` | All three OSes plus web subset |
| 5. ACL grants | Narrowed | App code is trusted Rust; checks stay on web content bridges and the automation socket (per-launch token) |
| 5. native plugin C ABI, `<nativeview>` | Replaced | `NativeWidget` (3.5) and `NativeSurface`; C libraries via FFI crates |
| 5. `hasWidget`/`hasCommand` | `cx.supports(Feature)` | Compile-time availability for widgets |
| 5. update system | `nd-update` | Adds deltas, channels, Windows |
| 6. two processes, NDP, binary encoder, vtable ABI, codegen | Dropped | Single process; perf gates become frame-time and view-render gates |
| 6. Solid renderer, HMR, `nd-solid-build` | Replaced | `nd dev`: Cranelift incremental, relaunch with state restore, live theme/settings/assets; Rust hot-patching (`subsecond`) evaluated in M10 |
| 6. null backend + conformance | `TestBackend` + backend conformance suite | Kept in spirit |
| 7. webview API, engines, CEF features, adblock | `nd-webview`, `nd-cef`, `nd-adblock` | Request interception still absent |
| 8. terminal | `nd-terminal` | One renderer per backend kind |
| 9. data, store, rpc, panes, ui package | `nd-data`, `Store<T>`, `nd-ipc`, panes model, built-ins | Panes keep the same-value-when-unchanged contract |
| 10. CLI, packaging, release, CPU baselines | `nd-cli` | Adds Windows installer, web, `nd check` |
| 10. npm lockstep rule | Replaced | crates.io lockstep from one workspace version |
| 11. harness, locators, MCP, drive gates | `nd-test`, `nd-mcp` | Drives become tier 1-3 tests; MCP `execute` stays JS |
| 12. docs | `nd-docs`, `apps/docs` | Rewritten around live examples |
| 13. examples | `examples/` | Gallery, counter, multiwindow, tabs, browser, terminal, settings, tasks, panes, sourcetree, locators, dialogs, gestures kept; probes become tests |
| 14. hard-won behaviours | Named regression tests in the owning crate | JS-specific ones drop with the JS runtime |

## 15. Agent-first

### 15.1 Agent docs
A docs section, copied into each new project: `CLAUDE.md` and
`docs/agents/{patterns,widgets,testing,styling,performance,donts}.md`; every code block compiles.
The do's and don'ts lead with the OS-structure rule (3.10): do use `scaffold`, `toolbar`,
`window_controls`, sheets and menus and let the OS draw them; do read metrics and colours from
tokens; don't draw a replica of a sidebar, toolbar, traffic light, caption button or system
control, don't hardcode a system colour, radius, material or metric, and don't use `.drawn()` on a
structural element without a reason comment.

### 15.2 `nd new <name>`
```
<name>/
  Cargo.toml  nativedesktop.toml  CLAUDE.md  README.md  flake.nix  justfile
  .cargo/config.toml          build-dir guard, profiles
  .config/nextest.toml        tiers and groups
  src/{main.rs, app.rs, settings.rs, actions.rs, demo.rs}
  tests/{notes.rs-style tier 1, pixels.rs, real.rs}
  tests/golden/{macos,windows,linux,web}/
  assets/icon.icon
  nix/hm-module.nix           generated
  docs/agents/
```
The generated app builds for every target, opens a scaffold with sidebar, toolbar and list, has one
test per tier, and a demo mode (`<APP>_DEMO=1`) for tests and screenshots. CLAUDE.md template:
app description; commands (`just dev`, `nd check`, `nd test`); framework rules (intents not `cfg`,
tokens not colour literals, `list()` for any list over 50 rows, overlays only through `cx.overlay`
or built-ins, executor timers only, small views, test in the tier matching the behaviour); read
`nd diagnose --last` before reproducing a bug.

### 15.3 README template
Shape from `docs/parity/apps.md` section 4: centered 160 px icon, `# Name`, one plain sentence;
how it works in 2-3 sentences with a boxed ASCII diagram; one measured number; hero screenshot and
a two-column screenshot table from `nd docs shots`; Features; Install (Nix with Home Manager,
other Linux, macOS, Windows); Configuration; Keyboard table ("`Cmd` on macOS."); Development with
aligned commented commands; License. `<!-- nd: ... -->` placeholders fail `nd check --release`.

### 15.4 Gates that fail bad output fast
`nd check` (seconds): fmt, clippy with the framework lint table (`unwrap_used`, `expect_used`
outside tests and `main`, `dbg_macro`, `todo`, `print_stdout` denied), and framework lints: `gpui`
or objc2 named outside allowed places; `cfg(target_os)` outside native widget modules without a
reason comment; colour literals outside theme files; `Instant::now`, `thread::sleep`,
`smol::Timer` in UI code; inline SVG; hand-rolled scroll lists; floating elements not created
through overlays; web-unavailable APIs without a `supports` check (`--target web`). Tier 1 audits:
every interactive node has a role and a name or test id; no overlay without modality. Golden drift
fails until re-blessed with `ND_BLESS=1`; the diff image sits next to the test.

## 16. Milestones

`just gate mN` wraps `cargo nextest run --profile gate -E 'test(/^gate_mN::/)'` plus the tier 3
tests listed. Lanes run as parallel subagents in separate worktrees.

**M0 Skeleton, first mac window.** Workspace, cargo config with build-dir on the drive and mount
guard, nix dev shell, CI (macOS + Linux), nextest profiles, `nd-core`, `nd-gpui`, `nd-ui` core
(views, retained tree, reconciler, Taffy, `TestBackend`), `nd-backend-appkit` with NSWindow, layer
and text primitives, one native `button`, traffic lights by coordinates, `examples/hello`.
Gate on the macbook: tier 1 + tier 2 pass; `cargo run -p hello -- --nd-selftest resize` drives 50
live-resize steps and fullscreen in/out, asserting frame size equals window size and traffic
lights at the configured point after each.
Lanes: workspace/CI/nix; `nd-ui` core; AppKit host.

**M1 Diagnostics and harness.** Frame clock, `nd-diag`, frame recorder, semantic tree export,
`nd-automation`, `nd-test` locators in all tiers, `nd-mcp`, minimal `nd new/check/test`; move
`docs/plan/examples` into `examples/`. Gate: forced panic in `hello` yields a seven-part bundle; a
tier 3 locator clicks the native button; MCP `snapshot` returns the tree; retention deletes bundle
21. Lanes: diag; automation + test; CLI + MCP.

**M2 GPUI backend on Linux and Windows, pacing, chrome.** `nd-backend-gpui`, `nd-os-linux`
(xdg-decoration, `wm_capabilities`, Hyprland no controls), `nd-os-windows` (Mica, caption buttons,
Snap Layouts, `WM_POINTER`), per-window display clocks everywhere, `drag_region`,
`window_controls` (both placements), multi-window, resize gate on three OSes, headless Hyprland
runner. Gate on macbook, g815 (240 + 144 Hz) and the Windows PC: resize gate; `nd diag pacing`
shows each window at its display's Hz after a monitor move; no controls under Hyprland. Lanes:
GPUI backend; Linux; Windows; pacing patches; Hyprland runner.

**M3 Theme, skins, patterns.** Tokens, presets, Fluent skin, materials, matugen watcher and
template, appearance/accent, intents (scaffold, toolbar, sidebar, sheet, settings window, menu
bar, alerts, pickers), override layers. Gate: tier 2 goldens of `examples/overrides` and the
gallery per skin, light and dark; Fluent goldens within tolerance of WinUI Gallery references; a
`theme.json` write recolours a running app within 100 ms. Lanes: tokens + matugen; mac chrome;
Fluent skin; Linux chrome.

**M4 Input.** Gestures on both backend kinds, haptics, focus and keyboard rules, overlays with
promotion. Gate: tier 1 arbitration and momentum on the test clock; tier 3 overlay leak test over a
native surface on three OSes; pinch and page swipe through `ndshell` on macOS. Lanes: gestures;
overlays; focus.

**M5 Virtualization.** `nd-virtual` model, NSTableView/NSOutlineView/NSCollectionView presenters,
GPUI presenter. Gate: the shared conformance suite green on both presenters; 100k-row scroll
within budget on e1504g (`nd diag scroll --rows 100000`, built on g815). Lanes: model; mac
presenters; GPUI presenter.

**M6 Components.** Parity widgets and the ranked app components (command palette, source list,
toast/banner, dialogs and forms, tabs and panes, image pipeline, media transport, lyrics view,
carousel, composer, inspector), each with AppKit and drawn impls. Gate: per component tier 1
tests, tier 2 goldens per skin, role audit; `examples/gallery` covers all. Lanes: one per family.

**M7 Settings, config, system.** `nd-config`, `nd-settings` + `settings_form` + HM generator,
`Store<T>`, `nd-system`, `nd-ipc`, `nd-data`, `Plugin` with a Discord RPC example. Gate: settings
round-trip keeps comments; store-symlinked file shows read-only; `nix flake check` evaluates the
generated module; system API tests per OS. Lanes: settings + Nix; system per OS; ipc/data/plugin.

**M8 Native webviews.** `NativeSurface`, WKWebView, WebView2 composition, WPE spike then engine.
Gate: ported webview suite; overlay leak test with a live page on three OSes; resize gate with a
webview. Lanes: mac; Windows; Linux.

**M9 CEF and terminal.** OSR, external begin frame, input/IME/select/menus/drag, Chrome-style
mode, helper packaging, adblock; `nd-terminal`. Gate: page paces at display Hz on 240 + 144 Hz;
per-item OSR tests; an app without CEF has zero Chromium bytes; terminal incl. remote session.
Lanes: CEF core; CEF input; Chrome-style; terminal.

**M10 CLI, packaging, updates.** Full `nd new`, `nd dev` (Cranelift, relaunch with state restore,
`subsecond` evaluation), `nd package` for three OSes with signing, `nd release`, `nd-update`,
`mkApp`, README template. Gate: `nd new demo && cd demo && nd check && nd test && nd package` on
each OS; signed delta update 0.1.0 to 0.1.1; tampered feed rejected; `nix build` works. Lanes: CLI;
packaging per OS; updates; Nix.

**M11 Web backend.** wasm build of `nd-backend-gpui`, `nd-os-web`, drawn menus, file input and
download, Clipboard API, localStorage settings, in-canvas windows, iframe webview, capability
checks, `nd build --target web`. Gate: every crate in `examples/` builds for wasm32 unchanged; tier
2 goldens of the plain web look in headless Chromium via `ndshell`; capability lint passes. Lanes: wasm
backend; web services; capability checks.

**M12 Docs.** `nd-docs` site with four-tab embeds (live web, captured macOS/Linux/Windows) and
view-source, gallery, docs app, agent docs. Gate: all examples compile for every target; site
builds; every web embed loads; `nd docs shots --check` finds no capture older than its example;
link check passes.
Lanes: generator; docs app; content by section.

**M13 messages port** (after M7, parallel with M8 to M12). **M14 youtubemusic port** (after M7;
the daemon stays a process on `nd-ipc`). **M15 nativebrowser port** (after M9). Gate per port:
the app's suite green on its OSes, its old feature list checked, and its hand-rolled theme, live
theme, chrome, menus, single instance, trace, motion, toast and primitives deleted. Order:
messages exercises lists, overlays, chrome and matugen without a webview; youtubemusic adds media,
tray, lyrics and IPC; nativebrowser needs CEF, tabs and tear-off.

Files/Finder preparation, built along the way: `grid`/`table` sized for 100k entries
(NSCollectionView/NSTableView on macOS), column view as a row of lists, file drag in and out with
promise providers on macOS, Quick Look style preview overlay, a file watcher and off-thread
thumbnails in `nd-system`, volumes and favourites as a source list, a path breadcrumb.

## 17. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Two backend kinds double widget work | One public `NativeWidget` trait; drawn impls shared by Linux, Windows and web; AppKit impls only for controls where native feel matters; conformance suite per widget |
| Layout disagreement between Taffy and AppKit containers | Taffy authority, containers as layout boundaries, measure cache, one transaction per frame (3.7); the old NDBoxView lessons |
| Embedded GPUI surface on macOS needs a `PlatformWindow` patch | Used only for GPU-only content; CEF on macOS avoids it via CALayer contents; patch kept small and upstreamed |
| Single Longbridge maintainer behind gpui-pre, gpui-fast, gpui-kit | `nd-gpui` facade; fork can publish snapshots; only `gpui-base` used; `fast` optional |
| GPUI churn | Exact pins, weekly gated bump, small patch queue |
| Fluent skin drifts from WinUI | Reference goldens from WinUI Gallery; `winui` seam reserved; re-evaluate `windows-reactor` when it leaves preview |
| CEF size and OSR work | Opt-in, zero bytes by default; OSR items gated one by one; windowed mode for extensions |
| WPE WebKit unproven in our stack | M8 spike first; CEF OSR or XWayland WebKitGTK fallback |
| Wayland gesture gaps | Framework momentum and rubber band; pointer-gestures v3 bound ourselves; finger counts documented |
| Windows pacing follows DWM | Per-window clock patch, proven on g815 first |
| `gpui_web` experimental (one canvas, no menus) | Gaps filled in `nd-os-web`; web is a docs target first; capability checks |
| objc2 releases trail Xcode | Pinned git rev; shim file for missing selectors |
| Swift toolchain in the build (Command Line Tools, SDKROOT quirks, Swift runtime linking) | Isolated in `nd-backend-appkit`'s build script; `nd doctor` check; Nix dev shell provides it; Swift kept to hosted leaves and newest-API branches |
| Private APIs in tests break per macOS release | Isolated in `ndshell`; tiers 1-2 independent of them |
| e1504g performance unknown | Measured from M2; view-render budget in the HUD |
| Hyprland lacks `xdg-toplevel-drag` | Drop-to-new-window fallback |
| External drive latency, APFS unreadable on Linux | Measured in M0; mount guard; g815 local NVMe |
| Rust reload slower than Solid HMR | Fast relaunch with state restore, live theme/settings; `subsecond` evaluated in M10 |

### Open questions for the user
1. Update feed host: Cloudflare R2 under `updates.canarycoders.es`, or GitHub Releases?
2. Signing: Azure Artifact Signing (about $10/month through CanaryCoders as an EU organization)
   for Windows, and is an Apple Developer ID available?
3. macOS floor: 14 (display links per view, vibrancy below 26) or 26 (glass only)?
4. Publishing: crates.io in lockstep under MIT like the apps, or git-only for now?
5. The `zig` branch: keep cutting patch releases for Lynk (nativebrowser on 0.4.56) until M15, or
   freeze it now?
