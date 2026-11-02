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
