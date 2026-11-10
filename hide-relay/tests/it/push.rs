use super::support::*;
use hide_relay::limits::{MAX_CONCURRENT_PUSHES, PushLimits};
use hide_relay_protocol::admission::{DeviceHello, DeviceKey, PushOutcome, Refusal, ServerMessage};
use hyper::StatusCode;
use std::sync::atomic::Ordering;

/// The device token a test relay forwards, and the one Apple sees.
const TOKEN: &str = "ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12";

fn limits(per_minute: usize, per_day: usize) -> PushLimits {
    PushLimits {
        per_room_per_minute: per_minute,
        per_room_per_day: per_day,
        concurrent: MAX_CONCURRENT_PUSHES,
        ..PushLimits::default()
    }
}

#[tokio::test]
async fn a_push_reaches_apple_with_the_fixed_headers_and_a_token_signed_by_the_configured_key() {
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(PushLimits::default(), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    let hello = register_push(&mut device, &DeviceKey::from_seed(&[71; 32])).await;
    let payload = payload();
    let outcome = answer_push(
        &mut device,
        &push_message(7, TOKEN, "collapse-opaque-1", &payload),
        7,
    )
    .await;
    assert_eq!(outcome, PushOutcome::Delivered);
    let requests = apple.wait_for(1).await;
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];
    assert_eq!(sent.path, format!("/3/device/{TOKEN}"));
    // The topic, the push type and the priority are the relay's, never the
    // request's.
    assert_eq!(sent.header("apns-topic"), Some("me.grab.hide.ios"));
    assert_eq!(sent.header("apns-push-type"), Some("alert"));
    assert_eq!(sent.header("apns-priority"), Some("10"));
    assert_eq!(sent.header("apns-collapse-id"), Some("collapse-opaque-1"));
    assert_eq!(sent.header("content-type"), Some("application/json"));
    let expiration: u64 = sent.header("apns-expiration").unwrap().parse().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(expiration > now && expiration <= now + 3600, "{expiration}");
    // The body Apple sees is the fixed envelope; only `hide` holds content.
    assert_eq!(
        String::from_utf8(sent.body.clone()).unwrap(),
        format!(
            r#"{{"aps":{{"alert":{{"loc-key":"hide-notification"}},"mutable-content":1}},"hide":"{payload}"}}"#
        )
    );
    assert!(sent.body.len() <= 4096);
    // The provider token is an ES256 JWT from the `.p8` the relay was given.
    let (header, claims) = key.verify(sent.header("authorization").unwrap());
    assert_eq!(header["alg"], "ES256");
    assert_eq!(claims["iss"], "TEAMID5678");
    // Nothing the relay logged carries the token, the ciphertext, the JWT or
    // the room.
    let log = serde_json::to_string(&*fixture.events.lock().unwrap()).unwrap();
    for secret in [
        TOKEN,
        payload.as_str(),
        sent.header("authorization").unwrap(),
        hello.room.as_str(),
    ] {
        assert!(!log.contains(secret), "the event log carries {secret}");
    }
    assert!(
        fixture
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.event == "delivered")
    );
    fixture.stop().await;
}

#[tokio::test]
async fn a_payload_at_the_limit_still_fits_in_apple_s_body() {
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(PushLimits::default(), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register_push(&mut device, &DeviceKey::from_seed(&[72; 32])).await;
    // 3,000 bytes of ciphertext, the most the wire allows.
    let payload = {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        URL_SAFE_NO_PAD.encode(vec![9u8; 3000])
    };
    let outcome = answer_push(
        &mut device,
        &push_message(1, TOKEN, "collapse", &payload),
        1,
    )
    .await;
    assert_eq!(outcome, PushOutcome::Delivered);
    let requests = apple.wait_for(1).await;
    assert!(
        requests[0].body.len() <= 4096,
        "{} bytes",
        requests[0].body.len()
    );
    fixture.stop().await;
}

#[tokio::test]
async fn apple_s_answers_land_on_the_documented_outcome() {
    for (status, reason, expected) in [
        (StatusCode::GONE, "", PushOutcome::TokenGone),
        (
            StatusCode::BAD_REQUEST,
            "BadDeviceToken",
            PushOutcome::TokenGone,
        ),
        (
            StatusCode::BAD_REQUEST,
            "DeviceTokenNotForTopic",
            PushOutcome::TokenGone,
        ),
        (StatusCode::TOO_MANY_REQUESTS, "", PushOutcome::Unavailable),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "",
            PushOutcome::Unavailable,
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "",
            PushOutcome::Unavailable,
        ),
        (
            StatusCode::BAD_REQUEST,
            "PayloadTooLarge",
            PushOutcome::Failed,
        ),
        (StatusCode::FORBIDDEN, "BadCertificate", PushOutcome::Failed),
    ] {
        let apple = Apple::start().await;
        apple.answer(status, reason);
        let key = TestKey::new();
        let mut fixture = Fixture::pushing(PushLimits::default(), &apple, &key).await;
        let mut device = fixture.socket("192.0.2.1").await;
        register_push(&mut device, &DeviceKey::from_seed(&[73; 32])).await;
        let outcome = answer_push(
            &mut device,
            &push_message(3, TOKEN, "collapse", &payload()),
            3,
        )
        .await;
        assert_eq!(outcome, expected, "{status} {reason}");
        assert_eq!(apple.wait_for(1).await.len(), 1, "{status} {reason}");
        fixture.stop().await;
    }
}

const MINUTE: u64 = 60_000;

#[tokio::test]
async fn one_provider_token_serves_every_push_until_it_is_fifty_minutes_old() {
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(PushLimits::default(), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register_push(&mut device, &DeviceKey::from_seed(&[82; 32])).await;
    let issued = fixture.unix.load(Ordering::SeqCst) / 1000;
    for (id, advance) in [(1, 0), (2, 49 * MINUTE), (3, MINUTE)] {
        fixture.unix.fetch_add(advance, Ordering::SeqCst);
        let message = push_message(id, TOKEN, "collapse", &payload());
        assert_eq!(
            answer_push(&mut device, &message, id).await,
            PushOutcome::Delivered
        );
    }
    let requests = apple.wait_for(3).await;
    let tokens: Vec<&str> = requests
        .iter()
        .map(|request| request.header("authorization").unwrap())
        .collect();
    // Apple reads `iat` as Unix time; the first token carries the clock's.
    let (_, claims) = key.verify(tokens[0]);
    assert_eq!(claims["iat"], issued);
    assert_eq!(tokens[0], tokens[1], "reused at 49 minutes");
    assert_ne!(tokens[1], tokens[2], "replaced at 50 minutes");
    let (_, claims) = key.verify(tokens[2]);
    assert_eq!(claims["iat"], issued + 50 * 60);
    fixture.stop().await;
}

#[tokio::test]
async fn an_expired_provider_token_is_signed_once_more_and_the_request_sent_once_more() {
    // The one retry: Apple refuses a token 21 minutes old, the relay signs a
    // new one and sends the same request again.
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(PushLimits::default(), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register_push(&mut device, &DeviceKey::from_seed(&[74; 32])).await;
    let first = push_message(1, TOKEN, "collapse", &payload());
    assert_eq!(
        answer_push(&mut device, &first, 1).await,
        PushOutcome::Delivered
    );
    fixture.unix.fetch_add(21 * MINUTE, Ordering::SeqCst);
    apple.answer_next(StatusCode::FORBIDDEN, "ExpiredProviderToken");
    let second = push_message(2, TOKEN, "collapse", &payload());
    assert_eq!(
        answer_push(&mut device, &second, 2).await,
        PushOutcome::Delivered
    );
    let requests = apple.wait_for(3).await;
    assert_eq!(requests.len(), 3, "one retry, no more");
    let refused = requests[1].header("authorization").unwrap();
    let retried = requests[2].header("authorization").unwrap();
    assert_eq!(requests[0].header("authorization").unwrap(), refused);
    assert_ne!(refused, retried, "the retry carries a newly signed token");
    key.verify(retried);
    fixture.stop().await;

    // A refusal that survives the retry is `unavailable`, still with only the
    // one retry.
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(PushLimits::default(), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register_push(&mut device, &DeviceKey::from_seed(&[75; 32])).await;
    let first = push_message(1, TOKEN, "collapse", &payload());
    assert_eq!(
        answer_push(&mut device, &first, 1).await,
        PushOutcome::Delivered
    );
    fixture.unix.fetch_add(21 * MINUTE, Ordering::SeqCst);
    apple.answer(StatusCode::FORBIDDEN, "ExpiredProviderToken");
    let second = push_message(2, TOKEN, "collapse", &payload());
    assert_eq!(
        answer_push(&mut device, &second, 2).await,
        PushOutcome::Unavailable
    );
    assert_eq!(apple.wait_for(3).await.len(), 3, "one retry, no more");
    fixture.stop().await;
}

#[tokio::test]
async fn a_token_younger_than_twenty_minutes_is_not_replaced_when_apple_calls_it_expired() {
    // Apple refuses providers that replace their token more often than every
    // 20 minutes, so a young token earns no retry.
    let apple = Apple::start().await;
    apple.answer(StatusCode::FORBIDDEN, "ExpiredProviderToken");
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(PushLimits::default(), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register_push(&mut device, &DeviceKey::from_seed(&[83; 32])).await;
    // The first send issues the token and Apple refuses it at once.
    let message = push_message(1, TOKEN, "collapse", &payload());
    assert_eq!(
        answer_push(&mut device, &message, 1).await,
        PushOutcome::Unavailable
    );
    assert_eq!(apple.wait_for(1).await.len(), 1);
    // At 19 minutes the same token is still too young to replace.
    fixture.unix.fetch_add(19 * MINUTE, Ordering::SeqCst);
    let message = push_message(2, TOKEN, "collapse", &payload());
    assert_eq!(
        answer_push(&mut device, &message, 2).await,
        PushOutcome::Unavailable
    );
    let requests = apple.wait_for(2).await;
    assert_eq!(requests.len(), 2, "no retry with a young token");
    assert_eq!(
        requests[0].header("authorization"),
        requests[1].header("authorization")
    );
    fixture.stop().await;
}

#[tokio::test]
async fn one_connection_carries_every_push() {
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(PushLimits::default(), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register_push(&mut device, &DeviceKey::from_seed(&[76; 32])).await;
    for id in 1..=3 {
        let outcome = answer_push(
            &mut device,
            &push_message(id, TOKEN, "collapse", &payload()),
            id,
        )
        .await;
        assert_eq!(outcome, PushOutcome::Delivered);
        apple.wait_for(id as usize).await;
    }
    // A connection per request would have made three here.
    assert_eq!(apple.connections(), 1, "the relay keeps one connection");
    fixture.stop().await;
}

#[tokio::test]
async fn the_room_and_the_relay_caps_answer_rate_limited_and_keep_the_socket() {
    // A room's minute cap, counted before the fields are read.
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(limits(2, 10), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register_push(&mut device, &DeviceKey::from_seed(&[77; 32])).await;
    for id in 1..=2 {
        let outcome = answer_push(
            &mut device,
            &push_message(id, TOKEN, "collapse", &payload()),
            id,
        )
        .await;
        assert_eq!(outcome, PushOutcome::Delivered);
    }
    let outcome = answer_push(
        &mut device,
        &push_message(3, TOKEN, "collapse", &payload()),
        3,
    )
    .await;
    assert_eq!(outcome, PushOutcome::RateLimited);
    fixture.cap("pushes_per_room_per_minute");
    // The socket is still the room's device.
    assert_eq!(
        answer_push(
            &mut device,
            &push_message(4, TOKEN, "collapse", &payload()),
            4
        )
        .await,
        PushOutcome::RateLimited
    );
    assert_eq!(
        apple.requests().len(),
        2,
        "a refused request never reaches Apple"
    );
    fixture.stop().await;

    // A room's day cap.
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(limits(10, 2), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register_push(&mut device, &DeviceKey::from_seed(&[78; 32])).await;
    for id in 1..=2 {
        let outcome = answer_push(
            &mut device,
            &push_message(id, TOKEN, "collapse", &payload()),
            id,
        )
        .await;
        assert_eq!(outcome, PushOutcome::Delivered);
    }
    assert_eq!(
        answer_push(
            &mut device,
            &push_message(3, TOKEN, "collapse", &payload()),
            3
        )
        .await,
        PushOutcome::RateLimited
    );
    fixture.cap("pushes_per_room_per_day");
    fixture.stop().await;
}

#[tokio::test]
async fn a_malformed_push_is_counted_answered_under_its_id_and_leaves_the_socket_open() {
    let apple = Apple::start().await;
    let key = TestKey::new();
    // One request per minute: the malformed one spends it.
    let mut fixture = Fixture::pushing(limits(1, 10), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register_push(&mut device, &DeviceKey::from_seed(&[79; 32])).await;
    // An odd-length, non-hex token is refused as a field, not as a frame.
    let outcome = answer_push(
        &mut device,
        &push_message(1, "zz", "collapse", &payload()),
        1,
    )
    .await;
    assert_eq!(outcome, PushOutcome::Failed);
    assert!(
        fixture
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.event == "push_token_invalid")
    );
    // It counted: the room's one request a minute is spent.
    let outcome = answer_push(
        &mut device,
        &push_message(2, TOKEN, "collapse", &payload()),
        2,
    )
    .await;
    assert_eq!(outcome, PushOutcome::RateLimited);
    // Nothing reached Apple, and the device socket is still working: the next
    // room-level call answers under its own id.
    assert_eq!(apple.requests().len(), 0);
    fixture.stop().await;
}

#[tokio::test]
async fn a_relay_without_apple_s_key_answers_unavailable_and_still_serves_the_room() {
    let mut fixture = Fixture::start(hide_relay::limits::Limits::default()).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register(&mut device, &DeviceKey::from_seed(&[80; 32])).await;
    let outcome = answer_push(
        &mut device,
        &push_message(1, TOKEN, "collapse", &payload()),
        1,
    )
    .await;
    assert_eq!(outcome, PushOutcome::Unavailable);
    // The socket stays open for everything it was already for.
    let mut phone = fixture.socket("192.0.2.2").await;
    let hello = DeviceHello::new(&DeviceKey::from_seed(&[80; 32]).public_key(), 7).unwrap();
    join(&mut phone, &hello.room, &mut device).await;
    fixture.stop().await;
}

#[tokio::test]
async fn a_phone_socket_cannot_ask_for_a_push() {
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(PushLimits::default(), &apple, &key).await;
    let mut device = fixture.socket("192.0.2.1").await;
    let hello = register_push(&mut device, &DeviceKey::from_seed(&[81; 32])).await;
    let mut phone = fixture.socket("192.0.2.2").await;
    join(&mut phone, &hello.room, &mut device).await;
    text(&mut phone, &push_message(1, TOKEN, "collapse", &payload())).await;
    refused(&mut phone, Refusal::InvalidMessage).await;
    assert_eq!(apple.requests().len(), 0);
    // The device learns the phone is gone and keeps its own socket.
    assert!(matches!(
        control(&mut device).await,
        ServerMessage::PeerLeft { .. }
    ));
    let outcome = answer_push(
        &mut device,
        &push_message(2, TOKEN, "collapse", &payload()),
        2,
    )
    .await;
    assert_eq!(outcome, PushOutcome::Delivered);
    fixture.stop().await;
}

/// The relay starts only with a key it can use: Apple's PEM `.p8`, owner-only.
/// A missing file, one others may read, or one that is not a key stops it.
#[cfg(unix)]
#[test]
fn the_relay_starts_only_with_an_owner_only_readable_key() {
    use hide_relay::{
        apns::{load, unix_clock},
        config::ApnsConfig,
    };
    use std::os::unix::fs::PermissionsExt;
    let folder = tempfile::tempdir().unwrap();
    let config = |key: &str| ApnsConfig {
        key: folder.path().join(key),
        key_id: "KEYID1234".into(),
        team_id: "TEAMID5678".into(),
        topic: "me.grab.hide.ios".into(),
        endpoint: None,
    };
    let write = |name: &str, bytes: &[u8], mode: u32| {
        let path = folder.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    let key = TestKey::new();
    write("apns.p8", key.pem().as_bytes(), 0o400);
    write("wide.p8", key.pem().as_bytes(), 0o440);
    write("other.p8", key.pem().as_bytes(), 0o604);
    write("garbage.p8", b"not a key", 0o400);
    write(
        "truncated.p8",
        b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
        0o400,
    );
    let limits = PushLimits::default();
    assert!(load(&config("apns.p8"), limits, unix_clock()).is_ok());
    for (name, reason) in [
        ("missing.p8", "apns_key_unreadable"),
        ("wide.p8", "apns_key_permissions"),
        ("other.p8", "apns_key_permissions"),
        ("garbage.p8", "apns_key_unreadable"),
        ("truncated.p8", "apns_key_unreadable"),
    ] {
        assert_eq!(
            load(&config(name), limits, unix_clock()).err(),
            Some(reason),
            "{name}"
        );
    }
}
