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
    let qr = PairingQr::new(&key.public_key(), &noise.public, &[17; 16]).unwrap();
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
