//! `nd-hm-gen --schema settings.schema.json --out nix/settings-options.nix [--check]`
//!
//! Writes the typed home-manager options for an app's settings schema. With `--check` nothing is
//! written; the exit code says whether the file on disk is current.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut schema = None;
    let mut out = None;
    let mut check = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--schema" => schema = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--check" => check = true,
            _ => return usage(),
        }
    }
    let (Some(schema), Some(out)) = (schema, out) else { return usage() };
    let text = match std::fs::read_to_string(&schema) {
        Ok(text) => text,
        Err(e) => return fail(format!("{}: {e}", schema.display())),
    };
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(json) => json,
        Err(e) => return fail(format!("{}: {e}", schema.display())),
    };
    let module = nd_settings::nix::options_module(&json);
    if check {
        if std::fs::read_to_string(&out).is_ok_and(|current| current == module) {
            return ExitCode::SUCCESS;
        }
        return fail(format!("{} is out of date, run nd-hm-gen without --check", out.display()));
    }
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&out, module) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(format!("{}: {e}", out.display())),
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: nd-hm-gen --schema <settings.schema.json> --out <settings-options.nix> [--check]");
    ExitCode::from(2)
}

fn fail(message: String) -> ExitCode {
    eprintln!("{message}");
    ExitCode::FAILURE
}
