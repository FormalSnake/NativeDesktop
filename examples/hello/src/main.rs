fn main() -> nd::Result<()> {
    hello::app().run(|cx| {
        hello::open(cx);
    })
}
