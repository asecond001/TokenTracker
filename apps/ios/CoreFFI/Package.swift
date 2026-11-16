// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "TokenTrackerCore",
    platforms: [
        .iOS(.v18)
    ],
    products: [
        .library(
            name: "TokenTrackerCore",
            targets: ["TokenTrackerCore"]
        )
    ],
    targets: [
        .binaryTarget(
            name: "TokenTrackerCoreFFI",
            path: "Artifacts/TokenTrackerCoreFFI.xcframework"
        ),
        .target(
            name: "TokenTrackerCore",
            dependencies: ["TokenTrackerCoreFFI"],
            path: "Sources/TokenTrackerCore"
        )
    ],
    swiftLanguageModes: [.v5]
)
