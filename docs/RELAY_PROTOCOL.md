# Mobile relay wire contract

`hide-relay-protocol` owns the routing messages, bounded binary envelope, Noise sessions and QR v2 format.
The Rust library and its WebAssembly build execute the same implementation.
It starts no sockets, timers, workers or persistence and makes no phone admission decision.
The mobile transport and phone loader integrate this library separately; adding it does not switch the current Tailscale transport on its own.

## Device admission and routing

The device has two independent keys: an Ed25519 signing seed for room admission, and a raw X25519 secret scalar for Noise.
Neither key is converted into the other, and neither secret travels to the relay or phone.
`DeviceKey` generates and signs with the former, and `NoiseKeypair` generates the latter through the cryptographic library's operating-system/browser randomness.
The room is base64url without padding of SHA-256 of the raw 32-byte Ed25519 public key.

Text control messages are JSON, capped at 2048 bytes, reject unknown fields and use `admission::{ClientMessage,ServerMessage}` and the matching TypeScript types.
A device sends `device` with `DeviceHello {version:1, room, device_key, phone_protocol}`; `phone_protocol` is a positive integer and identifies the phone application contract, separately from relay wire version 1.
The relay creates `PendingAdmission::new(hello, monotonic_ms)` for that socket and sends `challenge` with its fresh random 32-byte nonce.
The device answers `proof` with its Ed25519 signature.
The exact signed bytes are ASCII `hide-relay/device-admission/v1` followed by NUL, raw 32-byte nonce, raw 32-byte room hash, raw 32-byte Ed25519 public key, and four-byte big-endian phone protocol.
`PendingAdmission::verify(signature, monotonic_ms)` consumes the challenge on either success or failure and rejects an age of ten seconds or a backwards clock.
The relay must drop the pending value on socket close and on its ten-second socket timer, must not register a room before verification, and must not reuse a challenge.
The stateless `verify_proof` primitive alone does not establish freshness.

After `registered`, a phone sends `phone {version:1, room}` and receives `joined {connection, phone_protocol}`; the device receives `peer_joined {connection}`.
A connection is a nonzero u32, unique for the lifetime of the device's relay socket, and is never recycled within that lifetime.
`close {connection}`, `peer_left {connection}` and `refused {reason}` carry no user content.
The relay owns allocation, authorization to route each connection, room/phone limits, deadlines, frame-rate/traffic limits and teardown.

`hide-relay` implements that owner as a memory-only server: WebSocket `/ws` and `GET /health` (`200`, `ok\n`).
It has no database, state files, phone credentials or Noise session keys.
The device socket owns the room and its phone sockets; its departure closes those phones with `device_offline`.
A phone departure or device `close {connection}` removes just that peer and sends the device `peer_left`.
The device also receives `peer_left` after its own `close`; connection numbers never repeat during that device socket's lifetime.
An unknown or foreign connection number is an `invalid_message` routing violation, so callers must stop sending for a peer they close.

### Which side detects an attack

The relay verifies the device's challenge proof and rejects malformed envelopes and unauthorized routing.
It cannot authenticate the opaque Noise payload: tamper and replay are detected by the receiving endpoint's `Session`, which permanently fails that session.
The device endpoint logs its content-free integrity diagnostic and sends `close {connection}`; a phone endpoint closes its socket.
The relay then releases that peer and records `peer_closed`, without claiming to know why the endpoint closed it.
The server's integration journey sends tampered and replayed ciphertext through real sockets, observes the device endpoint reject it, then checks this close path while another phone continues exchanging encrypted messages.
Knowing a room hash grants no phone credential: the endpoint must still complete Noise and check phone authorization inside the encrypted channel.

### Server budgets and refusals

`hide-relay/src/limits.rs` is the numeric authority; the standalone binary uses its constants, and a library embedding may only lower them.

| Budget | Production cap | `refused.reason` | Log `cap` |
| --- | --- | --- | --- |
| Concurrent admitted WebSockets, pending device proof included | 512 | `connection_limit` | `connections` |
| Phones in one room | 4 | `room_limit` | `phones_per_room` |
| New WebSockets per source IP in a fixed 60-second window | 60 | `rate_limit` | `connections_per_ip` |
| Complete binary envelope | Protocol `MAX_FRAME`, 65,536 bytes | `frame_limit` | `frame_size` |
| Control message | Protocol `MAX_CONTROL`, 2,048 bytes | `frame_limit` | `control_size` |
| Forwarded bytes per room, both directions including binary headers and handshakes | 1 GiB in a 24-hour window | `daily_limit` | `bytes_per_room_per_day` |
| Unexpired IP / room accounting entries | 4,096 each | `rate_limit` / `connection_limit` | `ip_records` / `room_records` |
| Data output per socket, including an in-progress write | 8 frames | `connection_limit` | `outbound_queue` |
| Control output per socket, including reserved phone departures | 16 messages | `connection_limit` | `control_queue` |

A window starts at that IP's first attempt or the room's first admission, using monotonic uptime, and resets on the first operation at or after its end.
Room usage survives the last disconnect for the rest of that window; reconnecting cannot reset it.
At most 32 additional TCP tasks perform HTTP handshakes or deliver refusals, each with an 8 KiB HTTP buffer, 32 headers and a five-second deadline; beyond that task bound the listener applies backpressure before accepting another socket.
The WebSocket limit is enforced before reserving room state, but a valid upgrade receives a `refused` message before close so a browser can read the reason.
Health requests do not use WebSocket admission or IP-rate budget.
A refusal closes its sending socket, leaving unrelated sockets intact; closing the room's owning device necessarily disconnects that room's phones too.
If a phone recipient's data queue is full, that phone is refused and removed; the device receives `peer_left`, and other phones remain live.
If a phone fills the device's data queue, the sending phone is refused; an unauthenticated burst cannot evict the device.
Control messages have sixteen independently reserved slots, so data cannot exhaust lifecycle capacity; library embeddings may lower this through `Limits::queued_controls`.
Each successful join reserves its eventual `peer_left` slot before inserting the phone; new joins are refused when those slots run out.
One bounded FIFO merges data and control while preserving `peer_joined`, data and `peer_left` order; permits remain held through each write.
If the device itself cannot receive data or `peer_left`, it is disconnected so it cannot keep a ghost peer.
Undeliverable data is reported by the peer departure; it is never retained for a replacement connection.
The room budget charges received valid envelopes before delivery, including a full or retired destination, so receiver failure cannot make ingress free.
A device response racing an already closed phone is charged to the room budget and answered with `peer_left`, without closing the device or delivering to a replacement phone.
A retired ID may therefore produce `peer_left` again; clients must treat repeated notifications as idempotent.
Closing that retired peer again is idempotent; zero and never-issued connection IDs remain routing violations.

These caps bound the service's memory and initial free-service budget, not a measured production throughput promise.
Eight maximum-size data frames across 512 sockets occupy at most 256 MiB of payload; sixteen protocol-bounded control messages per socket add at most 16 MiB.
Receive buffers and task metadata add to that budget.
Every binary forward parses one bounded envelope, charges one room counter, and queues it for one recipient; there is no room broadcast or background sweep.
Admission alone prunes the bounded accounting tables; there is no per-frame full-table scan.

### Socket lifetime and client recovery

The first routing hello and device proof must complete within ten seconds of WebSocket establishment.
The binary expires admitted sockets after 90 seconds without incoming traffic; a library embedding may lower this idle timeout through `Limits` but cannot raise or disable it; clients keep them alive with a ping or an encrypted application heartbeat well before that deadline.
Each socket write has a five-second deadline, and its registration is released before a final close write is attempted.
A failed or expired write records `write_timeout` and attempts a `timeout` refusal with close code `1008`; it is not a cap event.
An exhausted shutdown deadline records `shutdown_timeout` without a cap field.
Every refusal half-closes TCP and discards at most `MAX_FRAME + 14` unread bytes within that same deadline.
If pipelined input fills that discard budget, the socket stays half-closed until the deadline so the peer can read the refusal before unread input causes a reset; no further input is retained or consumed.
Restart stops acceptance, closes admitted sockets with code `1012` (`restart`), clears every room and counter and drains or aborts all owned tasks within sixteen seconds.
An HTTP upgrade already in progress may instead see a failed upgrade or `timeout`; it must reconnect too.

| Close code | Meaning | Client action |
| --- | --- | --- |
| 1000 | Peer was closed | Discard its Noise session; reconnect only while the feature still wants it |
| 1008 | Admission, envelope, routing or timeout refusal | Read the preceding `refused.reason`; discard state and correct the cause before a fresh handshake |
| 1012 | Server restarting | Reconnect with jittered backoff and a new admission and Noise session |
| 1013 | A budget was crossed | Read `refused.reason`, show the limit state and retry with jittered backoff |

An EOF or failed upgrade also requires a fresh connection; the server retains no resumable session or queued message.
Clients own reconnect backoff and application intent deduplication; relay restart alone cannot prove an answer is applied once.

### Logs and deployment boundary

The binary writes JSON lines to stderr with only `event`, optional validated `room` hash, optional byte `size`, and optional `cap` name.
`peer_closed` includes the bytes received for forwarding; refusals name their category and cap, never a wire object or underlying exception.
No IP address, key, credential, URL, application payload or close reason supplied by a peer is logged.
A disconnected stderr collector does not panic connection tasks; deployment must monitor its collector too.
The relay itself retains no log files; deployment owns the stderr collector, seven-day retention and alerts on `cap_exceeded`, `server_failed` and missing health responses.
Those collection and alert settings are deployment work, not established by running the binary locally.

TLS terminates at the deployment's reverse proxy.
By default IP budgets use the TCP peer and ignore all forwarding headers.
With `--trusted-proxy <IP>`, only that exact peer must supply one `X-Real-IP` value containing a single valid address; missing, duplicate, malformed or chained addresses are refused.
The proxy must overwrite this header with the real client address and must not append or pass through a caller's value.
IPv4-mapped IPv6 addresses are canonicalized for accounting; native IPv6 addresses share their /64 attempt budget and accounting entry.
Do not put multiple callers behind a proxy without this explicit trust setting: they would share the proxy IP's budget.

## Encrypted frames and messages

Every binary WebSocket message is exactly one envelope: wire version u8 (1), kind u8 (1 handshake, 2 transport), connection u32 big-endian, then Noise bytes.
The total envelope limit is 65536 bytes, including its six-byte header.
Noise's own limits are 65535 ciphertext bytes and 65519 plaintext bytes, but the relay envelope makes this contract's usable limits 65530 and 65514 respectively.
The parser rejects unknown versions/kinds, zero connections, short payloads and oversized frames before decryption.

The protocol is exactly `Noise_NK_25519_ChaChaPoly_BLAKE2b` using locked `snow` 0.10.0 on native and WASM targets.
The prologue is ASCII `hide-mobile-relay/v1` followed by NUL and the connection's four-byte big-endian number.
The phone initiates with the QR's raw X25519 public key and hided responds with a matching `NoiseKeypair`, generated fresh or restored explicitly with `NoiseKeypair::from_secret`.
Its fields are private and `Session::responder` cannot accept an Ed25519 seed directly.
Each handshake message has exactly 48 Noise bytes, with an empty application payload: no credential, pairing code or user data travels in a handshake.
Only the responder is authenticated by NK; hided must check the existing phone credential or one-time pairing code in the first decrypted application message before accepting requests.
Ephemeral keys are random on every new session; deterministic ephemerals exist only inside the published-vector test.

`Session::send_message(bytes)` returns ordered envelopes, and `recv_message(frame, monotonic_ms)` returns a complete message or `None` while it is incomplete.
The maximum serialized application message is `noise::MAX_MESSAGE`, 256 KiB.
Inside each encrypted plaintext is an eight-byte chunk header: total message length u32 big-endian and byte offset u32 big-endian.
All but the final chunk fill the remaining 65506 bytes; an empty message is one header-only chunk.
Send all returned envelopes in order and never interleave messages in one direction.
Only one incomplete receive message exists per session, capped at 256 KiB, and its ten-second deadline starts with its first chunk and never moves on progress.
The socket owner calls `check_timeout(monotonic_ms)` from its timer even when no further frames arrive, using the same monotonic clock as `recv_message` (WASM takes bigint milliseconds).
A missing middle frame fails Noise's next ordered nonce; a missing final frame fails the timer.
Oversize, wrong connection/kind, malformed chunk, backwards clock, timeout, replay, out-of-order data or authentication failure permanently closes the session; callers close the associated socket and discard the object.
Never retry an operation on that session or reset its nonce.
Raw frame encryption is crate-private; callers have only the application-message path.

Work is linear in bytes, with one encryption/decryption per frame and at most five frames for a maximum message.
There is no fan-out, background task or retained send queue in the library.
Raw private-key buffers and incomplete plaintext buffers use zeroizing storage; this is not a claim that all internal library or JavaScript engine copies are erased.
The browser ABI exposes only the initiator, empty handshake, application-message send/receive/timeout/close, QR decoding, control-message validation and connection routing ID.
The implementation has conformance tests, not an independent cryptographic audit.

## QR v2

The only accepted URL is `https://app.withhide.dev/#pair=<base64url JSON>` with no query string or alternative origin/path.
The JSON is `{v:2, name, device_key, noise_key, room, code}`: both public keys are raw 32-byte base64url values, and the one-time code is 16 random bytes in the same encoding.
The display name, Ed25519 public key, X25519 public key, room and code all appear after `#`, so an HTTP request contains none of them.
`name` is nonblank UTF-8 plain text of at most 128 bytes, with control characters rejected; the phone renders it through an escaped text binding or `textContent`, never `innerHTML`, before connecting.
The decoder caps the URL at 1024 bytes, validates lengths/version/room derivation and rejects a zero Noise public key.
Pairing-code generation, expiry and single consumption remain hided's phone-admission responsibility.
The phone loader must clear the fragment after parsing and must never log or persist the full URL or decoded code.
The relay never receives the QR, code or encrypted phone credential in a routing control message.

## Build and evidence

[BUILD.md](BUILD.md#mobile-relay-webassembly) owns the reproducible source build and browser import.
`bash scripts/check-relay-protocol.sh` runs the official cacophony NK vector through native Rust and WASM, the same malformed/replay/limit/QR tests on both targets, and a TypeScript-driven native-to-WASM exchange in both directions, including a 256 KiB message.
`tests/fixtures/README.md` pins the upstream vector source and checksum; the fixture's public deterministic secrets must never be used in production.
The vector's nonempty handshake payload/prologue belongs only to the direct conformance test, while application sessions enforce the fixed prologue and empty handshake above.
The `relay-protocol` CI lane runs the native and WASM contract on Linux, Windows and macOS according to the repository's macOS lane selection policy.
