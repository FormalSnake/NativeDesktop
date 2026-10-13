use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = nd_cli::Cli::parse();
    match nd_cli::run(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("nd: {error:#}");
            ExitCode::FAILURE
        }
    }
}
