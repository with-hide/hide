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
    }
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
/// each side's struct, so a change here that breaks one fails this test).
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn an_older_side_reads_the_new_registered_and_an_older_relay_reads_the_old_shape() {
    #[derive(serde::Deserialize, serde::Serialize, Eq, PartialEq, Debug)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum OldServer {
        Registered,
        Refused { reason: Refusal },
    }
    // The old device ignores the new field on a unit variant.
    assert_eq!(
        serde_json::from_str::<OldServer>(r#"{"type":"registered","push":true}"#).unwrap(),
        OldServer::Registered
    );
    // The new device reads the old relay's answer as "no push here", so it
    // never sends a request an older relay would refuse.
    let ServerMessage::Registered { push } =
        ServerMessage::decode(r#"{"type":"registered"}"#).unwrap()
    else {
        panic!("registered")
    };
    assert!(!push);
    assert_eq!(
        ServerMessage::decode(&ServerMessage::Registered { push: true }.encode().unwrap()).unwrap(),
        ServerMessage::Registered { push: true }
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
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_push_result_names_one_outcome_and_keeps_its_request_id() {
    for (text, result) in [
        ("delivered", PushOutcome::Delivered),
        ("token_gone", PushOutcome::TokenGone),
        ("rate_limited", PushOutcome::RateLimited),
        ("unavailable", PushOutcome::Unavailable),
        ("failed", PushOutcome::Failed),
    ] {
        let message = ServerMessage::PushResult(PushResult { id: 4, result });
        assert_eq!(
            ServerMessage::decode(&message.encode().unwrap()).unwrap(),
            message
        );
        assert_eq!(
            message.encode().unwrap(),
            format!(r#"{{"type":"push_result","id":4,"result":"{text}"}}"#)
        );
    }
    assert_eq!(
        ServerMessage::decode(r#"{"type":"push_result","id":0,"result":"delivered"}"#),
        Err(Error::InvalidMessage)
    );
    assert_eq!(
        ServerMessage::decode(r#"{"type":"push_result","id":4,"result":"sent"}"#),
        Err(Error::InvalidMessage)
    );
    assert_eq!(
        ServerMessage::decode(r#"{"type":"push_result","id":4,"result":"delivered","token":"ab"}"#),
        Err(Error::InvalidMessage)
    );
}
