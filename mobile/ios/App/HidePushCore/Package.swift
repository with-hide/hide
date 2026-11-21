// swift-tools-version: 5.9
import PackageDescription

// What the app and its notification extension share and nothing else: the per-computer push keys in the Keychain,
// the RFC 8291 decryption and the words a notice is shown with. It links no Capacitor and no UIKit, so the extension
// (which has no network and no web view) takes this package alone. macOS is listed so the package's tests run
// with `swift test` on a Mac; the app is iOS only.
let package = Package(
    name: "HidePushCore",
    platforms: [.iOS(.v15), .macOS(.v12)],
    products: [
        .library(name: "HidePushCore", targets: ["HidePushCore"])
    ],
    targets: [
        .target(name: "HidePushCore"),
        .testTarget(name: "HidePushCoreTests", dependencies: ["HidePushCore"])
    ]
)
