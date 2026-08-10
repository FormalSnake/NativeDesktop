import Foundation

// Packaged-app launch contract: `nd package` writes Resources/app/nd-app.json
// ({ id, name, version, entry, cwd, pluginPaths }, entry/cwd app-root-relative)
// and this bootstrap turns a bare double-click launch into the same
// environment `nd dev` sets up by hand: ND_SCRIPT pointing at the bundled
// entry, the bundled `bun` first on PATH, the app's own directory as cwd (so
// getAppDataDir() and relative fs reads behave the same packaged as in dev),
// and ND_PLUGINS/ND_PLUGIN_PATHS for bundled native plugins.
//
// Runs before the ACL/plugin block and nd_start_runtime in main.swift: the
// plugin block reads ND_PLUGINS, and the core snapshots the environment at
// nd_start_runtime time.
enum NDBundleBootstrap {
    static func apply() {
        guard let resources = Bundle.main.resourceURL else { return }
        let appRoot = resources.appendingPathComponent("app")
        let manifestURL = appRoot.appendingPathComponent("nd-app.json")
        guard FileManager.default.fileExists(atPath: manifestURL.path) else { return }
        // An explicit ND_SCRIPT (dev override, gate scripts) wins wholesale:
        // the caller's script is relative to the caller's cwd, so the bundled
        // entry, chdir, and PATH prepend must all stand down together.
        guard getenv("ND_SCRIPT") == nil else { return }
        guard let data = try? Data(contentsOf: manifestURL),
              let manifest = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let entry = manifest["entry"] as? String
        else {
            FileHandle.standardError.write("ND_BUNDLE_BOOTSTRAP_ERROR unreadable nd-app.json\n".data(using: .utf8)!)
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
