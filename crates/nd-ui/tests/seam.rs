//! Backend seam conformance on `TestBackend` (3.15).

use std::rc::Rc;

use nd_ui::backend::{AvailableSpace, Backend, Constraint, NodeKind, Phase, Props, PropsPatch, WidgetKind};
use nd_ui::test_backend::{Recorded, TestBackend};
use nd_ui::transaction::{MeasureCache, Op, Transaction};
use nd_ui::{NodeId, Rect, Size, WindowId};

const W: WindowId = WindowId(1);
const ROOT: NodeId = NodeId(1);

#[derive(Debug, PartialEq)]
struct Label(&'static str);

struct Button;

fn props(text: &'static str) -> Props {
    Rc::new(Label(text))
}

fn create(node: u64, kind: NodeKind, text: &'static str) -> Op {
    Op::Create { node: NodeId(node), kind, props: props(text) }
}

fn insert(parent: u64, node: u64) -> Op {
    Op::Insert { parent: NodeId(parent), node: NodeId(node), before: None }
}

/// Root with two children, n2 (a view) and n3 (a button widget) inside n2.
fn mounted() -> (TestBackend, MeasureCache) {
    let mut backend = TestBackend::new();
    let mut cache = MeasureCache::default();
    let mut tx = Transaction::new(W);
    tx.push(create(1, NodeKind::Root, "root"));
    tx.push(create(2, NodeKind::View, "box"));
    tx.push(create(3, NodeKind::Widget(WidgetKind::of::<Button>()), "ok"));
    tx.push(insert(1, 2));
    tx.push(insert(2, 3));
    tx.commit(&mut backend, &mut cache);
    backend.take_log();
    (backend, cache)
}

fn phases(backend: &TestBackend) -> Vec<Phase> {
    backend.ops().map(Op::phase).collect()
}

#[test]
fn ops_reach_the_backend_in_phase_order() {
    let (mut backend, mut cache) = mounted();
    let mut tx = Transaction::new(W);
    // Pushed in an order the reconciler could produce, applied in phase order.
    tx.push(Op::Focus { node: Some(NodeId(4)) });
    tx.push(Op::Release { node: NodeId(3) });
    tx.push(Op::SetFrame { node: NodeId(4), frame: Rect::new(0., 0., 10., 10.) });
    tx.push(Op::Update { node: NodeId(2), patch: PropsPatch { old: props("box"), new: props("box2") } });
    tx.push(Op::Remove { node: NodeId(3) });
    tx.push(create(4, NodeKind::Text, "hi"));
    tx.push(insert(2, 4));
    tx.commit(&mut backend, &mut cache);

    assert_eq!(
        phases(&backend),
        [Phase::Create, Phase::Structure, Phase::Structure, Phase::Update, Phase::Frame, Phase::Focus, Phase::Release]
    );
    assert!(matches!(backend.log().first(), Some(Recorded::Begin(W))));
    assert!(matches!(backend.log().last(), Some(Recorded::Commit(W))));
    assert_eq!(backend.children(NodeId(2)), [NodeId(4)]);
    assert_eq!(backend.focused(), Some(NodeId(4)));
    assert!(!backend.is_live(NodeId(3)));
}

#[test]
fn structural_ops_keep_push_order_so_parents_attach_first() {
    let (mut backend, mut cache) = mounted();
    let mut tx = Transaction::new(W);
    tx.push(create(10, NodeKind::View, "a"));
    tx.push(create(11, NodeKind::View, "b"));
    tx.push(insert(1, 10));
    tx.push(insert(10, 11));
    tx.push(Op::Move { node: NodeId(3), parent: NodeId(11), before: None });
    tx.commit(&mut backend, &mut cache);
    assert_eq!(backend.parent(NodeId(3)), Some(NodeId(11)));
    assert_eq!(backend.children(ROOT), [NodeId(2), NodeId(10)]);
}

#[test]
fn only_the_last_focus_request_of_a_frame_is_sent() {
    let (mut backend, mut cache) = mounted();
    let mut tx = Transaction::new(W);
    tx.push(Op::Focus { node: Some(NodeId(2)) });
    tx.push(Op::Focus { node: Some(NodeId(3)) });
    tx.commit(&mut backend, &mut cache);
    assert_eq!(phases(&backend), [Phase::Focus]);
    assert_eq!(backend.focused(), Some(NodeId(3)));
}

#[test]
fn an_empty_transaction_sends_nothing() {
    let (mut backend, mut cache) = mounted();
    Transaction::new(W).commit(&mut backend, &mut cache);
    assert!(backend.log().is_empty());
}

#[test]
fn frames_are_dropped_for_a_backend_that_lays_out_itself() {
    let mut backend = TestBackend::new().without_frames();
    let mut cache = MeasureCache::default();
    let mut tx = Transaction::new(W);
    tx.push(create(1, NodeKind::Root, "root"));
    tx.push(Op::SetFrame { node: ROOT, frame: Rect::new(0., 0., 100., 100.) });
    tx.commit(&mut backend, &mut cache);
    assert_eq!(phases(&backend), [Phase::Create]);
}

#[test]
fn measure_is_cached_by_constraint_until_props_change() {
    let (mut backend, mut cache) = mounted();
    backend.on_measure(|_, c| match c.width {
        AvailableSpace::Definite(w) => Size { width: w, height: 20. },
        _ => Size { width: 80., height: 20. },
    });
    let wide = Constraint { width: AvailableSpace::Definite(200.), height: AvailableSpace::MaxContent };
    let fit = Constraint { width: AvailableSpace::MaxContent, height: AvailableSpace::MaxContent };
    let n = NodeId(3);
    assert_eq!(cache.measure(&mut backend, n, wide).width, 200.);
    assert_eq!(cache.measure(&mut backend, n, wide).width, 200.);
    assert_eq!(cache.measure(&mut backend, n, fit).width, 80.);
    let measures = |b: &TestBackend| b.log().iter().filter(|r| matches!(r, Recorded::Measure(..))).count();
    assert_eq!(measures(&backend), 2);

    let mut tx = Transaction::new(W);
    tx.push(Op::Update { node: n, patch: PropsPatch { old: props("ok"), new: props("okay") } });
    tx.commit(&mut backend, &mut cache);
    cache.measure(&mut backend, n, wide);
    assert_eq!(measures(&backend), 3);
}

#[test]
fn a_removed_subtree_releases_cleanly() {
    let (mut backend, mut cache) = mounted();
    let mut tx = Transaction::new(W);
    tx.push(Op::Remove { node: NodeId(2) });
    tx.push(Op::Release { node: NodeId(2) });
    tx.push(Op::Release { node: NodeId(3) });
    tx.commit(&mut backend, &mut cache);
    assert_eq!(backend.live_count(), 1);
    assert!(backend.children(ROOT).is_empty());
}

#[test]
#[should_panic(expected = "outside a transaction")]
fn ops_outside_a_transaction_are_rejected() {
    let mut backend = TestBackend::new();
    backend.create(ROOT, NodeKind::Root, &props("root"));
}

#[test]
#[should_panic(expected = "after Focus")]
fn out_of_order_ops_are_rejected() {
    let (mut backend, _) = mounted();
    backend.begin(W);
    backend.focus(Some(NodeId(2)));
    backend.insert(ROOT, NodeId(99), None);
}

#[test]
#[should_panic(expected = "still in a window")]
fn releasing_an_attached_node_is_rejected() {
    let (mut backend, mut cache) = mounted();
    let mut tx = Transaction::new(W);
    tx.push(Op::Release { node: NodeId(3) });
    tx.commit(&mut backend, &mut cache);
}

#[test]
#[should_panic(expected = "not live")]
fn naming_a_released_node_is_rejected() {
    let (mut backend, mut cache) = mounted();
    let mut tx = Transaction::new(W);
    tx.push(Op::Remove { node: NodeId(3) });
    tx.push(Op::Release { node: NodeId(3) });
    tx.commit(&mut backend, &mut cache);
    let mut tx = Transaction::new(W);
    tx.push(insert(2, 3));
    tx.commit(&mut backend, &mut cache);
}

#[test]
#[should_panic(expected = "with equal props")]
fn an_update_with_equal_props_is_rejected() {
    let (mut backend, mut cache) = mounted();
    let mut tx = Transaction::new(W);
    tx.push(Op::Update { node: NodeId(2), patch: PropsPatch { old: props("box"), new: props("box") } });
    tx.commit(&mut backend, &mut cache);
}

#[test]
#[should_panic(expected = "its own ancestor")]
fn a_cycle_is_rejected() {
    let (mut backend, mut cache) = mounted();
    let mut tx = Transaction::new(W);
    tx.push(Op::Move { node: NodeId(2), parent: NodeId(3), before: None });
    tx.commit(&mut backend, &mut cache);
}
