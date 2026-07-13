import CoreGraphics
import Foundation
import ScreenCaptureKit

/// Grant instructions printed to stderr any time a command needs Screen
/// Recording access and doesn't have it (doctor, list, capture).
func doctorInstructions() -> String {
    let path = executablePath()
    return """
        ndshot: Screen Recording access is NOT granted for this binary.

          binary: \(path)

        The first run of any command that touches ScreenCaptureKit calls
        CGRequestScreenCaptureAccess(), which shows the system permission prompt
        once per binary identity. If it didn't appear (or you dismissed it):

          1. Open System Settings -> Privacy & Security -> Screen Recording
             (named "Screen & System Audio Recording" on newer macOS).
          2. Click +, press Cmd-Shift-G in the picker, paste the directory of
             the binary path above, select ndshot, and enable its toggle.
          3. Re-run the command -- no restart needed on macOS 14+.

        The grant is tied to this binary's on-disk path and code signature, not
        to the name "ndshot". Rebuilding via tools/ndshot/build.sh re-signs the
        binary ad hoc under the stable identifier com.nativedesktop.ndshot, so
        the grant persists across rebuilds as long as the path is unchanged --
        but an ad hoc signature is derived from the binary's content hash, so a
        rebuild that actually changes the compiled bytes counts as a new
        identity to TCC and needs re-granting. Run `ndshot doctor` again after
        granting (or after a rebuild) to confirm the current state.
        """
}

/// Pulls just the identity-relevant lines out of `codesign -dvvv` so doctor
/// can show whether this binary's signature looks like the stable ad hoc
/// identity build.sh produces, which is what "detects that [a rebuild
/// invalidated the grant]" means in practice: show the identity, let the
/// operator compare it to what they last granted.
func codesignSummary(path: String) -> String {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/codesign")
    process.arguments = ["-dvvv", path]
    let errorPipe = Pipe()
    process.standardOutput = Pipe()
    process.standardError = errorPipe
    do {
        try process.run()
    } catch {
        return "  (codesign unavailable: \(error.localizedDescription))"
    }
    let data = errorPipe.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    let output = String(data: data, encoding: .utf8) ?? ""
    let interesting = output.split(separator: "\n").filter { line in
        line.hasPrefix("Executable=") || line.hasPrefix("Identifier=")
            || line.hasPrefix("Format=") || line.hasPrefix("CDHash=")
            || line.contains("adhoc") || line.hasPrefix("Signature=")
    }
    if interesting.isEmpty {
        return "  (no code signature found -- run tools/ndshot/build.sh to ad-hoc sign)"
    }
    return interesting.map { "  \($0)" }.joined(separator: "\n")
}

/// Ground truth for Screen Recording access: attempt a real ScreenCaptureKit
/// content enumeration. The legacy CGPreflightScreenCaptureAccess() returns a
/// stale `false` for CLI binaries on macOS 15+ (approvals moved to replayd's
/// ScreenCaptureApprovals store, which preflight doesn't consult) -- gating on
/// it rejects perfectly valid grants. Only an actual SCShareableContent call
/// answers truthfully.
func probeAccess() async -> Bool {
    do {
        _ = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        return true
    } catch {
        return false
    }
}

/// Returns true (and does nothing else) if Screen Recording access is
/// already granted. Otherwise triggers the system prompt for next time,
/// prints the grant instructions, and returns false -- callers exit 2.
func ensurePermission() async -> Bool {
    if await probeAccess() {
        return true
    }
    _ = CGRequestScreenCaptureAccess()
    eprint(doctorInstructions())
    return false
}

func cmdDoctor() async -> Int32 {
    let path = executablePath()
    print("ndshot doctor")
    print("  binary: \(path)")
    print(codesignSummary(path: path))

    if await probeAccess() {
        print("  Screen Recording: GRANTED (verified via ScreenCaptureKit)")
        if !CGPreflightScreenCaptureAccess() {
            print("  note: legacy CGPreflight reports false -- expected for CLI tools on macOS 15+, ignore it")
        }
        return 0
    }

    print("  Screen Recording: NOT GRANTED")
    _ = CGRequestScreenCaptureAccess()
    eprint(doctorInstructions())
    return 2
}
