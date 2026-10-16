//! The `nd` command line: `new`, `check`, `test`, `build`, `serve`, `diagnose`, `package`, `release`,
//! `diag`, `docs`, `doctor`.

pub mod check;
pub mod diag;
pub mod diagnose;
pub mod docs;
pub mod doctor;
pub mod lints;
pub mod new;
pub mod package;
pub mod project;
pub mod publish;
pub mod release;
pub mod testcmd;
pub mod vpk;
pub mod web;

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
    /// Build the app: a native release build, or with `--target web` the static web bundle.
    Build(web::BuildArgs),
    /// Build and serve the web bundle locally: `nd serve --target web`.
    Serve(web::ServeArgs),
    /// Build installers: macOS .app, .pkg and .dmg; Linux AppImage; Windows Setup.exe.
    Package(package::PackageArgs),
    /// Pack, sign and publish an update feed; `serve` and `keygen` subcommands.
    Release(release::ReleaseArgs),
    /// Measure framework budgets: `nd diag scroll --rows 100000`.
    Diag(diag::DiagArgs),
    /// The docs: `nd docs check`, `build`, `serve` and `capture`.
    Docs(docs::DocsArgs),
    /// Checks that what the app needs at run time is in place: its web engine.
    Doctor(doctor::DoctorArgs),
}

pub fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::New(args) => new::run(args).map(|()| ExitCode::SUCCESS),
        Command::Check(args) => check::run(args),
        Command::Test(args) => testcmd::run(args),
        Command::Diagnose(args) => diagnose::run(args),
        Command::Build(args) => web::run_build(args),
        Command::Serve(args) => web::run_serve(args),
        Command::Package(args) => package::run(args).map(|()| ExitCode::SUCCESS),
        Command::Release(args) => release::run(args).map(|()| ExitCode::SUCCESS),
        Command::Diag(args) => diag::run(args),
        Command::Docs(args) => docs::run(args),
        Command::Doctor(args) => doctor::run(args),
    }
}
