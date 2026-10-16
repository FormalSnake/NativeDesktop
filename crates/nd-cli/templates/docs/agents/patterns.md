# Patterns

How an nd app is put together, and the shape every new screen should take. The short list of
hard rules is in [donts.md](donts.md).

## Let the OS draw its own structure

Big structural pieces are intents. You say what the window has; each backend maps it to the
highest-level component its OS offers, so the next OS release restyles the app with no change
here.

| You write | macOS draws | Linux and Windows draw |
| --- | --- | --- |
| `scaffold().sidebar(sidebar())` | `NSSplitViewItem` sidebar with a source list | GPUI sidebar on the skin's sidebar surface |
| `scaffold().inspector(inspector())` | the inspector split item | a right pane |
| `toolbar()` | `NSToolbar` with system items | a header row |
| `source_list(id)` / `source_tree(id)` | source-list `NSOutlineView` | drawn tree on the sidebar surface |
| `titlebar()` + `window_controls()` | real traffic lights in the slot | caption buttons, or nothing on Hyprland |
| `.context_menu(..)` | `NSMenu` | menu in the overlay layer |
| `window.open_sheet(..)` | a window sheet | a dialog over a scrim |
| `window.alert(..)`, `window.confirm(..)` | `NSAlert` | task dialog or portal |
| `glass()` | `NSGlassEffectView` (26+), `NSVisualEffectView` below | Mica, Acrylic or compositor blur |

Do:

```rust
scaffold()
    .toolbar(toolbar().title("Inbox").sidebar_toggle()
        .item(icon_button("new", Icon::SquarePen).tooltip("New Message").dispatch(NewMessage)))
    .sidebar(sidebar().min_w(px(180.)).child(
        source_list("folders")
            .section("Mailboxes", [source_item("inbox", "Inbox").icon(Icon::Inbox).badge("12")])
            .selected(self.folder.clone())
            .on_select(cx.listener(|this, e: &SourceSelected, _, cx| {
                this.folder = e.id.clone();
                cx.notify();
            })),
    ))
    .content(self.detail.clone())
```

Don't:

```rust
// A sidebar replica: a fixed-width div with a hand-picked grey and hand-drawn rows.
h_flex()
    .child(v_flex().w(px(220.)).bg(rgb(0xf2f2f7)).children(folders.iter().map(row)))
    .child(detail)
```

The replica never picks up the system sidebar material, selection style, keyboard behaviour or
accessibility, and it trips `colour-literal` and `scroll-list` in `nd check`.

A custom titlebar is `WindowOptions::main("main").titlebar(Titlebar::Custom)` plus a
`titlebar()` element. `titlebar()` reserves the OS controls' side and size as insets (traffic
lights left on macOS, caption buttons right on Windows, nothing on Hyprland), so content never
pads past the controls by hand. Move the traffic lights with
`window_controls().at(point(px(18.), px(20.)))` or let toolbar content carry them with
`window_controls().embedded()`.

## State lives in entities

- One entity per piece of state with its own lifetime (`cx.new(|cx| ..)`). A view reads state in
  `render` and changes it in handlers, then calls `cx.notify()`.
- `cx.notify()` re-renders that view only. Child entities are boundaries: split a screen into
  views so typing in the editor does not rebuild the sidebar.
- `cx.observe(&entity, |this, entity, cx| ..)` reacts to another entity's `notify`. Keep the
  returned `Subscription` in the view, or `.detach()` it on purpose.
- Handlers come from `cx.listener(|this, event: &E, window, cx| ..)`; row renderers for lists come
  from `cx.processor(Self::row)`.

## Actions, keys and menus

Commands are actions. One action drives the button, the shortcut and the menu item, so they
never disagree.

```rust
actions!(mail, [NewMessage, DeleteMessage]);

cx.bind_keys([KeyBinding::new("secondary-n", NewMessage, Some("Mail"))]);

div().key_context("Mail").on_action(cx.listener(|this, _: &NewMessage, window, cx| this.compose(window, cx)))
```

`secondary` is Cmd on macOS and Ctrl elsewhere. Buttons fire actions with `.dispatch(Action)`;
the menu bar is built from the same actions with `set_menus(cx, ..)`.

## Controlled values

The app owns every value a control shows. A control reports a change as an event, the app
stores it and renders it back. Feeding the reported value straight back is always safe: nd
does not set a native control to the value it already holds, and events fired while nd applies
a value are dropped, so there is no echo loop and no need for "ignore the next change" flags.

```rust
slider("volume", self.volume).range(0.0..=1.0).on_change(cx.listener(|this, e: &SliderChanged, _, cx| {
    this.volume = e.value;
    cx.notify();
}))
```

To refuse a change, leave the state alone. The node emitted an event, so nd reaffirms it on the
next frame and the control shows the app's value again. Writing a native widget yourself? The
same contract is yours to keep with `Synced<T>`; see [widgets.md](widgets.md).

Text fields are native-owned while focused: `TextFieldState::set_text` from the app waits out
IME composition, and `TextFieldState::last_event()` says whether the user (`TextEvent::Edited`)
or the app (`TextEvent::Replaced`) made the last change.

## Floating things are overlays

Popovers, panels and tooltips are elements in the tree: `popover(id)`, `panel(id)`,
`tooltip()`. The parent is the anchor; the overlay takes no room in its layout and closes with
it. Dismissal is the app's: `on_dismiss` hears `Dismissed { reason }` and the app stops
rendering the overlay.

```rust
.when(self.popover, |d| d.child(
    popover("filters").p_3().child(filters)
        .on_dismiss(cx.listener(|this, _: &Dismissed, _, cx| {
            this.popover = false;
            cx.notify();
        })),
))
```

Modality decides input outside: `LightDismiss` (popover default) closes on an outside press and
eats it, `Modal` (panel default) eats it and stays, `Passive` (tooltip) takes nothing. Input
inside an overlay never reaches what is under it, web views included. Menus, the command
palette, toasts and sheets are built-ins on the same layer. An absolutely positioned div or
GPUI's `deferred`/`anchored` used as a popup leaks clicks and hover into what is beneath and
fails `floating-element`.

## Async and time

- Work off the main thread: `nd::system::spawn(future, |output, cx| ..)`. The future runs on a
  worker; the closure runs on the main thread and may touch entities.
- Time in a view comes from the frame clock: `window.now()` (seconds, the frame's target
  present time) or `cx.now()`. Tier 1 tests move it with `cx.advance_clock(1. / 60.)`, so
  anything driven by it is deterministic.
- Wall-clock timestamps: `nd::time::SystemTime`, which also works in a browser.
- No `Instant::now`, `thread::sleep` or `smol::Timer` in UI code (`clock-in-ui`). A sleep on the
  main thread freezes the window; a wall clock in a view makes its test flaky.

## Platform differences

Use intents and the `.on_macos(..)` family, never `cfg!(target_os)` in view code. When a
capability is missing on a target, ask: `nd::platform::has(Feature::MultiWindow)`. `cfg` belongs
in native widget modules (`src/native/`, or files named `appkit.rs`, `swift.rs`, `win32.rs`,
`wayland.rs`); anywhere else it needs a reason comment (`cfg-target-os`).

## Files and processes

Code and scripts delete only what they created, by a path they built and checked. Never
`rm -rf "$DIR/"` or `remove_dir_all(dir)` on a variable that can be empty or point outside a
directory you own; keep one helper that refuses an empty path and anything outside your root,
the way `nd`'s own CLI only clears directories under `target/nd`. Never `pkill -f <pattern>`:
it matches other agents' processes too. Stop what you started by pid.
