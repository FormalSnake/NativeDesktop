import Foundation

/// Writes a line to stderr. `print()` alone always goes to stdout, and this
/// CLI's contract (doctor instructions, candidate lists, error messages) is
/// specifically about keeping stdout clean for the success-path payload.
func eprint(_ message: String) {
    FileHandle.standardError.write(Data((message + "\n").utf8))
}

/// Path to the running binary, used both in doctor's grant instructions and
/// in the codesign identity summary -- the Screen Recording grant is tied to
/// this exact path/signature, not to the process name.
func executablePath() -> String {
    Bundle.main.executablePath ?? CommandLine.arguments[0]
}
