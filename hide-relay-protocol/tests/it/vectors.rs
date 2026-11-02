//! Cacophony's independent NK/25519/ChaChaPoly/BLAKE2b vector, through snow
//! natively and as WASM. Fixed ephemeral keys exist only in this test binary.
use serde_json::Value;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn official_cacophony_nk_messages_and_handshake_hash() {
    let vector: Value =
        serde_json::from_str(include_str!("../fixtures/cacophony-nk.json")).unwrap();
    assert_eq!(
        vector["protocol_name"],
        hide_relay_protocol::noise::PROTOCOL
    );
    let field = |name: &str| hex::decode(vector[name].as_str().unwrap()).unwrap();
    let init_e = field("init_ephemeral");
    let resp_e = field("resp_ephemeral");
    let resp_s = field("resp_static");
    let remote_s = field("init_remote_static");
    let prologue = field("init_prologue");
    assert_eq!(prologue, field("resp_prologue"));
    let builder = || snow::Builder::new(hide_relay_protocol::noise::PROTOCOL.parse().unwrap());
    let mut init = builder()
        .prologue(&prologue)
        .unwrap()
        .remote_public_key(&remote_s)
        .unwrap()
        .fixed_ephemeral_key_for_testing_only(&init_e)
        .build_initiator()
        .unwrap();
    let mut resp = builder()
        .prologue(&prologue)
        .unwrap()
        .local_private_key(&resp_s)
        .unwrap()
        .fixed_ephemeral_key_for_testing_only(&resp_e)
        .build_responder()
        .unwrap();
    let messages = vector["messages"].as_array().unwrap();
    let mut out = [0; 1024];
    let mut plain = [0; 1024];
    for (i, message) in messages[..2].iter().enumerate() {
        let payload = hex::decode(message["payload"].as_str().unwrap()).unwrap();
        let (writer, reader) = if i == 0 {
            (&mut init, &mut resp)
        } else {
            (&mut resp, &mut init)
        };
        let len = writer.write_message(&payload, &mut out).unwrap();
        assert_eq!(hex::encode(&out[..len]), message["ciphertext"]);
        let read = reader.read_message(&out[..len], &mut plain).unwrap();
        assert_eq!(&plain[..read], &payload);
    }
    assert_eq!(
        hex::encode(init.get_handshake_hash()),
        vector["handshake_hash"]
    );
    assert_eq!(init.get_handshake_hash(), resp.get_handshake_hash());
    let mut init = init.into_transport_mode().unwrap();
    let mut resp = resp.into_transport_mode().unwrap();
    for (i, message) in messages[2..].iter().enumerate() {
        let payload = hex::decode(message["payload"].as_str().unwrap()).unwrap();
        let (writer, reader) = if i % 2 == 0 {
            (&mut init, &mut resp)
        } else {
            (&mut resp, &mut init)
        };
        let len = writer.write_message(&payload, &mut out).unwrap();
        assert_eq!(hex::encode(&out[..len]), message["ciphertext"]);
        let read = reader.read_message(&out[..len], &mut plain).unwrap();
        assert_eq!(&plain[..read], &payload);
    }
}
