fn main() {
    nd::audit::Audit::new("pacing", pacing::app).window("clock", |cx| pacing::open(cx, "pacing-a").id()).run()
}
