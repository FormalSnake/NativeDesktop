import Darwin
import Foundation

// TCC attributes a CLI's permission checks to its RESPONSIBLE PROCESS — the
// terminal app that (transitively) spawned it — not to the binary itself.
// That is why a manual System Settings grant for this binary is ignored when
// ndshot runs from a terminal, and why the permission prompt names the
// terminal instead of ndshot. The escape hatch is the libSystem call
// responsibility_spawnattrs_setdisclaim(): a process spawned with it set
// becomes its OWN responsible process. ndshot re-spawns itself exactly once
// with the disclaim attribute (env marker breaks the recursion); stdio is
// inherited and the parent forwards the child's exit code, so the CLI
// contract is unchanged.

private typealias SetDisclaimFn = @convention(c) (
    UnsafeMutablePointer<posix_spawnattr_t?>, Int32
) -> Int32

private let disclaimMarker = "NDSHOT_DISCLAIMED"

/// Re-spawns this exact invocation with the responsibility disclaim set and
/// exits with the child's status. Returns normally (without re-spawning) when
/// already disclaimed, or when the private symbol/spawn is unavailable — in
/// that degraded case the process keeps the terminal's TCC identity, which
/// `doctor` surfaces.
func respawnDisclaimedIfNeeded() {
    if ProcessInfo.processInfo.environment[disclaimMarker] == "1" { return }
    guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "responsibility_spawnattrs_setdisclaim") else {
        return // RTLD_DEFAULT lookup failed; run undisclaimed
    }
    let setDisclaim = unsafeBitCast(sym, to: SetDisclaimFn.self)

    var attr: posix_spawnattr_t?
    guard posix_spawnattr_init(&attr) == 0 else { return }
    defer { posix_spawnattr_destroy(&attr) }
    guard setDisclaim(&attr, 1) == 0 else { return }

    var argv: [UnsafeMutablePointer<CChar>?] = CommandLine.arguments.map { strdup($0) }
    argv.append(nil)
    var env = ProcessInfo.processInfo.environment
    env[disclaimMarker] = "1"
    var envp: [UnsafeMutablePointer<CChar>?] = env.map { strdup("\($0.key)=\($0.value)") }
    envp.append(nil)

    var pid: pid_t = 0
    guard posix_spawn(&pid, executablePath(), nil, &attr, argv, envp) == 0 else {
        return // spawn failed; run undisclaimed
    }

    var status: Int32 = 0
    while waitpid(pid, &status, 0) == -1 && errno == EINTR {}
    let exited = (status & 0x7f) == 0
    exit(exited ? (status >> 8) & 0xff : 1)
}

/// True when this process runs as its own responsible process (post-respawn).
func isDisclaimed() -> Bool {
    ProcessInfo.processInfo.environment[disclaimMarker] == "1"
}
