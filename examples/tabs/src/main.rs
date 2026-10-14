fn main() -> nd::Result<()> {
    tabs::app().run(|cx| {
        tabs::init(cx);
    })
}
