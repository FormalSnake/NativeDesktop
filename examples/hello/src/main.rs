use hello::Counter;
use nd::prelude::*;

fn main() -> nd::Result<()> {
    Application::new(nd::manifest!()).run(|cx| {
        cx.open_window(WindowOptions::main("main").title("Hello"), |_window, cx| cx.new(|_cx| Counter::default()))
    })
}
