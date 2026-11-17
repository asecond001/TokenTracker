// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "TokenVaneCore",
    platforms: [
        .iOS(.v18)
    ],
    products: [
        .library(
            name: "TokenVaneCore",
            targets: ["TokenVaneCore"]
        )
    ],
    targets: [
        .binaryTarget(
            name: "TokenVaneCoreFFI",
            path: "Artifacts/TokenVaneCoreFFI.xcframework"
        ),
        .target(
            name: "TokenVaneCore",
            dependencies: ["TokenVaneCoreFFI"],
            path: "Sources/TokenVaneCore"
        )
    ],
    swiftLanguageModes: [.v5]
)
