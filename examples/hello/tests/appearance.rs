fn main() {
    nd::audit::Audit::new("hello", hello::app).window("main", |cx| hello::open(cx).id()).run()
}
