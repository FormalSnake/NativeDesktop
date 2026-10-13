fn main() -> nd::Result<()> {
    let rows = std::env::var("ND_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000);
    let step = std::env::var("ND_SCROLL_STEP").ok().and_then(|v| v.parse().ok());
    scroll_bench::app().run(move |cx| {
        scroll_bench::open(cx, rows, step);
    })
}
