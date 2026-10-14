fn main() -> nd::Result<()> {
    pacing::app().run(|cx| {
        for name in ["pacing-a", "pacing-b"] {
            pacing::open(cx, name);
        }
    })
}
