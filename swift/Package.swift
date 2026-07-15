// swift-tools-version:6.2
import PackageDescription

let repoRoot = "../"  // swift/ is one level under the repo root; zig-out/ lives at root.

let package = Package(
    name: "NDShell",
    platforms: [.macOS(.v26)],
    targets: [
        // System-library target: wraps the prebuilt libnd.a header (module.modulemap
        // + shim.h) so Swift can import CNd without SwiftPM demanding a
        // Sources/CNd/include/ layout (that's for regular C targets, not this
        // prebuilt-static-lib pattern).
        .systemLibrary(name: "CNd"),
        .executableTarget(
            name: "NDShell",
            dependencies: ["CNd"],
            // Generated widget code (swift/Sources/NDGen/Widgets.swift +
            // the hand-written NDGen/ListView.swift) references
            // NDShell-only symbols (EventDispatcher, withEchoSuppressed,
            // radioGroupIdentifier, gWindow, makeListView) and vice versa —
            // a separate NDGen module would be circular. Compiling both
            // directories into one target keeps codegen's output path
            // (swift/Sources/NDGen/Widgets.swift) while giving generated +
            // hand-written code one shared namespace.
            path: "Sources",
            sources: ["NDShell", "NDGen"],
            linkerSettings: [
                // Link the prebuilt static lib + the frameworks the AppKit
                // backend needs. libnd.a is GTK-free pure-Zig core.
                .unsafeFlags([
                    "-L", "\(repoRoot)zig-out/lib",
                    "-lnd",
                    // libghostty-vt resolves the ghostty_* externs that
                    // libnd's terminal core (ndterm_*) references. After -lnd so the
                    // archive satisfies libnd's undefined symbols.
                    "\(repoRoot)vendor/libghostty-vt/lib/libghostty-vt.a",
                ]),
                .linkedFramework("AppKit"),
                .linkedFramework("SwiftUI"),
                .linkedFramework("Foundation"),
                .linkedFramework("QuartzCore"),  // CALayer for the screenshot fidelity ladder
                .linkedFramework("WebKit"),      // WKWebView for the <webview> widget
                .linkedFramework("Security"),    // Keychain (SecItem*) for credentials.* (system seam)
                .linkedFramework("UserNotifications"),  // UNUserNotificationCenter for notification.show
                .linkedFramework("AVFoundation"),  // AVPlayer for audio.* playback (system seam)
                .linkedFramework("AVKit"),         // AVPlayerView for the <video> widget
                .linkedFramework("UniformTypeIdentifiers"),  // UTType for open/save panel filters
                .linkedFramework("MediaToolbox"),  // MTAudioProcessingTap for audio.* spectrum analysis
                .linkedFramework("Accelerate"),    // vDSP FFT for audio.* spectrum analysis
            ]
        ),
    ]
)
