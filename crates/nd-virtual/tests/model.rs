//! The conformance suite on the model alone, behind a simulated presenter.

use nd_virtual::conformance::{ModelHarness, Setup, checks};

fn open(setup: Setup) -> ModelHarness {
    ModelHarness::open(setup)
}

#[test]
fn model_conformance() {
    for (name, check) in checks::<fn(Setup) -> ModelHarness>() {
        eprintln!("model: {name}");
        check(&(open as fn(Setup) -> ModelHarness));
    }
}
