import XCTest

/// A check run only by `mobile/scripts/build-ios.sh allow` on the build machine, never by `build`: iOS shows the
/// notification permission question in SpringBoard, which `simctl` cannot answer and nothing in the app can, so this
/// test launches the app with the debug taps it was given, waits for the system question and presses Allow. The grant
/// stays in the simulator, so the captures that follow (a delivered notification) need no one at it.
final class AllowNotificationsUITests: XCTestCase {
    func testAllowTheNotificationQuestion() throws {
        let app = XCUIApplication()
        app.launchEnvironment["HIDE_DEBUG_TAPS"] = ProcessInfo.processInfo.environment["HIDE_DEBUG_TAPS"] ?? ""
        app.launch()
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons["Allow"]
        XCTAssertTrue(allow.waitForExistence(timeout: 60), "the notification question did not appear")
        allow.tap()
    }
}
