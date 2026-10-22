//! `nd-fspec-gen --schema settings.schema.json --name <app> --repo <git url> --out fspec.star
//! [--display-name <name>] [--package <cargo package>] [--build-dep <pkg>]... [--check]`
//!
//! Writes the fspec module for an app's settings schema, the counterpart of `nd-hm-gen`. With
//! `--check` nothing is written; the exit code says whether the file on disk is current.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let (mut schema, mut out, mut name, mut repo) = (None, None, None, None);
    let (mut display, mut package, mut deps, mut check) = (None, None, Vec::new(), false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--schema" => schema = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--name" => name = args.next(),
            "--repo" => repo = args.next(),
            "--display-name" => display = args.next(),
            "--package" => package = args.next(),
            "--build-dep" => deps.extend(args.next()),
            "--check" => check = true,
            _ => return usage(),
        }
    }
    let (Some(schema), Some(out), Some(name), Some(repo)) = (schema, out, name, repo) else { return usage() };
    let text = match std::fs::read_to_string(&schema) {
        Ok(text) => text,
        Err(e) => return fail(format!("{}: {e}", schema.display())),
    };
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(json) => json,
        Err(e) => return fail(format!("{}: {e}", schema.display())),
    };
    let mut app = nd_settings::fspec::App::new(&name, &repo);
    if let Some(display) = &display {
        app = app.display_name(display);
    }
    if let Some(package) = &package {
        app = app.package(package);
    }
    let deps: Vec<&str> = deps.iter().map(String::as_str).collect();
    let module = nd_settings::fspec::star_module(&app.build_deps(&deps), &json);
    if check {
        if std::fs::read_to_string(&out).is_ok_and(|current| current == module) {
            return ExitCode::SUCCESS;
        }
        return fail(format!("{} is out of date, run nd-fspec-gen without --check", out.display()));
    }
    match std::fs::write(&out, module) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(format!("{}: {e}", out.display())),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: nd-fspec-gen --schema <settings.schema.json> --name <app> --repo <git url> --out <fspec.star> \
         [--display-name <name>] [--package <cargo package>] [--build-dep <pkg>]... [--check]"
    );
    ExitCode::from(2)
}

fn fail(message: String) -> ExitCode {
    eprintln!("{message}");
    ExitCode::FAILURE
}
