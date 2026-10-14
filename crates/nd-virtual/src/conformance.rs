//! The list conformance suite (ARCHITECTURE.md 7). Every presenter runs the same checks through
//! a [`Harness`]: the model alone, the GPUI presenter in a headless GPUI app, a real
//! `NSTableView` in tier 2. Each check encodes one requirement from `docs/parity/apps.md` section
//! 3, and the last-row checks encode the "last rows disappearing" class directly: rows whose real
//! heights are far from the estimate, rows that grow after the list reached its end.
//!
//! Rows are keyed `u64`; a row's content is a box of a given height, which the harness can change
//! after layout as an image or embed loading would.

use crate::core::{Align, Alignment, Follow};

/// One painted row: its key, its top edge relative to the viewport's top, its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Painted {
    pub key: u64,
    pub y: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Setup {
    pub width: f64,
    pub height: f64,
    pub estimate: f64,
    pub alignment: Alignment,
    pub follow: Follow,
}

impl Default for Setup {
    fn default() -> Self {
        Self { width: 320., height: 400., estimate: 44., alignment: Alignment::Top, follow: Follow::None }
    }
}

/// A presenter under test.
pub trait Harness {
    /// Replaces the rows (key, content height), keyed, as an app's `set_items` would.
    fn set_rows(&mut self, rows: &[(u64, f64)]);
    /// The row's content changes height after it was laid out; only that row is invalidated.
    fn set_content_height(&mut self, key: u64, height: f64);
    /// A user scroll (wheel, touchpad or scroll view), positive towards the end.
    fn scroll_by(&mut self, dy: f64);
    fn scroll_to(&mut self, key: u64, align: Align, offset: f64);
    fn scroll_to_end(&mut self);
    /// The viewport changes size.
    fn resize(&mut self, width: f64, height: f64);
    /// Runs frames until nothing is pending.
    fn settle(&mut self);
    /// Rows on screen, in order, as the presenter painted them.
    fn painted(&mut self) -> Vec<Painted>;
    fn following(&mut self) -> bool;
    /// How many times the row's content was rendered.
    fn renders(&mut self, key: u64) -> u64;
    fn viewport_height(&self) -> f64;
}

/// A presenter factory for one check.
pub trait Factory {
    type H: Harness;
    fn open(&self, setup: Setup) -> Self::H;
}

impl<H: Harness, F: Fn(Setup) -> H> Factory for F {
    type H = H;
    fn open(&self, setup: Setup) -> H {
        self(setup)
    }
}

/// The checks by name, so a harness=false binary can list and run them one per process.
/// One named check.
pub type Check<F> = (&'static str, fn(&F));

pub fn checks<F: Factory>() -> Vec<Check<F>> {
    vec![
        ("last_row_reachable_by_wheel", last_row_reachable_by_wheel::<F>),
        ("last_row_reachable_by_jump", last_row_reachable_by_jump::<F>),
        ("last_row_reachable_after_it_grows", last_row_reachable_after_it_grows::<F>),
        ("heights_change_after_layout", heights_change_after_layout::<F>),
        ("prepend_keeps_view_still", prepend_keeps_view_still::<F>),
        ("tail_follow_stops_on_user_scroll", tail_follow_stops_on_user_scroll::<F>),
        ("jump_to_item_with_offset", jump_to_item_with_offset::<F>),
        ("keyed_updates_never_reset_scroll", keyed_updates_never_reset_scroll::<F>),
        ("bottom_alignment_rests_short_content", bottom_alignment_rests_short_content::<F>),
        ("clean_rows_do_not_render", clean_rows_do_not_render::<F>),
        ("resize_keeps_anchor", resize_keeps_anchor::<F>),
        ("hundred_thousand_rows", hundred_thousand_rows::<F>),
    ]
}

/// A libtest-compatible `main` for a `harness = false` test binary: lists the checks as
/// `<prefix>::<check>` for nextest, runs the selected ones and exits non-zero on a failure. Native
/// presenters own the main thread, so every check runs on it, one after another.
pub fn test_main<F: Factory>(prefix: &str, factory: &F) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tests: Vec<(String, Check<F>)> =
        checks::<F>().into_iter().map(|(name, f)| (format!("{prefix}::{name}"), (name, f))).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            for (name, _) in &tests {
                println!("{name}: test");
            }
        }
        return;
    }
    if has("--ignored") {
        return;
    }
    let exact = has("--exact");
    let mut skip = false;
    let filters: Vec<&String> = args
        .iter()
        .filter(|a| {
            if std::mem::take(&mut skip) {
                return false;
            }
            if matches!(a.as_str(), "--format" | "--test-threads" | "--color" | "--skip" | "-Z") {
                skip = true;
            }
            !a.starts_with('-')
        })
        .collect();
    let mut failed = 0;
    for (name, (_, test)) in &tests {
        let selected =
            filters.is_empty() || filters.iter().any(|f| if exact { *f == name } else { name.contains(f.as_str()) });
        if !selected {
            continue;
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test(factory))) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(101);
    }
}

/// As [`test_main`], but the whole suite is the one test `name`, every check run in one process.
/// For a presenter that opens a real app and window per check, where process start dominates.
pub fn test_main_as_one<F: Factory>(name: &str, factory: &F) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            println!("{name}: test");
        }
        return;
    }
    let exact = has("--exact");
    let mut skip = false;
    let selected = args
        .iter()
        .filter(|a| {
            if std::mem::take(&mut skip) {
                return false;
            }
            if matches!(a.as_str(), "--format" | "--test-threads" | "--color" | "--skip" | "-Z") {
                skip = true;
            }
            !a.starts_with('-')
        })
        .all(|f| if exact { f == name } else { name.contains(f.as_str()) });
    if has("--ignored") || !selected {
        return;
    }
    let mut failed = 0;
    for (check_name, check) in checks::<F>() {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(factory))) {
            Ok(()) => println!("check {check_name} ... ok"),
            Err(_) => {
                println!("check {check_name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(101);
    }
}

/// Runs every check.
pub fn run_all<F: Factory>(factory: &F) {
    for (name, check) in checks::<F>() {
        eprintln!("conformance: {name}");
        check(factory);
    }
}

/// Deterministic content heights, 20 to 220 points, far from the 44 point estimate both ways.
pub fn wild_height(key: u64) -> f64 {
    let x = key.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17);
    20. + (x % 201) as f64
}

pub fn rows(keys: impl IntoIterator<Item = u64>) -> Vec<(u64, f64)> {
    keys.into_iter().map(|k| (k, wild_height(k))).collect()
}

const TOL: f64 = 0.75;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= TOL
}

fn find(painted: &[Painted], key: u64) -> Option<Painted> {
    painted.iter().copied().find(|p| p.key == key)
}

/// Rows tile the viewport: each starts where the previous ended, no gaps or overlaps.
fn assert_tiled(painted: &[Painted], context: &str) {
    for pair in painted.windows(2) {
        assert!(
            close(pair[0].y + pair[0].height, pair[1].y),
            "{context}: row {} ends at {} but row {} starts at {}",
            pair[0].key,
            pair[0].y + pair[0].height,
            pair[1].key,
            pair[1].y
        );
    }
}

/// The last row is painted, with its real height, and its bottom edge sits on the viewport's.
fn assert_last_row_at_bottom<H: Harness>(h: &mut H, last: u64, height: f64, context: &str) {
    let painted = h.painted();
    let Some(row) = find(&painted, last) else {
        panic!("{context}: last row {last} is not painted; painted {painted:?}");
    };
    assert!(close(row.height, height), "{context}: last row painted {} high, content is {height}", row.height);
    let bottom = row.y + row.height;
    assert!(
        close(bottom, h.viewport_height()),
        "{context}: last row ends at {bottom}, viewport at {}",
        h.viewport_height()
    );
    assert_tiled(&painted, context);
}

/// The first visible row and its offset: what must not move.
fn anchor(painted: &[Painted]) -> Painted {
    *painted.first().expect("rows painted")
}

pub fn last_row_reachable_by_wheel<F: Factory>(f: &F) {
    let mut h = f.open(Setup::default());
    h.set_rows(&rows(0..500));
    h.settle();
    // Wheel ticks only, as a user would: no jump API.
    for _ in 0..2000 {
        h.scroll_by(120.);
        if h.painted().iter().any(|p| p.key == 499) {
            break;
        }
    }
    for _ in 0..20 {
        h.scroll_by(400.);
        h.settle();
    }
    assert_last_row_at_bottom(&mut h, 499, wild_height(499), "wheel to the end");
    // More wheel at the end changes nothing.
    h.scroll_by(1000.);
    h.settle();
    assert_last_row_at_bottom(&mut h, 499, wild_height(499), "wheel past the end");
}

pub fn last_row_reachable_by_jump<F: Factory>(f: &F) {
    for estimate in [8., 44., 400.] {
        let mut h = f.open(Setup { estimate, ..Setup::default() });
        h.set_rows(&rows(0..2000));
        h.settle();
        h.scroll_to_end();
        h.settle();
        assert_last_row_at_bottom(&mut h, 1999, wild_height(1999), &format!("jump to end, estimate {estimate}"));
        // Scrolling up and back down by the wheel lands on the same edge.
        h.scroll_by(-3000.);
        h.settle();
        h.scroll_by(6000.);
        h.settle();
        assert_last_row_at_bottom(&mut h, 1999, wild_height(1999), &format!("wheel back, estimate {estimate}"));
    }
}

pub fn last_row_reachable_after_it_grows<F: Factory>(f: &F) {
    let mut h = f.open(Setup::default());
    h.set_rows(&rows(0..300));
    h.settle();
    h.scroll_to_end();
    h.settle();
    // The tail rows load media after the list reached its end, as messages' image rows did.
    for k in 295..300 {
        h.set_content_height(k, 260.);
    }
    h.settle();
    h.scroll_by(5000.);
    h.settle();
    assert_last_row_at_bottom(&mut h, 299, 260., "tail rows grew");
    // And shrank again: no gap opens under the last row.
    for k in 295..300 {
        h.set_content_height(k, 30.);
    }
    h.settle();
    h.scroll_by(5000.);
    h.settle();
    assert_last_row_at_bottom(&mut h, 299, 30., "tail rows shrank");
}

pub fn heights_change_after_layout<F: Factory>(f: &F) {
    let mut h = f.open(Setup::default());
    h.set_rows(&rows(0..400));
    h.settle();
    h.scroll_to(200, Align::Start, 0.);
    h.settle();
    let before = anchor(&h.painted());
    assert_eq!(before.key, 200);
    // Rows above (offscreen and in the overscan) and below the anchor change height.
    for k in (150..200).chain(201..215) {
        h.set_content_height(k, wild_height(k) + 37.);
    }
    h.settle();
    let painted = h.painted();
    let after = find(&painted, 200).expect("anchor row still painted");
    assert!(close(after.y, before.y), "anchor row moved from {} to {}", before.y, after.y);
    assert_tiled(&painted, "after height changes");
    for p in &painted {
        if (201..215).contains(&p.key) {
            assert!(close(p.height, wild_height(p.key) + 37.), "row {} painted at a stale height {}", p.key, p.height);
        }
    }
    // The anchor row itself grows: its top edge stays.
    h.set_content_height(200, 300.);
    h.settle();
    let after = find(&h.painted(), 200).expect("anchor row");
    assert!(close(after.y, before.y), "growing anchor row moved from {} to {}", before.y, after.y);
    assert!(close(after.height, 300.));
}

pub fn prepend_keeps_view_still<F: Factory>(f: &F) {
    let mut h = f.open(Setup::default());
    h.set_rows(&rows(1000..1100));
    h.settle();
    h.scroll_to(1050, Align::Start, -10.);
    h.settle();
    let before = h.painted();
    // Older rows load above, twice, as infinite scroll up does.
    h.set_rows(&rows(900..1100));
    h.settle();
    h.set_rows(&rows(0..1100));
    h.settle();
    let after = h.painted();
    for b in &before {
        let a = find(&after, b.key).unwrap_or_else(|| panic!("row {} vanished after prepend", b.key));
        assert!(close(a.y, b.y), "row {} moved from {} to {} on prepend", b.key, b.y, a.y);
    }
}

pub fn tail_follow_stops_on_user_scroll<F: Factory>(f: &F) {
    let mut h = f.open(Setup { alignment: Alignment::Bottom, follow: Follow::Tail, ..Setup::default() });
    h.set_rows(&rows(0..50));
    h.settle();
    assert!(h.following(), "a bottom-aligned tail list opens following");
    assert_last_row_at_bottom(&mut h, 49, wild_height(49), "opened at the tail");
    h.set_rows(&rows(0..60));
    h.settle();
    assert_last_row_at_bottom(&mut h, 59, wild_height(59), "followed new rows");

    h.scroll_by(-300.);
    h.settle();
    assert!(!h.following(), "scrolling up stops following");
    let before = h.painted();
    h.set_rows(&rows(0..80));
    h.settle();
    let after = h.painted();
    assert_eq!(anchor(&before).key, anchor(&after).key, "new rows moved the view while not following");
    assert!(close(anchor(&before).y, anchor(&after).y));

    // A row at the end growing while not following moves nothing either.
    h.set_content_height(79, 500.);
    h.settle();
    assert!(close(anchor(&h.painted()).y, anchor(&before).y));

    h.scroll_by(1e6);
    h.settle();
    assert!(h.following(), "scrolling back to the end follows again");
    h.set_rows(&rows(0..90));
    h.settle();
    assert_last_row_at_bottom(&mut h, 89, wild_height(89), "following again");
    // A row at the end that grows while following stays fully in view.
    h.set_content_height(89, 320.);
    h.settle();
    assert_last_row_at_bottom(&mut h, 89, 320., "tail row grew while following");
}

pub fn jump_to_item_with_offset<F: Factory>(f: &F) {
    let mut h = f.open(Setup::default());
    h.set_rows(&rows(0..5000));
    h.settle();
    h.scroll_to(3210, Align::Start, -96.);
    h.settle();
    let row = find(&h.painted(), 3210).expect("target painted");
    assert!(close(row.y, 96.), "Start with -96 puts the row 96 below the top, got {}", row.y);
    assert_tiled(&h.painted(), "after jump");

    h.scroll_to(17, Align::Center, 0.);
    h.settle();
    let row = find(&h.painted(), 17).expect("target painted");
    let vp = h.viewport_height();
    assert!(close(row.y + row.height / 2., vp / 2.), "centred row's middle at {}", row.y + row.height / 2.);

    h.scroll_to(4000, Align::End, 0.);
    h.settle();
    let row = find(&h.painted(), 4000).expect("target painted");
    assert!(close(row.y + row.height, vp), "End puts the row's bottom on the viewport's, got {}", row.y + row.height);

    // Nearest: a fully visible row does not scroll.
    let before = h.painted();
    let visible = before[before.len() / 2].key;
    h.scroll_to(visible, Align::Nearest, 0.);
    h.settle();
    assert_eq!(h.painted(), before);
}

/// Seeded xorshift, so a failure replays.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

pub fn keyed_updates_never_reset_scroll<F: Factory>(f: &F) {
    for seed in [7u64, 99, 12345] {
        let mut rng = Rng(seed);
        let mut h = f.open(Setup::default());
        let mut keys: Vec<u64> = (0..600).collect();
        let mut next_key = 10_000u64;
        let mut heights = std::collections::HashMap::new();
        let row =
            |k: u64, heights: &std::collections::HashMap<u64, f64>| (k, *heights.get(&k).unwrap_or(&wild_height(k)));
        h.set_rows(&keys.iter().map(|k| row(*k, &heights)).collect::<Vec<_>>());
        h.settle();
        h.scroll_to(300, Align::Start, 0.);
        h.settle();
        for step in 0..40 {
            let painted = h.painted();
            let a = anchor(&painted);
            let shown: std::collections::HashSet<u64> = painted.iter().map(|p| p.key).collect();
            let a_ix = keys.iter().position(|k| *k == a.key).expect("anchor key");
            match rng.below(4) {
                // Insert a run somewhere not on screen.
                0 => {
                    let at = rng.below(keys.len() as u64 + 1) as usize;
                    let near = keys.get(at).is_some_and(|k| shown.contains(k))
                        || (at > 0 && shown.contains(&keys[at - 1]) && at > a_ix);
                    if !near {
                        let n = 1 + rng.below(20);
                        for i in 0..n {
                            keys.insert(at, next_key + i);
                        }
                        next_key += n;
                    }
                }
                // Remove an offscreen run.
                1 => {
                    let at = rng.below(keys.len() as u64) as usize;
                    let n = (1 + rng.below(15) as usize).min(keys.len() - at);
                    if !keys[at..at + n].iter().any(|k| shown.contains(k)) {
                        keys.drain(at..at + n);
                    }
                }
                // Swap two offscreen rows.
                2 => {
                    let (i, j) = (rng.below(keys.len() as u64) as usize, rng.below(keys.len() as u64) as usize);
                    let between = |ix: usize| (ix <= a_ix) == (i <= a_ix) && (ix <= a_ix) == (j <= a_ix);
                    if !shown.contains(&keys[i]) && !shown.contains(&keys[j]) && between(i) {
                        keys.swap(i, j);
                    }
                }
                // Resize an offscreen row.
                _ => {
                    let k = keys[rng.below(keys.len() as u64) as usize];
                    if !shown.contains(&k) {
                        heights.insert(k, 20. + rng.below(300) as f64);
                        h.set_content_height(k, heights[&k]);
                    }
                }
            }
            h.set_rows(&keys.iter().map(|k| row(*k, &heights)).collect::<Vec<_>>());
            h.settle();
            let after = h.painted();
            let moved = find(&after, a.key).unwrap_or_else(|| panic!("seed {seed} step {step}: anchor {} gone", a.key));
            assert!(close(moved.y, a.y), "seed {seed} step {step}: anchor {} moved {} -> {}", a.key, a.y, moved.y);
            assert_tiled(&after, &format!("seed {seed} step {step}"));
        }
        // Removing the anchor row itself: the next row takes its place, nothing below jumps.
        let painted = h.painted();
        let (a, b) = (painted[0], painted[1]);
        keys.retain(|k| *k != a.key);
        h.set_rows(&keys.iter().map(|k| row(*k, &heights)).collect::<Vec<_>>());
        h.settle();
        let after = find(&h.painted(), b.key).expect("next row");
        assert!(close(after.y, b.y), "seed {seed}: removing the anchor row moved the next row {} -> {}", b.y, after.y);
    }
}

pub fn bottom_alignment_rests_short_content<F: Factory>(f: &F) {
    let mut h = f.open(Setup { alignment: Alignment::Bottom, ..Setup::default() });
    h.set_rows(&[(1, 40.), (2, 60.), (3, 30.)]);
    h.settle();
    let painted = h.painted();
    assert_eq!(painted.iter().map(|p| p.key).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_last_row_at_bottom(&mut h, 3, 30., "short bottom-aligned list");
    assert!(close(painted[0].y, h.viewport_height() - 130.));
    // A row grows: the content stays on the bottom edge and grows upwards.
    h.set_content_height(2, 100.);
    h.settle();
    assert_last_row_at_bottom(&mut h, 3, 30., "grown row");
}

pub fn clean_rows_do_not_render<F: Factory>(f: &F) {
    let mut h = f.open(Setup::default());
    h.set_rows(&rows(0..100));
    h.settle();
    let shown: Vec<u64> = h.painted().iter().map(|p| p.key).collect();
    assert!(shown.len() >= 3, "rows painted: {shown:?}");
    let before: Vec<u64> = shown.iter().map(|k| h.renders(*k)).collect();
    assert!(before.iter().all(|r| *r >= 1), "visible rows rendered: {before:?}");
    // The same keys again, and one row's content changing: only that row renders.
    let changed = shown[1];
    h.set_rows(&rows(0..100));
    h.settle();
    h.set_content_height(changed, 99.);
    h.settle();
    for (k, b) in shown.iter().zip(before) {
        let expected = b + u64::from(*k == changed);
        assert_eq!(h.renders(*k), expected, "row {k} rendered {} times, expected {expected}", h.renders(*k));
    }
}

pub fn resize_keeps_anchor<F: Factory>(f: &F) {
    let mut h = f.open(Setup::default());
    h.set_rows(&rows(0..400));
    h.settle();
    h.scroll_to(120, Align::Start, 0.);
    h.settle();
    h.resize(500., 300.);
    h.settle();
    let painted = h.painted();
    let row = find(&painted, 120).expect("anchor row");
    assert!(close(row.y, 0.), "anchor row at {} after resize", row.y);
    assert_tiled(&painted, "after resize");
    h.scroll_to_end();
    h.settle();
    assert_last_row_at_bottom(&mut h, 399, wild_height(399), "end after resize");
}

pub fn hundred_thousand_rows<F: Factory>(f: &F) {
    let mut h = f.open(Setup::default());
    let started = std::time::Instant::now();
    h.set_rows(&rows(0..100_000));
    h.settle();
    h.scroll_to(50_000, Align::Center, 0.);
    h.settle();
    assert!(find(&h.painted(), 50_000).is_some());
    h.scroll_to_end();
    h.settle();
    assert_last_row_at_bottom(&mut h, 99_999, wild_height(99_999), "100k rows, end");
    // Keyed diff at full size: drop one row near the top.
    let mut keys: Vec<u64> = (0..100_000).collect();
    keys.remove(10);
    h.set_rows(&rows(keys));
    h.settle();
    assert_last_row_at_bottom(&mut h, 99_999, wild_height(99_999), "100k rows after a diff");
    let elapsed = started.elapsed();
    assert!(elapsed.as_secs_f64() < 20., "100k-row check took {elapsed:?}");
}

/// The model driven by a simulated presenter: rows in the render range are "rendered" (counted)
/// and measured at their content height, rows outside are not. The reference the real
/// presenters are compared against, and the fastest place to debug a failing check.
pub struct ModelHarness {
    core: crate::core::Core,
    content: std::collections::HashMap<u64, f64>,
    rendered: std::collections::HashMap<u64, bool>,
    renders: std::collections::HashMap<u64, u64>,
    width: f64,
    height: f64,
    layout: crate::core::Layout,
}

impl ModelHarness {
    pub fn open(setup: Setup) -> Self {
        let mut core = crate::core::Core::new();
        core.set_estimate(setup.estimate);
        core.set_alignment(setup.alignment);
        core.set_follow(setup.follow);
        core.set_viewport(setup.width, setup.height);
        Self {
            core,
            content: Default::default(),
            rendered: Default::default(),
            renders: Default::default(),
            width: setup.width,
            height: setup.height,
            layout: Default::default(),
        }
    }

    pub fn core(&self) -> &crate::core::Core {
        &self.core
    }

    fn frame(&mut self) -> bool {
        // Render what the model asks for; measure everything rendered.
        let range = self.core.render_range();
        let want: Vec<u64> = range.filter_map(|i| self.core.row(i)).map(|r| r.0).collect();
        let mut changed = false;
        let keep: std::collections::HashSet<u64> = want.iter().copied().collect();
        self.rendered.retain(|k, _| keep.contains(k));
        for k in want {
            let dirty = self.rendered.insert(k, false).is_none_or(|d| d);
            if dirty {
                *self.renders.entry(k).or_default() += 1;
                changed = true;
            }
        }
        for k in self.rendered.keys() {
            changed |= self.core.report_height(crate::core::RowId(*k), self.content[k]);
        }
        let layout = self.core.layout();
        changed |= layout != self.layout;
        self.layout = layout;
        changed
    }
}

impl Harness for ModelHarness {
    fn set_rows(&mut self, rows: &[(u64, f64)]) {
        for (k, h) in rows {
            if self.content.insert(*k, *h).is_some_and(|old| old != *h) {
                self.set_content_height(*k, *h);
            }
        }
        self.core.set_rows(rows.iter().map(|(k, _)| crate::core::RowId(*k)).collect());
    }

    fn set_content_height(&mut self, key: u64, height: f64) {
        self.content.insert(key, height);
        if let Some(dirty) = self.rendered.get_mut(&key) {
            *dirty = true;
        }
        self.core.invalidate(crate::core::RowId(key));
    }

    fn scroll_by(&mut self, dy: f64) {
        self.core.scroll_by(dy);
        self.frame();
    }

    fn scroll_to(&mut self, key: u64, align: Align, offset: f64) {
        self.core.scroll_to(crate::core::RowId(key), align, offset, false);
    }

    fn scroll_to_end(&mut self) {
        self.core.scroll_to_end();
    }

    fn resize(&mut self, width: f64, height: f64) {
        self.width = width;
        self.height = height;
        self.core.set_viewport(width, height);
    }

    fn settle(&mut self) {
        for _ in 0..32 {
            if !self.frame() {
                return;
            }
        }
        panic!("model harness did not settle");
    }

    fn painted(&mut self) -> Vec<Painted> {
        self.layout
            .visible
            .iter()
            .filter(|p| self.rendered.contains_key(&p.row.0))
            .map(|p| Painted { key: p.row.0, y: p.y, height: p.height })
            .collect()
    }

    fn following(&mut self) -> bool {
        self.core.following()
    }

    fn renders(&mut self, key: u64) -> u64 {
        self.renders.get(&key).copied().unwrap_or(0)
    }

    fn viewport_height(&self) -> f64 {
        self.height
    }
}
