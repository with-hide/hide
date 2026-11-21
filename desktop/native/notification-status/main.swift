// Prints whether macOS lets the app this runs inside show notifications, as
// one JSON line, and exits (PRD first-run-onboarding D-12).
//
// Electron has no API for UNUserNotificationCenter's authorization, and a
// notification macOS suppresses still shows without error, so the desktop
// host runs this instead (`desktop/src/main/notificationPermission.ts`). It
// must sit in the bundle's Contents/MacOS, since NSBundle finds its bundle
// by walking up from the executable, and it carries the app's identifier in
// an embedded Info.plist, since macOS keys notification settings to it
// (`desktop/scripts/package.mjs` builds and signs it).
//
// Derived from Orca's native/notification-status-macos/main.swift (MIT; see
// NOTICE). Hide took the authorization switch, its values, the three-second
// wait and the JSON line; it dropped the alert setting and exits with an error
// when macOS does not answer instead of printing "unknown".
import Foundation
import UserNotifications

let answered = DispatchSemaphore(value: 0)
var authorization = "unknown"
UNUserNotificationCenter.current().getNotificationSettings { settings in
  switch settings.authorizationStatus {
  case .authorized: authorization = "authorized"
  case .provisional: authorization = "provisional"
  case .ephemeral: authorization = "ephemeral"
  case .denied: authorization = "denied"
  case .notDetermined: authorization = "not-determined"
  @unknown default: authorization = "unknown"
  }
  answered.signal()
}
if answered.wait(timeout: .now() + 3) == .timedOut {
  FileHandle.standardError.write("getNotificationSettings did not answer\n".data(using: .utf8)!)
  exit(2)
}
print("{\"authorization\":\"\(authorization)\"}")
