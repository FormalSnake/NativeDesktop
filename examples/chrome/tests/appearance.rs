fn main() {
    nd::audit::Audit::new("chrome", chrome::app)
        .window("mail", |cx| chrome::init(cx).id())
        .window("settings", |cx| chrome::open_settings_window(cx).expect("settings window").id())
        .run()
}
