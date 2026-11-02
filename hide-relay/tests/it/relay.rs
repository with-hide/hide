use super::support::*;
use futures_util::SinkExt;
use hide_relay::{
    config::Config,
    limits::{DAY, IP_WINDOW, Limits, MAX_FRAME},
};
use hide_relay_protocol::{
    Error,
    admission::{ClientMessage, DeviceHello, DeviceKey, Refusal, ServerMessage},
    frame::{Frame, Kind},
};
use std::{sync::atomic::Ordering, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

#[tokio::test]
async fn pairs_multiple_phones_and_forwards_opaque_messages_in_both_directions() {
    let mut fixture = Fixture::start(Limits::default()).await;
    let mut device = fixture.socket("192.0.2.1").await;
    let key = DeviceKey::from_seed(&[1; 32]);
    let hello = register(&mut device, &key).await;
    let mut phone = fixture.socket("192.0.2.2").await;
    let connection = join(&mut phone, &hello.room, &mut device).await;
    let (mut phone_noise, mut device_noise) = noise(&mut phone, &mut device, connection).await;
    let mut other = fixture.socket("192.0.2.3").await;
    let other_id = join(&mut other, &hello.room, &mut device).await;
    assert_ne!(connection, other_id);
    let (mut other_noise, mut other_device) = noise(&mut other, &mut device, other_id).await;
    exchange(
        &mut phone,
        &mut device,
        &mut phone_noise,
        &mut device_noise,
        b"private pairing credential, never log",
    )
    .await;
    exchange(
        &mut device,
        &mut phone,
        &mut device_noise,
        &mut phone_noise,
        &vec![42; 256 * 1024],
    )
    .await;
    exchange(
        &mut other,
        &mut device,
        &mut other_noise,
        &mut other_device,
        b"other phone",
    )
    .await;
    send(&mut device, ClientMessage::Close { connection }).await;
    assert_eq!(
        control(&mut device).await,
        ServerMessage::PeerLeft { connection }
    );
    closed(&mut phone, 1000).await;
    // A response already being prepared must not tear down the device and its
    // other phones when the destination departs first.
    let late = device_noise
        .send_message(b"late response")
        .unwrap()
        .remove(0);
    device.send(Message::Binary(late.into())).await.unwrap();
    assert_eq!(
        control(&mut device).await,
        ServerMessage::PeerLeft { connection }
    );
    send(&mut device, ClientMessage::Close { connection }).await;
    device.send(Message::Ping(vec![1].into())).await.unwrap();
    assert_eq!(next(&mut device).await, Message::Pong(vec![1].into()));
    let mut replacement = fixture.socket("192.0.2.2").await;
    assert!(join(&mut replacement, &hello.room, &mut device).await > other_id);
    exchange(
        &mut device,
        &mut other,
        &mut other_device,
        &mut other_noise,
        b"still live",
    )
    .await;
    fixture.stop().await;
    let logs = serde_json::to_string(&*fixture.events.lock().unwrap()).unwrap();
    for secret in ["private pairing credential", "192.0.2.", &hello.device_key] {
        assert!(!logs.contains(secret));
    }
    for event in fixture.events.lock().unwrap().iter() {
        let value = serde_json::to_value(event).unwrap();
        assert!(
            value
                .as_object()
                .unwrap()
                .keys()
                .all(|k| ["event", "room", "size", "cap"].contains(&k.as_str()))
        );
    }
}

#[tokio::test]
async fn tamper_and_replay_close_only_the_noise_peer_and_other_phone_keeps_working() {
    for tamper in [true, false] {
        let mut fixture = Fixture::start(Limits::default()).await;
        let mut device = fixture.socket("192.0.2.1").await;
        let hello = register(&mut device, &DeviceKey::from_seed(&[2; 32])).await;
        let mut bad = fixture.socket("192.0.2.2").await;
        let bad_id = join(&mut bad, &hello.room, &mut device).await;
        let (mut bad_noise, mut bad_device) = noise(&mut bad, &mut device, bad_id).await;
        let mut good = fixture.socket("192.0.2.3").await;
        let good_id = join(&mut good, &hello.room, &mut device).await;
        let (mut good_noise, mut good_device) = noise(&mut good, &mut device, good_id).await;
        let mut frame = bad_noise
            .send_message(b"do not deliver this twice")
            .unwrap()
            .remove(0);
        if tamper {
            *frame.last_mut().unwrap() ^= 1;
        } else {
            bad.send(Message::Binary(frame.clone().into()))
                .await
                .unwrap();
            assert_eq!(
                bad_device
                    .recv_message(&binary(&mut device).await, 0)
                    .unwrap(),
                Some(b"do not deliver this twice".to_vec())
            );
        }
        bad.send(Message::Binary(frame.clone().into()))
            .await
            .unwrap();
        let received = binary(&mut device).await;
        assert_eq!(
            received, frame,
            "relay must not pretend it can authenticate ciphertext"
        );
        assert_eq!(
            bad_device.recv_message(&received, 1),
            Err(Error::AuthenticationFailed)
        );
        // Real endpoint failure tears down the logical peer over the public
        // control API; the relay never learns its encrypted contents or cause.
        send(&mut device, ClientMessage::Close { connection: bad_id }).await;
        assert_eq!(
            control(&mut device).await,
            ServerMessage::PeerLeft { connection: bad_id }
        );
        closed(&mut bad, 1000).await;
        exchange(
            &mut good,
            &mut device,
            &mut good_noise,
            &mut good_device,
            b"unaffected",
        )
        .await;
        fixture.stop().await;
        assert!(
            fixture
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.event == "peer_closed")
        );
    }
}

#[tokio::test]
async fn rejects_bad_replayed_and_expired_admission_without_occupying_room() {
    let mut fixture = Fixture::start(Limits::default()).await;
    let key = DeviceKey::from_seed(&[3; 32]);
    let hello = DeviceHello::new(&key.public_key(), 7).unwrap();
    let mut first = fixture.socket("192.0.2.1").await;
    send(
        &mut first,
        ClientMessage::Device {
            hello: hello.clone(),
        },
    )
    .await;
    let ServerMessage::Challenge { challenge } = control(&mut first).await else {
        panic!()
    };
    let signature = key.sign(&hello, &challenge).unwrap();
    let mut second = fixture.socket("192.0.2.1").await;
    send(
        &mut second,
        ClientMessage::Device {
            hello: hello.clone(),
        },
    )
    .await;
    let _ = control(&mut second).await;
    send(
        &mut second,
        ClientMessage::Proof {
            signature: signature.clone(),
        },
    )
    .await;
    refused(&mut second, Refusal::InvalidProof).await;
    closed(&mut second, 1008).await;
    let mut phone = fixture.socket("192.0.2.2").await;
    send(
        &mut phone,
        ClientMessage::Phone {
            version: 1,
            room: hello.room.clone(),
        },
    )
    .await;
    refused(&mut phone, Refusal::DeviceOffline).await;
    fixture.clock.store(10_000, Ordering::SeqCst);
    send(&mut first, ClientMessage::Proof { signature }).await;
    refused(&mut first, Refusal::Timeout).await;
    let mut valid = fixture.socket("192.0.2.1").await;
    register(&mut valid, &key).await;
    let mut occupied = fixture.socket("192.0.2.1").await;
    send(
        &mut occupied,
        ClientMessage::Device {
            hello: hello.clone(),
        },
    )
    .await;
    let ServerMessage::Challenge { challenge } = control(&mut occupied).await else {
        panic!()
    };
    send(
        &mut occupied,
        ClientMessage::Proof {
            signature: key.sign(&hello, &challenge).unwrap(),
        },
    )
    .await;
    refused(&mut occupied, Refusal::RoomOccupied).await;
    fixture.stop().await;
    let logs = fixture.events.lock().unwrap();
    assert!(logs.iter().any(|e| e.event == "invalid_proof"));
    assert!(logs.iter().any(|e| e.event == "admission_timeout"));
}

#[tokio::test]
async fn refuses_total_connections_and_health_remains_available() {
    let mut fixture = Fixture::start(Limits {
        connections: 2,
        ..Limits::default()
    })
    .await;
    let mut device = fixture.socket("192.0.2.1").await;
    let hello = register(&mut device, &DeviceKey::from_seed(&[4; 32])).await;
    let mut phone = fixture.socket("192.0.2.2").await;
    let id = join(&mut phone, &hello.room, &mut device).await;
    let mut excess = fixture.socket("192.0.2.3").await;
    refused(&mut excess, Refusal::ConnectionLimit).await;
    closed(&mut excess, 1013).await;
    let mut http = TcpStream::connect(fixture.address).await.unwrap();
    http.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    timeout(Duration::from_secs(15), http.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.ends_with("ok\n"));
    let (mut p, mut d) = noise(&mut phone, &mut device, id).await;
    exchange(
        &mut phone,
        &mut device,
        &mut p,
        &mut d,
        b"survives connection limit",
    )
    .await;
    fixture.cap("connections");
    fixture.stop().await;
}

#[tokio::test]
async fn refuses_fifth_phone_and_keeps_existing_peers() {
    let mut fixture = Fixture::start(Limits::default()).await;
    let mut device = fixture.socket("192.0.2.1").await;
    let hello = register(&mut device, &DeviceKey::from_seed(&[5; 32])).await;
    let mut phones = Vec::new();
    for n in 0..4 {
        let mut phone = fixture.socket(&format!("192.0.2.{}", n + 2)).await;
        let id = join(&mut phone, &hello.room, &mut device).await;
        phones.push((phone, id));
    }
    let mut fifth = fixture.socket("192.0.2.9").await;
    send(
        &mut fifth,
        ClientMessage::Phone {
            version: 1,
            room: hello.room.clone(),
        },
    )
    .await;
    refused(&mut fifth, Refusal::RoomLimit).await;
    closed(&mut fifth, 1013).await;
    let first_id = phones[0].1;
    let (mut p, mut d) = noise(&mut phones[0].0, &mut device, first_id).await;
    exchange(
        &mut phones[0].0,
        &mut device,
        &mut p,
        &mut d,
        b"room still live",
    )
    .await;
    fixture.cap("phones_per_room");
    fixture.stop().await;
}

#[tokio::test]
async fn ip_window_and_record_cap_do_not_reset_on_reconnect() {
    let mut fixture = Fixture::start(Limits {
        connections_per_ip: 2,
        ip_records: 2,
        ..Limits::default()
    })
    .await;
    for _ in 0..2 {
        let mut socket = fixture.socket("192.0.2.1").await;
        send(
            &mut socket,
            ClientMessage::Phone {
                version: 1,
                room: DeviceHello::new(&DeviceKey::from_seed(&[6; 32]).public_key(), 7)
                    .unwrap()
                    .room,
            },
        )
        .await;
        refused(&mut socket, Refusal::DeviceOffline).await;
        closed(&mut socket, 1008).await;
    }
    fixture
        .clock
        .store(IP_WINDOW.as_millis() as u64 - 1, Ordering::SeqCst);
    refused(&mut fixture.socket("192.0.2.1").await, Refusal::RateLimit).await;
    let _other = fixture.socket("192.0.2.2").await;
    refused(&mut fixture.socket("192.0.2.3").await, Refusal::RateLimit).await;
    fixture.cap("connections_per_ip");
    fixture.cap("ip_records");
    fixture
        .clock
        .store(IP_WINDOW.as_millis() as u64, Ordering::SeqCst);
    let mut reset = fixture.socket("192.0.2.1").await;
    register(&mut reset, &DeviceKey::from_seed(&[6; 32])).await;
    fixture.stop().await;
}

#[tokio::test]
async fn daily_bytes_survive_reconnect_and_reset_at_day_boundary() {
    // Four 54-byte handshakes plus a 31-byte authenticated message fit
    // exactly. The first additional byte of traffic must be refused.
    let mut fixture = Fixture::start(Limits {
        bytes_per_room_per_day: 247,
        ..Limits::default()
    })
    .await;
    let key = DeviceKey::from_seed(&[7; 32]);
    let mut device = fixture.socket("192.0.2.1").await;
    let hello = register(&mut device, &key).await;
    let mut phone = fixture.socket("192.0.2.2").await;
    let id = join(&mut phone, &hello.room, &mut device).await;
    let (mut p, mut d) = noise(&mut phone, &mut device, id).await;
    let mut other = fixture.socket("192.0.2.3").await;
    let other_id = join(&mut other, &hello.room, &mut device).await;
    let (_op, _od) = noise(&mut other, &mut device, other_id).await;
    exchange(&mut phone, &mut device, &mut p, &mut d, b"x").await;
    let rejected_frame = p.send_message(b"y").unwrap().remove(0);
    phone
        .send(Message::Binary(rejected_frame.into()))
        .await
        .unwrap();
    refused(&mut phone, Refusal::DailyLimit).await;
    closed(&mut phone, 1013).await;
    assert_eq!(
        control(&mut device).await,
        ServerMessage::PeerLeft { connection: id }
    );
    other.send(Message::Ping(vec![1].into())).await.unwrap();
    assert!(matches!(next(&mut other).await, Message::Pong(_)));
    fixture.cap("bytes_per_room_per_day");
    device.close(None).await.unwrap();
    refused(&mut other, Refusal::DeviceOffline).await;
    let mut device = fixture.socket("192.0.2.1").await;
    register(&mut device, &key).await;
    let mut phone = fixture.socket("192.0.2.2").await;
    let id = join(&mut phone, &hello.room, &mut device).await;
    fixture
        .clock
        .store(DAY.as_millis() as u64 - 1, Ordering::SeqCst);
    let frame = Frame {
        kind: Kind::Handshake,
        connection: id,
        payload: &[0; 48],
    }
    .encode()
    .unwrap();
    phone.send(Message::Binary(frame.into())).await.unwrap();
    refused(&mut phone, Refusal::DailyLimit).await;
    assert_eq!(
        control(&mut device).await,
        ServerMessage::PeerLeft { connection: id }
    );
    fixture
        .clock
        .store(DAY.as_millis() as u64, Ordering::SeqCst);
    let mut phone = fixture.socket("192.0.2.2").await;
    let id = join(&mut phone, &hello.room, &mut device).await;
    noise(&mut phone, &mut device, id).await;
    fixture.stop().await;
}

#[tokio::test]
async fn frame_limit_and_routing_violations_close_only_sender() {
    for case in ["oversize", "malformed", "foreign_id"] {
        let mut fixture = Fixture::start(Limits::default()).await;
        let mut device = fixture.socket("192.0.2.1").await;
        let hello = register(&mut device, &DeviceKey::from_seed(&[8; 32])).await;
        let mut phone = fixture.socket("192.0.2.2").await;
        let id = join(&mut phone, &hello.room, &mut device).await;
        let mut good = fixture.socket("192.0.2.3").await;
        let good_id = join(&mut good, &hello.room, &mut device).await;
        let (mut p, mut d) = noise(&mut good, &mut device, good_id).await;
        let mut frame = Frame {
            kind: Kind::Transport,
            connection: id,
            payload: &vec![0; MAX_FRAME - 6],
        }
        .encode()
        .unwrap();
        phone
            .send(Message::Binary(frame.clone().into()))
            .await
            .unwrap();
        assert_eq!(binary(&mut device).await.len(), MAX_FRAME);
        match case {
            "oversize" => frame.push(0),
            "malformed" => frame[1] = 99,
            _ => frame[2..6].copy_from_slice(&good_id.to_be_bytes()),
        }
        phone.send(Message::Binary(frame.into())).await.unwrap();
        refused(
            &mut phone,
            if case == "oversize" {
                Refusal::FrameLimit
            } else {
                Refusal::InvalidMessage
            },
        )
        .await;
        closed(&mut phone, if case == "oversize" { 1013 } else { 1008 }).await;
        drop(phone);
        assert_eq!(
            control(&mut device).await,
            ServerMessage::PeerLeft { connection: id }
        );
        exchange(&mut good, &mut device, &mut p, &mut d, b"healthy").await;
        if case == "oversize" {
            fixture.cap("frame_size");
        }
        fixture.stop().await;
    }
}

#[tokio::test]
async fn restart_closes_everyone_and_retains_no_room_or_quota() {
    let mut fixture = Fixture::start(Limits::default()).await;
    let address = fixture.address;
    let key = DeviceKey::from_seed(&[9; 32]);
    let mut device = fixture.socket("192.0.2.1").await;
    let hello = register(&mut device, &key).await;
    let mut phone = fixture.socket("192.0.2.2").await;
    let id = join(&mut phone, &hello.room, &mut device).await;
    noise(&mut phone, &mut device, id).await;
    fixture.stop().await;
    closed(&mut device, 1012).await;
    closed(&mut phone, 1012).await;
    let mut restarted = Fixture::at(Limits::default(), address).await;
    let mut orphan = restarted.socket("192.0.2.2").await;
    send(
        &mut orphan,
        ClientMessage::Phone {
            version: 1,
            room: hello.room.clone(),
        },
    )
    .await;
    refused(&mut orphan, Refusal::DeviceOffline).await;
    let mut device = restarted.socket("192.0.2.1").await;
    register(&mut device, &key).await;
    let mut phone = restarted.socket("192.0.2.2").await;
    let new_id = join(&mut phone, &hello.room, &mut device).await;
    let (mut p, mut d) = noise(&mut phone, &mut device, new_id).await;
    exchange(&mut phone, &mut device, &mut p, &mut d, b"new session").await;
    restarted.stop().await;
}

#[tokio::test]
async fn retained_room_records_are_bounded() {
    let mut fixture = Fixture::start(Limits {
        room_records: 1,
        ..Limits::default()
    })
    .await;
    let mut first = fixture.socket("192.0.2.1").await;
    let hello = register(&mut first, &DeviceKey::from_seed(&[10; 32])).await;
    let mut phone = fixture.socket("192.0.2.2").await;
    join(&mut phone, &hello.room, &mut first).await;
    first.close(None).await.unwrap();
    refused(&mut phone, Refusal::DeviceOffline).await;
    let key = DeviceKey::from_seed(&[11; 32]);
    let hello = DeviceHello::new(&key.public_key(), 7).unwrap();
    let mut excess = fixture.socket("192.0.2.3").await;
    send(
        &mut excess,
        ClientMessage::Device {
            hello: hello.clone(),
        },
    )
    .await;
    let ServerMessage::Challenge { challenge } = control(&mut excess).await else {
        panic!()
    };
    send(
        &mut excess,
        ClientMessage::Proof {
            signature: key.sign(&hello, &challenge).unwrap(),
        },
    )
    .await;
    refused(&mut excess, Refusal::ConnectionLimit).await;
    fixture.cap("room_records");
    fixture
        .clock
        .store(DAY.as_millis() as u64, Ordering::SeqCst);
    register(&mut fixture.socket("192.0.2.3").await, &key).await;
    fixture.stop().await;
}

#[tokio::test]
async fn trusted_proxy_requires_a_single_valid_address() {
    let mut fixture = Fixture::start(Limits::default()).await;
    for value in [None, Some("secret, 192.0.2.1"), Some("secret")] {
        let mut request = format!("ws://{}/ws", fixture.address)
            .into_client_request()
            .unwrap();
        if let Some(value) = value {
            request
                .headers_mut()
                .insert("x-real-ip", value.parse().unwrap());
        }
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        refused(&mut socket, Refusal::InvalidMessage).await;
        closed(&mut socket, 1008).await;
    }
    fixture.stop().await;
    assert!(
        !serde_json::to_string(&*fixture.events.lock().unwrap())
            .unwrap()
            .contains("secret")
    );
}

#[test]
fn cli_config_rejects_unknown_duplicate_and_malformed_arguments_without_values() {
    let config = Config::parse(Vec::new()).unwrap().unwrap();
    assert!(config.listen.ip().is_loopback());
    for args in [
        vec!["--unknown", "secret"],
        vec!["--listen", "secret"],
        vec!["--trusted-proxy", "secret"],
        vec!["--listen", "127.0.0.1:9", "--listen", "127.0.0.1:10"],
    ] {
        let error = Config::parse(args.into_iter().map(str::to_owned))
            .err()
            .unwrap();
        assert!(!error.contains("secret"));
    }
}
