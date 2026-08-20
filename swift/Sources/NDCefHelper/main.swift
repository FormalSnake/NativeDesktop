import CCef
import Foundation

// The executable behind all five macOS helper bundles (renderer, GPU, plugin,
// alerts, and the unsuffixed default). Chromium picks the bundle by appending
// its own suffix to the browser_subprocess_path name, so one binary copied
// under five names is the whole process model here.
//
// Nothing but cef_execute_process may run before it returns: this process is
// already a Chromium child, and anything that touches AppKit or spawns threads
// first is what turns a renderer into a second app.

/// The framework sits beside this helper's bundle, three levels up from the
/// helper executable (MacOS -> Contents -> "<name> Helper.app" -> Frameworks),
/// which is the layout CEF's own loader assumes.
private func frameworkBinaryPath() -> String {
    let executable = URL(fileURLWithPath: CommandLine.arguments[0]).resolvingSymlinksInPath()
    let frameworks = executable
        .deletingLastPathComponent()  // MacOS
        .deletingLastPathComponent()  // Contents
        .deletingLastPathComponent()  // <name> Helper.app
        .deletingLastPathComponent()  // Frameworks
    return frameworks
        .appendingPathComponent("Chromium Embedded Framework.framework")
        .appendingPathComponent("Chromium Embedded Framework")
        .path
}

let frameworkPath = frameworkBinaryPath()
guard nd_cef_load(frameworkPath) != 0 else {
    let reason = nd_cef_load_error().map { String(cString: $0) } ?? "unknown"
    FileHandle.standardError.write("ND_CEF_HELPER failed to load \(frameworkPath): \(reason)\n".data(using: .utf8)!)
    exit(1)
}

// The first CEF call after loading, in every process: the library reads the
// client's API version from it, and without it any struct this process hands
// over is rejected as version -1.
_ = nd_cef_api_hash(nd_cef_compiled_api_version(), 0)

// The helper carries the same app as the host: custom schemes have to be
// registered identically in every process or the renderer treats them as
// unknown.
let app = nd_cef_app_create(0)
var mainArgs = cef_main_args_t(argc: CommandLine.argc, argv: CommandLine.unsafeArgv)
exit(nd_cef_execute_process(&mainArgs, app, nil))
