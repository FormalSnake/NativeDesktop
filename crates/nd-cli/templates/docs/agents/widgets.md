# Widgets

Which element to reach for, how lists work, and how to add a widget the framework does not
ship. Rules in short: [donts.md](donts.md).

## Use a built-in first

Every built-in maps to the platform's own control where one exists (an `NSButton`, an
`NSSlider`, an `NSOutlineView`) and draws a matching control elsewhere. Pick by intent:

| Need | Element |
| --- | --- |
| Window structure | `scaffold`, `toolbar`, `sidebar`, `inspector`, `titlebar`, `window_controls`, `split`, `tab_view` |
| Navigation | `source_list` (sections of items), `source_tree` (flat id/parent nodes, expansion, row actions), `tab_strip`, `command_palette` |
| Buttons | `button`, `icon_button`, `link`, `button_group` |
| Text input | `text_field`, `secure_field`, `search_field`, `text_area`, `combo_box` |
| Choices | `switch`, `checkbox`, `radio_group`, `segmented`, `select`, `slider`, `stepper`, `date_picker`, `color_well` |
| Settings | `form`, `form_section`, `form_row`, `settings_group`, `switch_row`, `sheet_layout` |
| Feedback | `progress`, `spinner`, `toast`, `banner`, `status_page`, `window.alert(..)` |
| Data | `list`, `table`, `grid`, `tree` (virtual, see below) |
| Media | `image`, `avatar`, `seek_bar`, `player_bar`, `lyrics_view`, `shelf`, `chat_bubble`, `composer` |
| Surfaces | `glass`, `glass_group`, `webview` (feature `webview`), `native_surface` |

Icons are the `Icon` enum (Lucide names, SF Symbols on macOS); `icon(Icon::Moon, 12.)` shows one
on its own, tinted by `text_color`. No inline SVG (`inline-svg`).

Stateful elements take an id in their constructor: `text_field("title")`, `list("notes", ..)`,
`webview("page", &page)`. The id keeps the native view, its focus, IME state and scroll position
when a sibling appears or disappears above it. Give every interactive element an id or a label:
the id is its test id, and the label is its accessible name, so tests and screen readers both
find it.

## Lists: virtual, keyed, never a scroll div

Anything that can pass 50 rows is a `list()` (or `table`, `grid`, `tree`) over a
`ListState<K>`. Rows render only near the viewport; on macOS the presenter is a real
`NSTableView`.

```rust
pub struct Inbox {
    messages: Entity<Store>,
    list: ListState<MessageId>,
}

impl Inbox {
    fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let mut list = ListState::new(cx).follow(Follow::Tail).estimated_row_height(px(44.));
        list.set_items_versioned(store.read(cx).rows(), cx);
        cx.observe(&store, |this, store, cx| {
            let rows = store.read(cx).rows();
            this.list.set_items_versioned(rows, cx);
        }).detach();
        Self { messages: store, list }
    }

    fn row(&mut self, id: MessageId, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message = self.messages.read(cx).get(id);
        text(message.subject.clone())
    }
}

// in render:
list("messages", &self.list, cx.processor(Self::row))
    .selection(Selection::single())
    .on_select(cx.listener(|this, e: &SelectEvent<MessageId>, _, cx| this.open(e.key, cx)))
```

- Keys, not indexes. `set_items(keys)` diffs by key, so the scroll position survives every
  update and nothing resets. There is no splice and no reset to get wrong.
- Versions: `set_items_versioned((key, version))` re-renders only rows whose version changed.
  Bump a row's version when its content changes; leave the rest alone.
- Chat and logs: `Follow::Tail` pins to the end while the user is there and lets go when they
  scroll up. `.alignment(Alignment::Bottom)` rests short content on the bottom edge.
- Jump to a row with `state.scroll_to(key, Align::Center, px(-96.), cx)`. Paging:
  `.on_near_start(..)` and `.on_near_end(..)`.
- Rows may change height after first layout; the height tree updates in the same frame, so the
  last rows stay reachable and a prepend does not move what the user is reading.

Don't:

```rust
div().overflow_y_scroll().children(messages.iter().map(|m| row(m)))
```

That builds every row every render, loses the scroll anchor on updates and fails `scroll-list`.

Sorting stays the app's: `table(..)` reports `SortChanged`, the app re-sorts its keys and calls
`set_items` again. Expansion in `tree`/`source_tree` is app state too, so an unrelated render
never collapses a branch the user opened.

## App-defined native widgets

A widget the framework does not ship (a map, a gauge, a platform picker) implements the same
`NativeWidget` trait every built-in uses, and is called like any built-in:

```rust
pub struct MapWidget;

impl NativeWidget for MapWidget {
    type Props = MapProps;   // Clone + PartialEq: only changed props cross to the backend
    type Event = MapEvent;   // std::convert::Infallible when it emits nothing

    fn semantics(props: &MapProps) -> Semantics {
        Semantics::new(Role::Group).label("Map").value(format!("{} places", props.pins.len()))
    }

    fn drawn(props: &MapProps, emit: Emit<MapEvent>, window: &mut Window, cx: &mut App) -> AnyElement {
        drawn::render(props, emit, window, cx)
    }

    fn backends() -> Backends<Self> {
        let backends = Backends::drawn_only();
        #[cfg(target_os = "macos")]
        let backends = backends.appkit(appkit::MapKit);
        backends
    }
}
```

- `drawn` is required. It is the implementation on every backend without a native one (Linux,
  Windows, the web, tests), built from ordinary elements and theme tokens. Write it first.
- `backends()` adds native implementations: `.appkit(..)` for an objc2 view, `.swift(SwiftWidget::named("Gauge"))`
  for a Swift or SwiftUI view from the app's Swift package.
- The element is a struct with a `NativeBase<W>` and the props, `#[derive(IntoElement)]`, and an
  `impl NativeElement` with `base_mut` and `into_parts`. Expose a constructor function that takes
  the id: `map_view("map", center)`.
- Put objc2 and `cfg(target_os)` code in the widget's own module (a `native/` directory, or a
  file named `appkit.rs`); `nd check` allows them there and nowhere else
  (`framework-internals`, `cfg-target-os`).

Status: the trait and the element shape are fixed. Backends do not mount app-defined widgets yet
(`nd::native` is at its call-site stage), so check the widget in a real window before
relying on it.

### Native callbacks never call app code

AppKit fires delegates from inside setters (`setRegion` calls `regionDidChange`,
`selectRowIndexes` calls `selectionDidChange`). In an nd widget:

- Delegates emit through the `Emit` from `cx.emitter()`. It queues; the app's handler runs after
  the frame, outside every borrow.
- Wrap every programmatic setter in `cx.programmatic(|| ..)`. Events emitted inside it are
  dropped, so the app never hears its own write.
- Keep a `Synced<T>` per controlled prop in the widget's `State`:

```rust
// update(): set only when the control does not already hold the value.
state.viewport.borrow_mut().apply(&(new.center, new.zoom), |&(center, zoom)| {
    cx.programmatic(|| unsafe { map.setRegion_animated(region_for(center, zoom), true) });
    read_back(map) // what the control holds now, after it normalised the value
});

// delegate: record what the user produced, then emit it.
self.ivars().viewport.borrow_mut().report((center, zoom));
self.ivars().emit.emit(MapEvent::RegionChanged(RegionChanged { center, zoom }));
```

`apply` skips the setter when the value equals what the control holds (last applied or last
reported), and returns the normalised value when the control changed it (a map fitting a region
to its aspect ratio, a slider snapping to a step) so the widget can report it. An app that feeds
the reported value back therefore causes no second set and no loop. Test it: set a value the
control normalises from inside the widget's own event handler and assert one event, one set.

## Native surfaces

`webview(id, &page)` shows a `WebView` the app owns; unmounting parks the page instead of
reloading it. `native_surface(id, SurfaceSource::..)` hosts other platform views. Both sit in the
same layout and z-order as everything else, and overlays above them get the input, never the
page.
