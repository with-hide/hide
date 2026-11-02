use hide_relay_protocol::{Error, admission::*};

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
