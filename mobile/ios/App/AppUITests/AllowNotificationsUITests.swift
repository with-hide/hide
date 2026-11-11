import XCTest

/// Opt-in UI journeys run by `mobile/scripts/build-ios.sh allow` or `run`, never by `build`. iOS shows the
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

    /// `push` delivered this fixture while the app was terminated. Tap the actual notification rather than
    /// synthesising a plugin event, so the recording includes the cold start and native inbox handoff.
    func testOpenTheDeliveredNotification() throws {
        XCUIDevice.shared.press(.home)
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.01)).press(forDuration: 0.05,
            thenDragTo: springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.8)))
        let notice = springboard.buttons.matching(NSPredicate(format: "label CONTAINS 'Phone launch check'")).firstMatch
        guard notice.waitForExistence(timeout: 15) else { XCTFail(springboard.debugDescription); return }
        notice.tap()
        let app = XCUIApplication()
        // iOS first expands a stack when an earlier run left a notice from this app.
        if app.state != .runningForeground {
            let open = springboard.buttons["Open"].firstMatch
            if open.waitForExistence(timeout: 3) { open.tap() }
            else if notice.exists { notice.tap() }
        }
        guard app.wait(for: .runningForeground, timeout: 15) else { XCTFail(springboard.debugDescription); return }
        guard app.webViews.firstMatch.waitForExistence(timeout: 15) else { XCTFail(app.debugDescription); return }
        XCTAssertTrue(app.webViews.staticTexts["The encrypted connection is ready."].firstMatch.waitForExistence(timeout: 30))
    }

    func testSwipeBackStartsOnlyAtTheEdge() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["HIDE_DEBUG_TAPS"] = "[data-phone-agent]"
        app.launch()
        let back = app.webViews.buttons["List"].firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: 30))
        let middle = app.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5))
        middle.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5)))
        XCTAssertTrue(back.exists, "a swipe inside the conversation must stay in the conversation")
        let edge = app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
        edge.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5)))
        XCTAssertTrue(back.waitForNonExistence(timeout: 5), "an edge swipe must use the List back action")
    }
}
