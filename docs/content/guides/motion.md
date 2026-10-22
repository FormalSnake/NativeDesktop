+++
title = "Motion"
description = "Platform timings for every animation, hover groups that reveal actions, shared element transitions, presentations, and the lyrics depth of field."
+++

Every animation in nd goes through one set of platform timings. The theme's preset carries
`motion` (fast, normal and slow durations, the platform's curve and its spring), and the host
copies it into the app's `MotionTokens`. An element asks for a pace rather than a duration, and
gets the platform's timing for it:

| Pace | Use | macOS | GNOME | Windows |
| --- | --- | --- | --- | --- |
| `Animation::fast()` | hover, press, small state changes | 150 ms | 200 ms | 83 ms |
| `Animation::normal()` | something coming in or going out | 250 ms | 250 ms | 167 ms |
| `Animation::slow()` | larger moves | 400 ms | 400 ms | 250 ms |
| `Animation::natural()` | interruptible moves, the platform spring | response 0.35 | response 0.2 | response 0.3 |

The curve is `CAMediaTimingFunction`'s default on macOS, ease-out-cubic on GNOME and WinUI's
entrance curve on Windows. With reduce motion on, every paced animation becomes a short fade:
offsets, scales and sizes jump, and opacity still eases.

```rust
div()
    .opacity(if shown { 1. } else { 0. })
    .animate(Animation::normal())
```

`Animation::timed(..)` and `Animation::spring(..)` still take explicit values, for motion that
has a reason to differ from the platform. `ND_ANIMATION_SLOWDOWN=8` stretches every paced
animation eight times, to watch one frame by frame.

## Hover groups

A card or a row is a hover group. While the pointer is anywhere over it, descendants that ask
for `.group_hover(name, ..)` take their hover style, easing in and out on `Animation::fast()`.
`.reveal_on_hover(name)` is the common case: hidden at rest, faded in on hover.

```rust
div()
    .group("card")
    .child(artwork)
    .child(div().reveal_on_hover("card").child(icon_button("card.more", Icon::Ellipsis)))
```

The group name is looked up among the group's own descendants, so every card can use the same
name. Revealed controls stay in the accessibility tree at rest, so a screen reader reaches them
without a pointer.

### Actions over artwork

`artwork_actions` draws Music's hover: the artwork darkens and Play and More fade in over its
bottom corners. Put it in a box with the artwork that clips to the artwork's corners. On a row,
`.compact()` puts the Play glyph alone in the middle of the small artwork, and
`hover_menu_button` adds More at the end of the row.

```rust
let art = div()
    .relative()
    .rounded_lg()
    .overflow_hidden()
    .child(image(cover).size(px(168.)))
    .child(
        artwork_actions("album.actions", "card")
            .on_play(move |_, cx| play(cx))
            .menu(move |menu, _, cx| album_menu(menu, cx)),
    );
v_flex().group("card").child(art).child(label(title))
```

## Shared element transitions

`.hero_from(id, away)` makes an element travel from the frame of the element keyed `id` when it
mounts, and back onto that frame while `away` is true. It runs on the platform spring, so a new
target mid-flight turns it around from where it is. Its size animates when given in points, so
give it a box of its final size and place it absolutely in that box, and nothing else moves:

```rust
div().size(px(480.)).child(
    div()
        .absolute()
        .top_0()
        .left_0()
        .size(px(480.))
        .hero_from("player.bar.art", closing)
        .child(cover.size_full()),
)
```

The element shows nothing on its first frame, while its own frame is laid out. Then it sits on
the source for the fast pace and travels. With reduce motion on, it appears in place.

## Presentations

`presentation` shows a view over a region: an expanded player over the page, a detail over its
list. The view keeps a `Presentation` handle and opens or closes it. The base stays mounted, so
it keeps its scroll position, and is hidden while the presentation covers it.

The motion is built around a hero inside the content, `away` set from `is_closing()`. Opening,
the backdrop fades in on the platform spring while the hero travels out of its source, and the
rest of the content, marked `.reveal_with(&state)`, fades in once the hero has landed
(`Pace::Settled`). Closing runs backwards: the revealed content fades first, then the hero
travels home while the backdrop fades out with it (`Pace::Exit`). The hero is never half
transparent and nothing draws over it in flight. Closing mid-open turns everything around from
where it is.

```rust
// In the view: self.expanded = Presentation::new();
self.expanded.set_open(true);

presentation("now-playing", &self.expanded)
    .base(page)
    .backdrop(image(cover).size_full().blur(px(64.)))
    .content(self.now_playing.clone())
    .bg(cx.theme().surface.base)
```

`image(..).blur(radius)` decodes the picture a few pixels across and stretches it, which gives
the same wash of its colours on every backend. Lay a scrim of a surface token over it for text.

## Player layout

`player_layout` lays out an expanded player for the room it is given. From 760 points wide the
artwork and the details sit in a column on the left and the panel (lyrics, the queue) on the
right; narrower, a header row holds a 64 point artwork beside the details, above the panel. The
artwork gets its side in points from the layout, so it never leaves its column and can be a hero.

```rust
// In the view: self.layout = PlayerLayoutState::new();
player_layout("now-playing", &self.layout)
    .artwork(move |side| cover.size(px(side)))
    .details(v_flex().child(label(title)).child(label(artist)))
    .panel(lyrics)
```

## Lyrics depth of field

`lyrics_view` keeps the line being sung sharp and whole, and fades and blurs the others more the
further they are from it (`DEPTH_OPACITY`, `DEPTH_BLUR`). As the line advances, each row eases
to its new depth on the slow pace. While the user scrolls, every row is sharp and whole; resync
brings the depth back. The app only supplies the lines and the clock.

On macOS each row's layer carries a Core Image gaussian blur whose radius animates. GPUI has no
blur filter for text, so on Linux and Windows a blurred row is drawn as faint copies on a ring
of the blur's radius, which add up to the row's opacity with soft edges. Only rows on screen get
the copies.
