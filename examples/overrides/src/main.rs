fn main() -> nd::Result<()> {
    // The main window is always dark: a window-level override over the system appearance.
    overrides::app().run(|cx| overrides::open(cx, nd::theme::Appearance::Dark))
}
