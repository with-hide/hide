use hide_relay_protocol::{Error, admission::*};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn challenge_proves_the_device_and_binds_room_version_and_protocol() {
    let key = DeviceKey::generate().unwrap();
    let hello = DeviceHello::new(&key.public_key(), 1).unwrap();
    let pending = PendingAdmission::new(hello.clone(), 100).unwrap();
    let proof = key.sign(&hello, pending.challenge()).unwrap();
    let mut changed = hello.clone();
    changed.phone_protocol = 2;
    assert_eq!(
        verify_proof(&changed, pending.challenge(), &proof),
        Err(Error::InvalidSignature)
    );
    assert_eq!(pending.verify(&proof, 101).unwrap(), hello);
    let new_socket = PendingAdmission::new(hello.clone(), 100).unwrap();
    assert_eq!(new_socket.verify(&proof, 101), Err(Error::InvalidSignature));
    for now in [99, 100 + ADMISSION_TIMEOUT_MS] {
        let expired = PendingAdmission::new(hello.clone(), 100).unwrap();
        let proof = key.sign(&hello, expired.challenge()).unwrap();
        assert_eq!(expired.verify(&proof, now), Err(Error::Timeout));
    }
    changed.room = room_id(&DeviceKey::generate().unwrap().public_key());
    assert_eq!(changed.validate(), Err(Error::InvalidMessage));
    changed = hello.clone();
    changed.version = 2;
    assert_eq!(changed.validate(), Err(Error::UnsupportedVersion));
    let message = ClientMessage::Device { hello };
    assert_eq!(
        ClientMessage::decode(&message.encode().unwrap()).unwrap(),
        message
    );
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn malformed_or_unbounded_controls_are_rejected_without_echoing_content() {
    for text in [
        r#"{"type":"close","connection":0}"#,
        r#"{"type":"close","connection":1,"credential":"secret"}"#,
        r#"{"type":"proof","signature":"secret"}"#,
    ] {
        assert_eq!(
            ClientMessage::decode(text).unwrap_err(),
            Error::InvalidMessage
        );
    }
    assert_eq!(
        ClientMessage::decode(&"x".repeat(MAX_CONTROL + 1)),
        Err(Error::SizeLimit)
    );
    assert!(
        ServerMessage::decode(r#"{"type":"joined","connection":1,"phone_protocol":0}"#).is_err()
    );
}

fn request() -> PushRequest {
    PushRequest {
        id: 7,
        environment: PushEnvironment::Sandbox,
        token: "ab".repeat(32),
        collapse: "K3Yf-_0aBcDeF".into(),
        payload: URL_SAFE_NO_PAD.encode([9_u8; 3000]),
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_push_request_is_bounded_in_every_field_it_carries() {
    let message = ClientMessage::Push(request());
    let text = message.encode().unwrap();
    assert_eq!(ClientMessage::decode(&text).unwrap(), message);

    let mut cases = Vec::new();
    let mut zero = request();
    zero.id = 0;
    cases.push(zero);
    let mut short = request();
    short.token = "ab".repeat(MIN_TOKEN / 2 - 1);
    cases.push(short);
    let mut long = request();
    long.token = "ab".repeat(MAX_TOKEN / 2 + 1);
    cases.push(long);
    let mut odd = request();
    odd.token.pop();
    cases.push(odd);
    let mut not_hex = request();
    not_hex.token = "zz".repeat(32);
    cases.push(not_hex);
    let mut no_collapse = request();
    no_collapse.collapse.clear();
    cases.push(no_collapse);
    let mut wide_collapse = request();
    wide_collapse.collapse = "k".repeat(MAX_COLLAPSE + 1);
    cases.push(wide_collapse);
    let mut padded_collapse = request();
    padded_collapse.collapse = "abc=".into();
    cases.push(padded_collapse);
    let mut empty_payload = request();
    empty_payload.payload = String::new();
    cases.push(empty_payload);
    let mut large_payload = request();
    large_payload.payload = URL_SAFE_NO_PAD.encode([1_u8; MAX_CIPHERTEXT + 1]);
    cases.push(large_payload);
    let mut not_b64 = request();
    not_b64.payload = "not base64".into();
    cases.push(not_b64);
    for case in cases {
        let text = serde_json::to_string(&ClientMessage::Push(case.clone())).unwrap();
        assert_eq!(
            ClientMessage::decode(&text),
            Err(Error::InvalidMessage),
            "{case:?}"
        );
        // The relay's reader refuses the same table, under the request's id.
        match decode_push(&text) {
            PushAttempt::Push { id, request } => {
                assert_eq!(id, case.id);
                assert!(request.is_err(), "{case:?}");
            }
            PushAttempt::Unaddressable(_) => assert_eq!(case.id, 0),
            PushAttempt::NotPush => panic!("{case:?} read as another message"),
        }
    }
    // Fields the relay fixes itself, an unknown environment and a missing
    // field are each a wrong request, answered and never sent.
    let good: serde_json::Value = serde_json::to_value(ClientMessage::Push(request())).unwrap();
    for (name, value, field) in [
        (
            "topic",
            serde_json::json!("com.example.other"),
            PushField::Shape,
        ),
        (
            "apns-push-type",
            serde_json::json!("background"),
            PushField::Shape,
        ),
        (
            "environment",
            serde_json::json!("staging"),
            PushField::Environment,
        ),
        (
            "environment",
            serde_json::Value::Null,
            PushField::Environment,
        ),
        ("token", serde_json::json!(7), PushField::Shape),
    ] {
        let mut wrong = good.clone();
        wrong[name] = value;
        assert_eq!(
            decode_push(&wrong.to_string()),
            PushAttempt::Push {
                id: 7,
                request: Err(field)
            },
            "{name}"
        );
        // Past the control budget only a valid push request is a message.
        assert!(ClientMessage::decode(&wrong.to_string()).is_err());
    }
    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("collapse");
    assert_eq!(
        decode_push(&missing.to_string()),
        PushAttempt::Push {
            id: 7,
            request: Err(PushField::Shape)
        }
    );
    assert_eq!(
        decode_push(&ClientMessage::Push(request()).encode().unwrap()),
        PushAttempt::Push {
            id: 7,
            request: Ok(Push::Apns(request()))
        }
    );
    // Neither the token nor the ciphertext is printed.
    let printed = format!("{:?}", request());
    assert!(!printed.contains(&request().token) && !printed.contains(&request().payload[..16]));
    // A body past the control budget is the push message's own bound, and a
    // message of any other kind that large is still refused at MAX_CONTROL.
    assert!(ClientMessage::Push(request()).encode().unwrap().len() > MAX_CONTROL);
    let padded = format!(
        r#"{{"type":"close","connection":1}}{}"#,
        " ".repeat(MAX_CONTROL)
    );
    assert_eq!(ClientMessage::decode(&padded), Err(Error::SizeLimit));
    assert_eq!(
        ClientMessage::decode(&"x".repeat(MAX_PUSH_CONTROL + 1)),
        Err(Error::SizeLimit)
    );
}

/// What an older device and an older relay each read (measured; a copy of
/// each side's enum, so a change here that breaks one fails this test).
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn an_older_side_reads_the_new_registered_and_an_older_relay_reads_the_old_shape() {
    #[derive(serde::Deserialize, serde::Serialize, Eq, PartialEq, Debug)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum OldServer {
        Registered,
        Refused { reason: Refusal },
    }
    // A device from before push ignores the new field on a unit variant.
    assert_eq!(
        serde_json::from_str::<OldServer>(r#"{"type":"registered","push":true}"#).unwrap(),
        OldServer::Registered
    );
    // The new device reads the old relay's answer as "no push here", so it
    // never sends a request an older relay would refuse.
    assert_eq!(
        ServerMessage::decode(r#"{"type":"registered"}"#).unwrap(),
        ServerMessage::Registered {
            push: false,
            fcm: false
        }
    );
    // A relay from before FCM: `push` only, so `fcm` reads false and the
    // device never sends `fcm_request`.
    assert_eq!(
        ServerMessage::decode(r#"{"type":"registered","push":true}"#).unwrap(),
        ServerMessage::Registered {
            push: true,
            fcm: false
        }
    );
    // `fcm` is written only when true, so a relay without FCM writes exactly
    // what a released device reads.
    assert_eq!(
        ServerMessage::Registered {
            push: true,
            fcm: false
        }
        .encode()
        .unwrap(),
        r#"{"type":"registered","push":true}"#
    );
    let both = ServerMessage::Registered {
        push: true,
        fcm: true,
    };
    assert_eq!(
        both.encode().unwrap(),
        r#"{"type":"registered","push":true,"fcm":true}"#
    );
    assert_eq!(
        ServerMessage::decode(&both.encode().unwrap()).unwrap(),
        both
    );
    // A new message an older relay does not know is refused by it rather
    // than misread; the deploy order keeps that from happening
    // (RELAY_PROTOCOL.md, Deploy order).
    #[derive(serde::Deserialize, serde::Serialize)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum OldClient {
        #[allow(dead_code)]
        Close { connection: u32 },
    }
    let push = ClientMessage::Push(request()).encode().unwrap();
    assert!(serde_json::from_str::<OldClient>(&push).is_err());
    let fcm = ClientMessage::Fcm(fcm_request()).encode().unwrap();
    assert!(serde_json::from_str::<OldClient>(&fcm).is_err());
}

/// Why the relay writes `fcm` only to a device that names it: the released
/// device (a copy of its `ServerMessage` as v1.2.8 shipped it) refuses a
/// `registered` with a field it does not know, which would drop its link on
/// every connect.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_released_device_refuses_an_unknown_registered_field_so_fcm_waits_for_the_header() {
    #[derive(serde::Deserialize, Eq, PartialEq, Debug)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum Released {
        Registered {
            #[serde(default)]
            push: bool,
        },
    }
    assert_eq!(
        serde_json::from_str::<Released>(r#"{"type":"registered","push":true}"#).unwrap(),
        Released::Registered { push: true }
    );
    assert!(
        serde_json::from_str::<Released>(r#"{"type":"registered","push":true,"fcm":true}"#)
            .is_err()
    );
    // The header a device sends to say it reads the field.
    for value in ["fcm", " fcm ", "other, fcm", "fcm,other"] {
        assert!(
            registered_fields_name(value, REGISTERED_FIELD_FCM),
            "{value}"
        );
    }
    for value in ["", "fcmx", "FCM", "other", "f cm"] {
        assert!(
            !registered_fields_name(value, REGISTERED_FIELD_FCM),
            "{value}"
        );
    }
}

/// The FCM token shape observed from Google: an instance id, a colon, and the
/// rest base64url.
const FCM_TOKEN: &str = "dQw4w9WgXcQ:APA91bHun4MxP5egoKMwt2KZFBaFUH-1RYqx_fVwP3K5D6p8nY0FzSvjMe2VxHdLw3KYq1rT7sNcPjB0gEh8uZk2mV1lQ4aRxS9tOyW6iJbCfD3pAeGnHoL5zXuI7vYw";

fn fcm_request() -> FcmRequest {
    FcmRequest {
        id: 7,
        token: FCM_TOKEN.into(),
        payload: URL_SAFE_NO_PAD.encode([9_u8; MAX_CIPHERTEXT]),
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn an_fcm_request_is_bounded_in_every_field_it_carries() {
    let message = ClientMessage::Fcm(fcm_request());
    let text = message.encode().unwrap();
    assert_eq!(ClientMessage::decode(&text).unwrap(), message);
    assert_eq!(
        decode_push(&text),
        PushAttempt::Push {
            id: 7,
            request: Ok(Push::Fcm(fcm_request()))
        }
    );
    // The largest token and ciphertext together stay inside the push bound.
    let mut largest = fcm_request();
    largest.token = "a".repeat(MAX_FCM_TOKEN);
    largest.id = u32::MAX;
    let text = ClientMessage::Fcm(largest.clone()).encode().unwrap();
    assert!(text.len() > MAX_CONTROL && text.len() <= MAX_PUSH_CONTROL);

    let mut cases = Vec::new();
    let mut zero = fcm_request();
    zero.id = 0;
    cases.push((zero, PushField::Shape));
    let mut short = fcm_request();
    short.token = "a".repeat(MIN_FCM_TOKEN - 1);
    cases.push((short, PushField::Token));
    let mut long = fcm_request();
    long.token = "a".repeat(MAX_FCM_TOKEN + 1);
    cases.push((long, PushField::Token));
    for wrong in ['"', '\\', ' ', '/', '+', '=', '.', 'é'] {
        let mut odd = fcm_request();
        odd.token.insert(10, wrong);
        cases.push((odd, PushField::Token));
    }
    let mut empty_payload = fcm_request();
    empty_payload.payload = String::new();
    cases.push((empty_payload, PushField::Payload));
    let mut large_payload = fcm_request();
    large_payload.payload = URL_SAFE_NO_PAD.encode([1_u8; MAX_CIPHERTEXT + 1]);
    cases.push((large_payload, PushField::Payload));
    let mut padded = fcm_request();
    padded.payload = "AAA=".into();
    cases.push((padded, PushField::Payload));
    for (case, field) in cases {
        let text = serde_json::to_string(&ClientMessage::Fcm(case.clone())).unwrap();
        assert!(ClientMessage::decode(&text).is_err(), "{case:?}");
        match decode_push(&text) {
            PushAttempt::Push { id, request } => {
                assert_eq!((id, request), (case.id, Err(field)), "{case:?}");
            }
            PushAttempt::Unaddressable(_) => assert_eq!(case.id, 0),
            PushAttempt::NotPush => panic!("{case:?} read as another message"),
        }
    }
    // Apple's fields, an unknown field and a missing field are each a wrong
    // request, answered and never sent; none is read as an environment.
    let good: serde_json::Value = serde_json::to_value(ClientMessage::Fcm(fcm_request())).unwrap();
    for (name, value) in [
        ("environment", serde_json::json!("production")),
        ("collapse", serde_json::json!("abc")),
        ("notification", serde_json::json!({"title": "x"})),
        ("token", serde_json::json!(7)),
    ] {
        let mut wrong = good.clone();
        wrong[name] = value;
        assert_eq!(
            decode_push(&wrong.to_string()),
            PushAttempt::Push {
                id: 7,
                request: Err(PushField::Shape)
            },
            "{name}"
        );
    }
    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("payload");
    assert_eq!(
        decode_push(&missing.to_string()),
        PushAttempt::Push {
            id: 7,
            request: Err(PushField::Shape)
        }
    );
    // Neither the token nor the ciphertext is printed.
    let printed = format!("{:?}", fcm_request());
    assert!(!printed.contains(FCM_TOKEN) && !printed.contains(&fcm_request().payload[..16]));
}
