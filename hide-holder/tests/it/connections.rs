//! One connection holds a pane at a time (PRD engine-bundle-2-pane-holder
//! D-10, D-11, B9): a connection takes it over only once its hello is
//! accepted, the one it replaces is told so and what it sends after no
//! longer applies, a wrong, slow or foreign opening never disturbs the
//! connection that holds the pane, and every connection the holder does not
//! take is told why.

use std::io::Write;

use hide_holder_protocol::{
    Capability, ClientMessage, CloseReason, Hello, HolderMessage, Opening, PROTOCOL_VERSION,
    read_holder, write_opening,
};
use hide_platform::ipc::LocalStream;

use crate::support::{
    HANG_LIMIT, Holder, Home, current_version, print_ok, shell, source, start_ready, wait_until,
};

fn shell_holder(home: &Home) -> Holder {
    let folder = home.prepare(current_version(), &source());
    start_ready(&folder.program, &home.socket("p"), &shell(), &[])
}

fn stale_line() -> &'static [u8] {
    if cfg!(windows) {
        b"set S=stale& call echo %S%-input\r"
    } else {
        b"S=stale; echo \"$S-input\"\r"
    }
}

#[test]
fn a_new_connection_takes_the_pane_over_and_the_old_ones_input_no_longer_applies() {
    let home = Home::new();
    let holder = shell_holder(&home);
    let mut old = holder.open();
    old.send(ClientMessage::Input(print_ok().to_vec())).unwrap();
    old.wait_for_text("hide-ok");

    let mut new = holder.open();
    assert_eq!(new.welcome.generation, 2);
    let closed = old.wait_closed();
    assert_eq!(closed.reason, CloseReason::TakenOver, "{closed:?}");

    // The old connection's input after the takeover, then the new one's: the
    // program takes input in order, so the new line's output coming without
    // the old line's output shows the old line never reached it.
    let _ = old.send(ClientMessage::Input(stale_line().to_vec()));
    new.send(ClientMessage::Input(print_ok().to_vec())).unwrap();
    // The first line printed hide-ok once; the new one's makes it twice.
    let seen = wait_for_count(&mut new, "hide-ok", 2);
    assert!(!seen.contains("stale-input"), "{seen}");
}

/// Waits until the pane's text holds `text` `count` times, and answers it.
fn wait_for_count(connection: &mut crate::support::Connection, text: &str, count: usize) -> String {
    let started = std::time::Instant::now();
    loop {
        let seen = connection.wait_for_text(text);
        if seen.matches(text).count() >= count {
            return seen;
        }
        assert!(
            started.elapsed() < HANG_LIMIT,
            "{text:?} never came {count} times: {seen}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Opens a raw stream to the holder and answers the holder's reply to what
/// `opening` writes, or to nothing.
fn raw_reply(holder: &Holder, opening: Option<&[u8]>) -> HolderMessage {
    let mut stream =
        LocalStream::connect_until(&holder.socket, std::time::Instant::now() + HANG_LIMIT).unwrap();
    if let Some(bytes) = opening {
        stream.write_all(bytes).unwrap();
    }
    let mut reading = stream.duplicate();
    reading.set_deadline(std::time::Instant::now() + HANG_LIMIT);
    read_holder(&mut reading).unwrap()
}

#[test]
fn a_wrong_or_slow_opening_leaves_the_connection_that_holds_the_pane() {
    let home = Home::new();
    let holder = shell_holder(&home);
    let mut holding = holder.open();

    let not_a_hello = raw_reply(&holder, Some(b"\x10\x00\x00\x00\x01GET / HTTP/1.1\r\n"));
    assert!(
        matches!(&not_a_hello, HolderMessage::Closed(closed) if closed.reason == CloseReason::HelloInvalid),
        "{not_a_hello:?}"
    );
    let too_long = raw_reply(&holder, Some(&(64u32 * 1024).to_le_bytes()));
    assert!(
        matches!(&too_long, HolderMessage::Closed(closed) if closed.reason == CloseReason::FrameTooLarge),
        "{too_long:?}"
    );
    let silent = raw_reply(&holder, None);
    assert!(
        matches!(&silent, HolderMessage::Closed(closed) if closed.reason == CloseReason::HelloTimeout),
        "{silent:?}"
    );

    // A node that speaks only versions this holder does not.
    let mut stream =
        LocalStream::connect_until(&holder.socket, std::time::Instant::now() + HANG_LIMIT).unwrap();
    write_opening(
        &mut stream,
        &Opening::Hello(Hello {
            versions: (PROTOCOL_VERSION + 1, PROTOCOL_VERSION + 5),
            accepts: vec![Capability::Screen],
        }),
    )
    .unwrap();
    stream.set_deadline(std::time::Instant::now() + HANG_LIMIT);
    match read_holder(&mut stream).unwrap() {
        HolderMessage::Closed(closed) => {
            assert_eq!(closed.reason, CloseReason::UnsupportedVersion);
            assert_eq!(closed.version, PROTOCOL_VERSION);
        }
        other => panic!("{other:?}"),
    }

    // None of them took the pane: the first connection still holds it.
    holding
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    holding.wait_for_text("hide-ok");
    assert!(holding.closed.is_none());
    let next = holder.open();
    assert_eq!(
        next.welcome.generation, 2,
        "a refused opening took a generation"
    );
}

#[test]
fn a_size_outside_the_bounds_closes_the_connection_and_the_pane_runs_on() {
    let home = Home::new();
    let holder = shell_holder(&home);
    let mut connection = holder.open();
    connection
        .send(ClientMessage::Resize(hide_holder_protocol::Size {
            rows: hide_holder_protocol::MAX_ROWS + 1,
            cols: 10,
        }))
        .unwrap();
    assert_eq!(connection.wait_closed().reason, CloseReason::SizeInvalid);
    let mut next = holder.open();
    next.send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    next.wait_for_text("hide-ok");
}

/// A stream that has connected and said nothing yet.
fn silent_stream(holder: &Holder) -> LocalStream {
    LocalStream::connect_until(&holder.socket, std::time::Instant::now() + HANG_LIMIT).unwrap()
}

fn hello() -> Opening {
    Opening::Hello(Hello {
        versions: (PROTOCOL_VERSION, PROTOCOL_VERSION),
        accepts: vec![Capability::Screen],
    })
}

#[test]
fn a_connection_past_the_opening_bound_is_told_so_and_the_pane_runs_on() {
    let home = Home::new();
    let holder = shell_holder(&home);
    let mut holding = holder.open();
    // Nine connections that say nothing: eight read their hello at once, the
    // bound, and wait out its bound; the one the holder takes ninth, which on
    // Windows need not be the ninth to connect, is told at once.
    let silent: Vec<LocalStream> = (0..9).map(|_| silent_stream(&holder)).collect();
    let reasons: Vec<CloseReason> = silent
        .into_iter()
        .map(|stream| {
            let mut reading = stream.duplicate();
            reading.set_deadline(std::time::Instant::now() + HANG_LIMIT);
            match read_holder(&mut reading).unwrap() {
                HolderMessage::Closed(closed) => closed.reason,
                other => panic!("{other:?}"),
            }
        })
        .collect();
    let told = |reason: CloseReason| reasons.iter().filter(|seen| **seen == reason).count();
    assert_eq!(told(CloseReason::OpeningFull), 1, "{reasons:?}");
    assert_eq!(told(CloseReason::HelloTimeout), 8, "{reasons:?}");
    holding
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    holding.wait_for_text("hide-ok");
    assert!(holding.closed.is_none());
}

#[test]
fn a_connection_that_opens_while_the_pane_ends_is_told_it_ended() {
    let home = Home::new();
    let holder = shell_holder(&home);
    let mut late = silent_stream(&holder);
    let mut holding = holder.open();
    holding.send(ClientMessage::Terminate).unwrap();
    // The pane has begun to end; the connection that was still opening says
    // its hello only now, within its hello's bound.
    wait_until("the pane's end", || {
        holder.log_text().contains("\"holder.ending\"")
    });
    write_opening(&mut late, &hello()).unwrap();
    late.set_deadline(std::time::Instant::now() + HANG_LIMIT);
    match read_holder(&mut late).unwrap() {
        HolderMessage::Closed(closed) => {
            assert_eq!(closed.reason, CloseReason::Terminated, "{closed:?}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(holding.wait_closed().reason, CloseReason::Terminated);
    holder.wait_gone();
}
