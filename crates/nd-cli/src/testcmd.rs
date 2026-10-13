//! `nd test`: the nextest tiers of ARCHITECTURE.md 10.1 behind one command.

use std::process::{Command, ExitCode};

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum Tier {
    /// Tiers 1 and 2: logic on `TestBackend` and in-process backends. Parallel, no window server.
    Fast,
    /// Tier 3: real windows, one at a time. Needs a window server.
    Real,
    /// Fast, then real.
    All,
}

#[derive(Args, Debug)]
pub struct TestArgs {
    #[arg(long, value_enum, default_value = "fast")]
    pub tier: Tier,
    /// Run only these packages.
    #[arg(short, long = "package")]
    pub packages: Vec<String>,
    /// Test name filters, as nextest takes them.
    pub filters: Vec<String>,
}

fn profile(tier: Tier) -> &'static str {
    match tier {
        Tier::Fast => "fast",
        Tier::Real => "real",
        Tier::All => unreachable!("expanded by the caller"),
    }
}

pub fn nextest_args(args: &TestArgs, tier: Tier) -> Vec<String> {
    let mut out =
        vec!["nextest".into(), "run".into(), "--profile".into(), profile(tier).into(), "--no-tests=pass".into()];
    for package in &args.packages {
        out.push("-p".into());
        out.push(package.clone());
    }
    out.extend(args.filters.iter().cloned());
    out
}

pub fn run(args: TestArgs) -> Result<ExitCode> {
    let installed = Command::new("cargo").args(["nextest", "--version"]).output().is_ok_and(|o| o.status.success());
    if !installed {
        bail!("cargo-nextest is not installed (`cargo install cargo-nextest --locked`, or enter the nix dev shell)");
    }
    let tiers: &[Tier] = if args.tier == Tier::All { &[Tier::Fast, Tier::Real] } else { &[args.tier] };
    for &tier in tiers {
        let status = Command::new("cargo").args(nextest_args(&args, tier)).status()?;
        if !status.success() {
            return Ok(ExitCode::FAILURE);
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tiers_to_profiles_and_passes_filters() {
        let args = TestArgs { tier: Tier::Fast, packages: vec!["hello".into()], filters: vec!["clicks".into()] };
        assert_eq!(
            nextest_args(&args, Tier::Real),
            ["nextest", "run", "--profile", "real", "--no-tests=pass", "-p", "hello", "clicks"]
        );
    }
}
