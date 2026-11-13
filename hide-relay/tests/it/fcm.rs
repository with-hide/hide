use super::support::*;
use hide_relay::limits::{MAX_CONCURRENT_PUSHES, PushLimits};
use hide_relay_protocol::admission::{DeviceHello, DeviceKey, PushOutcome, Refusal, ServerMessage};
use hyper::StatusCode;
use std::{sync::atomic::Ordering, time::Duration};

/// An FCM registration token in the shape Google issues.
const TOKEN: &str = "dQw4w9WgXcQ:APA91bHun4MxP5egoKMwt2KZFBaFUH-1RYqx_fVwP3K5D6p8nY0FzSvjMe2VxHdLw3KYq1rT7sNcPjB0gEh8uZk2mV1lQ4aRxS9tOyW6iJbCfD3pAeGnHoL5zXuI7vYw";
/// The Apple device token the shared-budget test sends beside it.
const APNS_TOKEN: &str = "ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12";
const MINUTE: u64 = 60_000;

/// A relay with FCM only, and a device that named `fcm` and was told FCM is
/// forwarded.
async fn fcm_device(push: PushLimits, google: &Google, seed: u8) -> (Fixture, Socket) {
    let fixture = Fixture::google(push, google, None).await;
    let mut device = fixture.socket_reading_fcm("192.0.2.1").await;
    register_fcm(&mut device, &DeviceKey::from_seed(&[seed; 32]), false, true).await;
    (fixture, device)
}

#[tokio::test]
async fn an_fcm_request_reaches_google_as_a_data_only_high_priority_message_with_the_vm_s_token() {
    let google = Google::start().await;
    let mut fixture = Fixture::google(PushLimits::default(), &google, None).await;
    let mut device = fixture.socket_reading_fcm("192.0.2.1").await;
    let hello = register_fcm(&mut device, &DeviceKey::from_seed(&[91; 32]), false, true).await;
    let payload = payload();
    let outcome = answer_push(&mut device, &fcm_message(7, TOKEN, &payload), 7).await;
    assert_eq!(outcome, PushOutcome::Delivered);

    // The token came from the VM's own service account, asked the way the
    // metadata server requires.
    let fetched = google.metadata.requests();
    assert_eq!(fetched.len(), 1);
    assert_eq!(
        fetched[0].path,
        "/computeMetadata/v1/instance/service-accounts/default/token"
    );
    assert_eq!(fetched[0].header("metadata-flavor"), Some("Google"));

    let sent = &google.fcm.wait_for(1).await[0];
    assert_eq!(sent.path, "/v1/projects/hide-android/messages:send");
    assert_eq!(sent.header("authorization"), Some("Bearer access-1"));
    assert_eq!(sent.header("content-type"), Some("application/json"));
    // Exactly this body: the token, the ciphertext under `hide`, high
    // priority, one hour; no notification block and no collapse key.
    let body = String::from_utf8(sent.body.clone()).unwrap();
    assert_eq!(
        body,
        format!(
            r#"{{"message":{{"token":"{TOKEN}","data":{{"hide":"{payload}"}},"android":{{"priority":"high","ttl":"3600s"}}}}}}"#
        )
    );
    let message: serde_json::Value = serde_json::from_str(&body).unwrap();
    let keys = |value: &serde_json::Value| {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    assert_eq!(keys(&message["message"]), ["android", "data", "token"]);
    assert_eq!(keys(&message["message"]["android"]), ["priority", "ttl"]);
    // Nothing the relay logged carries the token, the ciphertext, the access
    // token, Google's answer or the room.
    let log = serde_json::to_string(&*fixture.events.lock().unwrap()).unwrap();
    for secret in [TOKEN, payload.as_str(), "access-1", hello.room.as_str()] {
        assert!(!log.contains(secret), "the event log carries {secret}");
    }
    assert!(
        fixture
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.event == "fcm_delivered")
    );
    fixture.stop().await;
}

#[tokio::test]
async fn a_payload_at_the_limit_still_fits_in_fcm_s_data() {
    let google = Google::start().await;
    let (mut fixture, mut device) = fcm_device(PushLimits::default(), &google, 92).await;
    // 3,000 bytes of ciphertext, the most the wire allows, and the longest
    // token it allows.
    let payload = {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        URL_SAFE_NO_PAD.encode(vec![9u8; 3000])
    };
    let token = "a".repeat(hide_relay_protocol::admission::MAX_FCM_TOKEN);
    let outcome = answer_push(&mut device, &fcm_message(1, &token, &payload), 1).await;
    assert_eq!(outcome, PushOutcome::Delivered);
    let sent: serde_json::Value =
        serde_json::from_slice(&google.fcm.wait_for(1).await[0].body).unwrap();
    let data = &sent["message"]["data"];
    assert!("hide".len() + data["hide"].as_str().unwrap().len() <= 4096);
    fixture.stop().await;
}

#[tokio::test]
async fn fcm_s_answers_land_on_the_documented_outcome() {
    let token_error = Google::error(
        "INVALID_ARGUMENT",
        "INVALID_ARGUMENT",
        Some("message.token"),
    );
    let other_argument = Google::error("INVALID_ARGUMENT", "INVALID_ARGUMENT", None);
    let unregistered = Google::error("NOT_FOUND", "UNREGISTERED", None);
    let mismatch = Google::error("PERMISSION_DENIED", "SENDER_ID_MISMATCH", None);
    let denied = Google::error("PERMISSION_DENIED", "", None);
    let unavailable = Google::error("UNAVAILABLE", "UNAVAILABLE", None);
    for (status, body, expected, event) in [
        (
            StatusCode::NOT_FOUND,
            "",
            PushOutcome::TokenGone,
            "fcm_not_found",
        ),
        (
            StatusCode::NOT_FOUND,
            unregistered.as_str(),
            PushOutcome::TokenGone,
            "fcm_unregistered",
        ),
        (
            StatusCode::BAD_REQUEST,
            unregistered.as_str(),
            PushOutcome::TokenGone,
            "fcm_unregistered",
        ),
        (
            StatusCode::BAD_REQUEST,
            token_error.as_str(),
            PushOutcome::TokenRejected,
            "fcm_token_rejected",
        ),
        (
            StatusCode::FORBIDDEN,
            mismatch.as_str(),
            PushOutcome::TokenRejected,
            "fcm_sender_id_mismatch",
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            "",
            PushOutcome::Unavailable,
            "fcm_too_many_requests",
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "",
            PushOutcome::Unavailable,
            "fcm_server_error",
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            unavailable.as_str(),
            PushOutcome::Unavailable,
            "fcm_server_error",
        ),
        (
            StatusCode::BAD_REQUEST,
            other_argument.as_str(),
            PushOutcome::Failed,
            "fcm_bad_request",
        ),
        (
            StatusCode::FORBIDDEN,
            denied.as_str(),
            PushOutcome::Failed,
            "fcm_forbidden",
        ),
        // A redirect is an answer, never followed.
        (
            StatusCode::MOVED_PERMANENTLY,
            "",
            PushOutcome::Failed,
            "fcm_refused",
        ),
    ] {
        let google = Google::start().await;
        google.fcm.answer_with(status, body);
        let (mut fixture, mut device) = fcm_device(PushLimits::default(), &google, 93).await;
        let outcome = answer_push(&mut device, &fcm_message(3, TOKEN, &payload()), 3).await;
        assert_eq!(outcome, expected, "{status} {body}");
        assert_eq!(google.fcm.wait_for(1).await.len(), 1, "{status} {body}");
        let events = fixture.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e.event == event), "{status} {body}");
        // Google's answer body is never logged.
        let log = serde_json::to_string(&events).unwrap();
        assert!(!log.contains("secret-free text"), "{status}");
        fixture.stop().await;
    }
}

#[tokio::test]
async fn no_answer_in_time_is_unavailable_and_a_metadata_server_that_is_not_one_is_too() {
    // FCM holds its answer past the request deadline.
    let google = Google::start().await;
    google.fcm.hold();
    let push = PushLimits {
        request_timeout: Duration::from_millis(300),
        ..PushLimits::default()
    };
    let (mut fixture, mut device) = fcm_device(push, &google, 94).await;
    let outcome = answer_push(&mut device, &fcm_message(1, TOKEN, &payload()), 1).await;
    assert_eq!(outcome, PushOutcome::Unavailable);
    fixture.logged("fcm_unreachable");
    google.fcm.release();
    fixture.stop().await;

    // An answer without `Metadata-Flavor: Google`, or a refusal, gives no
    // token, and Google is never asked.
    for (status, flavored) in [(StatusCode::OK, false), (StatusCode::FORBIDDEN, true)] {
        let google = Google::start().await;
        google.metadata.answer(status, flavored);
        let (mut fixture, mut device) = fcm_device(PushLimits::default(), &google, 95).await;
        let outcome = answer_push(&mut device, &fcm_message(1, TOKEN, &payload()), 1).await;
        assert_eq!(outcome, PushOutcome::Unavailable, "{status} {flavored}");
        fixture.logged("fcm_metadata_refused");
        assert!(google.fcm.requests().is_empty());
        fixture.stop().await;
    }
}

#[tokio::test]
async fn one_access_token_serves_every_send_until_shortly_before_it_expires() {
    let google = Google::start().await;
    google.metadata.expires_in.store(3600, Ordering::SeqCst);
    let (mut fixture, mut device) = fcm_device(PushLimits::default(), &google, 96).await;
    // At 54 minutes six are left, past the five-minute margin; at 56 four
    // are, inside it.
    for (id, advance) in [(1, 0), (2, 54 * MINUTE), (3, 2 * MINUTE)] {
        fixture.unix.fetch_add(advance, Ordering::SeqCst);
        let outcome = answer_push(&mut device, &fcm_message(id, TOKEN, &payload()), id).await;
        assert_eq!(outcome, PushOutcome::Delivered);
    }
    let tokens: Vec<String> = google
        .fcm
        .wait_for(3)
        .await
        .iter()
        .map(|sent| sent.header("authorization").unwrap().to_owned())
        .collect();
    assert_eq!(
        tokens,
        ["Bearer access-1", "Bearer access-1", "Bearer access-2"]
    );
    assert_eq!(google.metadata.requests().len(), 2);
    fixture.stop().await;
}

#[tokio::test]
async fn a_401_fetches_the_access_token_once_more_and_sends_once_more() {
    let google = Google::start().await;
    google.fcm.answer_next_with(StatusCode::UNAUTHORIZED, "");
    let (mut fixture, mut device) = fcm_device(PushLimits::default(), &google, 97).await;
    let outcome = answer_push(&mut device, &fcm_message(1, TOKEN, &payload()), 1).await;
    assert_eq!(outcome, PushOutcome::Delivered);
    let sent = google.fcm.wait_for(2).await;
    assert_eq!(sent.len(), 2, "one retry");
    assert_eq!(sent[0].header("authorization"), Some("Bearer access-1"));
    assert_eq!(sent[1].header("authorization"), Some("Bearer access-2"));
    assert_eq!(sent[0].body, sent[1].body, "the same request, sent again");
    assert_eq!(google.metadata.requests().len(), 2);
    // The refused token is not used again: the next send keeps the new one.
    let outcome = answer_push(&mut device, &fcm_message(2, TOKEN, &payload()), 2).await;
    assert_eq!(outcome, PushOutcome::Delivered);
    assert_eq!(
        google.fcm.wait_for(3).await[2].header("authorization"),
        Some("Bearer access-2")
    );
    fixture.stop().await;

    // A 401 the fresh token does not fix is `unavailable`, after exactly one
    // refresh and one retry.
    let google = Google::start().await;
    google.fcm.answer_with(StatusCode::UNAUTHORIZED, "");
    let (mut fixture, mut device) = fcm_device(PushLimits::default(), &google, 98).await;
    let outcome = answer_push(&mut device, &fcm_message(1, TOKEN, &payload()), 1).await;
    assert_eq!(outcome, PushOutcome::Unavailable);
    assert_eq!(google.fcm.wait_for(2).await.len(), 2, "one retry, no more");
    assert_eq!(google.metadata.requests().len(), 2);
    fixture.logged("fcm_unauthorized_again");
    fixture.stop().await;
}

#[tokio::test]
async fn apple_and_fcm_requests_spend_one_set_of_push_budgets() {
    // A room's minute cap counts both services' requests.
    let google = Google::start().await;
    let apple = Apple::start().await;
    let key = TestKey::new();
    let push = PushLimits {
        per_room_per_minute: 2,
        per_room_per_day: 10,
        concurrent: MAX_CONCURRENT_PUSHES,
        ..PushLimits::default()
    };
    let mut fixture = Fixture::google(push, &google, Some((&apple, &key))).await;
    let mut device = fixture.socket_reading_fcm("192.0.2.1").await;
    register_fcm(&mut device, &DeviceKey::from_seed(&[99; 32]), true, true).await;
    let apns = |id| push_message(id, APNS_TOKEN, "collapse", &payload());
    let fcm = |id| fcm_message(id, TOKEN, &payload());
    assert_eq!(
        answer_push(&mut device, &apns(1), 1).await,
        PushOutcome::Delivered
    );
    assert_eq!(
        answer_push(&mut device, &fcm(2), 2).await,
        PushOutcome::Delivered
    );
    assert_eq!(
        answer_push(&mut device, &fcm(3), 3).await,
        PushOutcome::RateLimited
    );
    assert_eq!(
        answer_push(&mut device, &apns(4), 4).await,
        PushOutcome::RateLimited
    );
    fixture.cap("pushes_per_room_per_minute");
    assert_eq!(
        (apple.requests().len(), google.fcm.requests().len()),
        (1, 1)
    );
    fixture.stop().await;

    // The relay-wide per-second cap counts both.
    let google = Google::start().await;
    let apple = Apple::start().await;
    let push = PushLimits {
        per_second: 1,
        ..PushLimits::default()
    };
    let mut fixture = Fixture::google(push, &google, Some((&apple, &key))).await;
    let mut device = fixture.socket_reading_fcm("192.0.2.1").await;
    register_fcm(&mut device, &DeviceKey::from_seed(&[100; 32]), true, true).await;
    assert_eq!(
        answer_push(&mut device, &fcm(1), 1).await,
        PushOutcome::Delivered
    );
    assert_eq!(
        answer_push(&mut device, &apns(2), 2).await,
        PushOutcome::RateLimited
    );
    fixture.cap("pushes_per_second");
    fixture.stop().await;

    // The requests in flight count both: Apple holds the only slot, so FCM
    // is refused until it answers.
    let google = Google::start().await;
    let apple = Apple::start().await;
    apple.hold();
    let push = PushLimits {
        concurrent: 1,
        ..PushLimits::default()
    };
    let mut fixture = Fixture::google(push, &google, Some((&apple, &key))).await;
    let mut device = fixture.socket_reading_fcm("192.0.2.1").await;
    register_fcm(&mut device, &DeviceKey::from_seed(&[101; 32]), true, true).await;
    text(&mut device, &apns(1)).await;
    apple.wait_for(1).await;
    assert_eq!(
        answer_push(&mut device, &fcm(2), 2).await,
        PushOutcome::RateLimited
    );
    fixture.cap("concurrent_pushes");
    apple.release();
    let ServerMessage::PushResult(result) = control(&mut device).await else {
        panic!("push_result")
    };
    assert_eq!((result.id, result.result), (1, PushOutcome::Delivered));
    assert_eq!(
        answer_push(&mut device, &fcm(3), 3).await,
        PushOutcome::Delivered
    );
    assert!(google.fcm.requests().len() == 1);
    fixture.stop().await;
}

/// `registered` carries `fcm` only for a device that named it in its upgrade
/// request, so a released hided, which refuses an unknown field, keeps its
/// link and its phones on a relay that forwards FCM.
#[tokio::test]
async fn a_released_device_keeps_its_link_and_phones_on_a_relay_that_forwards_fcm() {
    /// The released device's reading, as v1.2.8 shipped it.
    #[derive(serde::Deserialize, Debug, PartialEq)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum Released {
        Challenge {
            challenge: serde_json::Value,
        },
        Registered {
            #[serde(default)]
            push: bool,
        },
    }
    let google = Google::start().await;
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::google(PushLimits::default(), &google, Some((&apple, &key))).await;
    // No header: exactly the shape a released device reads.
    let mut released = fixture.socket("192.0.2.1").await;
    let device_key = DeviceKey::from_seed(&[102; 32]);
    let hello = DeviceHello::new(&device_key.public_key(), 7).unwrap();
    send(
        &mut released,
        hide_relay_protocol::admission::ClientMessage::Device {
            hello: hello.clone(),
        },
    )
    .await;
    let ServerMessage::Challenge { challenge } = control(&mut released).await else {
        panic!("challenge")
    };
    send(
        &mut released,
        hide_relay_protocol::admission::ClientMessage::Proof {
            signature: device_key.sign(&hello, &challenge).unwrap(),
        },
    )
    .await;
    let tokio_tungstenite::tungstenite::Message::Text(text) = next(&mut released).await else {
        panic!("registered")
    };
    assert_eq!(text.as_str(), r#"{"type":"registered","push":true}"#);
    assert_eq!(
        serde_json::from_str::<Released>(&text).unwrap(),
        Released::Registered { push: true }
    );
    // What the same relay writes for a device that names `fcm` would have
    // ended the released device's link.
    let mut reading = fixture.socket_reading_fcm("192.0.2.2").await;
    let (_, registered) = admit(&mut reading, &DeviceKey::from_seed(&[103; 32])).await;
    let written = registered.encode().unwrap();
    assert_eq!(written, r#"{"type":"registered","push":true,"fcm":true}"#);
    assert!(serde_json::from_str::<Released>(&written).is_err());
    // The released device still serves its phones.
    let mut phone = fixture.socket("192.0.2.3").await;
    join(&mut phone, &hello.room, &mut released).await;
    assert!(apple.requests().is_empty() && google.fcm.requests().is_empty());
    fixture.stop().await;
}

/// A device that names `fcm` reads `fcm: false` from a relay without an FCM
/// project, and a field name it does not know is ignored; such a relay
/// answers an `fcm_request` sent anyway `unavailable` and keeps the room.
#[tokio::test]
async fn a_relay_without_fcm_says_so_and_answers_unavailable() {
    let apple = Apple::start().await;
    let key = TestKey::new();
    let mut fixture = Fixture::pushing(PushLimits::default(), &apple, &key).await;
    let mut device = fixture
        .socket_naming("192.0.2.1", Some("fcm, something-later"))
        .await;
    let hello = register_fcm(&mut device, &DeviceKey::from_seed(&[104; 32]), true, false).await;
    let outcome = answer_push(&mut device, &fcm_message(1, TOKEN, &payload()), 1).await;
    assert_eq!(outcome, PushOutcome::Unavailable);
    let mut phone = fixture.socket("192.0.2.2").await;
    join(&mut phone, &hello.room, &mut device).await;
    assert!(apple.requests().is_empty());
    fixture.stop().await;
}

/// An FCM request in the wrong place is refused like any other message; a
/// malformed one on the device socket is answered `failed` and counted.
#[tokio::test]
async fn an_fcm_request_in_the_wrong_place_is_refused_and_a_malformed_one_is_failed() {
    let google = Google::start().await;
    let push = PushLimits {
        per_room_per_minute: 1,
        per_room_per_day: 10,
        concurrent: MAX_CONCURRENT_PUSHES,
        ..PushLimits::default()
    };
    let mut fixture = Fixture::google(push, &google, None).await;
    // Before admission.
    let mut early = fixture.socket_reading_fcm("192.0.2.1").await;
    text(&mut early, &fcm_message(1, TOKEN, &payload())).await;
    refused(&mut early, Refusal::InvalidMessage).await;
    // From a phone.
    let mut device = fixture.socket_reading_fcm("192.0.2.2").await;
    let hello = register_fcm(&mut device, &DeviceKey::from_seed(&[105; 32]), false, true).await;
    let mut phone = fixture.socket("192.0.2.3").await;
    join(&mut phone, &hello.room, &mut device).await;
    text(&mut phone, &fcm_message(1, TOKEN, &payload())).await;
    refused(&mut phone, Refusal::InvalidMessage).await;
    assert!(matches!(
        control(&mut device).await,
        ServerMessage::PeerLeft { .. }
    ));
    // A token with a quote is a field, answered `failed`, and it counted.
    let outcome = answer_push(&mut device, &fcm_message(2, "bad\\\"token", &payload()), 2).await;
    assert_eq!(outcome, PushOutcome::Failed);
    fixture.logged("push_token_invalid");
    let outcome = answer_push(&mut device, &fcm_message(3, TOKEN, &payload()), 3).await;
    assert_eq!(outcome, PushOutcome::RateLimited);
    assert!(google.fcm.requests().is_empty() && google.metadata.requests().is_empty());
    fixture.stop().await;
}
