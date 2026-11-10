use std::ffi::CStr;
use std::ptr;

use hide_relay_ios::*;
use hide_relay_protocol::noise::{NoiseKeypair, Session};

fn bytes(buf: HideBuf) -> Vec<u8> {
    let copy = if buf.ptr.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec() };
    unsafe { hide_relay_buf_free(buf) };
    copy
}

fn empty() -> HideBuf {
    HideBuf { ptr: ptr::null_mut(), len: 0 }
}

fn unpack(mut packed: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    while !packed.is_empty() {
        let len = u32::from_be_bytes(packed[..4].try_into().unwrap()) as usize;
        frames.push(packed[4..4 + len].to_vec());
        packed = &packed[4 + len..];
    }
    frames
}

/// An app-side session through the C interface and the computer's responder, handshake done.
fn linked() -> (*mut HideRelaySession, Session) {
    let key = NoiseKeypair::generate().unwrap();
    let mut device = Session::responder(&key, 7).unwrap();
    let mut app = ptr::null_mut();
    assert_eq!(unsafe { hide_relay_session_initiator(key.public_key().as_ptr(), 7, &mut app) }, 0);
    let mut first = empty();
    assert_eq!(unsafe { hide_relay_write_handshake(app, &mut first) }, 0);
    device.read_handshake(&bytes(first)).unwrap();
    let second = device.write_handshake().unwrap();
    assert_eq!(unsafe { hide_relay_read_handshake(app, second.as_ptr(), second.len()) }, 0);
    (app, device)
}

#[test]
fn a_message_goes_both_ways_between_the_c_interface_and_the_protocol_crates_responder() {
    let (app, mut device) = linked();
    let mut packed = empty();
    let message = vec![7_u8; 100_000];
    assert_eq!(unsafe { hide_relay_send_message(app, message.as_ptr(), message.len(), &mut packed) }, 0);
    let frames = unpack(&bytes(packed));
    assert!(frames.len() > 1, "a message past one chunk is several frames, in order");
    let mut received = None;
    for frame in &frames {
        received = device.recv_message(frame, 1).unwrap();
    }
    assert_eq!(received.unwrap(), message);

    let reply = device.send_message(br#"{"type":"hello"}"#).unwrap();
    let mut out = empty();
    let mut last = None;
    for frame in &reply {
        assert_eq!(unsafe { hide_relay_recv_message(app, frame.as_ptr(), frame.len(), 2, &mut out) }, 0);
        last = Some(bytes(std::mem::replace(&mut out, empty())));
    }
    assert_eq!(last.unwrap(), br#"{"type":"hello"}"#);
    unsafe { hide_relay_session_free(app) };
}

#[test]
fn a_wrong_call_is_refused_with_a_code_and_never_crashes() {
    let mut app = ptr::null_mut();
    assert_eq!(unsafe { hide_relay_session_initiator(ptr::null(), 7, &mut app) }, 100, "no key");
    assert!(app.is_null());
    assert_eq!(unsafe { hide_relay_session_initiator([0_u8; 32].as_ptr(), 0, &mut app) }, 1, "relay connection zero is invalid");
    let mut out = empty();
    assert_eq!(unsafe { hide_relay_write_handshake(ptr::null_mut(), &mut out) }, 100);
    let (app, _device) = linked();
    let garbage = [1_u8; 40];
    assert_ne!(unsafe { hide_relay_recv_message(app, garbage.as_ptr(), garbage.len(), 1, &mut out) }, 0, "a frame that is not the session's ends it");
    assert_eq!(unsafe { hide_relay_close(app) }, 0);
    let mut after = empty();
    assert_eq!(unsafe { hide_relay_send_message(app, b"x".as_ptr(), 1, &mut after) }, 8, "a closed session sends nothing");
    unsafe { hide_relay_session_free(app) };
}

#[test]
fn control_messages_are_the_protocol_crates_and_the_source_marker_is_recorded_or_says_it_is_not() {
    let hello = br#"{"type":"phone","version":1,"room":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#;
    let mut out = empty();
    assert_eq!(unsafe { hide_relay_client_message(hello.as_ptr(), hello.len(), &mut out) }, 0);
    assert!(String::from_utf8(bytes(out)).unwrap().contains("\"phone\""));
    let junk = br#"{"type":"phone","extra":1}"#;
    assert_ne!(unsafe { hide_relay_client_message(junk.as_ptr(), junk.len(), &mut empty()) }, 0);
    let marker = unsafe { CStr::from_ptr(hide_relay_source().cast()) }.to_str().unwrap();
    assert_eq!(marker, "hide-relay-ios-source:unrecorded", "a build without the script's digest says so, which the release check refuses");
}
