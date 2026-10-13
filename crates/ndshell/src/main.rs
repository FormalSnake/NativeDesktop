//! `ndshell grant <binary>` and `ndshell preflight`; see the library modules.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = run(&args);
    if let Err(message) = result {
        eprintln!("ndshell: {message}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "macos")]
fn run(args: &[String]) -> Result<(), String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["grant", binary] => {
            let (path, identifier) = ndshell::grant::grant(std::path::Path::new(binary))?;
            println!(
                "ndshell: Screen Recording, Accessibility and PostEvent granted to {path} (identifier \"{identifier}\")"
            );
            Ok(())
        }
        ["preflight"] => {
            ndshell::preflight::real_input()?;
            println!("ndshell: ready for real input");
            Ok(())
        }
        _ => Err("usage: ndshell grant <binary> | ndshell preflight".into()),
    }
}

#[cfg(not(target_os = "macos"))]
fn run(_args: &[String]) -> Result<(), String> {
    Err("ndshell has no helpers for this OS yet".into())
}
