import AVFoundation
import UIKit
import XCTest
@testable import HideShell

/// The scanner's ways out: the close button in every state of the camera, a screen that says what to do instead of a
/// black one, one answer per scan, and a code that is not a Hide link left alone.
final class QRScannerTests: XCTestCase {
    private let words = ScannerText(["title": "T", "hint": "H", "whereTitle": "W", "whereBody": "B", "deniedTitle": "D", "deniedBody": "DB", "openSettings": "Open", "close": "Close"])!
    private let link = "https://app.withhide.dev/#pair=" + String(repeating: "a", count: 40)

    private func scanner(authorized: Bool, input: @escaping () -> AVCaptureDeviceInput? = { nil }, opened: @escaping () -> Void = {}, done: @escaping (String?) -> Void) -> QRScannerViewController {
        let controller = QRScannerViewController(text: words, authorized: authorized, makeInput: input, openSettings: opened, completion: done)
        controller.loadViewIfNeeded()
        return controller
    }

    func testARefusedCameraShowsGuidanceWithTheWayToSettingsAndACloseThatEndsTheScan() {
        var answers: [String?] = []
        var opened = 0
        let controller = scanner(authorized: false, opened: { opened += 1 }, done: { answers.append($0) })
        XCTAssertEqual(controller.mode, .unavailable)
        XCTAssertNotNil(controller.closeButton.superview)
        XCTAssertFalse(controller.settingsButton.isHidden || controller.settingsButton.superview?.isHidden == true)
        controller.settingsButton.sendActions(for: .touchUpInside)
        XCTAssertEqual(opened, 1)
        controller.closeButton.sendActions(for: .touchUpInside)
        controller.closeButton.sendActions(for: .touchUpInside)
        XCTAssertEqual(answers.count, 1, "one answer however many presses")
        XCTAssertNil(answers.first ?? "x")
    }

    /// A camera that cannot be opened (held by another client, a configuration failure) is the same guidance screen, not a black view without a button.
    func testACameraThatCannotBeOpenedIsTheGuidanceScreenWithAClose() {
        var answers: [String?] = []
        let controller = scanner(authorized: true, input: { nil }, done: { answers.append($0) })
        XCTAssertEqual(controller.mode, .unavailable)
        XCTAssertNotNil(controller.closeButton.superview)
        controller.closeButton.sendActions(for: .touchUpInside)
        XCTAssertEqual(answers.count, 1)
    }

    func testACodeThatIsNotAHideLinkIsIgnoredAndTheNextOneEndsTheScan() {
        var answers: [String?] = []
        let controller = scanner(authorized: false, done: { answers.append($0) })
        XCTAssertFalse(controller.handle("https://example.com/#pair=" + String(repeating: "a", count: 40)))
        XCTAssertFalse(controller.handle("WIFI:S:home;T:WPA;P:secret;;"))
        XCTAssertTrue(answers.isEmpty, "a foreign code does not end the scan")
        XCTAssertTrue(controller.handle(link))
        XCTAssertEqual(answers.count, 1)
        XCTAssertEqual(answers.first ?? nil, link)
    }

    func testTheScannerNeedsEveryWordTheNativeSideDraws() {
        XCTAssertNil(ScannerText(nil))
        XCTAssertNil(ScannerText(["title": "T"]))
        XCTAssertNotNil(ScannerText(["title": "T", "hint": "H", "whereTitle": "W", "whereBody": "B", "deniedTitle": "D", "deniedBody": "DB", "openSettings": "Open", "close": "Close"]))
    }
}
