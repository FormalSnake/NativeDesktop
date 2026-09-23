import Foundation
import Security

// `doctor --grant`: writes the Screen Recording, Accessibility and PostEvent
// rows into the system TCC database directly. That file is SIP-protected, so this only works with SIP
// disabled, and it needs root (sudo prompts on the terminal if it has to).
//
// The row's csreq is `identifier "com.nativedesktop.ndshot"` rather than the
// cdhash requirement System Settings stores for an ad hoc binary, so the grant
// survives rebuilds that change the compiled bytes.

private let tccDatabase = "/Library/Application Support/com.apple.TCC/TCC.db"
private let signingIdentifier = "com.nativedesktop.ndshot"
private let grantedServices = ["kTCCServiceScreenCapture", "kTCCServiceAccessibility", "kTCCServicePostEvent"]

private func sipDisabled() -> Bool {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/csrutil")
    process.arguments = ["status"]
    let pipe = Pipe()
    process.standardOutput = pipe
    process.standardError = Pipe()
    guard (try? process.run()) != nil else { return false }
    let data = pipe.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    return (String(data: data, encoding: .utf8) ?? "").contains("status: disabled")
}

private func requirementHex(_ text: String) -> String? {
    var requirement: SecRequirement?
    guard SecRequirementCreateWithString(text as CFString, [], &requirement) == errSecSuccess,
          let requirement
    else { return nil }
    var data: CFData?
    guard SecRequirementCopyData(requirement, [], &data) == errSecSuccess, let data else { return nil }
    return (data as Data).map { String(format: "%02x", $0) }.joined()
}

func cmdGrant() async -> Int32 {
    guard sipDisabled() else {
        eprint("ndshot: --grant needs SIP disabled (csrutil status); grant through System Settings instead.")
        return 2
    }
    guard let csreq = requirementHex("identifier \"\(signingIdentifier)\"") else {
        eprint("ndshot: could not compile the code requirement")
        return 4
    }
    // TCC matches the real path, not a symlink to it.
    let path = URL(fileURLWithPath: executablePath()).resolvingSymlinksInPath().path
    let client = path.replacingOccurrences(of: "'", with: "''")
    let now = Int(Date().timeIntervalSince1970)
    // Screen Recording for capture, Accessibility for focusing the window
    // before it, and PostEvent for driving the real pointer.
    let sql = grantedServices.map { service in
        """
        INSERT OR REPLACE INTO access
          (service, client, client_type, auth_value, auth_reason, auth_version, csreq,
           indirect_object_identifier, flags, last_modified, last_reminded)
        VALUES ('\(service)', '\(client)', 1, 2, 4, 1, X'\(csreq)',
           'UNUSED', 0, \(now), \(now));
        """
    }.joined(separator: "\n")
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/sudo")
    process.arguments = ["/usr/bin/sqlite3", tccDatabase, sql]
    do {
        try process.run()
    } catch {
        eprint("ndshot: could not run sudo: \(error.localizedDescription)")
        return 4
    }
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
        eprint("ndshot: writing \(tccDatabase) failed (exit \(process.terminationStatus))")
        return 4
    }
    print("ndshot: Screen Recording, Accessibility and PostEvent granted to \(path)")
    print("  requirement: identifier \"\(signingIdentifier)\" (survives rebuilds)")
    return 0
}
