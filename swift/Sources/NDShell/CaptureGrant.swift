import Foundation
import Security

// `NDShell --nd-grant`: writes this binary's Screen Recording row into the
// system TCC database, so the capture helper (AutomationCapture.swift) works
// without a trip through System Settings. That file is SIP-protected, so this
// only works with SIP disabled, and it needs root (sudo prompts on the
// terminal). The row's requirement is the signing identifier rather than the
// cdhash Settings would store for an ad hoc binary, so it survives rebuilds.
// Peer of tools/ndshot's Grant.swift.

private let ndTCCDatabase = "/Library/Application Support/com.apple.TCC/TCC.db"

private func ndRun(_ tool: String, _ args: [String], capture: Bool = false) -> (Int32, String) {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: tool)
    process.arguments = args
    let pipe = Pipe()
    if capture {
        process.standardOutput = pipe
        process.standardError = Pipe()
    }
    guard (try? process.run()) != nil else { return (-1, "") }
    let data = capture ? pipe.fileHandleForReading.readDataToEndOfFile() : Data()
    process.waitUntilExit()
    return (process.terminationStatus, String(data: data, encoding: .utf8) ?? "")
}

private func ndSigningIdentifier(_ path: String) -> String? {
    var code: SecStaticCode?
    guard SecStaticCodeCreateWithPath(URL(fileURLWithPath: path) as CFURL, [], &code) == errSecSuccess,
          let code
    else { return nil }
    var info: CFDictionary?
    guard SecCodeCopySigningInformation(code, [], &info) == errSecSuccess,
          let dict = info as? [String: Any]
    else { return nil }
    return dict[kSecCodeInfoIdentifier as String] as? String
}

private func ndRequirementHex(_ text: String) -> String? {
    var requirement: SecRequirement?
    guard SecRequirementCreateWithString(text as CFString, [], &requirement) == errSecSuccess,
          let requirement
    else { return nil }
    var data: CFData?
    guard SecRequirementCopyData(requirement, [], &data) == errSecSuccess, let data else { return nil }
    return (data as Data).map { String(format: "%02x", $0) }.joined()
}

func ndCaptureGrantMain() -> Int32 {
    func fail(_ message: String, _ code: Int32) -> Int32 {
        FileHandle.standardError.write("NDShell: \(message)\n".data(using: .utf8)!)
        return code
    }
    guard ndRun("/usr/bin/csrutil", ["status"], capture: true).1.contains("status: disabled") else {
        return fail("--nd-grant needs SIP disabled (csrutil status); grant Screen Recording in System Settings instead", 2)
    }
    // TCC matches the real path, and SwiftPM's .build/release is a symlink.
    guard let path = Bundle.main.executableURL?.resolvingSymlinksInPath().path,
          let identifier = ndSigningIdentifier(path)
    else {
        return fail("this binary has no code signature to key the grant to", 4)
    }
    guard let csreq = ndRequirementHex("identifier \"\(identifier)\"") else {
        return fail("could not compile the code requirement", 4)
    }
    let client = path.replacingOccurrences(of: "'", with: "''")
    let now = Int(Date().timeIntervalSince1970)
    let sql = """
        INSERT OR REPLACE INTO access
          (service, client, client_type, auth_value, auth_reason, auth_version, csreq,
           indirect_object_identifier, flags, last_modified, last_reminded)
        VALUES ('kTCCServiceScreenCapture', '\(client)', 1, 2, 4, 1, X'\(csreq)',
           'UNUSED', 0, \(now), \(now));
        """
    let (status, _) = ndRun("/usr/bin/sudo", ["/usr/bin/sqlite3", ndTCCDatabase, sql])
    guard status == 0 else { return fail("writing \(ndTCCDatabase) failed (exit \(status))", 4) }
    print("NDShell: Screen Recording granted to \(path) (identifier \"\(identifier)\")")
    return 0
}
