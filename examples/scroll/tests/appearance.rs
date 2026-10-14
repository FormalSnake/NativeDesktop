fn main() {
    nd::audit::Audit::new("scroll-bench", scroll_bench::app)
        .window("rows", |cx| scroll_bench::open(cx, 500, None).id())
        .run()
}
