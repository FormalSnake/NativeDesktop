// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Widgets",
    platforms: [.macOS(.v14)],
    products: [.library(name: "Widgets", type: .static, targets: ["Widgets"])],
    dependencies: [.package(path: "../../.nd/NDKit")], // materialized by nd::build from the nd crate
    targets: [.target(name: "Widgets", dependencies: ["NDKit"])]
)
