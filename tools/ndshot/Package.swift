// swift-tools-version:6.2
import PackageDescription

let package = Package(
    name: "ndshot",
    platforms: [.macOS(.v14)],
    targets: [
        // Zero external deps by design so `swift build` works offline.
        .executableTarget(
            name: "ndshot",
            path: "Sources/ndshot"
        )
    ]
)
