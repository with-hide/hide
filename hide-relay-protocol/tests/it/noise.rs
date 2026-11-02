use hide_relay_protocol::{
    Error,
    frame::{Frame, Kind, MAX_FRAME, MAX_PAYLOAD},
    noise::{NoiseKeypair, Session},
};

fn pair() -> (Session, Session) {
    let key = NoiseKeypair::generate().unwrap();
    let mut phone = Session::initiator(&key.public_key(), 7).unwrap();
    let mut device = Session::responder(&key, 7).unwrap();
    let first = phone.write_handshake().unwrap();
    assert_eq!(first.len(), 54);
    device.read_handshake(&first).unwrap();
    phone
        .read_handshake(&device.write_handshake().unwrap())
        .unwrap();
    assert!(phone.is_transport() && device.is_transport());
    (phone, device)
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn bidirectional_payloads_and_exact_envelope_cap() {
    let (mut phone, mut device) = pair();
    let plaintext = vec![42; 65_506];
    let encrypted = phone.send_message(&plaintext).unwrap().remove(0);
    assert_eq!(encrypted.len(), 65_536);
    assert_eq!(
        device.recv_message(&encrypted, 0).unwrap().unwrap(),
        plaintext
    );
    let reply = device
        .send_message(b"credential accepted")
        .unwrap()
        .remove(0);
    assert_eq!(
        phone.recv_message(&reply, 0).unwrap().unwrap(),
        b"credential accepted"
    );
    assert_eq!(
        Frame {
            kind: Kind::Transport,
            connection: 7,
            payload: &vec![0; MAX_PAYLOAD + 1]
        }
        .encode(),
        Err(Error::SizeLimit)
    );
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn tampering_replay_reordering_and_wrong_envelopes_end_sessions() {
    for fault in 0..5 {
        let (mut phone, mut device) = pair();
        let mut frame = phone.send_message(b"private reply").unwrap().remove(0);
        match fault {
            0 => {
                *frame.last_mut().unwrap() ^= 1;
            }
            1 => {
                device.recv_message(&frame, 0).unwrap();
            }
            2 => {
                frame = phone.send_message(b"out of order").unwrap().remove(0);
            }
            3 => {
                frame[5] = 8;
            }
            4 => {
                frame[1] = 1;
            }
            _ => unreachable!(),
        }
        assert!(device.recv_message(&frame, 0).is_err());
        assert_eq!(device.recv_message(&frame, 0), Err(Error::SessionClosed));
        assert_eq!(device.send_message(b"retry"), Err(Error::SessionClosed));
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn handshake_authenticates_key_and_connection_without_carrying_user_data() {
    let key = NoiseKeypair::generate().unwrap();
    let other = NoiseKeypair::generate().unwrap();
    for (public, connection) in [(other.public_key(), 7), (key.public_key(), 8)] {
        let mut phone = Session::initiator(&public, connection).unwrap();
        let mut device = Session::responder(&key, 7).unwrap();
        assert!(
            device
                .read_handshake(&phone.write_handshake().unwrap())
                .is_err()
        );
        assert_eq!(device.write_handshake(), Err(Error::SessionClosed));
    }
    let mut phone = Session::initiator(&key.public_key(), 7).unwrap();
    assert_eq!(phone.send_message(b"credential"), Err(Error::InvalidState));
    assert_eq!(phone.write_handshake(), Err(Error::SessionClosed));
    let mut a = Session::initiator(&key.public_key(), 7).unwrap();
    let mut b = Session::initiator(&key.public_key(), 7).unwrap();
    assert_ne!(a.write_handshake().unwrap(), b.write_handshake().unwrap());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn messages_are_bounded_ordered_and_expire_without_extending_the_deadline() {
    use hide_relay_protocol::noise::{MAX_MESSAGE, REASSEMBLY_TIMEOUT_MS};
    let (mut phone, mut device) = pair();
    for message in [vec![], vec![42; 256 * 1024], b"next message".to_vec()] {
        let frames = phone.send_message(&message).unwrap();
        let mut result = None;
        for frame in &frames {
            assert!(frame.len() <= MAX_FRAME);
            assert!(result.is_none());
            result = device.recv_message(frame, 100).unwrap();
        }
        assert_eq!(result.unwrap(), message);
    }
    assert_eq!(
        phone.send_message(&vec![0; 256 * 1024 + 1]),
        Err(Error::SizeLimit)
    );
    assert_eq!(phone.send_message(b"again"), Err(Error::SessionClosed));
    for fault in 0..4 {
        let (mut phone, mut device) = pair();
        let frames = phone.send_message(&vec![7; MAX_MESSAGE]).unwrap();
        assert_eq!(device.recv_message(&frames[0], 100).unwrap(), None);
        match fault {
            0 => assert_eq!(
                device.recv_message(&frames[2], 101),
                Err(Error::AuthenticationFailed)
            ),
            1 => {
                assert_eq!(
                    device
                        .recv_message(&frames[1], 100 + REASSEMBLY_TIMEOUT_MS - 1)
                        .unwrap(),
                    None
                );
                assert_eq!(
                    device.check_timeout(100 + REASSEMBLY_TIMEOUT_MS),
                    Err(Error::Timeout)
                );
            }
            2 => assert_eq!(
                device.recv_message(&frames[1], 99),
                Err(Error::InvalidState)
            ),
            3 => {
                // A malicious authenticated peer starts another message early.
                let nested = phone.send_message(b"nested").unwrap();
                assert!(device.recv_message(&nested[0], 101).is_err());
            }
            _ => unreachable!(),
        }
        assert_eq!(
            device.recv_message(&frames[1], 102),
            Err(Error::SessionClosed)
        );
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn persisted_noise_scalar_restores_the_independent_vector_public_key() {
    let vector: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/cacophony-nk.json")).unwrap();
    let raw: [u8; 32] = hex::decode(vector["resp_static"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let key = NoiseKeypair::from_secret(&raw).unwrap();
    assert_eq!(hex::encode(key.public_key()), vector["init_remote_static"]);
    assert_eq!(*key.secret(), raw);
    let mut phone = Session::initiator(&key.public_key(), 7).unwrap();
    let mut device = Session::responder(&key, 7).unwrap();
    device
        .read_handshake(&phone.write_handshake().unwrap())
        .unwrap();
    phone
        .read_handshake(&device.write_handshake().unwrap())
        .unwrap();
    let frame = phone.send_message(b"restored device").unwrap().remove(0);
    assert_eq!(
        device.recv_message(&frame, 0).unwrap().unwrap(),
        b"restored device"
    );
}
