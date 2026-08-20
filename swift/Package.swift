// swift-tools-version:6.2
import Foundation
import PackageDescription

let repoRoot = "../"  // swift/ is one level under the repo root; zig-out/ lives at root.

// CEF is opt-in and never vendored: the headers come from an extracted
// distribution found through ND_CEF_ROOT, or the dev cache the packaging lane
// fills. With neither present the CCef target is left out of the package and
// the engine compiles away behind `#if canImport(CCef)`, so a machine without
// CEF still builds the system-engine host.
let cefAPIVersion = "15101"
let cefVersion = "151.3.23-macosarm64"

func resolveCefRoot() -> String? {
    let environment = ProcessInfo.processInfo.environment
    // The system-only build, for proving an engine=system host carries nothing
    // from CEF even on a machine whose dev cache is full.
    if environment["ND_CEF_DISABLE"] == "1" { return nil }
    var candidates: [String] = []
    if let root = environment["ND_CEF_ROOT"], !root.isEmpty { candidates.append(root) }
    candidates.append("\(NSHomeDirectory())/.cache/nativedesktop/cef/\(cefVersion)")
    for candidate in candidates
    where FileManager.default.fileExists(atPath: "\(candidate)/include/capi/cef_client_capi.h") {
        return candidate
    }
    return nil
}

let cefRoot = resolveCefRoot()
// The CCef module map pulls in the distribution's capi headers, so the Swift
// half of the build needs the same search path and API pin the C half gets.
let cefSwiftSettings: [SwiftSetting] = cefRoot.map {
    [.unsafeFlags(["-Xcc", "-I\($0)", "-Xcc", "-DCEF_API_VERSION=\(cefAPIVersion)"])]
} ?? []

var cefTargets: [Target] = []
if let cefRoot {
    cefTargets.append(
        .target(
            name: "CCef",
            path: "Sources/CCef",
            publicHeadersPath: "include",
            cSettings: [
                .unsafeFlags(["-I", cefRoot]),
                .define("CEF_API_VERSION", to: cefAPIVersion),
            ]
        )
    )
    // The five macOS helper bundles all run this one executable. The dev
    // bundler (scripts/mac/dev-cef-bundle.sh) copies it in under the names
    // Chromium derives per process type.
    cefTargets.append(
        .executableTarget(
            // Named for the binary, not the module: `nd package` looks for
            // nd-cef-helper beside the host (packages/nd/src/package/cef.ts).
            name: "nd-cef-helper",
            dependencies: ["CCef"],
            path: "Sources/NDCefHelper",
            swiftSettings: cefSwiftSettings
        )
    )
}

let package = Package(
    name: "NDShell",
    platforms: [.macOS(.v26)],
    targets: cefTargets + [
        // System-library target: wraps the prebuilt libnd.a header (module.modulemap
        // + shim.h) so Swift can import CNd without SwiftPM demanding a
        // Sources/CNd/include/ layout (that's for regular C targets, not this
        // prebuilt-static-lib pattern).
        .systemLibrary(name: "CNd"),
        .executableTarget(
            name: "NDShell",
            dependencies: cefRoot == nil ? ["CNd"] : ["CNd", "CCef"],
            // Generated widget code (swift/Sources/NDGen/Widgets.swift +
            // the hand-written NDGen/ListView.swift) references
            // NDShell-only symbols (EventDispatcher, withEchoSuppressed,
            // radioGroupIdentifier, gWindow, makeListView) and vice versa —
            // a separate NDGen module would be circular. Compiling both
            // directories into one target keeps codegen's output path
            // (swift/Sources/NDGen/Widgets.swift) while giving generated +
            // hand-written code one shared namespace.
            path: "Sources",
            // Sibling targets living under the same Sources/ root; without
            // this SwiftPM reports their files as stray inputs to NDShell.
            exclude: ["CCef", "NDCefHelper"],
            sources: ["NDShell", "NDGen"],
            swiftSettings: cefSwiftSettings,
            linkerSettings: [
                // Link the prebuilt static lib + the frameworks the AppKit
                // backend needs. libnd.a is GTK-free pure-Zig core.
                .unsafeFlags([
                    "-L", "\(repoRoot)zig-out/lib",
                    "-lnd",
                    // libghostty-vt resolves the ghostty_* externs that
                    // libnd's terminal core (ndterm_*) references. After -lnd so the
                    // archive satisfies libnd's undefined symbols.
                    "\(repoRoot)vendor/libghostty-vt/lib/libghostty-vt-macos-aarch64.a",
                ]),
                .linkedFramework("AppKit"),
                .linkedFramework("SwiftUI"),
                .linkedFramework("Foundation"),
                .linkedFramework("QuartzCore"),  // CALayer for the screenshot fidelity ladder
                .linkedFramework("Charts"),      // Swift Charts for the <chart> widget
                .linkedFramework("WebKit"),      // WKWebView for the <webview> widget
                .linkedFramework("Security"),    // Keychain (SecItem*) for credentials.* (system seam)
                .linkedFramework("UserNotifications"),  // UNUserNotificationCenter for notification.show
                .linkedFramework("AVFoundation"),  // AVPlayer for audio.* playback (system seam)
                .linkedFramework("AVKit"),         // AVPlayerView for the <video> widget
                .linkedFramework("UniformTypeIdentifiers"),  // UTType for open/save panel filters
                .linkedFramework("MediaToolbox"),  // MTAudioProcessingTap for audio.* spectrum analysis
                .linkedFramework("Accelerate"),    // vDSP FFT for audio.* spectrum analysis
                .linkedFramework("ScreenCaptureKit"),  // opt-in rung-0 window capture (AutomationCapture.swift)
                .linkedFramework("ImageIO"),       // PNG write for the SCK capture path
            ]
        ),
    ]
)
