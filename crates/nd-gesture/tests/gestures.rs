//! Tier 1: arbitration and momentum on the test clock (ARCHITECTURE.md 6.1, M4 gate).

mod gate_m4 {
    use nd_core::{NodeId, Point, Rect, Size};
    use nd_gesture::*;

    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    fn size(width: f32, height: f32) -> Size {
        Size { width, height }
    }

    const BOUNDS: Rect = Rect::new(0., 0., 400., 600.);

    /// Feeds `steps` scroll deltas at 120 Hz starting at `t`; returns the time after the last.
    fn fingers(router: &mut GestureRouter, hit: &[HitEntry<'_>], delta: Point, steps: usize, t: f64) -> f64 {
        let mut t = t;
        let mut out = Vec::new();
        out.extend(router.scroll(
            &ScrollInput {
                position: p(10., 10.),
                delta: p(0., 0.),
                phase: ScrollPhase::Began,
                momentum: MomentumPhase::None,
                time: t,
            },
            hit,
        ));
        for _ in 0..steps {
            t += 1. / 120.;
            out.extend(router.scroll(
                &ScrollInput {
                    position: p(10., 10.),
                    delta,
                    phase: ScrollPhase::Changed,
                    momentum: MomentumPhase::None,
                    time: t,
                },
                hit,
            ));
        }
        t
    }

    fn lift(router: &mut GestureRouter, t: f64) -> Vec<Output> {
        router.scroll(
            &ScrollInput {
                position: p(10., 10.),
                delta: p(0., 0.),
                phase: ScrollPhase::Ended,
                momentum: MomentumPhase::None,
                time: t,
            },
            &[],
        )
    }

    /// Steps the frame clock at `hz` until nothing animates; returns the offsets seen and the end.
    fn run(router: &mut GestureRouter, node: NodeId, from: f64, hz: f64) -> (Vec<f32>, f64) {
        let mut t = from;
        let mut seen = Vec::new();
        for _ in 0..(hz as usize * 10) {
            t += 1. / hz;
            router.step(t);
            seen.push(router.scroll_offset(node).y);
            if !router.needs_frame() {
                break;
            }
        }
        (seen, t)
    }

    fn vertical() -> GestureSet {
        GestureSet { scroll: Axes { horizontal: false, vertical: true }, ..GestureSet::default() }
    }

    /// A trackpad that sends only moves (GPUI on Wayland): the gesture ends by itself once the
    /// deltas stop, and the next scroll over another scroller goes to that one.
    #[test]
    fn a_scroll_with_no_began_or_ended_ends_when_the_deltas_stop() {
        let mut router = GestureRouter::new(GestureConfig::default());
        let (sidebar, thread) = (NodeId(1), NodeId(2));
        let set = vertical();
        router.set_scroll_extent(sidebar, &set, size(300., 600.), size(300., 3000.));
        router.set_scroll_extent(thread, &set, size(400., 600.), size(400., 3000.));
        let on_sidebar = [HitEntry { node: sidebar, gestures: &set, bounds: Rect::new(0., 0., 300., 600.) }];
        let on_thread = [HitEntry { node: thread, gestures: &set, bounds: Rect::new(300., 0., 400., 600.) }];
        let moved = |router: &mut GestureRouter, hit: &[HitEntry<'_>], t: f64| {
            router.scroll(
                &ScrollInput {
                    position: p(10., 10.),
                    delta: p(0., 10.),
                    phase: ScrollPhase::Changed,
                    momentum: MomentumPhase::None,
                    time: t,
                },
                hit,
            );
        };
        let mut t = 0.;
        for _ in 0..10 {
            t += 1. / 120.;
            moved(&mut router, &on_sidebar, t);
        }
        assert!(router.scroll_offset(sidebar).y > 0., "the sidebar follows the fingers");
        assert!(router.needs_frame(), "frames keep coming while the gesture may still be live");
        let (_, end) = run(&mut router, sidebar, t, 120.);
        assert!(!router.needs_frame(), "the gesture ended and its fling settled by {end}");
        let settled = router.scroll_offset(sidebar).y;
        t = end + 0.5;
        for _ in 0..10 {
            t += 1. / 120.;
            moved(&mut router, &on_thread, t);
        }
        assert!(router.scroll_offset(thread).y > 0., "the next scroll goes to the scroller under it");
        assert_eq!(router.scroll_offset(sidebar).y, settled, "and the sidebar stays put");
    }

    // Momentum and rubber band.

    #[test]
    fn a_fling_travels_its_projected_distance_and_stops() {
        let mut s = Scroller::new(Axes::BOTH);
        s.set_extent(size(400., 600.), size(400., 100_000.));
        s.begin(0.);
        let mut t = 0.;
        for _ in 0..12 {
            t += 1. / 120.;
            s.track(p(0., 10.), None, t);
        }
        let released = s.offset().y;
        s.end(t);
        assert!(s.is_animating(), "a 1200 pt/s release flings");
        while s.is_animating() {
            t += 1. / 60.;
            s.step(t);
        }
        let travelled = s.offset().y - released;
        let expected = project(1200.);
        assert!((travelled - expected).abs() < expected * 0.02, "travelled {travelled}, projected {expected}");
    }

    #[test]
    fn the_fling_path_is_the_same_at_60_and_240_hz() {
        let fling = |hz: f64| {
            let mut s = Scroller::new(Axes::BOTH);
            s.set_extent(size(400., 600.), size(400., 100_000.));
            s.begin(0.);
            for i in 1..=12 {
                s.track(p(0., 10.), None, f64::from(i) / 120.);
            }
            s.end(0.1);
            let mut t = 0.1;
            while t < 0.6 {
                t += 1. / hz;
                s.step(t.min(0.6));
            }
            s.offset().y
        };
        assert!((fling(60.) - fling(240.)).abs() < 0.01);
    }

    #[test]
    fn pulling_past_the_edge_rubber_bands_and_springs_back_without_overshoot() {
        let list = NodeId(1);
        let set = vertical();
        let mut router = GestureRouter::default();
        router.set_scroll_extent(list, &set, size(400., 600.), size(400., 2000.));
        let hit = [HitEntry { node: list, gestures: &set, bounds: BOUNDS }];
        // Pull down 120 pt from the top: offset goes negative, resisted.
        let t = fingers(&mut router, &hit, p(0., -10.), 12, 0.);
        let shown = router.scroll_offset(list).y;
        assert!(shown < 0., "content moved past the top");
        assert!((shown - rubber_band(-120., 600.)).abs() < 1., "shown {shown}");
        assert!(shown > -120., "resisted");
        // Hold still, then lift: no fling, a spring back to 0.
        lift(&mut router, t + 0.1);
        let (seen, end) = run(&mut router, list, t + 0.1, 120.);
        assert_eq!(router.scroll_offset(list).y, 0.);
        assert!(seen.iter().all(|y| *y <= 0.), "critically damped: never crosses the edge");
        assert!(end - t < 1.2, "settles within a second, took {}", end - t);
    }

    #[test]
    fn a_fling_into_the_end_bounces_past_it_and_back() {
        let list = NodeId(1);
        let set = vertical();
        let mut router = GestureRouter::default();
        router.set_scroll_extent(list, &set, size(400., 600.), size(400., 1000.));
        let hit = [HitEntry { node: list, gestures: &set, bounds: BOUNDS }];
        let t = fingers(&mut router, &hit, p(0., 25.), 8, 0.);
        lift(&mut router, t);
        let (seen, _) = run(&mut router, list, t, 120.);
        let peak = seen.iter().copied().fold(f32::MIN, f32::max);
        assert!(peak > 400., "carried past the end ({peak})");
        assert!(peak < 400. + 120., "but not far ({peak})");
        assert_eq!(router.scroll_offset(list).y, 400.);
    }

    #[test]
    fn os_momentum_replaces_the_framework_fling() {
        let list = NodeId(1);
        let set = vertical();
        let mut router = GestureRouter::default();
        router.set_scroll_extent(list, &set, size(400., 600.), size(400., 10_000.));
        let hit = [HitEntry { node: list, gestures: &set, bounds: BOUNDS }];
        let t = fingers(&mut router, &hit, p(0., 10.), 12, 0.);
        lift(&mut router, t);
        assert!(router.needs_frame(), "framework fling started");
        let at_lift = router.scroll_offset(list).y;
        router.scroll(
            &ScrollInput {
                position: p(10., 10.),
                delta: p(0., 7.),
                phase: ScrollPhase::Changed,
                momentum: MomentumPhase::Began,
                time: t + 0.008,
            },
            &hit,
        );
        assert!(!router.needs_frame(), "the OS drives it now");
        assert_eq!(router.scroll_offset(list).y, at_lift + 7.);
        router.step(t + 0.5);
        assert_eq!(router.scroll_offset(list).y, at_lift + 7., "no framework motion on top");
    }

    #[test]
    fn touching_during_a_fling_catches_it() {
        let list = NodeId(1);
        let set = vertical();
        let mut router = GestureRouter::default();
        router.set_scroll_extent(list, &set, size(400., 600.), size(400., 100_000.));
        let hit = [HitEntry { node: list, gestures: &set, bounds: BOUNDS }];
        let t = fingers(&mut router, &hit, p(0., 20.), 12, 0.);
        lift(&mut router, t);
        router.step(t + 0.1);
        let caught_at = router.scroll_offset(list).y;
        router.scroll(
            &ScrollInput {
                position: p(10., 10.),
                delta: p(0., 0.),
                phase: ScrollPhase::Began,
                momentum: MomentumPhase::None,
                time: t + 0.1,
            },
            &hit,
        );
        router.step(t + 0.5);
        assert_eq!(router.scroll_offset(list).y, caught_at);
    }

    #[test]
    fn gpui_style_phases_mark_os_momentum() {
        let mut m = MomentumInference::default();
        assert_eq!(m.classify(Some(ScrollPhase::Began)), (ScrollPhase::Began, MomentumPhase::None));
        assert_eq!(m.classify(Some(ScrollPhase::Changed)), (ScrollPhase::Changed, MomentumPhase::None));
        assert_eq!(m.classify(Some(ScrollPhase::Ended)), (ScrollPhase::Ended, MomentumPhase::None));
        assert_eq!(m.classify(Some(ScrollPhase::Changed)).1, MomentumPhase::Began);
        assert_eq!(m.classify(Some(ScrollPhase::Changed)).1, MomentumPhase::Changed);
        assert_eq!(m.classify(Some(ScrollPhase::Ended)).1, MomentumPhase::Ended);
        assert_eq!(m.classify(None), (ScrollPhase::Wheel, MomentumPhase::None));
    }

    #[test]
    fn velocity_is_the_slope_of_recent_samples_and_zero_after_a_pause() {
        let mut v = VelocityTracker::default();
        for i in 0..10 {
            v.add(f64::from(i) / 100., i as f32 * 5.);
        }
        assert!((v.velocity(0.09) - 500.).abs() < 1.);
        assert_eq!(v.velocity(0.2), 0.);
    }

    // Scroll arbitration.

    #[test]
    fn a_row_swipe_wins_horizontal_and_the_list_wins_vertical() {
        let (list, row) = (NodeId(1), NodeId(2));
        let list_set = vertical();
        let row_set = GestureSet {
            row_swipe: Some(RowSwipeConfig { leading: 0., trailing: 80., full_swipe: Some(RowEdge::Trailing) }),
            ..GestureSet::default()
        };
        let mut router = GestureRouter::default();
        router.set_scroll_extent(list, &list_set, size(400., 600.), size(400., 4000.));
        router.set_row_config(row, &row_set, 400.);
        let hit = [
            HitEntry { node: row, gestures: &row_set, bounds: Rect::new(0., 0., 400., 44.) },
            HitEntry { node: list, gestures: &list_set, bounds: BOUNDS },
        ];

        // Mostly horizontal, fingers moving left (offset delta positive x): the row.
        fingers(&mut router, &hit, p(6., 1.), 6, 0.);
        assert_eq!(router.scroll_offset(list).y, 0., "the list never moved");
        assert!(router.row(row).unwrap().offset() < -30., "row followed the fingers");
        lift(&mut router, 1.);

        // Mostly vertical: the list, and the row stays as it is.
        let row_before = router.row(row).unwrap().offset();
        fingers(&mut router, &hit, p(1., 6.), 6, 2.);
        assert!(router.scroll_offset(list).y > 30.);
        assert_eq!(router.row(row).unwrap().offset(), row_before);
    }

    #[test]
    fn an_inner_scroller_at_its_edge_yields_to_the_page_swipe() {
        let (pager, carousel) = (NodeId(1), NodeId(2));
        let page_set = GestureSet { swipe_page: true, ..GestureSet::default() };
        let carousel_set = GestureSet { scroll: Axes { horizontal: true, vertical: false }, ..GestureSet::default() };
        let mut router = GestureRouter::default();
        router.set_scroll_extent(carousel, &carousel_set, size(400., 200.), size(1200., 200.));
        let hit = [
            HitEntry { node: carousel, gestures: &carousel_set, bounds: Rect::new(0., 0., 400., 200.) },
            HitEntry { node: pager, gestures: &page_set, bounds: BOUNDS },
        ];
        // Fingers right at offset 0 (offset delta negative): the carousel has no room, so back.
        let mut out = Vec::new();
        out.extend(router.scroll(
            &ScrollInput {
                position: p(10., 10.),
                delta: p(0., 0.),
                phase: ScrollPhase::Began,
                momentum: MomentumPhase::None,
                time: 0.,
            },
            &hit,
        ));
        for i in 1..=10 {
            out.extend(router.scroll(
                &ScrollInput {
                    position: p(10., 10.),
                    delta: p(-30., 0.),
                    phase: ScrollPhase::Changed,
                    momentum: MomentumPhase::None,
                    time: f64::from(i) / 120.,
                },
                &hit,
            ));
        }
        out.extend(lift(&mut router, 10. / 120.));
        let swipes: Vec<SwipePageEvent> = out
            .iter()
            .filter_map(|o| match o {
                Output::Event(n, GestureEvent::SwipePage(e)) if *n == pager => Some(*e),
                _ => None,
            })
            .collect();
        assert_eq!(swipes.first().map(|e| e.phase), Some(GesturePhase::Began));
        assert_eq!(swipes.last().map(|e| (e.direction, e.phase)), Some((SwipeDirection::Back, GesturePhase::Ended)));
        assert_eq!(router.scroll_offset(carousel).x, 0.);

        // Fingers left: the carousel has room and takes it.
        fingers(&mut router, &hit, p(30., 0.), 10, 1.);
        assert!(router.scroll_offset(carousel).x > 200.);
    }

    #[test]
    fn a_nested_scroller_with_no_room_rubber_bands_when_nothing_outer_takes_it() {
        let (outer, inner) = (NodeId(1), NodeId(2));
        let set = vertical();
        let mut router = GestureRouter::default();
        router.set_scroll_extent(outer, &set, size(400., 600.), size(400., 3000.));
        router.set_scroll_extent(inner, &set, size(400., 300.), size(400., 900.));
        let hit = [
            HitEntry { node: inner, gestures: &set, bounds: Rect::new(0., 0., 400., 300.) },
            HitEntry { node: outer, gestures: &set, bounds: BOUNDS },
        ];
        // Down, at the inner's top: the outer has room down too? No, both at 0; pulling down
        // (negative) gives neither room, so the innermost rubber bands.
        fingers(&mut router, &hit, p(0., -10.), 6, 0.);
        assert!(router.scroll_offset(inner).y < 0.);
        assert_eq!(router.scroll_offset(outer).y, 0.);
        lift(&mut router, 0.2);
        run(&mut router, inner, 0.2, 120.);

        // Scroll the outer forward a bit, then pull down over the inner: the outer has room.
        router.scroller_mut(outer).unwrap().scroll_to(p(0., 200.));
        fingers(&mut router, &hit, p(0., -10.), 6, 5.);
        assert_eq!(router.scroll_offset(inner).y, 0.);
        assert!(router.scroll_offset(outer).y < 200.);
    }

    #[test]
    fn a_full_row_swipe_ticks_a_haptic_and_settles_fully_open() {
        let row = NodeId(2);
        let set = GestureSet {
            row_swipe: Some(RowSwipeConfig { leading: 0., trailing: 80., full_swipe: Some(RowEdge::Trailing) }),
            ..GestureSet::default()
        };
        let mut router = GestureRouter::default();
        router.set_row_config(row, &set, 400.);
        let hit = [HitEntry { node: row, gestures: &set, bounds: Rect::new(0., 0., 400., 44.) }];
        let mut out = Vec::new();
        out.extend(router.scroll(
            &ScrollInput {
                position: p(10., 10.),
                delta: p(0., 0.),
                phase: ScrollPhase::Began,
                momentum: MomentumPhase::None,
                time: 0.,
            },
            &hit,
        ));
        for i in 1..=30 {
            out.extend(router.scroll(
                &ScrollInput {
                    position: p(10., 10.),
                    delta: p(10., 0.),
                    phase: ScrollPhase::Changed,
                    momentum: MomentumPhase::None,
                    time: f64::from(i) / 120.,
                },
                &hit,
            ));
        }
        assert_eq!(out.iter().filter(|o| **o == Output::Haptic(Haptic::LevelChange)).count(), 1);
        out.extend(lift(&mut router, 30. / 120.));
        let end = out.iter().rev().find_map(|o| match o {
            Output::Event(_, GestureEvent::RowSwipe(e)) => Some(*e),
            _ => None,
        });
        assert_eq!(end.map(|e| (e.phase, e.open, e.full)), Some((GesturePhase::Ended, Some(RowEdge::Trailing), true)));
        let mut t = 30. / 120.;
        while router.needs_frame() {
            t += 1. / 120.;
            router.step(t);
        }
        assert_eq!(router.row(row).unwrap().offset(), -400.);
    }

    #[test]
    fn a_short_row_swipe_springs_closed() {
        let row = NodeId(2);
        let set = GestureSet {
            row_swipe: Some(RowSwipeConfig { leading: 0., trailing: 80., full_swipe: None }),
            ..GestureSet::default()
        };
        let mut router = GestureRouter::default();
        router.set_row_config(row, &set, 400.);
        let hit = [HitEntry { node: row, gestures: &set, bounds: Rect::new(0., 0., 400., 44.) }];
        let t = fingers(&mut router, &hit, p(5., 0.), 4, 0.);
        lift(&mut router, t + 0.1);
        let mut t = t + 0.1;
        while router.needs_frame() {
            t += 1. / 120.;
            router.step(t);
        }
        assert_eq!(router.row(row).unwrap().offset(), 0.);
        assert_eq!(router.row(row).unwrap().open_edge(), None);
    }

    // The pointer arena.

    fn press(router: &mut GestureRouter, hit: &[HitEntry<'_>], at: Point, t: f64) -> Vec<Output> {
        router.pointer(&PointerInput::new(PointerPhase::Down, at, t), hit)
    }

    fn pointer(router: &mut GestureRouter, phase: PointerPhase, at: Point, t: f64) -> Vec<Output> {
        router.pointer(&PointerInput::new(phase, at, t), &[])
    }

    fn events(out: &[Output]) -> Vec<(NodeId, GestureEvent)> {
        out.iter()
            .filter_map(|o| match o {
                Output::Event(n, e) => Some((*n, *e)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_tap_inside_a_draggable_card_is_a_tap_and_a_drag_is_a_drag() {
        let (card, button) = (NodeId(1), NodeId(2));
        let card_set = GestureSet { drag: Some(None), ..GestureSet::default() };
        let button_set = GestureSet { tap: true, ..GestureSet::default() };
        let hit = [
            HitEntry { node: button, gestures: &button_set, bounds: BOUNDS },
            HitEntry { node: card, gestures: &card_set, bounds: BOUNDS },
        ];
        let mut router = GestureRouter::default();

        let mut out = press(&mut router, &hit, p(50., 50.), 0.);
        out.extend(pointer(&mut router, PointerPhase::Move, p(51., 51.), 0.02));
        out.extend(pointer(&mut router, PointerPhase::Up, p(51., 51.), 0.05));
        assert_eq!(
            events(&out),
            [(button, GestureEvent::Tap(TapEvent { position: p(51., 51.), count: 1 }))],
            "a jitter under the slop stays a tap"
        );

        let mut out = press(&mut router, &hit, p(50., 50.), 1.);
        for i in 1..=10 {
            out.extend(pointer(&mut router, PointerPhase::Move, p(50. + i as f32 * 5., 50.), 1. + f64::from(i) / 120.));
        }
        out.extend(pointer(&mut router, PointerPhase::Up, p(100., 50.), 1. + 10. / 120.));
        let got = events(&out);
        assert!(got.iter().all(|(n, e)| *n == card && matches!(e, GestureEvent::Drag(_))), "{got:?}");
        let GestureEvent::Drag(last) = got.last().unwrap().1 else { unreachable!() };
        assert_eq!(last.phase, GesturePhase::Ended);
        assert_eq!(last.translation, p(50., 0.));
        assert!((last.velocity.x - 600.).abs() < 5., "fling velocity {}", last.velocity.x);
    }

    #[test]
    fn nested_taps_go_to_the_innermost() {
        let (outer, inner) = (NodeId(1), NodeId(2));
        let set = GestureSet { tap: true, ..GestureSet::default() };
        let hit = [
            HitEntry { node: inner, gestures: &set, bounds: BOUNDS },
            HitEntry { node: outer, gestures: &set, bounds: BOUNDS },
        ];
        let mut router = GestureRouter::default();
        let mut out = press(&mut router, &hit, p(5., 5.), 0.);
        out.extend(pointer(&mut router, PointerPhase::Up, p(5., 5.), 0.05));
        assert_eq!(events(&out).iter().map(|(n, _)| *n).collect::<Vec<_>>(), [inner]);
    }

    #[test]
    fn a_long_press_beats_the_tap_on_the_clock() {
        let node = NodeId(1);
        let set = GestureSet { tap: true, long_press: true, ..GestureSet::default() };
        let hit = [HitEntry { node, gestures: &set, bounds: BOUNDS }];
        let mut router = GestureRouter::default();
        let mut out = press(&mut router, &hit, p(5., 5.), 0.);
        assert_eq!(router.next_deadline(), Some(0.5));
        out.extend(router.step(0.49));
        assert!(events(&out).is_empty());
        out.extend(router.step(0.51));
        out.extend(pointer(&mut router, PointerPhase::Up, p(5., 5.), 0.8));
        let phases: Vec<_> = events(&out)
            .into_iter()
            .map(|(_, e)| match e {
                GestureEvent::LongPress(e) => e.phase,
                other => panic!("only the long press reports, got {other:?}"),
            })
            .collect();
        assert_eq!(phases, [GesturePhase::Began, GesturePhase::Ended]);
    }

    #[test]
    fn a_double_tap_never_fires_the_single_tap() {
        let node = NodeId(1);
        let set = GestureSet { tap: true, double_tap: true, ..GestureSet::default() };
        let hit = [HitEntry { node, gestures: &set, bounds: BOUNDS }];
        let mut router = GestureRouter::default();
        let mut out = press(&mut router, &hit, p(5., 5.), 0.);
        out.extend(pointer(&mut router, PointerPhase::Up, p(5., 5.), 0.05));
        out.extend(router.step(0.2));
        out.extend(press(&mut router, &hit, p(6., 5.), 0.25));
        out.extend(pointer(&mut router, PointerPhase::Up, p(6., 5.), 0.3));
        out.extend(router.step(2.));
        assert_eq!(events(&out), [(node, GestureEvent::Tap(TapEvent { position: p(6., 5.), count: 2 }))]);

        // A single tap fires once the double-tap wait runs out.
        let mut out = press(&mut router, &hit, p(5., 5.), 10.);
        out.extend(pointer(&mut router, PointerPhase::Up, p(5., 5.), 10.05));
        out.extend(router.step(10.3));
        assert!(events(&out).is_empty());
        out.extend(router.step(10.6));
        assert_eq!(events(&out), [(node, GestureEvent::Tap(TapEvent { position: p(5., 5.), count: 1 }))]);
    }

    #[test]
    fn an_axis_drag_leaves_cross_axis_movement_to_the_outer_drag() {
        let (slider, sheet) = (NodeId(1), NodeId(2));
        let slider_set = GestureSet { drag: Some(Some(Axis::Horizontal)), ..GestureSet::default() };
        let sheet_set = GestureSet { drag: Some(Some(Axis::Vertical)), ..GestureSet::default() };
        let hit = [
            HitEntry { node: slider, gestures: &slider_set, bounds: BOUNDS },
            HitEntry { node: sheet, gestures: &sheet_set, bounds: BOUNDS },
        ];
        let mut router = GestureRouter::default();
        let mut out = press(&mut router, &hit, p(50., 50.), 0.);
        for i in 1..=5 {
            out.extend(pointer(&mut router, PointerPhase::Move, p(50., 50. + i as f32 * 4.), f64::from(i) / 120.));
        }
        assert!(events(&out).iter().all(|(n, _)| *n == sheet));
        assert_eq!(router.pointer_winner(), Some(sheet));
    }

    #[test]
    fn os_gestures_route_to_the_innermost_listener() {
        let (canvas, editor) = (NodeId(1), NodeId(2));
        let none = GestureSet::default();
        let pinch = GestureSet { pinch: true, ..GestureSet::default() };
        let hit = [
            HitEntry { node: canvas, gestures: &none, bounds: BOUNDS },
            HitEntry { node: editor, gestures: &pinch, bounds: BOUNDS },
        ];
        let router = GestureRouter::default();
        let event = GestureEvent::Pinch(PinchEvent { position: p(1., 1.), scale: 1.1, phase: GesturePhase::Changed });
        assert_eq!(router.route(event, &hit), Some(Output::Event(editor, event)));
        let rotate =
            GestureEvent::Rotate(RotateEvent { position: p(1., 1.), degrees: 3., phase: GesturePhase::Changed });
        assert_eq!(router.route(rotate, &hit), None);
    }

    #[test]
    fn springs_carry_velocity_and_settle_on_target() {
        let m = Spring::SETTLE.start(0., 100., 0., 0.);
        assert!(m.settled(1.2));
        let (x, _) = m.sample(0.2);
        assert!(x > 50. && x < 100.);
        let fling = Spring::FLING.start(0., 100., 2000., 0.);
        let peak = (1..200).map(|i| fling.sample(f64::from(i) / 200.).0).fold(f64::MIN, f64::max);
        assert!(peak > 100., "an underdamped spring with velocity overshoots");
    }
}

#[test]
fn a_shift_keeps_a_fling_going() {
    use nd_core::{Point, Size};
    use nd_gesture::{Axes, Scroller};
    let mut s = Scroller::new(Axes { horizontal: false, vertical: true });
    s.set_extent(Size { width: 100., height: 100. }, Size { width: 100., height: 10_000. });
    s.begin(0.);
    for i in 1..=10 {
        s.track(Point { x: 0., y: 20. }, None, f64::from(i) * 0.008);
    }
    s.end(0.08);
    assert!(s.is_animating());
    s.step(0.1);
    let before = s.offset().y;
    s.shift(Point { x: 0., y: 500. });
    assert_eq!(s.offset().y, before + 500.);
    assert!(s.is_animating(), "the fling survives the shift");
    s.step(0.12);
    assert!(s.offset().y > before + 500., "and keeps moving forward from the shifted position");
}
