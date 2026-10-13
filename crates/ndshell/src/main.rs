//! `ndshell grant <binary>`, `ndshell preflight`, `ndshell lock` and `ndshell agent`; see the
//! library modules.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("lock") => lock(&args[1..]),
        Some("agent") => agent(&args[1..]),
        _ => run(&args),
    };
    if let Err(message) = result {
        eprintln!("ndshell: {message}");
        std::process::exit(1);
    }
}

const USAGE: &str = "usage: ndshell grant <binary> | ndshell preflight | ndshell lock [status | -- <command>...] \
                     | ndshell agent [install | uninstall | stop | run | ping | call <json>]";

/// `ping` and `call` talk to an agent (on this machine, or forwarded with `ssh -L`); the rest
/// manage it on Windows.
fn agent(args: &[String]) -> Result<(), String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["ping"] => {
            let reply = ndshell::agent::Client::connect()?.call(serde_json::json!({ "cmd": "ping" }))?;
            println!("{reply}");
            Ok(())
        }
        ["call", json] => {
            let request = serde_json::from_str(json).map_err(|e| format!("request: {e}"))?;
            println!("{}", ndshell::agent::Client::connect()?.call(request)?);
            Ok(())
        }
        #[cfg(windows)]
        ["install"] => ndshell::agent::install::install().map(|done| println!("ndshell: {done}")),
        #[cfg(windows)]
        ["uninstall"] => ndshell::agent::install::uninstall(),
        #[cfg(windows)]
        ["stop"] => ndshell::agent::install::stop(),
        #[cfg(windows)]
        ["run"] => ndshell::agent::server::run(),
        _ => Err(USAGE.into()),
    }
}

/// `ndshell lock status` prints the holder; `ndshell lock -- <command>` runs the command holding
/// the machine-wide real-input lock, queueing behind the current holder.
fn lock(args: &[String]) -> Result<(), String> {
    match args {
        [status] if status == "status" => {
            let holder = ndshell::lock::holder();
            println!("{}", if holder.is_empty() { "ndshell: the real-input lock is free".into() } else { holder });
            Ok(())
        }
        [dashes, program, rest @ ..] if dashes == "--" => {
            let purpose = std::iter::once(program).chain(rest).cloned().collect::<Vec<_>>().join(" ");
            let _lock = ndshell::lock::acquire(&purpose).map_err(|e| format!("lock: {e}"))?;
            let status =
                std::process::Command::new(program).args(rest).status().map_err(|e| format!("{program}: {e}"))?;
            if status.success() { Ok(()) } else { Err(format!("{program} exited with {status}")) }
        }
        _ => Err(USAGE.into()),
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
        _ => Err(USAGE.into()),
    }
}

#[cfg(not(target_os = "macos"))]
fn run(_args: &[String]) -> Result<(), String> {
    Err(format!("ndshell has only the lock and the agent on this OS; {USAGE}"))
}
