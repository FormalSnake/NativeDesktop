//! The `nd` command line. M1 ships `new`, `check`, `test` and `diagnose`; `dev`, `package` and `release`
//! arrive in M10.

pub mod check;
pub mod diagnose;
pub mod lints;
pub mod new;
pub mod testcmd;

use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "nd", version, about = "NativeDesktop command line")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a new app.
    New(new::NewArgs),
    /// fmt, clippy with the framework lint table, framework lints.
    Check(check::CheckArgs),
    /// Run the test tiers through nextest.
    Test(testcmd::TestArgs),
    /// Summarise the app's incident bundles: panics, crashes, hangs, reports.
    Diagnose(diagnose::DiagnoseArgs),
}

pub fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::New(args) => new::run(args).map(|()| ExitCode::SUCCESS),
        Command::Check(args) => check::run(args),
        Command::Test(args) => testcmd::run(args),
        Command::Diagnose(args) => diagnose::run(args),
    }
}
