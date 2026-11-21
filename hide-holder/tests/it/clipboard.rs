//! What a program writes to the clipboard (OSC 52) reaches the connection
//! that holds the pane once, apart from the screen
//! (with-hide/hide-private#659): a redraw or a connection that takes the
//! pane over later never gets an older write, and a node built before the
//! clipboard was sent any is sent none and keeps its pane.

use hide_holder_protocol::Capability;

use crate::support::{Home, Script, wait_until};

/// A copy of `text`, as Codex and pi send it, then `marker` drawn after it.
fn copy(text: &str, marker: &str) -> Vec<u8> {
    format!(
        "\x1b]52;c;{}\x07{marker}\r\n",
        hide_node_link::terminal::encode_base64(text.as_bytes())
    )
    .into_bytes()
}

const ACCEPTS: [Capability; 2] = [Capability::Screen, Capability::Clipboard];

#[test]
fn a_programs_copy_reaches_the_holding_connection_once() {
    let home = Home::new();
    let script = Script::new(
        &home,
        vec![
            b"ready\r\n".to_vec(),
            copy("first copy", "copied-1"),
            copy("second copy", "copied-2"),
        ],
    );
    let holder = script.start(&home);
    let mut first = holder.open_accepting(ACCEPTS.to_vec());
    assert!(first.welcome.sends.contains(&Capability::Clipboard));

    // Step zero runs at once. Hold the copy at step one until this
    // connection is holding the pane and has drawn the ready screen.
    first.wait_for_drawn("ready");
    assert!(first.clipboards.is_empty());
    script.go(1);
    first.wait_for_drawn("copied-1");
    wait_until("the copy to arrive", || {
        first.drain();
        !first.clipboards.is_empty()
    });
    // The screen drawn whole again carries no copy. The holder sends what
    // waits ahead of the screen, so all it sent before is in by the time the
    // whole screen is.
    first.redraw();
    assert_eq!(first.clipboards, vec![b"first copy".to_vec()]);

    // A connection that takes the pane over (a reattach, a node that
    // started again) draws the screen whole and is never sent the old copy.
    let mut second = holder.open_accepting(ACCEPTS.to_vec());
    second.wait_for_drawn("copied-1");
    first.wait_closed();
    assert!(second.clipboards.is_empty(), "{:?}", second.clipboards);

    script.go(2);
    second.wait_for_drawn("copied-2");
    wait_until("the second copy to arrive", || {
        second.drain();
        !second.clipboards.is_empty()
    });
    second.redraw();
    assert_eq!(second.clipboards, vec![b"second copy".to_vec()]);
    assert_eq!(first.clipboards, vec![b"first copy".to_vec()]);
}

/// A node built before the clipboard (it accepts only the screen) is sent
/// no copy, which it could not read, and keeps drawing the pane.
#[test]
fn a_node_that_does_not_accept_copies_is_sent_none_and_keeps_its_pane() {
    let home = Home::new();
    let script = Script::new(
        &home,
        vec![
            b"ready\r\n".to_vec(),
            copy("unsent", "copied-1"),
            b"after\r\n".to_vec(),
        ],
    );
    let holder = script.start(&home);
    let mut connection = holder.open();
    assert!(!connection.welcome.sends.contains(&Capability::Clipboard));

    connection.wait_for_drawn("ready");
    script.go(1);
    connection.wait_for_drawn("copied-1");
    holder.wait_logged("holder.clipboard_dropped", 1, &mut connection);
    script.go(2);
    connection.wait_for_drawn("after");
    assert!(connection.closed.is_none(), "{:?}", connection.closed);
    assert!(connection.clipboards.is_empty());
}
