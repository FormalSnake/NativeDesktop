//! Tier 2: the forms family on the real AppKit backend. Each control mounts as AppKit's own
//! class, its props reach the native object, a native change comes back as the control's
//! event, feeding that event's value back sets nothing (echo), and props set from inside the
//! control's own handler neither loop nor emit twice (reentrancy). Automation's `setValue`
//! acts through the native control.
//! Prints `ND_FORMS_MAPPING_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("gate_m6::forms_mapping", || forms::run());
    #[cfg(not(target_os = "macos"))]
    harness::main("gate_m6::forms_mapping", || {});
}

#[cfg(target_os = "macos")]
#[path = "forms"]
mod forms {
    mod pickers;
    mod rig;
    mod text;
    mod toggle;

    pub use rig::{Checks, Rig};

    pub fn run() -> ! {
        let rig = Rig::new();
        let mut c = Checks(0);
        text::run(&rig, &mut c);
        toggle::run(&rig, &mut c);
        pickers::run(&rig, &mut c);
        if c.0 == 0 {
            println!("ND_FORMS_MAPPING_OK");
            std::process::exit(0);
        }
        println!("ND_FORMS_MAPPING_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }
}
