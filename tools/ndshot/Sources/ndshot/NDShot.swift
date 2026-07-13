import Foundation

let usageText = """
    Usage: ndshot <command> [options]

    Commands:
      doctor
          Report Screen Recording permission state for this binary.
          Exit 0 if granted, 2 if not (grant instructions on stderr).

      list
          Enumerate capturable windows, one JSON object per line:
          {"pid":…, "windowID":…, "app":"…", "title":"…", "x":…, "y":…,
           "width":…, "height":…, "onScreen":…}
          Exit 2 if Screen Recording access is missing.

      capture --out <path.png> [--pid <pid>] [--title <substring>] [--window-id <id>]
          Capture the first matching window to a PNG file. --window-id wins
          outright; --pid and --title compose (both must match if both are
          given); --title is a case-insensitive substring match.
          Exit codes: 0 success, 2 no permission, 3 no matching window,
          4 capture/write failure.

    Examples:
      ndshot doctor
      ndshot list
      ndshot capture --title "ND Notes" --out /tmp/nd.png
    """

@main
struct NDShot {
    static func main() async {
        respawnDisclaimedIfNeeded()
        let arguments = Array(CommandLine.arguments.dropFirst())

        guard let command = arguments.first else {
            eprint(usageText)
            exit(64)
        }
        let rest = Array(arguments.dropFirst())

        let code: Int32
        switch command {
        case "doctor":
            code = await cmdDoctor()
        case "list":
            code = await cmdList()
        case "capture":
            code = await cmdCapture(rest)
        case "help", "-h", "--help":
            print(usageText)
            code = 0
        default:
            eprint("ndshot: unknown command '\(command)'\n")
            eprint(usageText)
            code = 64
        }
        exit(code)
    }
}
