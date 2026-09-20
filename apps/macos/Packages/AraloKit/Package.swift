// swift-tools-version: 6.0
import PackageDescription

// AraloBridge is generated: run `make xcframework` (or `make bootstrap`) first.
let package = Package(
    name: "AraloKit",
    platforms: [.macOS(.v14)],
    products: [.library(name: "AraloKit", targets: ["AraloKit"])],
    dependencies: [.package(path: "../../Generated/AraloBridge")],
    targets: [
        .target(
            name: "AraloKit",
            dependencies: [.product(name: "AraloBridge", package: "AraloBridge")]
        ),
        .testTarget(name: "AraloKitTests", dependencies: ["AraloKit"])
    ]
)
