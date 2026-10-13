use nd::theme::Appearance;

fn main() {
    nd::audit::Audit::new("overrides", overrides::app)
        .allow(overrides::theme_colors())
        .window("light", |cx| overrides::open(cx, Appearance::Light).id())
        .window("dark", |cx| overrides::open(cx, Appearance::Dark).id())
        .run()
}
