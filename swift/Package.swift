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
