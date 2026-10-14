// swift-tools-version: 6.0
import PackageDescription

// Built only through the app's build script, which aggregates it with the framework's packages
// in OUT_DIR, so nothing lands here. The package name must match its library product.
let package = Package(
    name: "Widgets",
    platforms: [.macOS(.v14)],
    products: [.library(name: "Widgets", type: .static, targets: ["Widgets"])],
    targets: [.target(name: "Widgets")]
)
