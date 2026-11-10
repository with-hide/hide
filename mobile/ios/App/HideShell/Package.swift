// swift-tools-version: 5.9
import PackageDescription

// The app's own native code: what stands between the web view and Capacitor's bridge.
// The Capacitor version here must equal the one in CapApp-SPM/Package.swift and mobile/package.json;
// mobile/scripts/check-versions.mjs refuses a mismatch.
let package = Package(
    name: "HideShell",
    platforms: [.iOS(.v15)],
    products: [
        .library(name: "HideShell", targets: ["HideShell"])
    ],
    dependencies: [
        .package(url: "https://github.com/ionic-team/capacitor-swift-pm.git", exact: "8.6.0"),
        .package(path: "../HidePushCore")
    ],
    targets: [
        // The relay protocol library behind a C interface (`hide-relay-ios`), built by scripts/build-relay-ios.sh and wrapped
        // into this folder by mobile/scripts/mini-ios.sh on the build machine; the folder is not kept in git.
        .binaryTarget(name: "HideRelayC", path: "Frameworks/HideRelayC.xcframework"),
        .target(
            name: "HideShell",
            // Capacitor's headers import Cordova, so a target that links one links both, as CapApp-SPM does.
            dependencies: [
                .product(name: "Capacitor", package: "capacitor-swift-pm"),
                .product(name: "Cordova", package: "capacitor-swift-pm"),
                .product(name: "HidePushCore", package: "HidePushCore"),
                "HideRelayC"
            ]
        ),
        .testTarget(
            name: "HideShellTests",
            dependencies: [
                "HideShell",
                .product(name: "Capacitor", package: "capacitor-swift-pm"),
                .product(name: "Cordova", package: "capacitor-swift-pm")
            ]
        )
    ]
)
