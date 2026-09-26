#if canImport(CCef)
import Foundation

/// The framework's own extension, loaded into Chrome style beside the app's.
/// Its only job is `chrome.downloads.setUiOptions({ enabled: false })`, the one
/// switch that keeps Chromium's download-started animation from being created
/// at all (no feature, command-line switch or pref in CEF 151 does, and
/// --load-component-extension is not in the release build). The files are
/// written under the root cache path at startup and the path is handed to
/// nd_cef.c, which adds it to --load-extension. The same files are
/// src/cef/framework-extension/ for the GTK engine.
enum NDCefFrameworkExtension {
    /// Fixed by the manifest's "key"; the registry commands leave it out of their lists.
    static let id = "pfbmaghgajhpjaobhbamhamgbcelckhd"

    private static let manifest = #"""
{
  "manifest_version": 3,
  "name": "NativeDesktop",
  "version": "1.0",
  "description": "Keeps Chromium's own download UI off screen; the app reports downloads.",
  "key": "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAtucR0zLECfN0dhTysVJ8oK3uLOZk8euXQu682Ijae7hjSeclMdftiYLwL6gu8MXTQpHSdgDcIoZQG2T/HKy5jkxo4zR5gvQSyXN3iyRqfKVN4565+1MC53A+qZDT/URSUVckufDPN8ScUy4HFG2QEf5orM2Nx4I4rvw0/9gv/tWobGedEuA4w792CUhuCZKqlKplQSmD/29EjqiK+KPC/PcSiBFqznVue2XXpUefKTGpw8fKzAzLWXEKkm/nQ7DsGdPjqFcofHXPWNk/ebHrGbek1M2i7b2jhaiOKWS20BxkKqY1HU7bT2EL1utWMLG1vNsAS/zvfPDVBytjMAVs9QIDAQAB",
  "permissions": ["downloads", "downloads.ui"],
  "background": { "service_worker": "background.js" }
}
"""#

    private static let background = #"""
// Chromium's download bubble and download-started animation are drawn against
// a toolbar the embedding does not have, and no switch or pref turns the
// animation off. setUiOptions does, for as long as this extension is loaded.
const off = () => chrome.downloads.setUiOptions({ enabled: false }).catch(() => {});
off();
chrome.runtime.onStartup.addListener(off);
chrome.runtime.onInstalled.addListener(off);
"""#

    /// Writes the extension under `rootCache` and exports its directory for
    /// the command-line hook. A failure leaves Chromium's UI to the sweep.
    static func install(rootCache: String) {
        let dir = (rootCache as NSString).appendingPathComponent("nd-framework-extension")
        do {
            try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
            try manifest.write(toFile: (dir as NSString).appendingPathComponent("manifest.json"), atomically: true, encoding: .utf8)
            try background.write(toFile: (dir as NSString).appendingPathComponent("background.js"), atomically: true, encoding: .utf8)
        } catch {
            ndCefWarn("CEF: the framework extension was not written: \(error)")
            return
        }
        setenv("ND_CEF_FRAMEWORK_EXTENSION", dir, 1)
    }
}
#endif
