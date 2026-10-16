+++
title = "Gestures"
description = "Tap, double tap, long press and drag, trackpad pinch, rotate and page swipes, momentum scrolling, haptics, and the focus rules that go with them."
+++

Gestures are handlers on elements: `.on_tap(..)`, `.on_pinch(..)` and so on. Recognition is
native where the OS has it (macOS) and done by [`nd-gesture`](../../../crates/nd-gesture/src/lib.rs)
elsewhere. Your handler gets the same event type either way, and always on the UI thread, never
from inside a native callback.

<!-- docs:example notes -->

## Pointer gestures

| Handler | Event | Notes |
| --- | --- | --- |
| `.on_tap` | `TapEvent { position, count }` | a press released in place |
| `.on_double_tap` | `DoubleTapEvent` | with it set, a single tap waits out the double-click interval |
| `.on_long_press` | `LongPressEvent { position, phase }` | after 0.5 s without moving |
| `.on_drag` | `DragEvent` | any direction; `.on_drag_axis(axis, ..)` for one |

A press is a tap until it moves more than 4 points, then a drag can claim it. Positions are in
window coordinates.

Phased events carry a `GesturePhase`: `Began`, `Changed`, `Ended` or `Cancelled`. Treat
`Cancelled` as undo. It means another recognizer won the press, the OS took the pointer, or a
swipe fell short, so whatever you previewed on `Began` and `Changed` has to go. A drag's `Ended`
carries `velocity` in points per second, for a fling.

<!-- docs:snippet examples/scroll/src/gestures.rs#handlers -->

Wiring is one handler per gesture:

<!-- docs:snippet examples/scroll/src/gestures.rs#wiring -->

This `Pad` lives in the scroll example so that it compiles with CI. The benchmark window does not
open it.

### Nested recognizers

Handlers on nested elements do not both fire for one press. The innermost recognizer to claim
the press wins, and ties go to the innermost. An inner tap and an outer drag never both fire
for one press: the tap is claimed on release, the drag once the pointer passes the slop.

## Trackpad gestures

`.on_pinch` delivers `PinchEvent { scale, phase, position }`. `scale` multiplies, so the handler
is `zoom *= event.scale`. The notes editor zooms its text this way and plays a haptic when the
zoom crosses 100%:

<!-- docs:snippet examples/notes/src/editor.rs#pinch -->

- `.on_rotate` gives `RotateEvent { degrees }`, the change since the last event, counterclockwise
  positive.
- `.on_swipe_page` gives `SwipePageEvent { direction, progress, phase }` for a two-finger
  horizontal swipe between pages, `Back` or `Forward`. `progress` runs from 0 to 1 while the
  fingers track; `Ended` means it committed, `Cancelled` that it fell back. On macOS it follows
  the user's "Swipe between pages" setting.
- `.on_force_click` is a deep press on a Force Touch trackpad. `.on_smart_magnify` is the
  two-finger double tap.

Where these arrive:

| | macOS | Linux, Windows, web |
| --- | --- | --- |
| Pinch | `NSResponder` magnify events | forwarded from GPUI where its platform reports one |
| Rotate, force click, smart magnify | yes | never delivered |
| Page swipe | `trackSwipeEvent` | not delivered yet |

Windows DirectManipulation and Wayland swipe and hold gestures need patches to the GPUI fork
that have not landed. Finger counts the compositor owns, such as Hyprland's `gesture =` rules,
are never promised. Build the same action behind a button or a key as well.

## Scrolling

`div().overflow_y_scroll()` (and `overflow_x_scroll`, `overflow_scroll`) makes a scroll view.
On macOS it is an `NSScrollView` with native momentum and elastic overscroll. On the GPUI
backends the framework does both:

- Momentum decays exponentially at Apple's 0.998 per millisecond, evaluated in closed form, so
  the path is identical at 60 Hz and 240 Hz. Wayland has no kinetic scrolling and GPUI ignores
  `momentumPhase`, so the framework owns it.
- Rubber band uses `x * d * c / (d + c * x)` with `c = 0.55`, and a critically damped spring
  brings the content back.

Lists scroll the same way; see [virtualization](virtualization.md).

### Who gets a scroll

A two-finger scroll locks to an axis after 8 points. After the lock the innermost consumer of
that axis wins: a scroller with room, a row swipe, or a page swipe. A row swipe inside a
vertical list therefore never fights the list, and a horizontal scroller inside a vertical one
keeps its own axis.

### Row actions

Swipe actions on list rows come from the list (`list(..).row_actions(|key, cx| ..)`), not from
a gesture handler, because macOS builds them from the table's delegate and a misuse should not
compile. The example is in
[virtualization](virtualization.md#selection-activation-and-row-actions).

## Haptics

`cx.haptic(Haptic::Alignment)` plays a trackpad haptic timed to the frame being drawn.

| Haptic | Use |
| --- | --- |
| `Alignment` | something snapped: a guide, a detent, a zoom reaching 100% |
| `LevelChange` | a discrete threshold: a full-swipe point, a stepper |
| `Generic` | anything else |

macOS has no custom waveforms, so these map onto the three `NSHapticFeedbackManager` patterns.
It is a no-op on every other OS and on trackpads without Force Touch, so call it without
checking.

## Overlays and gestures

Gesture hit testing starts inside the topmost overlay, or hits nothing. A tap on a popover never
reaches a handler underneath it. See [overlays](overlays.md#input-never-leaks).

## Focus and keyboard

These rules are the framework's, not yours to implement:

- Return activates the window's default button, the first one in tree order inside the innermost
  focus scope. Losing focus never activates anything. A multi-line editor keeps Return for
  itself with `.takes_return()`.
- Tab walks focusable elements in tree order, or in `.tab_index(n)` order. `.tab_stop(false)`
  keeps an element out of the walk while code can still focus it.
- A focus scope saves focus when it mounts and gives it back when it unmounts. A trapping scope
  keeps Tab inside it. Popovers, panels and sheets are scopes.
- Escape closes the innermost overlay that nothing took it from.
- Focus rings show for keyboard focus and not for pointer focus.
- Declared accelerators run before an embedded page sees the key, so a web view cannot swallow
  Cmd-T.

Text fields are `NSTextField` and `NSTextView` on macOS and GPUI's text input elsewhere, so
IME composition goes through the platform's own path on each.
