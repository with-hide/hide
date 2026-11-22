//! `hide_platform::sockets`: a TCP stream ended from another thread.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Two ends of one loopback connection.
fn connected() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let near = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (far, _) = listener.accept().unwrap();
    (near, far)
}

/// The peer stays open and silent, so only the abort can end the read. The
/// answer is the same when the abort lands before the read blocks, so a slow
/// thread can make this pass without blocking but never fail.
#[test]
#[allow(clippy::disallowed_methods)] // time for the other thread to block in the call it frees: no portable state says a thread is inside a system call
fn an_abort_from_another_thread_ends_a_blocked_read() {
    let (near, _far) = connected();
    let held = near.try_clone().unwrap();
    let (reading, about_to_read) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut near = near;
        reading.send(()).unwrap();
        let mut buffer = [0_u8; 16];
        let ended = loop {
            match near.read(&mut buffer) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Ok(0) | Err(_) => break true,
                Ok(_) => break false,
            }
        };
        let _ = done.send(ended);
    });
    about_to_read
        .recv_timeout(Duration::from_secs(10))
        .expect("the reading thread never started");
    // Only the last step, from the signal into the read, is left to time.
    thread::sleep(Duration::from_millis(300)); // wait-ok: the reading thread is inside its blocking read
    let started = Instant::now();
    hide_platform::sockets::abort(&held);
    let ended = finished
        .recv_timeout(Duration::from_secs(5))
        .expect("the blocked read outlived the abort");
    assert!(ended, "a read on an aborted socket took bytes");
    assert!(started.elapsed() < Duration::from_secs(2));
    reader.join().unwrap();
}

/// A peer that stops reading holds a writer once the buffers between them
/// are full; an abort from another thread ends that write with an error, and
/// the peer then reads what was sent and the end.
#[test]
fn an_abort_from_another_thread_ends_a_write_a_silent_peer_holds() {
    let (near, mut far) = connected();
    let held = near.try_clone().unwrap();
    let (done, finished) = mpsc::channel();
    let writer = thread::spawn(move || {
        let mut near = near;
        let chunk = vec![7_u8; 64 * 1024];
        // Writes until the peer's silence blocks one: no buffer on any
        // system holds a gigabyte.
        let mut written = 0_usize;
        let result = loop {
            if let Err(error) = near.write_all(&chunk) {
                break Err(error.kind());
            }
            written += chunk.len();
            if written >= 1 << 30 {
                break Ok(written);
            }
        };
        let _ = done.send(result);
    });
    // The writer is blocked once it stops making progress; wait for that
    // rather than for a time.
    assert!(
        finished.recv_timeout(Duration::from_secs(2)).is_err(),
        "the write never blocked on the silent peer"
    );
    let started = Instant::now();
    hide_platform::sockets::abort(&held);
    let result = finished
        .recv_timeout(Duration::from_secs(5))
        .expect("the blocked write outlived the abort");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(result.is_err(), "a freed write fails: {result:?}");
    writer.join().unwrap();
    // The peer's side ends too, whatever it had not read yet.
    far.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        match far.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

/// An abort before the call gives the answer a blocked call gets, and a
/// second abort changes nothing.
#[test]
fn an_abort_before_the_call_ends_it_at_once() {
    let (mut near, _far) = connected();
    hide_platform::sockets::abort(&near);
    hide_platform::sockets::abort(&near);
    let mut buffer = [0_u8; 1];
    assert!(matches!(near.read(&mut buffer), Ok(0) | Err(_)));
    assert!(near.write_all(&[0_u8; 1]).is_err());
}
