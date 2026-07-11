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
            // T3's hand-written NDGen/ListView.swift) references
            // NDShell-only symbols (EventDispatcher, withEchoSuppressed,
            // radioGroupIdentifier, gWindow, makeListView) and vice versa —
            // a separate NDGen module would be circular. Compiling both
            // directories into one target keeps the file paths the plan
            // mandates (swift/Sources/NDGen/Widgets.swift) while giving
            // generated + hand-written code one shared namespace.
            path: "Sources",
            sources: ["NDShell", "NDGen"],
            linkerSettings: [
                // Link the prebuilt static lib + the frameworks the AppKit
                // backend (T3+) needs. libnd.a is GTK-free pure-Zig core.
                .unsafeFlags([
                    "-L", "\(repoRoot)zig-out/lib",
                    "-lnd",
                ]),
                .linkedFramework("AppKit"),
                .linkedFramework("Foundation"),
                .linkedFramework("QuartzCore"),  // CALayer for the T5 fidelity ladder
            ]
        ),
    ]
)
