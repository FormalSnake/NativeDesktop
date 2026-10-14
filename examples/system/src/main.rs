fn main() -> nd::Result<()> {
    system_demo::app().run(|cx| {
        system_demo::open(cx, nd::system::system());
    })
}
