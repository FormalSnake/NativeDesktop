fn main() {
    nd::audit::Audit::new("notes", notes::app)
        .window("main", |cx| {
            notes::actions::init(cx);
            notes::demo::seed(cx);
            notes::window(cx).id()
        })
        .run()
}
