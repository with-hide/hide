use hide_relay_protocol::{
    admission::DeviceKey,
    noise::NoiseKeypair,
    qr::{APP_URL, PairingQr},
};

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn pairing_material_is_only_in_the_fragment_and_room_matches_device_key() {
    let key = DeviceKey::generate().unwrap();
    let noise = NoiseKeypair::generate().unwrap();
    let qr = PairingQr::new(
        &key.public_key(),
        &noise.public_key(),
        &[17; 16],
        "기기 <img src=x onerror=alert(1)>",
    )
    .unwrap();
    let url = qr.encode().unwrap();
    assert_eq!(url.split('#').next().unwrap(), APP_URL);
    assert!(PairingQr::decode(&url).unwrap() == qr);
    for bad in [
        url.replace("#pair=", "?pair="),
        url.replace("https://", "http://"),
        format!("{url}&extra=1"),
    ] {
        assert!(PairingQr::decode(&bad).is_err());
    }
    let mut wrong = qr.clone();
    wrong.room = hide_relay_protocol::admission::room_id(&[0; 32]);
    assert!(wrong.encode().is_err());
    wrong = qr;
    wrong.v = 1;
    assert!(wrong.encode().is_err());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn device_name_is_bounded_utf8_plain_text_inside_the_fragment() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use hide_relay_protocol::Error;
    let key = DeviceKey::generate().unwrap();
    let noise = NoiseKeypair::generate().unwrap();
    let mut qr = PairingQr::new(
        &key.public_key(),
        &noise.public_key(),
        &[17; 16],
        "기기 <img src=x onerror=alert(1)>",
    )
    .unwrap();
    let url = qr.encode().unwrap();
    assert_eq!(url.split('#').next().unwrap(), APP_URL);
    assert_eq!(PairingQr::decode(&url).unwrap().name, qr.name);
    for name in ["a".repeat(128), "한".repeat(42)] {
        qr.name = name.clone();
        assert_eq!(PairingQr::decode(&qr.encode().unwrap()).unwrap().name, name);
    }
    for name in [
        "a".repeat(129),
        "한".repeat(43),
        String::new(),
        " \t".into(),
        "device\nname".into(),
    ] {
        qr.name = name;
        assert_eq!(qr.encode(), Err(Error::InvalidQr));
    }
    qr.name = "valid".into();
    let mut json = serde_json::to_vec(&qr).unwrap();
    let start = json.windows(5).position(|part| part == b"valid").unwrap();
    json[start] = 0xff;
    assert_eq!(
        PairingQr::decode(&format!("{APP_URL}#pair={}", URL_SAFE_NO_PAD.encode(json))).err(),
        Some(Error::InvalidQr)
    );
}
