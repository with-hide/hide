import Foundation
import HidePushCore
import HideRelayC

/// One encrypted session to one computer over the relay, for one action. The sockets are the system's
/// (`URLSessionWebSocketTask`); the Noise handshake, the frame format and its limits are the relay protocol library's,
/// the same Rust code the phone's web app runs as WebAssembly (`hide-relay-ios`), so this file only orders the steps:
/// join, handshake, sign in, wait for hello, send, wait for the result. Nothing here is logged: not a token, a key, a
/// frame or the text.
public final class RelayPhoneLink: PhoneLink {
    /// The phone protocol this app speaks (`PHONE_PROTOCOL` in `web/src/mobile/phoneVersion.ts`; `check-versions.mjs`
    /// holds the two equal). A computer that speaks another one is not sent anything.
    public static let phoneProtocol = 5
    public static let defaultURL = "wss://relay.withhide.dev/ws"

    private let url: URL
    private let session: URLSession
    private var task: URLSessionWebSocketTask?
    private var relay: OpaquePointer?
    private var connection: UInt32 = 0
    private let lock = NSLock()
    private var closed = false
    private var handedOver = false

    /// The relay the app is built for: the Info.plist's, and in a debug build the variable the simulator was launched
    /// with (so a check can name a relay of its own). Release builds read the Info.plist only.
    public static func relayURL(bundle: Bundle = .main, environment: [String: String] = ProcessInfo.processInfo.environment) -> URL? {
        #if DEBUG
        if let override = environment["HIDE_RELAY_URL"], let url = URL(string: override) { return url }
        #endif
        return URL(string: (bundle.object(forInfoDictionaryKey: "HideRelayURL") as? String) ?? defaultURL)
    }

    /// The marker the library carries (`hide-relay-ios-source:<digest>`): read at every start so the linker keeps it in the
    /// app, where the release check looks for it.
    public static var librarySource: String { String(cString: hide_relay_source()) }

    public init(url: URL) {
        _ = Self.librarySource
        self.url = url
        self.session = URLSession(configuration: .ephemeral)
    }

    deinit { close() }

    // MARK: PhoneLink

    public func connect(room: String, noiseKey: Data, credential: String) async throws {
        do {
            guard noiseKey.count == 32 else { throw LinkFailure.notSent }
            let task = session.webSocketTask(with: url)
            lock.lock()
            if closed { lock.unlock(); throw LinkFailure.notSent }
            self.task = task
            lock.unlock()
            task.resume()
            try await task.send(.string(try control(["type": "phone", "version": 1, "room": room], server: false)))
            var joined: [String: Any]?
            while joined == nil {
                guard case .string(let text) = try await task.receive() else { throw LinkFailure.notSent }
                let object = try Self.object(try control(text, server: true))
                guard object["type"] as? String == "joined" else { throw LinkFailure.notSent }
                joined = object
            }
            guard let joined, let number = joined["connection"] as? NSNumber, number.uint64Value <= UInt64(UInt32.max),
                  joined["phone_protocol"] as? Int == Self.phoneProtocol else { throw LinkFailure.notSent }
            connection = number.uint32Value
            var created: OpaquePointer?
            try noiseKey.withUnsafeBytes { raw in
                try Self.check(hide_relay_session_initiator(raw.bindMemory(to: UInt8.self).baseAddress, connection, &created))
            }
            lock.lock(); relay = created; lock.unlock()
            let handshake = try withRelay { session -> Data in
                var out = HideBuf(ptr: nil, len: 0)
                try Self.check(hide_relay_write_handshake(session, &out))
                return Self.take(out)
            }
            try await task.send(.data(handshake))
            guard case .data(let reply) = try await task.receive() else { throw LinkFailure.notSent }
            try withRelay { session in
                try reply.withUnsafeBytes { raw in
                    try Self.check(hide_relay_read_handshake(session, raw.bindMemory(to: UInt8.self).baseAddress, reply.count))
                }
            }
            try await sendMessage(["client_kind": "phone", "token": credential])
            while true {
                let message = try await nextMessage()
                switch message["type"] as? String {
                case "hello": return
                case "refused": throw LinkFailure.notSent
                default: continue
                }
            }
        } catch let failure as LinkFailure {
            throw failure
        } catch {
            throw LinkFailure.notSent
        }
    }

    public func request(_ message: [String: Any], resultType: String, requestId: String) async throws -> [String: Any] {
        do { try await sendMessage(message) } catch { throw LinkFailure.notSent }
        lock.lock(); handedOver = true; lock.unlock()
        let kind = message["type"] as? String
        do {
            while true {
                let answer = try await nextMessage()
                let type = answer["type"] as? String
                if type == resultType, answer["request_id"] as? String == requestId { return answer }
                if type == "refused_request", answer["request"] as? String == kind { throw LinkFailure.outdated }
            }
        } catch {
            // The message left; the one rule says what any end of the wait means now.
            throw ReplyRule.afterHandover(error)
        }
    }

    public func close() {
        lock.lock()
        closed = true
        let task = self.task
        self.task = nil
        let relay = self.relay
        self.relay = nil
        lock.unlock()
        task?.cancel(with: .goingAway, reason: nil)
        if let relay {
            _ = hide_relay_close(relay)
            hide_relay_session_free(relay)
        }
        session.invalidateAndCancel()
    }

    // MARK: Steps

    private func sendMessage(_ message: [String: Any]) async throws {
        let text = try JSONSerialization.data(withJSONObject: message, options: [.sortedKeys])
        let frames = try withRelay { session -> Data in
            var out = HideBuf(ptr: nil, len: 0)
            try text.withUnsafeBytes { raw in
                try Self.check(hide_relay_send_message(session, raw.bindMemory(to: UInt8.self).baseAddress, text.count, &out))
            }
            return Self.take(out)
        }
        guard let task = currentTask() else { throw LinkFailure.notSent }
        var offset = 0
        while offset < frames.count {
            guard offset + 4 <= frames.count else { throw LinkFailure.notSent }
            let length = frames[offset..<offset + 4].reduce(0) { ($0 << 8) | Int($1) }
            offset += 4
            guard length > 0, offset + length <= frames.count else { throw LinkFailure.notSent }
            try await task.send(.data(frames.subdata(in: offset..<offset + length)))
            offset += length
        }
    }

    /// The next application message the computer sent; control messages and frames for another connection are not one.
    private func nextMessage() async throws -> [String: Any] {
        while true {
            guard let task = currentTask() else { throw LinkFailure.notSent }
            switch try await task.receive() {
            case .string(let text):
                let object = try Self.object(try control(text, server: true))
                if object["type"] as? String == "peer_left" || object["type"] as? String == "refused" { throw LinkFailure.notSent }
            case .data(let frame):
                var number: UInt32 = 0
                try frame.withUnsafeBytes { raw in
                    try Self.check(hide_relay_frame_connection(raw.bindMemory(to: UInt8.self).baseAddress, frame.count, &number))
                }
                guard number == connection else { continue }
                let message = try withRelay { session -> Data in
                    var out = HideBuf(ptr: nil, len: 0)
                    try frame.withUnsafeBytes { raw in
                        try Self.check(hide_relay_recv_message(session, raw.bindMemory(to: UInt8.self).baseAddress, frame.count, Self.nowMs(), &out))
                    }
                    return Self.take(out)
                }
                try withRelay { session in try Self.check(hide_relay_check_timeout(session, Self.nowMs())) }
                // An empty answer is a message that is not complete yet.
                if !message.isEmpty { return try Self.object(String(decoding: message, as: UTF8.self)) }
            @unknown default:
                continue
            }
        }
    }

    private func control(_ json: [String: Any], server: Bool) throws -> String {
        try control(String(decoding: try JSONSerialization.data(withJSONObject: json, options: [.sortedKeys]), as: UTF8.self), server: server)
    }

    /// A control message through the library's own validation, so what is sent or read has the shape the relay allows.
    private func control(_ json: String, server: Bool) throws -> String {
        var out = HideBuf(ptr: nil, len: 0)
        let bytes = Array(json.utf8)
        let code = bytes.withUnsafeBufferPointer { raw in
            server ? hide_relay_server_message(raw.baseAddress, raw.count, &out) : hide_relay_client_message(raw.baseAddress, raw.count, &out)
        }
        try Self.check(code)
        return String(decoding: Self.take(out), as: UTF8.self)
    }

    // MARK: Plumbing

    private func currentTask() -> URLSessionWebSocketTask? {
        lock.lock(); defer { lock.unlock() }
        return task
    }

    /// Runs one library call while the session is alive: `close` waits for it, and a call after `close` is refused.
    private func withRelay<T>(_ body: (OpaquePointer) throws -> T) throws -> T {
        lock.lock(); defer { lock.unlock() }
        guard let relay, !closed else { throw handedOver ? LinkFailure.unconfirmed : LinkFailure.notSent }
        return try body(relay)
    }

    private static func check(_ code: Int32) throws {
        if code != 0 { throw LinkFailure.notSent }
    }

    private static func take(_ buffer: HideBuf) -> Data {
        guard let pointer = buffer.ptr else { return Data() }
        defer { hide_relay_buf_free(buffer) }
        return Data(bytes: pointer, count: buffer.len)
    }

    private static func object(_ text: String) throws -> [String: Any] {
        guard let object = (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? [String: Any] else { throw LinkFailure.notSent }
        return object
    }

    private static func nowMs() -> UInt64 { DispatchTime.now().uptimeNanoseconds / 1_000_000 }
}
