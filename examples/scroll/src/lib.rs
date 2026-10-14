//! A virtualized list of `ND_ROWS` rows (100 000 by default) with heights from 28 to 124 points,
//! what `nd diag scroll` measures. With `ND_SCROLL_STEP=<points>` it scrolls that far every
//! display frame, back to the top whenever it reaches the end; combined with `ND_ANIMATE=<s>` the
//! host prints the frame interval line and quits.

use nd::prelude::*;

mod gestures;

pub use gestures::Pad;

pub struct Rows {
    list: ListState<u32>,
    step: Option<f64>,
    passes: u32,
}

fn height(row: u32) -> f32 {
    28. + (row.wrapping_mul(2_654_435_761) >> 7) as f32 % 97.
}

impl Rows {
    pub fn new(rows: u32, step: Option<f64>, cx: &mut Context<Self>) -> Self {
        // docs:begin state
        let mut list = ListState::new(cx).estimated_row_height(px(60.));
        list.set_items_versioned((0..rows).map(|r| (r, 0)), cx);
        // docs:end state
        Self { list, step, passes: 0 }
    }
}

// docs:begin log
/// A log that stays pinned to its newest line until the user scrolls away from the end.
pub fn log_state<T: 'static>(lines: u32, cx: &mut Context<T>) -> ListState<u32> {
    let mut log = ListState::new(cx).follow(Follow::Tail);
    log.set_items((0..lines).collect::<Vec<_>>(), cx);
    log.scroll_to_end(cx);
    log
}

/// Brings one row to the middle of the list, 96 points above where it would rest.
pub fn reveal(list: &mut ListState<u32>, row: u32, cx: &mut App) {
    list.scroll_to(row, Align::Center, px(-96.), cx);
}
// docs:end log

// docs:begin table
/// The same keys as a table: one cell per column, sorting reported but left to the app.
pub fn row_table(state: &ListState<u32>) -> impl IntoElement {
    let columns = vec![column("row", "Row").width(120.).fixed(), column("height", "Height").sortable()];
    table("rows-table", state, columns, |row, col, _, _| match col {
        0 => text(format!("Row {row}")).tabular_nums(),
        _ => text(format!("{:.0} pt", height(row))).tabular_nums(),
    })
    .on_sort(|sort, _, _| println!("sort {:?}", sort.direction))
}
// docs:end table

impl Render for Rows {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(step) = self.step {
            if self.list.at_end() {
                self.passes += 1;
                println!("nd-scroll: reached_end pass={} rows={}", self.passes, self.list.len());
                self.list.scroll_to_top(cx);
            } else {
                self.list.scroll_by(step, cx);
            }
            cx.notify();
        }
        // docs:begin render
        list("rows", &self.list, |row: u32, _, cx| {
            h_flex()
                .px(px(12.))
                .py(px(4.))
                .gap_2()
                .child(div().w(px(6.)).h(px(height(row) - 8.)).bg(cx.theme().accent))
                .child(text(format!("Row {row}")).tabular_nums())
        })
        .size_full()
        // docs:end render
    }
}

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

pub fn open(cx: &mut App, rows: u32, step: Option<f64>) -> WindowHandle<Rows> {
    cx.open_window(WindowOptions::main("scroll").title("Scroll").size(size(px(480.), px(720.))), move |_, cx| {
        cx.new(|cx| Rows::new(rows, step, cx))
    })
}
