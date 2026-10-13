//! `ndshell grant <binary>`: writes Screen Recording, Accessibility and PostEvent rows for a binary
//! into the system TCC database. The file is SIP-protected, so this needs SIP disabled and root
//! (sudo prompts on the terminal). Rows are keyed to the signing identifier rather than the cdhash
//! System Settings stores for an ad hoc binary, so they survive rebuilds.
//!
//! TCC checks the responsible process: for a test launched from a shell that is the terminal (or
//! `sshd-keygen-wrapper` over ssh), not the test binary, so grant that.

use std::path::Path;
use std::process::Command;

const TCC_DB: &str = "/Library/Application Support/com.apple.TCC/TCC.db";
const SERVICES: [&str; 3] = ["kTCCServiceScreenCapture", "kTCCServiceAccessibility", "kTCCServicePostEvent"];

fn output(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))
}

fn signing_identifier(path: &str) -> Result<String, String> {
    let out = output("/usr/bin/codesign", &["-dv", path])?;
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .find_map(|l| l.strip_prefix("Identifier=").map(str::to_owned))
        .ok_or_else(|| format!("{path} has no code signature to key the grant to"))
}

fn requirement_hex(identifier: &str) -> Result<String, String> {
    let text = format!("identifier \"{identifier}\"");
    let out = output("/usr/bin/csreq", &["-r", &text, "-b", "/dev/stdout"])?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(format!("csreq could not compile `{text}`"));
    }
    Ok(out.stdout.iter().map(|b| format!("{b:02x}")).collect())
}

/// Grants `binary`; returns the resolved path and identifier it keyed the rows to.
pub fn grant(binary: &Path) -> Result<(String, String), String> {
    let status = output("/usr/bin/csrutil", &["status"])?;
    if !String::from_utf8_lossy(&status.stdout).contains("disabled") {
        return Err("grant needs SIP disabled (csrutil status); use System Settings instead".into());
    }
    // TCC matches the real path, not a symlink to it.
    let path = binary.canonicalize().map_err(|e| format!("{}: {e}", binary.display()))?;
    let path = path.to_str().ok_or("path is not UTF-8")?.to_owned();
    let identifier = signing_identifier(&path)?;
    let csreq = requirement_hex(&identifier)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let client = path.replace('\'', "''");
    let sql: String = SERVICES
        .iter()
        .map(|service| {
            format!(
                "INSERT OR REPLACE INTO access (service, client, client_type, auth_value, auth_reason, auth_version, \
                 csreq, indirect_object_identifier, flags, last_modified, last_reminded) VALUES ('{service}', \
                 '{client}', 1, 2, 4, 1, X'{csreq}', 'UNUSED', 0, {now}, {now});\n"
            )
        })
        .collect();
    let status = Command::new("/usr/bin/sudo")
        .args(["/usr/bin/sqlite3", TCC_DB, &sql])
        .status()
        .map_err(|e| format!("sudo: {e}"))?;
    if !status.success() {
        return Err(format!("writing {TCC_DB} failed ({status})"));
    }
    Ok((path, identifier))
}
