// swift-tools-version: 6.0
import PackageDescription

// The injection matrix and the latency harness: a command-line tool that types
// into real apps while Aralo runs, and reports what arrived. Not part of the
// app. AraloBridge is generated: run `make xcframework` (or `make bootstrap`) first.
let package = Package(
    name: "AraloHarness",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "aralo-harness", targets: ["aralo-harness"])],
    dependencies: [
        .package(path: "../AraloKit"),
        .package(path: "../../Generated/AraloBridge")
    ],
    targets: [
        .target(
            name: "AraloHarness",
            dependencies: [
                .product(name: "AraloKit", package: "AraloKit"),
                .product(name: "AraloBridge", package: "AraloBridge")
            ]
        ),
        .executableTarget(name: "aralo-harness", dependencies: ["AraloHarness"]),
        .testTarget(name: "AraloHarnessTests", dependencies: ["AraloHarness"])
    ]
)
