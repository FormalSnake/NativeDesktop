//! `<App> Helper*.app/Contents/MacOS/<App> Helper*`: every Chromium subprocess of a packaged
//! app (renderer, GPU, network, plugin, alerts).

fn main() {
    std::process::exit(nd_cef::run_helper());
}
