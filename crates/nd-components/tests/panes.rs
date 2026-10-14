//! Ports of the split-pane model, dock and tiles suites. The identity contract is the point:
//! every no-op asserts `same`, not just equality.

use std::rc::Rc;

use nd_components::panes::*;

type M = PaneTree<&'static str>;

fn ids<T>(m: &PaneTree<T>) -> Vec<String> {
    pane_leaves(m).iter().map(|l| l.id.to_string()).collect()
}

fn datas(m: &M) -> Vec<&'static str> {
    pane_leaves(m).iter().map(|l| *l.data).collect()
}

fn root_split<T>(m: &PaneTree<T>) -> (&PaneId, Axis, f32, &[PaneNode<T>; 2]) {
    match &**m.root().expect("root") {
        Node::Split { id, axis, ratio, children } => (id, *axis, *ratio, children),
        Node::Leaf { .. } => panic!("root is a leaf"),
    }
}

fn split_of<T>(node: &PaneNode<T>) -> (&PaneId, Axis, f32, &[PaneNode<T>; 2]) {
    match &**node {
        Node::Split { id, axis, ratio, children } => (id, *axis, *ratio, children),
        Node::Leaf { .. } => panic!("leaf"),
    }
}

fn two() -> M {
    split_pane(&seed_panes(vec!["a"]), "1", Axis::Horizontal, "b", None)
}

// ---- model ----------------------------------------------------------------------------------

#[test]
fn clamp_ratio_bounds_and_non_finite() {
    assert_eq!(clamp_pane_ratio(f32::NAN), 0.5);
    assert_eq!(clamp_pane_ratio(f32::INFINITY), 0.5);
    assert_eq!(clamp_pane_ratio(f32::NEG_INFINITY), 0.5);
    assert_eq!(clamp_pane_ratio(-1.0), 0.05);
    assert_eq!(clamp_pane_ratio(0.0), 0.05);
    assert_eq!(clamp_pane_ratio(0.3), 0.3);
    assert_eq!(clamp_pane_ratio(2.0), 0.95);
}

#[test]
fn seed_empty_is_the_empty_model() {
    assert_eq!(seed_panes::<&str>(vec![]), empty_panes());
}

#[test]
fn seed_one_leaf() {
    let m = seed_panes(vec!["a"]);
    assert_eq!(m.root(), Some(&PaneNode::leaf("1", "a")));
    assert_eq!(m.focused_id().as_ref(), "1");
    assert_eq!(m.next_id(), 2);
}

#[test]
fn seed_three_leaves_builds_a_binary_tree_in_order() {
    let m = seed_panes(vec!["a", "b", "c"]);
    assert_eq!(datas(&m), ["a", "b", "c"]);
    assert!(!m.root().unwrap().is_leaf());
}

#[test]
fn seed_with_custom_ids() {
    let m = seed_panes_with(vec!["a", "b"], |_, i| format!("pane-{i}"));
    assert_eq!(ids(&m), ["pane-0", "pane-1"]);
}

#[test]
fn numeric_custom_ids_advance_next_id() {
    let m = seed_panes_with(vec!["a", "b"], |d, _| (if *d == "a" { "7" } else { "8" }).to_string());
    assert_eq!(ids(&m), ["7", "8"]);
    assert_eq!(root_split(&m).0.as_ref(), "s9");
    assert_eq!(m.next_id(), 10);
    let split = split_pane(&m, "7", Axis::Horizontal, "c", None);
    assert_eq!(ids(&split), ["7", "10", "8"]);
}

#[test]
fn split_focuses_the_new_leaf_and_mints_ids() {
    let m1 = two();
    let (id, axis, ratio, _) = root_split(&m1);
    assert_eq!(id.as_ref(), "s2");
    assert_eq!(axis, Axis::Horizontal);
    assert_eq!(ratio, 0.5);
    assert_eq!(ids(&m1), ["1", "2"]);
    assert_eq!(m1.focused_id().as_ref(), "2");
    assert_eq!(m1.next_id(), 3);
}

#[test]
fn split_with_explicit_id() {
    let m = split_pane(&seed_panes(vec!["a"]), "1", Axis::Vertical, "b", Some("named"));
    assert_eq!(ids(&m), ["1", "named"]);
    assert_eq!(m.focused_id().as_ref(), "named");
}

#[test]
fn split_unknown_or_split_id_is_a_same_tree_no_op() {
    let m = seed_panes(vec!["a"]);
    assert!(split_pane(&m, "nope", Axis::Vertical, "b", None).same(&m));
    let m = two();
    assert!(split_pane(&m, "s2", Axis::Vertical, "c", None).same(&m));
    let empty: M = empty_panes();
    assert!(split_pane(&empty, "1", Axis::Vertical, "c", None).same(&empty));
}

#[test]
fn split_shares_untouched_subtrees() {
    let m = split_pane(&two(), "2", Axis::Vertical, "c", None);
    let before = two();
    let (_, _, _, old) = root_split(&before);
    let (_, _, _, new) = root_split(&m);
    assert_eq!(old[0], new[0]);
    let m3 = split_pane(&m, "2", Axis::Vertical, "d", None);
    let (_, _, _, a) = root_split(&m);
    let (_, _, _, b) = root_split(&m3);
    assert!(a[0].same(&b[0]));
}

#[test]
fn close_collapses_to_the_sibling() {
    let m2 = close_pane(&two(), "2");
    assert_eq!(m2.root(), Some(&PaneNode::leaf("1", "a")));
    assert_eq!(m2.focused_id().as_ref(), "1");
}

#[test]
fn closing_an_unfocused_leaf_keeps_the_focus() {
    let m2 = close_pane(&two(), "1");
    assert_eq!(m2.focused_id().as_ref(), "2");
}

#[test]
fn closing_the_root_leaf_empties_the_tree() {
    let m = close_pane(&seed_panes(vec!["a"]), "1");
    assert!(m.root().is_none());
    assert_eq!(m.focused_id().as_ref(), "");
}

#[test]
fn close_unknown_is_a_same_tree_no_op() {
    let m = seed_panes(vec!["a"]);
    assert!(close_pane(&m, "9").same(&m));
    let empty: M = empty_panes();
    assert!(close_pane(&empty, "1").same(&empty));
}

#[test]
fn nested_close_replaces_the_parent_with_the_sibling_subtree() {
    let m = split_pane(&two(), "2", Axis::Vertical, "c", None);
    let closed = close_pane(&m, "3");
    assert_eq!(root_split(&closed).0.as_ref(), "s2");
    assert_eq!(ids(&closed), ["1", "2"]);
    assert_eq!(closed.focused_id().as_ref(), "2");
}

#[test]
fn focus_pane_moves_focus_and_repeats_are_same_tree() {
    let m2 = focus_pane(&two(), "1");
    assert_eq!(m2.focused_id().as_ref(), "1");
    assert!(focus_pane(&m2, "1").same(&m2));
}

#[test]
fn focus_pane_on_unknown_or_split_id_is_a_no_op() {
    let m = two();
    assert!(focus_pane(&m, "s2").same(&m));
    assert!(focus_pane(&m, "zzz").same(&m));
}

#[test]
fn focus_at_and_id_at_index_leaves_in_layout_order() {
    let m = split_pane(&two(), "2", Axis::Vertical, "c", None);
    assert_eq!(pane_id_at(&m, 0).unwrap().as_ref(), "1");
    assert_eq!(pane_id_at(&m, 2).unwrap().as_ref(), "3");
    assert!(pane_id_at(&m, 3).is_none());
    assert_eq!(focus_pane_at(&m, 0).focused_id().as_ref(), "1");
    assert!(focus_pane_at(&m, 9).same(&m));
}

#[test]
fn focus_neighbor_walks_across_the_matching_axis_split() {
    // s2 horizontal [1, s3 vertical [2, 3]]
    let m = split_pane(&two(), "2", Axis::Vertical, "c", None);
    let m = focus_pane(&m, "1");
    let right = focus_neighbor(&m, Direction::Right);
    assert_eq!(right.focused_id().as_ref(), "2");
    let down = focus_neighbor(&right, Direction::Down);
    assert_eq!(down.focused_id().as_ref(), "3");
    let up = focus_neighbor(&down, Direction::Up);
    assert_eq!(up.focused_id().as_ref(), "2");
    let left = focus_neighbor(&up, Direction::Left);
    assert_eq!(left.focused_id().as_ref(), "1");
}

#[test]
fn focus_neighbor_with_no_neighbor_is_a_same_tree_no_op() {
    let m = two();
    assert!(focus_neighbor(&m, Direction::Right).same(&m));
    assert!(focus_neighbor(&m, Direction::Up).same(&m));
    let empty: M = empty_panes();
    assert!(focus_neighbor(&empty, Direction::Left).same(&empty));
}

#[test]
fn set_ratio_clamps() {
    let m2 = set_pane_ratio(&two(), "s2", 0.3);
    assert_eq!(root_split(&m2).2, 0.3);
    let m3 = set_pane_ratio(&m2, "s2", 99.0);
    assert_eq!(root_split(&m3).2, 0.95);
}

#[test]
fn set_ratio_echo_is_a_same_tree_no_op() {
    let m2 = set_pane_ratio(&two(), "s2", 0.3);
    assert!(set_pane_ratio(&m2, "s2", 0.3).same(&m2));
    assert!(!set_pane_ratio(&m2, "s2", f32::NAN).same(&m2));
    assert_eq!(root_split(&set_pane_ratio(&m2, "s2", f32::NAN)).2, 0.5);
    let pinned = set_pane_ratio(&m2, "s2", 5.0);
    assert!(set_pane_ratio(&pinned, "s2", 7.0).same(&pinned));
}

#[test]
fn set_ratio_on_leaf_or_unknown_id_is_a_no_op() {
    let m = two();
    assert!(set_pane_ratio(&m, "1", 0.3).same(&m));
    assert!(set_pane_ratio(&m, "zzz", 0.3).same(&m));
}

#[test]
fn set_ratio_shares_the_untouched_children() {
    let m = two();
    let m2 = set_pane_ratio(&m, "s2", 0.3);
    let (_, _, _, a) = root_split(&m);
    let (_, _, _, b) = root_split(&m2);
    assert!(a[0].same(&b[0]) && a[1].same(&b[1]));
}

#[test]
fn update_pane_replaces_data_and_equal_data_is_a_no_op() {
    let m1 = seed_panes(vec![1]);
    let m2 = update_pane(&m1, "1", |d| d + 1);
    assert_eq!(*pane_leaves(&m2)[0].data, 2);
    assert!(update_pane(&m2, "1", |d| *d).same(&m2));
    assert!(update_pane(&m2, "zzz", |d| *d).same(&m2));
}

#[test]
fn same_pane_shape_ignores_ratio_and_data() {
    let m1 = two();
    let ratio_changed = set_pane_ratio(&m1, "s2", 0.2);
    let data_changed = update_pane(&m1, "1", |_| "z");
    assert!(same_pane_shape(m1.root(), ratio_changed.root()));
    assert!(same_pane_shape(m1.root(), data_changed.root()));
    assert!(!same_pane_shape(m1.root(), split_pane(&m1, "2", Axis::Vertical, "c", None).root()));
    assert!(!same_pane_shape(m1.root(), close_pane(&m1, "2").root()));
    assert!(same_pane_shape::<&str>(None, None));
    assert!(!same_pane_shape(m1.root(), None));
}

#[test]
fn same_pane_shape_sees_an_axis_change() {
    let h = two();
    let v = split_pane(&seed_panes(vec!["a"]), "1", Axis::Vertical, "b", None);
    assert!(!same_pane_shape(h.root(), v.root()));
}

#[test]
fn migrate_round_trips_a_real_model_through_json() {
    let m = split_pane(&two(), "2", Axis::Vertical, "c", None);
    let m = set_pane_ratio(&m, "s2", 0.3);
    let json = serde_json::to_string(&serialize_panes(&m)).unwrap();
    let snapshot: PaneSnapshot<String> = serde_json::from_str(&json).unwrap();
    let revived = migrate_panes(snapshot);
    assert_eq!(datas_owned(&revived), ["a", "b", "c"]);
    assert_eq!(revived.focused_id(), m.focused_id());
    assert_eq!(revived.next_id(), m.next_id());
    assert_eq!(root_split(&revived).2, 0.3);
    let again = migrate_panes(serialize_panes(&m));
    assert_eq!(again, m);
}

fn datas_owned(m: &PaneTree<String>) -> Vec<String> {
    pane_leaves(m).iter().map(|l| l.data.clone()).collect()
}

#[test]
fn migrate_uses_the_old_persisted_shape() {
    let raw = r#"{"root":{"kind":"split","id":"s9","orientation":"horizontal","ratio":47,
        "children":[{"kind":"leaf","id":"4","data":"a"},{"kind":"leaf","id":"7","data":"b"}]},
        "focusedId":"gone"}"#;
    let m = migrate_panes(serde_json::from_str::<PaneSnapshot<String>>(raw).unwrap());
    assert_eq!(root_split(&m).2, 0.95);
    assert_eq!(m.focused_id().as_ref(), "4");
    assert_eq!(m.next_id(), 10);
}

#[test]
fn migrate_missing_ratio_defaults_to_half() {
    let raw = r#"{"root":{"kind":"split","id":"s1","orientation":"vertical",
        "children":[{"kind":"leaf","id":"1","data":"a"},{"kind":"leaf","id":"2","data":"b"}]}}"#;
    let m = migrate_panes(serde_json::from_str::<PaneSnapshot<String>>(raw).unwrap());
    let (_, axis, ratio, _) = root_split(&m);
    assert_eq!((axis, ratio), (Axis::Vertical, 0.5));
}

#[test]
fn migrate_rejects_garbage_and_malformed_trees() {
    assert!(serde_json::from_str::<PaneSnapshot<String>>(r#"{"root":{"kind":"what"}}"#).is_err());
    assert!(
        serde_json::from_str::<PaneSnapshot<String>>(
            r#"{"root":{"kind":"split","id":"s1","orientation":"diagonal","children":[]}}"#
        )
        .is_err()
    );
    let empty_id =
        PaneSnapshot { root: Some(SnapshotNode::Leaf { id: "".into(), data: "a" }), focused_id: "".into(), next_id: 3 };
    assert_eq!(migrate_panes(empty_id), empty_panes());
}

#[test]
fn migrate_leaf_data_failing_the_guard_rejects_the_whole_tree() {
    let bad = PaneSnapshot {
        root: Some(SnapshotNode::Leaf { id: "1".into(), data: 42 }),
        focused_id: "1".into(),
        next_id: 2,
    };
    let m = migrate_panes_with(bad, |d| (d != 42).then_some(d));
    assert_eq!(m, empty_panes());
}

#[test]
fn migrate_preserves_an_explicitly_empty_root() {
    let raw: PaneSnapshot<String> = serde_json::from_str(r#"{"root":null,"focusedId":"","nextId":5}"#).unwrap();
    let m = migrate_panes(raw);
    assert!(m.root().is_none());
    assert_eq!(m.next_id(), 5);
}

// ---- dock -----------------------------------------------------------------------------------

type D = DockModel<&'static str>;
type Panel = DockPanel<&'static str>;

fn tab(id: &'static str) -> DockTab<&'static str> {
    DockTab::new(id, id.to_uppercase(), id)
}

/// s3 horizontal [ panel 1 (a, b), panel 2 (c) ], focus on panel 1.
fn base() -> D {
    seed_dock(vec![vec![tab("a"), tab("b")], vec![tab("c")]])
}

fn tab_ids(m: &D, panel: &str) -> Vec<String> {
    find_dock_panel(m, panel).unwrap().tabs.iter().map(|t| t.id.to_string()).collect()
}

fn panel_ids(m: &D) -> Vec<String> {
    ids(m)
}

fn child_ids(node: &PaneNode<Panel>) -> Vec<String> {
    split_of(node).3.iter().map(|c| c.id().to_string()).collect()
}

fn active(m: &D, panel: &str) -> String {
    find_dock_panel(m, panel).unwrap().active_tab_id.to_string()
}

#[test]
fn seed_dock_one_panel_per_group() {
    let m = base();
    assert_eq!(panel_ids(&m), ["1", "2"]);
    assert_eq!(tab_ids(&m, "1"), ["a", "b"]);
    assert_eq!(active(&m, "1"), "a");
    assert_eq!(m.focused_id().as_ref(), "1");
    assert_eq!(root_split(&m).1, Axis::Horizontal);
}

#[test]
fn seed_dock_drops_empty_groups() {
    assert_eq!(panel_ids(&seed_dock(vec![vec![tab("a")], vec![]])), ["1"]);
    assert_eq!(seed_dock::<&str>(vec![vec![], vec![]]), empty_dock());
}

#[test]
fn dock_edges_pick_axis_and_side() {
    let cases = [
        (DockZone::Left, Axis::Horizontal, ["1", "2"]),
        (DockZone::Right, Axis::Horizontal, ["2", "1"]),
        (DockZone::Top, Axis::Vertical, ["1", "2"]),
        (DockZone::Bottom, Axis::Vertical, ["2", "1"]),
    ];
    for (zone, axis, order) in cases {
        let m = dock_panel(&base(), "1", "2", zone);
        assert_eq!(root_split(&m).1, axis, "{zone:?}");
        assert_eq!(child_ids(m.root().unwrap()), order, "{zone:?}");
        assert_eq!(m.focused_id().as_ref(), "1");
    }
}

#[test]
fn docking_out_of_a_nested_split_collapses_the_split_it_left() {
    // s3 [1, s4 [2, 4]] -> dock 4 to the left of 1 -> s3 [s5 [4, 1], 2]
    let m = seed_dock(vec![vec![tab("a")], vec![tab("b")]]);
    let m = undock_tab(&add_tab(&m, "2", tab("c"), None), "c", DockEdgeZone::Right);
    assert_eq!(panel_ids(&m), ["1", "2", "4"]);
    let moved = dock_panel(&m, "4", "1", DockZone::Left);
    let (id, _, _, children) = root_split(&moved);
    assert_eq!(id.as_ref(), "s3");
    assert_eq!(child_ids(&children[0]), ["4", "1"]);
    assert_eq!(children[1].id().as_ref(), "2");
    assert_eq!(panel_ids(&moved), ["4", "1", "2"]);
}

#[test]
fn tabs_and_the_active_tab_ride_along() {
    let m = dock_panel(&activate_tab(&base(), "b"), "1", "2", DockZone::Top);
    assert_eq!(tab_ids(&m, "1"), ["a", "b"]);
    assert_eq!(active(&m, "1"), "b");
}

#[test]
fn dock_center_merges_and_collapses() {
    let m = dock_panel(&base(), "2", "1", DockZone::Center);
    assert_eq!(&**m.root().unwrap(), &Node::Leaf { id: "1".into(), data: find_dock_panel(&m, "1").unwrap().clone() });
    assert_eq!(tab_ids(&m, "1"), ["a", "b", "c"]);
    assert_eq!(active(&m, "1"), "c");
    assert_eq!(m.focused_id().as_ref(), "1");
}

#[test]
fn dock_center_appends_and_the_target_keeps_its_order() {
    let m = dock_panel(&base(), "1", "2", DockZone::Center);
    assert_eq!(tab_ids(&m, "2"), ["c", "a", "b"]);
    assert_eq!(panel_ids(&m), ["2"]);
}

#[test]
fn dock_panel_no_ops_are_same_tree() {
    let m = base();
    assert!(dock_panel(&m, "1", "1", DockZone::Left).same(&m));
    assert!(dock_panel(&m, "9", "1", DockZone::Center).same(&m));
    assert!(dock_panel(&m, "1", "9", DockZone::Bottom).same(&m));
    assert!(dock_panel(&m, "s3", "1", DockZone::Left).same(&m));
}

#[test]
fn docking_a_panel_where_it_already_sits_is_same_tree() {
    let m = base();
    assert!(dock_panel(&m, "2", "1", DockZone::Right).same(&m));
    assert!(dock_panel(&m, "1", "2", DockZone::Left).same(&m));
    assert!(!dock_panel(&m, "2", "1", DockZone::Bottom).same(&m));
}

#[test]
fn add_tab_appends_activates_and_rejects_duplicates() {
    let m = add_tab(&base(), "2", tab("d"), None);
    assert_eq!(tab_ids(&m, "2"), ["c", "d"]);
    assert_eq!(active(&m, "2"), "d");
    assert!(add_tab(&m, "2", tab("a"), None).same(&m));
    assert!(add_tab(&m, "9", tab("z"), None).same(&m));
}

#[test]
fn add_tab_honours_an_index() {
    assert_eq!(tab_ids(&add_tab(&base(), "1", tab("d"), Some(0)), "1"), ["d", "a", "b"]);
    assert_eq!(tab_ids(&add_tab(&base(), "1", tab("d"), Some(99)), "1"), ["a", "b", "d"]);
}

#[test]
fn activate_tab_selects_without_moving_focus() {
    let m = activate_tab(&base(), "c");
    assert_eq!(active(&m, "2"), "c");
    assert_eq!(m.focused_id().as_ref(), "1");
    assert!(activate_tab(&m, "c").same(&m));
    assert!(activate_tab(&m, "zzz").same(&m));
}

#[test]
fn close_tab_hands_the_active_slot_to_the_neighbour() {
    let m = close_tab(&base(), "a");
    assert_eq!(tab_ids(&m, "1"), ["b"]);
    assert_eq!(active(&m, "1"), "b");
    assert!(close_tab(&m, "zzz").same(&m));
}

#[test]
fn close_tab_on_the_last_tab_of_the_stack_activates_the_previous() {
    let m = activate_tab(&base(), "b");
    let m = close_tab(&m, "b");
    assert_eq!(active(&m, "1"), "a");
}

#[test]
fn closing_the_last_tab_in_a_panel_collapses_the_split() {
    let m = close_tab(&base(), "c");
    assert_eq!(m.root().unwrap().id().as_ref(), "1");
    assert!(m.root().unwrap().is_leaf());
    assert_eq!(panel_ids(&m), ["1"]);
}

#[test]
fn closing_the_last_tab_of_the_last_panel_empties_the_dock() {
    let m = close_tab(&base(), "c");
    let m = close_tab(&close_tab(&m, "a"), "b");
    assert!(m.root().is_none());
}

#[test]
fn close_panel_is_close_pane() {
    let m = close_panel(&base(), "1");
    assert_eq!(m.root().unwrap().id().as_ref(), "2");
    assert_eq!(m.focused_id().as_ref(), "2");
}

#[test]
fn move_tab_reorders_inside_one_panel() {
    let m = move_tab(&base(), "a", "1", Some(1));
    assert_eq!(tab_ids(&m, "1"), ["b", "a"]);
    assert_eq!(active(&m, "1"), "a");
    assert_eq!(m.focused_id().as_ref(), "1");
}

#[test]
fn move_tab_to_its_current_index_is_same_tree() {
    let m = base();
    assert!(move_tab(&m, "a", "1", Some(0)).same(&m));
    assert!(move_tab(&m, "b", "1", None).same(&m));
    assert!(move_tab(&m, "zzz", "1", Some(0)).same(&m));
    assert!(move_tab(&m, "a", "9", Some(0)).same(&m));
}

#[test]
fn move_tab_across_panels_activates_and_takes_focus() {
    let m = move_tab(&base(), "a", "2", Some(0));
    assert_eq!(tab_ids(&m, "1"), ["b"]);
    assert_eq!(tab_ids(&m, "2"), ["a", "c"]);
    assert_eq!(active(&m, "2"), "a");
    assert_eq!(m.focused_id().as_ref(), "2");
}

#[test]
fn emptying_the_source_panel_collapses_it() {
    let m = move_tab(&base(), "c", "1", None);
    assert_eq!(panel_ids(&m), ["1"]);
    assert_eq!(tab_ids(&m, "1"), ["a", "b", "c"]);
    assert_eq!(m.focused_id().as_ref(), "1");
}

#[test]
fn undock_pulls_the_tab_into_a_new_panel_and_focuses_it() {
    let m = undock_tab(&base(), "b", DockEdgeZone::Right);
    let (_, _, _, children) = root_split(&m);
    assert_eq!(panel_ids(&m), ["1", "4", "2"]);
    assert_eq!(tab_ids(&m, "1"), ["a"]);
    assert_eq!(tab_ids(&m, "4"), ["b"]);
    assert_eq!(m.focused_id().as_ref(), "4");
    assert_eq!(split_of(&children[0]).1, Axis::Horizontal);
}

#[test]
fn undock_zone_picks_the_side() {
    let left = undock_tab(&base(), "b", DockEdgeZone::Left);
    assert_eq!(child_ids(&root_split(&left).3[0]), ["4", "1"]);
    let down = undock_tab(&base(), "b", DockEdgeZone::Bottom);
    assert_eq!(split_of(&root_split(&down).3[0]).1, Axis::Vertical);
}

#[test]
fn undock_with_nothing_to_undock_is_same_tree() {
    let m = base();
    assert!(undock_tab(&m, "c", DockEdgeZone::Right).same(&m));
    assert!(undock_tab(&m, "zzz", DockEdgeZone::Right).same(&m));
}

#[test]
fn lookups() {
    let m = base();
    let found = find_dock_tab(&m, "b").unwrap();
    assert_eq!((found.panel_id.as_ref(), found.index), ("1", 1));
    assert!(find_dock_tab(&m, "zzz").is_none());
    let panel = find_dock_panel(&m, "1").unwrap();
    assert_eq!(active_dock_tab(panel).unwrap().id.as_ref(), "a");
    assert_eq!(active_dock_tab_index(panel), 0);
    assert_eq!(active_dock_tab_index(&dock_panel_of(vec![tab("a")], Some("gone"))), 0);
    let stale = DockPanel { tabs: vec![tab("a")], active_tab_id: "gone".into() };
    assert_eq!(active_dock_tab_index(&stale), 0);
    assert!(find_dock_panel(&m, "s3").is_none());
    assert_eq!(dock_panels(&m).len(), 2);
}

const UNIT: DockRect = DockRect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 };

#[test]
fn hit_test_outer_quarter_is_the_edge_zone() {
    assert_eq!(hit_test_dock_zone(UNIT, 5.0, 50.0, None), DockZone::Left);
    assert_eq!(hit_test_dock_zone(UNIT, 95.0, 50.0, None), DockZone::Right);
    assert_eq!(hit_test_dock_zone(UNIT, 50.0, 5.0, None), DockZone::Top);
    assert_eq!(hit_test_dock_zone(UNIT, 50.0, 95.0, None), DockZone::Bottom);
    assert_eq!(hit_test_dock_zone(UNIT, 50.0, 50.0, None), DockZone::Center);
    assert_eq!(hit_test_dock_zone(UNIT, 30.0, 50.0, None), DockZone::Center);
}

#[test]
fn hit_test_honours_origin_and_past_edge_points() {
    let rect = DockRect { x: 200.0, y: 100.0, width: 100.0, height: 100.0 };
    assert_eq!(hit_test_dock_zone(rect, 205.0, 150.0, None), DockZone::Left);
    assert_eq!(hit_test_dock_zone(UNIT, -40.0, 50.0, None), DockZone::Left);
    assert_eq!(hit_test_dock_zone(UNIT, 50.0, 140.0, None), DockZone::Bottom);
}

#[test]
fn hit_test_band_is_configurable_and_clamped_to_a_half() {
    assert_eq!(hit_test_dock_zone(UNIT, 30.0, 50.0, Some(0.4)), DockZone::Left);
    assert_eq!(hit_test_dock_zone(UNIT, 49.0, 50.0, Some(9.0)), DockZone::Left);
}

#[test]
fn hit_test_degenerate_input_is_center() {
    let flat = DockRect { width: 0.0, ..UNIT };
    assert_eq!(hit_test_dock_zone(flat, 0.0, 50.0, None), DockZone::Center);
    let nan = DockRect { height: f32::NAN, ..UNIT };
    assert_eq!(hit_test_dock_zone(nan, 50.0, 50.0, None), DockZone::Center);
    assert_eq!(hit_test_dock_zone(UNIT, f32::NAN, 50.0, None), DockZone::Center);
}

#[test]
fn dock_serialize_round_trips_through_json() {
    let m = activate_tab(&dock_panel(&add_tab(&base(), "2", tab("d"), None), "1", "2", DockZone::Bottom), "d");
    let snapshot = serialize_dock(&m);
    let json = serde_json::to_string(&snapshot).unwrap();
    let parsed: DockSnapshot<String> = serde_json::from_str(&json).unwrap();
    let back = deserialize_dock(parsed, |_| true);
    assert_eq!(panel_ids_owned(&back), panel_ids(&m));
    assert_eq!(back.focused_id(), m.focused_id());
    assert_eq!(back.next_id(), m.next_id());
    assert_eq!(deserialize_dock(snapshot, |_| true), m);
}

fn panel_ids_owned(m: &DockModel<String>) -> Vec<String> {
    ids(m)
}

fn snapshot_of(json: &str) -> DockSnapshot<String> {
    serde_json::from_str(json).unwrap()
}

#[test]
fn dock_deserialize_rejects_malformed_layouts() {
    let empty = empty_dock::<String>();
    let accept = |_: &String| true;
    let no_tabs = r#"{"root":{"kind":"leaf","id":"1","data":{"tabs":[],"activeTabId":""}}}"#;
    assert_eq!(deserialize_dock(snapshot_of(no_tabs), accept), empty);
    let empty_tab_id =
        r#"{"root":{"kind":"leaf","id":"1","data":{"tabs":[{"id":"","title":"A","data":"a"}],"activeTabId":""}}}"#;
    assert_eq!(deserialize_dock(snapshot_of(empty_tab_id), accept), empty);
    let ok =
        r#"{"root":{"kind":"leaf","id":"1","data":{"tabs":[{"id":"a","title":"A","data":"a"}],"activeTabId":"a"}}}"#;
    assert_eq!(deserialize_dock(snapshot_of(ok), |d: &String| d != "a"), empty);
    assert!(
        serde_json::from_str::<DockSnapshot<String>>(
            r#"{"root":{"kind":"leaf","id":"1","data":{"tabs":[{"title":"A","data":"a"}]}}}"#
        )
        .is_err()
    );
}

#[test]
fn dock_tab_id_used_in_two_panels_rejects_the_layout() {
    let mut clash = serialize_dock(&base());
    let Some(SnapshotNode::Split { children, .. }) = &mut clash.root else { panic!("split") };
    let SnapshotNode::Leaf { data, .. } = &mut children[1] else { panic!("leaf") };
    data.tabs[0].id = "a".into();
    assert_eq!(deserialize_dock(clash, |_| true), empty_dock());
}

#[test]
fn dock_active_tab_naming_a_gone_tab_is_repaired() {
    let raw = r#"{"root":{"kind":"leaf","id":"1","data":{"tabs":[{"id":"a","title":"A","data":"a"}],"activeTabId":"gone"}},"focusedId":"1","nextId":2}"#;
    let m = deserialize_dock(snapshot_of(raw), |_| true);
    assert_eq!(active_owned(&m, "1"), "a");
}

fn active_owned(m: &DockModel<String>, panel: &str) -> String {
    find_dock_panel(m, panel).unwrap().active_tab_id.to_string()
}

#[test]
fn dock_unknown_tab_fields_are_dropped_and_the_icon_survives() {
    let raw = r#"{"root":{"kind":"leaf","id":"1","data":{"tabs":[{"id":"a","title":"A","icon":"folder-symbolic","data":"a","stale":true}],"activeTabId":"a"}},"focusedId":"1","nextId":2}"#;
    let m = deserialize_dock(snapshot_of(raw), |_| true);
    let tab = &find_dock_panel(&m, "1").unwrap().tabs[0];
    assert_eq!(
        tab,
        &DockTab { id: "a".into(), title: "A".into(), icon: Some("folder-symbolic".into()), data: "a".to_string() }
    );
}

#[test]
fn dock_drag_payloads_round_trip_and_foreign_text_is_rejected() {
    let drag = |kind, id: &str| Some(DockDrag { kind, id: id.to_string().into() });
    assert_eq!(parse_dock_drag(&dock_drag_payload(DockDragKind::Tab, "a")), drag(DockDragKind::Tab, "a"));
    assert_eq!(parse_dock_drag(&dock_drag_payload(DockDragKind::Panel, "1")), drag(DockDragKind::Panel, "1"));
    assert_eq!(
        parse_dock_drag(&dock_drag_payload(DockDragKind::Tab, "file:///a.txt")),
        drag(DockDragKind::Tab, "file:///a.txt")
    );
    for foreign in ["", "https://example.com", "nd-dock:", "nd-dock:tab:", "nd-dock:widget:x"] {
        assert!(parse_dock_drag(foreign).is_none(), "{foreign:?}");
    }
}

fn rect(m: &D, size: DockSize, id: &str) -> DockRect {
    dock_panel_rects(m, size).into_iter().find(|(k, _)| k.as_ref() == id).unwrap().1
}

const SIZE: DockSize = DockSize { width: 800.0, height: 600.0 };

#[test]
fn panel_rects_come_from_the_split_ratios() {
    let m = base();
    assert_eq!(rect(&m, SIZE, "1"), DockRect { x: 0.0, y: 0.0, width: 400.0, height: 600.0 });
    assert_eq!(rect(&m, SIZE, "2"), DockRect { x: 400.0, y: 0.0, width: 400.0, height: 600.0 });
}

#[test]
fn vertical_split_divides_the_height_and_a_moved_divider_moves_the_rects() {
    let m = set_pane_ratio(&dock_panel(&base(), "2", "1", DockZone::Bottom), "s4", 0.25);
    assert_eq!(rect(&m, SIZE, "1"), DockRect { x: 0.0, y: 0.0, width: 800.0, height: 150.0 });
    assert_eq!(rect(&m, SIZE, "2"), DockRect { x: 0.0, y: 150.0, width: 800.0, height: 450.0 });
}

#[test]
fn zone_at_outer_quarters_and_middle() {
    let m = base();
    let at = |panel, x, y| dock_zone_at(&m, panel, x, y, Some(SIZE), None);
    assert_eq!(at("1", 10.0, 300.0), DockZone::Left);
    assert_eq!(at("1", 390.0, 300.0), DockZone::Right);
    assert_eq!(at("1", 200.0, 10.0), DockZone::Top);
    assert_eq!(at("1", 200.0, 590.0), DockZone::Bottom);
    assert_eq!(at("1", 200.0, 300.0), DockZone::Center);
}

#[test]
fn zone_at_uses_the_target_panels_own_coordinates() {
    let m = base();
    let at = |x, y| dock_zone_at(&m, "2", x, y, Some(SIZE), None);
    assert_eq!(at(10.0, 300.0), DockZone::Left);
    assert_eq!(at(200.0, 300.0), DockZone::Center);
    assert_eq!(at(410.0, 300.0), DockZone::Right);
    assert_eq!(at(390.0, 10.0), DockZone::Top);
}

#[test]
fn zone_at_without_size_or_panel_is_center() {
    assert_eq!(dock_zone_at(&base(), "1", 10.0, 300.0, None, None), DockZone::Center);
    assert_eq!(dock_zone_at(&base(), "nope", 10.0, 300.0, Some(SIZE), None), DockZone::Center);
}

fn tab_payload(id: &str) -> String {
    dock_drag_payload(DockDragKind::Tab, id)
}

fn panel_payload(id: &str) -> String {
    dock_drag_payload(DockDragKind::Panel, id)
}

#[test]
fn drop_tab_on_another_panels_center_moves_into_its_stack() {
    let m = apply_dock_drop(&base(), &tab_payload("a"), "2", DockZone::Center);
    assert_eq!(tab_ids(&m, "1"), ["b"]);
    assert_eq!(tab_ids(&m, "2"), ["c", "a"]);
    assert_eq!(active(&m, "2"), "a");
}

#[test]
fn drop_tab_on_another_panels_edge_lands_in_a_new_panel() {
    let m = apply_dock_drop(&base(), &tab_payload("a"), "2", DockZone::Top);
    assert_eq!(tab_ids(&m, "1"), ["b"]);
    assert_eq!(tab_ids(&m, "2"), ["c"]);
    let fresh = panel_ids(&m).into_iter().find(|id| id != "1" && id != "2").unwrap();
    assert_eq!(tab_ids(&m, &fresh), ["a"]);
    assert_eq!(split_of(&root_split(&m).3[1]).1, Axis::Vertical);
}

#[test]
fn drop_tab_on_an_edge_of_its_own_panel_undocks_it_there() {
    let m = apply_dock_drop(&base(), &tab_payload("b"), "1", DockZone::Right);
    assert_eq!(tab_ids(&m, "1"), ["a"]);
    let fresh = panel_ids(&m).into_iter().find(|id| id != "1" && id != "2").unwrap();
    assert_eq!(tab_ids(&m, &fresh), ["b"]);
}

#[test]
fn drop_last_tab_of_a_panel_takes_the_panel_path() {
    let m = apply_dock_drop(&base(), &tab_payload("c"), "1", DockZone::Left);
    assert_eq!(panel_ids(&m), ["2", "1"]);
    assert_eq!(tab_ids(&m, "2"), ["c"]);
}

#[test]
fn drop_panel_payload_docks_the_whole_panel() {
    let m = apply_dock_drop(&base(), &panel_payload("2"), "1", DockZone::Center);
    assert_eq!(panel_ids(&m), ["1"]);
    assert_eq!(tab_ids(&m, "1"), ["a", "b", "c"]);
}

#[test]
fn drop_back_on_own_panel_center_is_same_tree() {
    let m = base();
    assert!(apply_dock_drop(&m, &tab_payload("a"), "1", DockZone::Center).same(&m));
    assert!(apply_dock_drop(&m, &tab_payload("b"), "1", DockZone::Center).same(&m));
}

#[test]
fn drop_where_it_already_sits_is_same_tree() {
    let m = base();
    assert!(apply_dock_drop(&m, &panel_payload("2"), "2", DockZone::Center).same(&m));
    assert!(apply_dock_drop(&m, &panel_payload("2"), "1", DockZone::Right).same(&m));
    assert!(apply_dock_drop(&m, &tab_payload("c"), "1", DockZone::Right).same(&m));
}

#[test]
fn drop_lone_tab_on_an_edge_of_its_own_panel_is_same_tree() {
    let m = base();
    assert!(apply_dock_drop(&m, &tab_payload("c"), "2", DockZone::Top).same(&m));
}

#[test]
fn drop_foreign_payload_and_unknown_ids_are_ignored() {
    let m = base();
    assert!(apply_dock_drop(&m, "https://example.com", "1", DockZone::Left).same(&m));
    assert!(apply_dock_drop(&m, &tab_payload("gone"), "1", DockZone::Left).same(&m));
    assert!(apply_dock_drop(&m, &panel_payload("9"), "1", DockZone::Left).same(&m));
    assert!(apply_dock_drop(&m, &tab_payload("a"), "9", DockZone::Center).same(&m));
}

// ---- tiles ----------------------------------------------------------------------------------

type T = TileModel<&'static str>;

/// Two 2x2 tiles side by side on a 4-column grid, plus one below them.
fn tiles_base() -> T {
    seed_tiles(
        vec![
            TilePlacement::new(0, 0, 2, 2, "a"),
            TilePlacement::new(2, 0, 2, 2, "b"),
            TilePlacement::new(0, 2, 4, 1, "c"),
        ],
        4,
    )
}

fn boxes<D>(m: &TileModel<D>) -> Vec<String> {
    m.tiles().iter().map(|t| format!("{}@{},{} {}x{}", t.id, t.x, t.y, t.w, t.h)).collect()
}

fn tile_ids(m: &T) -> Vec<String> {
    m.tiles().iter().map(|t| t.id.to_string()).collect()
}

#[test]
fn place_mints_sequential_ids_and_keeps_what_fits() {
    assert_eq!(boxes(&tiles_base()), ["1@0,0 2x2", "2@2,0 2x2", "3@0,2 4x1"]);
    assert_eq!(tiles_base().next_id(), 4);
}

#[test]
fn place_keeps_its_cells_and_pushes_the_occupant_down() {
    let m = place_tile(&tiles_base(), TilePlacement::new(0, 0, 4, 1, "d"));
    assert_eq!(boxes(&m), ["1@0,1 2x2", "2@2,1 2x2", "3@0,3 4x1", "4@0,0 4x1"]);
}

#[test]
fn place_clamps_to_the_grid() {
    let m = place_tile(&empty_tiles::<&str>(4), TilePlacement::new(9, -3, 99, 0, "a"));
    assert_eq!(boxes(&m), ["1@0,0 4x1"]);
    let m = place_tile(&empty_tiles::<&str>(4), TilePlacement::new(3, 0, 2, 1, "a"));
    assert_eq!(boxes(&m), ["1@2,0 2x1"]);
}

#[test]
fn place_with_an_explicit_id_advances_next_id_and_rejects_duplicates() {
    let m = place_tile(&empty_tiles::<&str>(4), TilePlacement::new(0, 0, 1, 1, "a").with_id("7"));
    assert_eq!(m.next_id(), 8);
    assert!(place_tile(&m, TilePlacement::new(2, 0, 1, 1, "b").with_id("7")).same(&m));
}

#[test]
fn move_pushes_the_tiles_it_lands_on_down_cascading() {
    let m = move_tile(&tiles_base(), "1", 2, 0);
    assert_eq!(boxes(&m), ["1@2,0 2x2", "2@2,2 2x2", "3@0,4 4x1"]);
}

#[test]
fn tiles_that_did_not_move_keep_their_identity() {
    let m = tiles_base();
    let moved = move_tile(&m, "3", 0, 3);
    assert!(Rc::ptr_eq(&moved.tiles()[0], &m.tiles()[0]));
    assert!(Rc::ptr_eq(&moved.tiles()[1], &m.tiles()[1]));
    assert!(!Rc::ptr_eq(&moved.tiles()[2], &m.tiles()[2]));
}

#[test]
fn no_upward_compaction() {
    let m = remove_tile(&tiles_base(), "1");
    assert_eq!(boxes(&m), ["2@2,0 2x2", "3@0,2 4x1"]);
    assert_eq!(boxes(&move_tile(&m, "3", 0, 6)), ["2@2,0 2x2", "3@0,6 4x1"]);
}

#[test]
fn move_no_ops_are_same_tree() {
    let m = tiles_base();
    assert!(move_tile(&m, "1", 0, 0).same(&m));
    assert!(move_tile(&m, "1", -5, -5).same(&m));
    assert!(move_tile(&m, "zzz", 1, 1).same(&m));
}

#[test]
fn growing_over_a_neighbour_pushes_it_down() {
    let m = resize_tile(&tiles_base(), "1", 4, 2);
    assert_eq!(boxes(&m), ["1@0,0 4x2", "2@2,2 2x2", "3@0,4 4x1"]);
}

#[test]
fn a_width_past_the_grid_edge_pulls_the_tile_back_inside() {
    assert_eq!(boxes(&resize_tile(&tiles_base(), "2", 4, 2)), ["1@0,2 2x2", "2@0,0 4x2", "3@0,4 4x1"]);
}

#[test]
fn resize_no_ops_are_same_tree() {
    let m = tiles_base();
    assert!(resize_tile(&m, "1", 2, 2).same(&m));
    assert!(!resize_tile(&m, "1", 0, -4).same(&m));
    assert!(resize_tile(&m, "zzz", 1, 1).same(&m));
}

#[test]
fn raise_moves_the_tile_last_without_moving_cells() {
    let m = raise_tile(&tiles_base(), "1");
    assert_eq!(tile_ids(&m), ["2", "3", "1"]);
    let mut got = boxes(&m);
    got.sort();
    let mut want = boxes(&tiles_base());
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn raise_when_last_or_unknown_is_same_tree() {
    let m = tiles_base();
    assert!(raise_tile(&m, "3").same(&m));
    assert!(raise_tile(&m, "zzz").same(&m));
}

#[test]
fn remove_tile_drops_it_and_unknown_is_same_tree() {
    let m = tiles_base();
    assert_eq!(tile_ids(&remove_tile(&m, "2")), ["1", "3"]);
    assert!(remove_tile(&m, "zzz").same(&m));
}

#[test]
fn update_tile_replaces_data_and_equal_data_is_same_tree() {
    let m = tiles_base();
    let updated = update_tile(&m, "1", |d| if *d == "a" { "z" } else { "a" });
    assert_eq!(find_tile(&updated, "1").unwrap().data, "z");
    assert!(update_tile(&updated, "1", |d| *d).same(&updated));
    assert!(update_tile(&updated, "zzz", |d| *d).same(&updated));
    assert!(Rc::ptr_eq(&updated.tiles()[1], &m.tiles()[1]));
}

#[test]
fn tiles_serialize_round_trips_through_json() {
    let m = raise_tile(&move_tile(&tiles_base(), "1", 2, 0), "3");
    let json = serde_json::to_string(&serialize_tiles(&m)).unwrap();
    let parsed: TileSnapshot<String> = serde_json::from_str(&json).unwrap();
    let back = deserialize_tiles(parsed, 4, |_| true);
    assert_eq!(boxes(&back), boxes(&m));
    assert_eq!(back.next_id(), m.next_id());
    assert_eq!(deserialize_tiles(serialize_tiles(&m), 4, |_| true), m);
}

#[test]
fn tiles_snapshot_is_unaffected_by_later_edits() {
    let m = tiles_base();
    let snapshot = serialize_tiles(&m);
    let _ = move_tile(&m, "1", 2, 0);
    let tile = &snapshot.tiles[0];
    assert_eq!((tile.id.as_ref(), tile.x, tile.y, tile.w, tile.h, tile.data), ("1", 0, 0, 2, 2, "a"));
}

#[test]
fn tiles_malformed_input_falls_back_to_the_empty_layout() {
    let empty = empty_tiles::<String>(6);
    let tile =
        |id: &str, data: &str| Tile { id: id.to_string().into(), x: 0, y: 0, w: 1, h: 1, data: data.to_string() };
    let snap = |tiles| TileSnapshot { columns: None, tiles, next_id: 0 };
    assert_eq!(deserialize_tiles(snap(vec![tile("", "a")]), 6, |_| true), empty);
    assert_eq!(deserialize_tiles(snap(vec![tile("1", "x")]), 6, |d| d != "x"), empty);
    assert!(serde_json::from_str::<TileSnapshot<String>>(r#"{"tiles":"nope"}"#).is_err());
    assert!(
        serde_json::from_str::<TileSnapshot<String>>(r#"{"tiles":[{"x":0,"y":0,"w":1,"h":1,"data":"a"}]}"#).is_err()
    );
}

#[test]
fn tiles_a_duplicate_id_rejects_the_layout() {
    let raw = r#"{"columns":4,"tiles":[{"id":"1","x":0,"y":0,"w":1,"h":1,"data":"a"},
        {"id":"1","x":1,"y":0,"w":1,"h":1,"data":"b"}],"nextId":2}"#;
    let snap: TileSnapshot<String> = serde_json::from_str(raw).unwrap();
    assert_eq!(deserialize_tiles(snap, 4, |_| true), empty_tiles(4));
}

#[test]
fn tiles_an_overlapping_layout_is_pushed_apart_on_load() {
    let raw = r#"{"columns":4,"tiles":[{"id":"1","x":0,"y":0,"w":2,"h":2,"data":"a"},
        {"id":"9","x":0,"y":0,"w":2,"h":2,"data":"b"}],"nextId":1}"#;
    let snap: TileSnapshot<String> = serde_json::from_str(raw).unwrap();
    let m = deserialize_tiles(snap, 4, |_| true);
    assert_eq!(boxes(&m), ["1@0,0 2x2", "9@0,2 2x2"]);
    assert_eq!(m.next_id(), 10);
}

#[test]
fn tiles_missing_columns_fall_back_to_the_argument() {
    let snap: TileSnapshot<String> = serde_json::from_str(r#"{"tiles":[]}"#).unwrap();
    assert_eq!(deserialize_tiles(snap, 6, |_| true).columns(), 6);
}

#[test]
fn tile_drag_payloads_round_trip_and_foreign_text_is_rejected() {
    assert_eq!(parse_tile_drag(&tile_drag_payload("3")), Some("3"));
    for foreign in ["", "3", "nd-tiles:tile:", "nd-dock:tab:3"] {
        assert!(parse_tile_drag(foreign).is_none(), "{foreign:?}");
    }
}

const GRID: TileSize = TileSize { width: 400.0, height: 300.0 };

#[test]
fn tile_cell_at_maps_a_point_to_its_cell() {
    let m = tiles_base();
    assert_eq!(tile_rows(&m), 3);
    assert_eq!(tile_cell_at(&m, GRID, 0.0, 0.0), TileCell { column: 0, row: 0 });
    assert_eq!(tile_cell_at(&m, GRID, 250.0, 150.0), TileCell { column: 2, row: 1 });
    assert_eq!(tile_cell_at(&m, GRID, 399.0, 299.0), TileCell { column: 3, row: 2 });
}

#[test]
fn tile_cell_boundaries_belong_to_the_cell_they_open() {
    let m = tiles_base();
    assert_eq!(tile_cell_at(&m, GRID, 99.0, 99.0), TileCell { column: 0, row: 0 });
    assert_eq!(tile_cell_at(&m, GRID, 100.0, 100.0), TileCell { column: 1, row: 1 });
}

#[test]
fn tile_cell_at_clamps_outside_points_and_degenerate_sizes() {
    let m = tiles_base();
    assert_eq!(tile_cell_at(&m, GRID, -20.0, -20.0), TileCell { column: 0, row: 0 });
    assert_eq!(tile_cell_at(&m, GRID, 900.0, 150.0), TileCell { column: 3, row: 1 });
    assert_eq!(tile_cell_at(&m, TileSize { width: 0.0, height: 0.0 }, 40.0, 40.0), TileCell { column: 0, row: 0 });
    assert_eq!(
        tile_cell_at(&m, TileSize { width: f32::NAN, height: 300.0 }, 40.0, 40.0),
        TileCell { column: 0, row: 0 }
    );
}

#[test]
fn tile_drop_cell_becomes_the_top_left() {
    let m = apply_tile_drop(&tiles_base(), &tile_drag_payload("3"), GRID, 250.0, 40.0);
    assert_eq!(boxes(&m), ["1@0,1 2x2", "2@2,1 2x2", "3@0,0 4x1"]);
}

#[test]
fn tile_dropped_inside_a_neighbour_lands_on_its_origin_cell() {
    let m = apply_tile_drop(&tiles_base(), &tile_drag_payload("1"), GRID, 250.0, 40.0);
    let tile = find_tile(&m, "1").unwrap();
    assert_eq!((tile.x, tile.y), (2, 0));
}

#[test]
fn tile_dropped_back_on_its_own_cell_is_same_tree() {
    let m = tiles_base();
    assert!(apply_tile_drop(&m, &tile_drag_payload("2"), GRID, 250.0, 40.0).same(&m));
    assert!(apply_tile_drop(&m, &tile_drag_payload("2"), GRID, 210.0, 10.0).same(&m));
}

#[test]
fn tile_drop_foreign_payload_and_unknown_tile_are_ignored() {
    let m = tiles_base();
    assert!(apply_tile_drop(&m, "https://example.com", GRID, 250.0, 40.0).same(&m));
    assert!(apply_tile_drop(&m, &tile_drag_payload("gone"), GRID, 250.0, 40.0).same(&m));
}
