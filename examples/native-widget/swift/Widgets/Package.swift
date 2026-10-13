// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Widgets",
    platforms: [.macOS(.v14)],
    products: [.library(name: "Widgets", type: .static, targets: ["Widgets"])],
    // Built only through the app's build script, which generates one aggregate package in OUT_DIR
    // and passes the framework's NDKit path in, so NDKit is compiled once and nothing lands here.
    dependencies: [.package(path: Context.environment["ND_KIT_PATH"] ?? "NDKit")],
    targets: [.target(name: "Widgets", dependencies: ["NDKit"])]
)
