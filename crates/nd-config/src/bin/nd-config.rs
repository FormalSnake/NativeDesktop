//! `nd-config schema` prints the JSON Schema; `nd-config check [path]` validates a file.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("schema") => {
            print!("{}", nd_config::schema_json());
            ExitCode::SUCCESS
        }
        Some("check") => {
            let path = args.next().map_or_else(|| PathBuf::from("nativedesktop.toml"), PathBuf::from);
            match nd_config::Config::load(&path) {
                Ok(_) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("usage: nd-config schema | nd-config check [path]");
            ExitCode::from(2)
        }
    }
}
