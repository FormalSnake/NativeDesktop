fn main() -> nd::Result<()> {
    terminal::app().run(|cx| {
        terminal::open(cx);
    })
}
