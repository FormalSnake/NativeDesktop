import Foundation

// Packaged-app launch contract: `nd package` writes Resources/app/nd-app.json
// ({ id, name, version, entry, cwd, pluginPaths, engine, schemes }, entry/cwd
// app-root-relative) and this bootstrap turns a bare double-click launch into
// the same environment `nd dev` sets up by hand: ND_SCRIPT pointing at the
// bundled entry, the bundled `bun` first on PATH, the app's own directory as
// cwd (so getAppDataDir() and relative fs reads behave the same packaged as in
// dev), ND_PLUGINS/ND_PLUGIN_PATHS for bundled native plugins, and
// ND_WEBVIEW_ENGINE/ND_CEF_SCHEMES for the webview engine.
//
// Two phases, because the engine is decided earlier than everything else.
// `applyEngine()` runs first in main.swift, ahead of NDCefRuntime.prepare() and
// therefore ahead of anything reaching NSApplication; `apply()` runs before the
// ACL/plugin block and nd_start_runtime, since the plugin block reads
// ND_PLUGINS and the core snapshots the environment at nd_start_runtime time.
enum NDBundleBootstrap {
    nonisolated(unsafe) private static var manifestCache: [String: Any]?
    nonisolated(unsafe) private static var manifestLoaded = false

    /// The bundle's nd-app.json, read once. A bare (unpackaged) run has none.
    private static func manifest() -> [String: Any]? {
        if manifestLoaded { return manifestCache }
        manifestLoaded = true
        guard let resources = Bundle.main.resourceURL else { return nil }
        let manifestURL = resources.appendingPathComponent("app/nd-app.json")
        guard FileManager.default.fileExists(atPath: manifestURL.path) else { return nil }
        guard let data = try? Data(contentsOf: manifestURL),
              let parsed = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            FileHandle.standardError.write("ND_BUNDLE_BOOTSTRAP_ERROR unreadable nd-app.json\n".data(using: .utf8)!)
            return nil
        }
        manifestCache = parsed
        return parsed
    }

    /// The engine the bundle was staged for and the schemes its processes
    /// register at startup. Unlike the paths below, these stand even when the
    /// caller supplied its own ND_SCRIPT: they describe what is inside the
    /// bundle, not where the app's code lives. An explicit ND_WEBVIEW_ENGINE or
    /// ND_CEF_SCHEMES still wins, the same precedence the config resolution in
    /// `nd dev` gives them.
    static func applyEngine() {
        guard let manifest = manifest() else { return }
        if getenv("ND_WEBVIEW_ENGINE") == nil, let engine = manifest["engine"] as? String, !engine.isEmpty {
            setenv("ND_WEBVIEW_ENGINE", engine, 1)
        }
        if getenv("ND_CEF_SCHEMES") == nil, let schemes = manifest["schemes"] as? [String], !schemes.isEmpty {
            setenv("ND_CEF_SCHEMES", schemes.joined(separator: ","), 1)
        }
    }

    static func apply() {
        guard let manifest = manifest(), let resources = Bundle.main.resourceURL else { return }
        let appRoot = resources.appendingPathComponent("app")
        // An explicit ND_SCRIPT (dev override, gate scripts) wins wholesale:
        // the caller's script is relative to the caller's cwd, so the bundled
        // entry, chdir, and PATH prepend must all stand down together.
        guard getenv("ND_SCRIPT") == nil else { return }
        guard let entry = manifest["entry"] as? String else {
            FileHandle.standardError.write("ND_BUNDLE_BOOTSTRAP_ERROR nd-app.json has no entry\n".data(using: .utf8)!)
            return
        }
        setenv("ND_SCRIPT", appRoot.appendingPathComponent(entry).path, 1)
        let macOSDir = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS").path
        if let path = getenv("PATH") {
            setenv("PATH", "\(macOSDir):\(String(cString: path))", 1)
        } else {
            setenv("PATH", macOSDir, 1)
        }
        if let cwd = manifest["cwd"] as? String {
            FileManager.default.changeCurrentDirectoryPath(appRoot.appendingPathComponent(cwd).path)
        }
        if let plugins = manifest["pluginPaths"] as? [String], !plugins.isEmpty {
            setenv("ND_PLUGINS", "1", 1)
            setenv("ND_PLUGIN_PATHS", plugins.map { appRoot.appendingPathComponent($0).path }.joined(separator: ":"), 1)
        }
    }
}
