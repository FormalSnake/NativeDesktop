// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "NDSwift",
    platforms: [.macOS(.v14)],
    products: [.library(name: "NDSwift", type: .static, targets: ["NDSwift"])],
    targets: [.target(name: "NDSwift")]
)
