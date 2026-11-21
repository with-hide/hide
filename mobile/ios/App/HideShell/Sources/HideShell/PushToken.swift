import Foundation
import UIKit

/// The device token Apple gives the app, handed to whoever asked for it. `AppDelegate` forwards the two callbacks;
/// the plugin asks. A token is never logged or written anywhere here: it goes to the computer inside the encrypted
/// session and nowhere else.
public final class PushToken {
    public static let shared = PushToken()

    public enum Failure: Error, Equatable { case unavailable, timedOut }

    private var waiting: [(Result<String, Failure>) -> Void] = []
    /// Counts the rounds of waiting, so the timer of one round never ends a later one.
    private var round = 0
    private var last: String?
    private let lock = NSLock()
    /// How long a request waits for Apple before it answers `timedOut`.
    public var wait: TimeInterval = 10
    /// Asks the system to register; a seam so a test does not need the app.
    public var register: () -> Void = {
        DispatchQueue.main.async { UIApplication.shared.registerForRemoteNotifications() }
    }

    /// Tells the system the app wants no more notices; a seam so a test does not need the app.
    public var unregister: () -> Void = {
        DispatchQueue.main.async { UIApplication.shared.unregisterForRemoteNotifications() }
    }

    public init() {}

    /// Called after a computer's key was deleted, with how many are left. With none left no notice could be opened, so
    /// the app stops receiving them altogether: Apple then answers `gone` to every computer that still sends one,
    /// including a computer that was removed while it was off and could not be told.
    public func keysChanged(remaining: Int) {
        guard remaining == 0 else { return }
        lock.lock(); last = nil; lock.unlock()
        unregister()
    }

    /// Registers with Apple and answers with the token, once. A registration already in flight answers every waiter.
    public func request(_ completion: @escaping (Result<String, Failure>) -> Void) {
        lock.lock()
        let first = waiting.isEmpty
        waiting.append(completion)
        if first { round += 1 }
        let mine = round
        lock.unlock()
        guard first else { return }
        register()
        DispatchQueue.global().asyncAfter(deadline: .now() + wait) { [weak self] in self?.finish(.failure(.timedOut), round: mine) }
    }

    public func didRegister(deviceToken: Data) {
        let hex = deviceToken.map { String(format: "%02x", $0) }.joined()
        lock.lock(); last = hex; lock.unlock()
        finish(.success(hex))
    }

    public func didFail() { finish(.failure(.unavailable)) }

    /// Answers every waiter; with `round`, only when that round is still the one waiting.
    private func finish(_ result: Result<String, Failure>, round only: Int? = nil) {
        lock.lock()
        if let only, only != round { lock.unlock(); return }
        let done = waiting
        waiting = []
        lock.unlock()
        for completion in done { completion(result) }
    }
}

/// The environment Apple's push service answers for this build: decided by the build configuration, never a default.
/// A development build is registered with the sandbox, TestFlight and the App Store with the production service, and
/// a token sent to the other one is refused as `BadDeviceToken`.
public enum PushEnvironment {
    public static var current: String {
        #if DEBUG
        return "sandbox"
        #else
        return "production"
        #endif
    }
}
