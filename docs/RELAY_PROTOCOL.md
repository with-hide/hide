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
The one exception is a device's `push_request`, which carries ciphertext and has its own 6,144-byte bound (Push relay below).
A device sends `device` with `DeviceHello {version:1, room, device_key, phone_protocol}`; `phone_protocol` is a positive integer and identifies the phone application contract, separately from relay wire version 1.
The relay creates `PendingAdmission::new(hello, monotonic_ms)` for that socket and sends `challenge` with its fresh random 32-byte nonce.
The device answers `proof` with its Ed25519 signature.
The exact signed bytes are ASCII `hide-relay/device-admission/v1` followed by NUL, raw 32-byte nonce, raw 32-byte room hash, raw 32-byte Ed25519 public key, and four-byte big-endian phone protocol.
`PendingAdmission::verify(signature, monotonic_ms)` consumes the challenge on either success or failure and rejects an age of ten seconds or a backwards clock.
The relay must drop the pending value on socket close and on its ten-second socket timer, must not register a room before verification, and must not reuse a challenge.
The stateless `verify_proof` primitive alone does not establish freshness.

`registered {push}` confirms admission; `push` says whether this relay forwards push requests (Push relay below).
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
Restart stops acceptance, closes admitted sockets with code `1012` (`restart`), clears every room and counter and drains or aborts all owned tasks within sixteen seconds; a push request still waiting on Apple ends unanswered with its socket.
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

The binary writes JSON lines to stderr with only `event`, optional byte `size`, and optional `cap` name.
No line names a computer or a phone: not the room hash, an address, a key or a push token, so a log cannot tell which computer a line belongs to or link two lines to one computer.
The collector's own receive time dates each line, and `size` keeps the byte counts.
`peer_closed` includes the bytes received for forwarding; refusals name their category and cap, never a wire object or underlying exception.
No IP address, key, credential, URL, application payload or close reason supplied by a peer is logged.
A disconnected stderr collector does not panic connection tasks; deployment must monitor its collector too.
The relay itself retains no log files; deployment owns the stderr collector, seven-day retention and alerts on `cap_exceeded`, `server_failed` and missing health responses.
Those collection and alert settings are deployment work, not established by running the binary locally.
The public statement of what the relay sees, logs and keeps is `site/public/privacy.html`, section 3; a change to the log fields or the retention edits it in the same pull request.
Installing and operating the production relay, with its collector, retention and alerts, is owned by a private operations document.

TLS terminates at the deployment's reverse proxy.
By default IP budgets use the TCP peer and ignore all forwarding headers.
With `--trusted-proxy <IP>`, only that exact peer must supply one `X-Real-IP` value containing a single valid address; missing, duplicate, malformed or chained addresses are refused.
The proxy must overwrite this header with the real client address and must not append or pass through a caller's value.
IPv4-mapped IPv6 addresses are canonicalized for accounting; native IPv6 addresses share their /64 attempt budget and accounting entry.
Do not put multiple callers behind a proxy without this explicit trust setting: they would share the proxy IP's budget.

## Push relay

The iOS app cannot receive Web Push, so its notifications reach Apple's push service (APNs) through the relay, which holds Apple's signing key.
The Android app's notifications reach Google's Firebase Cloud Messaging (FCM) the same way, with the access token of the relay VM's own service account.
The relay is stateless in the shape of Matrix's sygnal gateway: each request carries the device token and the ciphertext, the relay signs and forwards it while the request is in flight, and it stores neither.
There is no new HTTP entrance: a request arrives only on an admitted device socket, and Caddy still forwards only `/health` and `/ws`.

### Messages

| Direction | Message | Fields and bounds |
| --- | --- | --- |
| relay → device | `registered {push, fcm}` | `push` is true only when this relay was given Apple's key, `fcm` only when it was given an FCM project; an absent field reads as false, and `fcm` is written only when true and only to a device that named it (Compatibility and deploy order) |
| device → relay | `push_request {id, environment, token, collapse, payload}` | `id` nonzero u32, the device's own and echoed back; `environment` `sandbox` or `production`, no default; `token` hex, 32 to 200 characters, even length (`MIN_TOKEN`, `MAX_TOKEN`); `collapse` base64url, 1 to 64 characters (`MAX_COLLAPSE`); `payload` base64url of 1 to 3,000 bytes of ciphertext (`MAX_CIPHERTEXT`); the whole message at most 6,144 bytes (`MAX_PUSH_CONTROL`) |
| device → relay | `fcm_request {id, token, payload}` | `id` as above; `token` an FCM registration token, 32 to 1,024 characters of `A-Z a-z 0-9 - _ :` (`MIN_FCM_TOKEN`, `MAX_FCM_TOKEN`, `admission::fcm_token_valid`); `payload` as above, the same body hided builds for Apple; the whole message at most 6,144 bytes (`MAX_PUSH_CONTROL`) |
| relay → device | `push_result {id, result}` | answers either request; `result` is one of `delivered`, `token_gone`, `token_rejected`, `rate_limited`, `unavailable`, `failed` |

A device sends `push_request` only after a `registered` whose `push` is true, and `fcm_request` only after one whose `fcm` is true.
A relay without an FCM project answers an `fcm_request` sent anyway `unavailable`, as a relay without Apple's key answers a `push_request`.
A `push_request` or `fcm_request` from a phone socket, or before admission, is an `invalid_message` refusal like any other message in the wrong place.
Every other answer is a `push_result` under the request's id and leaves the socket open, because closing a device socket would disconnect every phone in its room.
A request whose fields are wrong, an extra field or an environment other than the two included, is answered `failed` and reaches Apple or Google with nothing; one whose id cannot be read is an `invalid_message` refusal, since no answer could name it.
`admission::decode_push` is the relay's one reader of both messages: it parses the same `ClientMessage` every other side decodes and reads only the id on its own, to answer a request it refuses.

The relay fixes everything Apple reads besides the token, the collapse value and the ciphertext: the topic (the app's bundle id, from its configuration), `apns-push-type: alert`, `apns-priority: 10` and `apns-expiration` one hour after it sends.
A request has no field for them, and an extra field makes it malformed.
The environment chooses the host: `api.sandbox.push.apple.com` for a development build, `api.push.apple.com` for TestFlight and the App Store.

For FCM the relay fixes everything Google reads besides the token and the ciphertext: the project (from its configuration), a data-only message with no notification block and no collapse key, Android priority `high` and a time to live of one hour.
It sends `POST https://fcm.googleapis.com/v1/projects/<project>/messages:send` to that host only, with `Authorization: Bearer <access token>`, never follows a redirect, and the body Google receives is exactly:

```json
{"message":{"token":"<token>","data":{"hide":"<payload>"},"android":{"priority":"high","ttl":"3600s"}}}
```

`hide` and the ciphertext are the whole data block, at most 4,004 bytes, inside FCM's 4,096.
Google sees the token, the time and size of each message, and the project it was sent to; it cannot read the ciphertext, and the Android app decrypts it the way the iOS extension does (The notification body).

### Budgets

`hide-relay/src/limits.rs` holds these constants beside the socket budgets.
One set of counters serves Apple and Google: a `push_request` and an `fcm_request` each count once against the same per-room, relay-wide and in-flight caps, so a room's budget is the same whichever service its phones use.
Every request from an admitted device socket is counted, a malformed or refused one included: its bytes are charged to the room's daily byte budget like any forwarded frame, which closes the socket with `daily_limit` once spent, and it counts against the request caps below, so neither bad requests nor requests past a cap spend the relay without a ceiling.
A cap logs `cap_exceeded` once per window, not once per refused request: once per room for each of the minute and the day window, once per second for the relay-wide cap, and at most once a second for the concurrency cap.

| Budget | Production cap | Past it | Log `cap` |
| --- | --- | --- | --- |
| Requests per room in a fixed 60-second window | 20 | `rate_limited` | `pushes_per_room_per_minute` |
| Requests per room in a 24-hour window | 1,000 | `rate_limited` | `pushes_per_room_per_day` |
| Requests across the relay per second | 50 | `rate_limited` | `pushes_per_second` |
| Requests in flight to Apple and Google together, as streams of their two HTTP/2 connections | 32 | `rate_limited` | `concurrent_pushes` |
| One request to Apple or Google, its answer, its body and the one retry included, an access token fetch too | 10 seconds | `unavailable` | none |
| Apple's answer body read | 4,096 bytes | `unavailable` | none |
| Google's answer body read | 8,192 bytes | past it, classified by status alone | none |
| The metadata server's answer read | 8,192 bytes | `unavailable` (`fcm_metadata_unreadable`) | none |

A room's push counters live in its byte record, start their day with it and are bounded by the same `room_records`, so a room's record expiring cannot open a second day's allowance inside one.
The relay keeps no queue and makes no retry except the one below; a refused request is the sender's to drop.

Known limitation: the caps are per room and relay-wide, with no share per address.
Admission is self-signed, so one address that opens many rooms can hold the relay-wide 50 a second, and other computers' requests are answered `rate_limited` meanwhile.
Volume to Apple and Google stays bounded by the relay-wide cap and each room's daily cap, and one address can already hold every socket the relay admits; a share per address would need a record the relay does not keep, since it stores no address past the 60-second admission rate.

### Apple's answers

| Apple answers | `result` |
| --- | --- |
| 2xx | `delivered` |
| 410 | `token_gone`: the token names no install of this app, and the sender deletes it |
| 400 `BadDeviceToken` or `DeviceTokenNotForTopic` | `token_rejected`: Apple refuses the token itself, which is what a token made for the other environment or for another app, or a malformed one, looks like; it does not say the install is gone, so the sender keeps the registration and writes a configuration diagnostic (`registration.rejected`, with the environment), because deleting it would only make the app register the same token again on its next connection |
| 403 `ExpiredProviderToken` | the provider token is signed again and the request sent once more, the only retry, inside the same 10 seconds; if the token is younger than 20 minutes, the retry finds the token expired again or it gets no answer, `unavailable`; any other answer to the retry is mapped by this table |
| 429, 5xx, no answer within the deadline | `unavailable` |
| anything else | `failed` |

One provider token (an ES256 JWT with `kid`, `iss` and `iat` in Unix seconds) serves every request and is replaced at 50 minutes, never more often than every 20 minutes, since Apple refuses providers that replace it faster (`TooManyProviderTokenUpdates`).
One HTTP/2 connection to Apple is kept and reused; a dropped one is replaced by the next request.
Tests stand in for Apple with a local h2c server, so the production TLS and ALPN path is exercised only by a real deployment; the client code is the same function with a different scheme.

### Google's answers

| Google answers | `result` |
| --- | --- |
| 2xx | `delivered` |
| any answer whose FCM error code is `UNREGISTERED` | `token_gone`: the token names no install of the app any more, and the sender deletes it |
| 404 without that code | `failed` (`fcm_not_found`): a wrong `--fcm-project` or path, which must not make every computer delete its phones' registrations |
| 400 with FCM error code `INVALID_ARGUMENT` and no field violation (Google's documented answer to an invalid token), or a field violation on `message.token` | `token_rejected`: Google refuses the token itself, so the sender keeps the registration and writes a configuration diagnostic, as for Apple |
| 403 `SENDER_ID_MISMATCH` | `token_rejected`: the token belongs to another Firebase project |
| 401 | the access token is fetched again once and the request sent once more, the only retry, inside the same 10 seconds; a 401 the fresh token does not fix is `unavailable`; any other answer to the retry is mapped by this table |
| 429, 5xx, no answer within the deadline | `unavailable` |
| anything else, a 400 whose field violation names another field, a 403 without `SENDER_ID_MISMATCH` and every 3xx included | `failed` |

The relay reads only the FCM error code, from the detail typed `type.googleapis.com/google.firebase.fcm.v1.FcmError` (`errorCode`, one of `UNREGISTERED`, `INVALID_ARGUMENT`, `SENDER_ID_MISMATCH`), and whether the fields a `type.googleapis.com/google.rpc.BadRequest` detail faults are `message.token` or another; nothing else of Google's body is kept or logged.
The access token is the VM's own: the relay asks the GCE metadata server (`GET http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/token`, `Host: metadata.google.internal`, `Metadata-Flavor: Google`) over a connection of its own that closes with the answer, and takes the answer only when it carries `Metadata-Flavor: Google`.
One token serves every request and is fetched again once it has five minutes or less left (`FCM_ACCESS_TOKEN_MARGIN`), and the relay never keeps one past an hour whatever lifetime the answer names; one fetch serves every send waiting for it.
A metadata server that cannot be reached, refuses or answers something else makes the request `unavailable` (`fcm_metadata_unreachable`, `fcm_metadata_refused`, `fcm_metadata_unreadable`).
One HTTP/2 connection to `fcm.googleapis.com` is kept and reused through the same client code as Apple's; tests stand in for FCM with a local h2c server and for the metadata server with a local HTTP/1 server, so the production TLS path and the real metadata server are exercised only by a real deployment.

### The notification body

This is the contract the iOS app's notification extension implements; the Android app receives the same `payload` as the `hide` value of the FCM data message.
hided encrypts the same notice JSON a Web Push phone receives (`push::payload`, with the `room` of the computer that sent it) for the key the app registered, using RFC 8291 `aes128gcm` in one record: a 16-byte salt, a 4-byte record size of 4096, a 1-byte key length of 65, hided's 65-byte ephemeral P-256 public key, then the AES-128-GCM ciphertext of the JSON followed by the delimiter byte `0x02`.
The plaintext is at most 2,897 bytes (`push::MAX_APP_PLAINTEXT`), so the body stays within 3,000 bytes; past it hided drops tags to close from the end of `clear`, and a notice that still does not fit is not sent.
The relay sends Apple exactly:

```json
{"aps":{"alert":{"loc-key":"hide-notification"},"mutable-content":1},"hide":"<payload>"}
```

Every base64url value in this section, `payload` and `collapse` included, is unpadded (RFC 4648 section 5 without `=`).

`hide-notification` is a localization key the app must carry; it is the only text Apple and a phone without the key can show, so it names nothing.
No title, state, project, badge count or tag is in plain text.
The extension tries each computer's key in turn and shows the decrypted notice; a body no key opens keeps the fixed text.
It shows a notice only when the `room` the notice names is the room of the key that opened it, so a key exists only for a computer the phone is still connected to, and a notice that names another room keeps the fixed text.
In every case it does not show, it also clears the notification's own data, so a tap on the fixed text opens the app and no agent.
The extension has no network and no web view: it words the notice from a small table of its own, in the language of the phone's system (the in-app language choice is not readable from the extension), and a notice whose reason has a templated sentence shows the plain sentence the computer sent (`detail`).
The largest body is 4,079 bytes, inside Apple's 4,096.

`collapse` is Apple's plaintext `apns-collapse-id`, so it never carries an agent key, tag or title: hided sends the first 24 bytes of HMAC-SHA-256 keyed with the phone's 16-byte authentication secret over `hide-collapse\0` and the agent's tag, base64url (32 characters).
It is stable for one agent on one phone, so a later notice replaces the earlier one, and different across phones.

Test vector: RFC 8291 Appendix A; its receiver private key, `q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94`, opens it.
Its inputs (receiver public key `BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4`, sender public key `BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8`, ECDH secret `kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs`, authentication secret `BTBZMqHH6r4Tts7J_aSIgg`, salt `DGv6ra1nlYgDCS1FRnbzlw`, plaintext `When I grow up, I want to be a watermelon`) give the `payload`:

```text
DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN
```

`hided` pins this value (`the_app_body_vector_is_appendix_a_in_base64url`) and `hide-relay` pins the body built around it (`the_documented_apple_body_wraps_the_vector_ciphertext`).

### Registration on the phone protocol

Inside its encrypted session the app sends `apns_registration {token, environment, key: {p256dh, auth}}`: the device token as hex, either case (`admission::token_valid`), its environment, and a per-computer P-256 public key (65 bytes uncompressed) and 16-byte secret, base64url, the same shape as a Web Push subscription's keys.
hided answers `push_state {notifications: "on" | "refused"}` and keeps it in the phone's record in `phones.json` (0600), where it replaces a Web Push subscription and the reverse.
`push_permission {state: "denied"}` removes it, unpairing removes it with the phone, and a `token_gone` answer removes it; a `token_rejected` answer does not.
The `token_rejected` result is new with the first release that sends push, which the relay is deployed ahead of (below): no released hided has read a `push_result` before.
The `auth` secret is part of this message, so it crosses the bridge from the page to the app's plugin (`HidePush`), as does the token the plugin hands back; the private key never does.
Only the app's main frame can call the plugin (`BridgeGate`), and the secret opens nothing without the private key, which stays in the Keychain's shared group.
An older hided answers the message `refused_request` and keeps the session, so the addition keeps `PHONE_PROTOCOL` at 5.
hided sends only while its relay link is up and said `push: true`.
A notice that finds no link is held for that phone, replaced by a newer notice for the same agent tag, at most 8 per phone and at most 60 seconds, and sent once when the link comes up; past those limits, or when the phone is gone, it is dropped with a diagnostic.
Notices about the person's own letters (`letter_undelivered`) are not held.

The app registers once per computer it is connected to, and again on every connect, because Apple may change the token and a computer only holds what it was last told.
Turning notifications off sends `push_permission {state: "denied"}` to the connected computer and deletes every computer's key on the phone; a computer that was not connected is told on its next connect.
Removing a computer on the phone sends the same message first when that computer is connected, then deletes its key.
When no key is left the app also unregisters from Apple (`unregisterForRemoteNotifications`), so Apple answers `token_gone` to every computer that still sends, and a computer removed while it was off stops sending at its next notice.
A computer removed while it is offline keeps sending until Apple reports the token gone or someone unpairs the phone on the computer; the phone has no key for it by then, so only the fixed text appears.
Hiding even that would need Apple's notification-filtering entitlement, which the app does not ask for.

### Notification taps and older reply clients

The iOS app offers no notification actions: a tap opens the named conversation, where the person can reply.
The encrypted notification still carries `ask {kind, turn}` for older phones that offer Reply.
`turn` is the agent's last change of state in Unix milliseconds, as a decimal string.
The computer retains `input` with a `turn` for those older phones: it answers `stale` unless the agent is still waiting on a question or an approval and its `changed_at_unix_ms` matches.
This prevents a reply written for one question from typing into a later prompt.
`input` without a `turn` remains the conversation's own input.
The computer's request-id claims are shared by those clients and the in-app conversation, so a confirmed write is answered `ok` without being written twice, and an ambiguous write is answered `uncertain`.
The claims live in memory only, so a repeat right after the computer restarts is checked as new.
These wire fields and checks remain compatibility paths; the current iOS app has no native reply sender, reply ledger or held replies.

### Compatibility and deploy order

An older hided reads `{"type":"registered","push":true}` as the unit `registered` it knew, and never sends `push_request`.
A newer hided reads an older relay's `{"type":"registered"}` as `push: false` and never sends `push_request`, which that relay would refuse by closing the device socket.
Both directions are tested (`an_older_device_reads_a_pushing_relay_and_keeps_its_phones`, `an_older_relay_keeps_the_link_and_is_never_sent_a_push_request`).

`fcm` cannot be added the same way: a hided released with `push` (1.2.8 and earlier) reads `registered` as a message with a `push` field that denies unknown fields, so `{"type":"registered","push":true,"fcm":true}` would end its link on every connect.
A device that reads `fcm` therefore says so in its WebSocket upgrade request, with the header `Hide-Registered-Fields: fcm` (`admission::REGISTERED_FIELDS_HEADER`, comma-separated names, spaces around each ignored), and the relay writes `fcm` only to a device that named it and only when it is true.
A device that named nothing gets exactly the `registered` it got before; a relay that does not know the header ignores it, as it ignores every header but `X-Real-IP`, and its `registered` reads as `fcm: false`, so a newer hided never sends `fcm_request` to a relay that would refuse it by closing the device socket.
A field added to `registered` later follows the same rule under its own name.
Both directions are tested: `a_released_device_keeps_its_link_and_phones_on_a_relay_that_forwards_fcm` and `a_released_device_refuses_an_unknown_registered_field_so_fcm_waits_for_the_header` keep a copy of the released reader, and `an_older_side_reads_the_new_registered_and_an_older_relay_reads_the_old_shape` reads an older relay's answer as `fcm: false`.
Deploy the relay first: a hided released before it simply has no push to send.
For FCM the order is the relay with `--fcm-project` (`deploy/relay-fcm.conf.example`), then a hided that sends `fcm_request`, then the Android app that registers a token.

### The keys on the relay host

Apple's `.p8` is the relay's only secret.
It sits at `/etc/hide-relay/apns.p8`, root-owned, mode 0400, in a 0700 directory, and reaches the service only through systemd `LoadCredential=`, never the repository, CI, an environment variable or a log; `deploy/relay-apns.conf.example` is the drop-in.
`--apns-key`, `--apns-key-id`, `--apns-team-id` and `--apns-topic` come together or not at all; without them the relay answers every request `unavailable`, and with them it refuses to start on a key it cannot read, one that is not a P-256 key, or one whose mode has any bit for others or a group write bit (`apns_key_permissions`).
A group read bit is allowed: systemd writes a credential root-owned 0400 and grants the service user read through a POSIX ACL, and with an ACL entry the group bits of the mode show the ACL mask, so the credential the relay is given reads as 0440.
Checking the drop-in on the production host is part of its deploy check.
`--apns-endpoint` names a loopback stand-in for tests and is refused for any other host, by the configuration and by `apns::Apns::new` alike.

FCM has no secret on the relay host and no key file anywhere: `--fcm-project <id>` names the Firebase project, and the access token is the VM's own (Google's answers).
The VM's service account needs the IAM role `roles/firebasecloudmessaging.admin` on that project, and the VM's access scopes must allow it (`cloud-platform`, or `https://www.googleapis.com/auth/firebase.messaging`); `deploy/relay-fcm.conf.example` is the drop-in.
The relay refuses to start on a project id that is not Google's shape (6 to 30 lowercase letters, digits and hyphens, starting with a letter).
`--fcm-endpoint` and `--fcm-metadata-endpoint` name loopback stand-ins for tests and are refused for any other host, by the configuration and by `fcm::Fcm::new` alike.
A push log line names a fixed event (`delivered`, `unregistered`, `fcm_delivered`, `fcm_unregistered`, `push_token_invalid`, `cap_exceeded` and the rest in `hide-relay/src/apns.rs`, `fcm.rs` and `server.rs`) and a cap, never a token, ciphertext, provider token, access token, Apple's or Google's answer body or the room.

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
The iOS app takes the same URL in two ways and both go through the same decoder: its own camera scan (`HideScanner`), and a universal link, since `app.withhide.dev` is an associated domain of the app (the `apple-app-site-association` file the hosted phone site serves matches `/` with the fragment `pair=*`).
The app registers no custom URL scheme, so no other app can hand it a pairing link; a link it is given is held in memory for one minute, handed to the page once, and shown to the person, who presses Connect, so it never pairs a phone without a tap.
Whether iOS passes the fragment of a universal link to the app is a platform behavior to confirm on a device with the deployed association file; the simulator run in this repository cannot show it.

## Build and evidence

[BUILD.md](BUILD.md#the-hosted-phone-release) owns the hosted phone release: main CI uploads the matching `/p/<phone_protocol>/` before desktop release preparation, keeps every older protocol bundle and stops the release on upload or production-check failure.
The relay's `phone_protocol` is only a routing hint for selecting that bundle; it grants no credential and does not bypass the Noise handshake or hided's phone admission.
The hosted app loads no external scripts or analytics, and its production CSP permits connections only to its own origin and the relay.
The relay forwards encrypted content without reading it; the phone-site build and its host remain trusted endpoints because the browser executes that site's code.

[BUILD.md](BUILD.md#mobile-relay-webassembly) owns the reproducible source build and browser import.
`bash scripts/check-relay-protocol.sh` runs the official cacophony NK vector through native Rust and WASM, the same malformed/replay/limit/QR tests on both targets, and a TypeScript-driven native-to-WASM exchange in both directions, including a 256 KiB message.
`tests/fixtures/README.md` pins the upstream vector source and checksum; the fixture's public deterministic secrets must never be used in production.
The vector's nonempty handshake payload/prologue belongs only to the direct conformance test, while application sessions enforce the fixed prologue and empty handshake above.
The `relay-protocol` CI lane runs the native and WASM contract on Linux, Windows and macOS according to the repository's macOS lane selection policy.
