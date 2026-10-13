//! Per-user install of the agent: the binary copied to `%LOCALAPPDATA%\nd\agent\ndshell.exe` (so
//! a rebuild never fights a running exe for its file), a token, and a Task Scheduler task that
//! runs it in the interactive session at logon. `/IT` is what puts it on the user's desktop when
//! started from an ssh session, which has none.

use std::process::Command;

const TASK: &str = "nd-agent";

fn schtasks(args: &[&str]) -> Result<String, String> {
    let out = Command::new("schtasks").args(args).output().map_err(|e| format!("schtasks: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() { Ok(text) } else { Err(format!("schtasks {}: {}", args.join(" "), text.trim())) }
}

/// Installs or updates the agent and (re)starts it in the desktop session.
pub fn install() -> Result<String, String> {
    let home = super::home();
    std::fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    let token = super::token_path();
    if !token.exists() {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| format!("random token: {e}"))?;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        std::fs::write(&token, hex).map_err(|e| format!("{}: {e}", token.display()))?;
    }
    if stop().is_ok() {
        // The exe is locked until the ended process is gone.
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let exe = home.join("ndshell.exe");
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    if current != exe {
        std::fs::copy(&current, &exe).map_err(|e| format!("copy to {}: {e}", exe.display()))?;
    }
    let action = format!("\"{}\" agent run", exe.display());
    schtasks(&["/Create", "/F", "/TN", TASK, "/SC", "ONLOGON", "/IT", "/RL", "LIMITED", "/TR", &action])?;
    schtasks(&["/Run", "/TN", TASK])?;
    Ok(format!("installed {} as task `{TASK}`, token at {}", exe.display(), token.display()))
}

/// Ends the running agent, if any; the task stays for the next logon.
pub fn stop() -> Result<(), String> {
    schtasks(&["/End", "/TN", TASK]).map(|_| ())
}

pub fn uninstall() -> Result<(), String> {
    let _ = stop();
    schtasks(&["/Delete", "/F", "/TN", TASK]).map(|_| ())
}
