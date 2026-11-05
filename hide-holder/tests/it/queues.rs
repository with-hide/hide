//! The bounds on what waits for the pane's program (PRD
//! engine-bundle-2-pane-holder D-17, B14): input past 1 MiB closes the
//! connection that sent it with `input_queue_full` and the pane runs on;
//! answers past 64 KiB are refused with a reason, logged, and told to a
//! connection that accepts notices, while the program's output goes on being
//! read, and the end of the refusals is told with what was refused.
//!
//! The holder is started with its writes to the program held
//! (`HOLD_WRITES`), so the queues fill as they do for a program that reads
//! nothing, on every system alike.

use std::ffi::OsString;
use std::path::PathBuf;

use hide_holder_protocol::{Capability, ClientMessage, CloseReason, MAX_INPUT, NoticeReason};

use crate::support::{Connection, HOLD_WRITES, Holder, Home, Script, node_theme, wait_until};

/// A color question, which every system's terminal passes to the holder to
/// answer from the node's theme (on Windows ConPTY answers none of them
/// itself, `meaning.rs`).
const QUESTION: &[u8] = b"\x1b]11;?\x07";

/// Enough questions that their answers, about 25 bytes each, pass 64 KiB.
const QUESTIONS: usize = 6000;

/// How many single questions follow the burst, each told on its own.
const LATER: usize = 20;

/// A holder whose program, once told (step 1), asks [`QUESTIONS`] questions
/// and says `asked`, then at each later step asks one more and says
/// `again-<step>`; its writes to the program are held until the file it
/// answers exists.
fn questioning(home: &Home) -> (Script, Holder, PathBuf) {
    let mut burst = QUESTION.repeat(QUESTIONS);
    burst.extend_from_slice(b"asked");
    let mut steps = vec![b"ready".to_vec(), burst];
    for step in 2..2 + LATER {
        let mut one = QUESTION.to_vec();
        one.extend_from_slice(format!("again-{step}").as_bytes());
        steps.push(one);
    }
    let script = Script::new(home, steps).with_theme(node_theme());
    let release = home.path().join("release");
    let hold: OsString = release.clone().into();
    let holder = script.start_with(home, &[(HOLD_WRITES, &hold)]);
    (script, holder, release)
}

/// Lets the writes go on, then asks one question at a time until `resumed`
/// says the end of the refusals was told: the queue empties as the program
/// reads, and a question asked before it has is refused like the rest.
fn ask_until_resumed(
    script: &Script,
    connection: &mut Connection,
    release: &std::path::Path,
    resumed: impl Fn(&Connection) -> bool,
) {
    std::fs::write(release, b"").unwrap();
    for step in 2..2 + LATER {
        script.go(step);
        connection.wait_for_text(&format!("again-{step}"));
        connection.drain();
        if resumed(connection) {
            return;
        }
    }
}

fn refused_logged(holder: &Holder) {
    wait_until("the refusal in the holder's log", || {
        holder.logged("holder.answers_refused") == 1
    });
    let log = holder.log_text();
    assert!(log.contains("\"answer_queue_full\""), "{log}");
}

#[test]
fn input_past_its_bound_closes_the_connection_that_sent_it_and_the_pane_runs_on() {
    let home = Home::new();
    let script = Script::new(&home, vec![b"ready".to_vec()]);
    let release = home.path().join("release");
    let hold: OsString = release.clone().into();
    let holder = script.start_with(&home, &[(HOLD_WRITES, &hold)]);
    let mut connection = holder.open();
    let chunk = vec![b'a'; MAX_INPUT];
    // 17 chunks of 64 KiB are past the 1 MiB that may wait; the holder may
    // close the stream before the last is written.
    for _ in 0..17 {
        if connection
            .send(ClientMessage::Input(chunk.clone()))
            .is_err()
        {
            break;
        }
    }
    let closed = connection.wait_closed();
    assert_eq!(closed.reason, CloseReason::InputQueueFull, "{closed:?}");
    assert!(closed.detail.contains("1048576"), "{closed:?}");
    assert_eq!(holder.logged("holder.input_queue_full"), 1);
    assert!(
        holder.process.is_running(),
        "the pane ended with its connection"
    );
    // The pane is there for the next connection.
    std::fs::write(&release, b"").unwrap();
    let mut again = holder.open();
    again.wait_for_text("ready");
    assert!(again.closed.is_none(), "{:?}", again.closed);
}

/// A node that accepts notices is told the answers were refused, with the
/// bound in the detail, and later how many were, once the program takes its
/// answers again; the program's output is read all the while.
#[test]
fn answers_past_their_bound_are_refused_and_told_to_a_node_that_accepts_notices() {
    let home = Home::new();
    let (script, holder, release) = questioning(&home);
    let mut connection = holder.open_accepting(vec![Capability::Screen, Capability::Notices]);
    assert_eq!(
        connection.welcome.sends,
        vec![Capability::Screen, Capability::Notices]
    );
    script.go(1);
    let refused = connection.wait_notice(|notice| notice.reason == NoticeReason::AnswersRefused);
    assert_eq!(refused.count, 1, "{refused:?}");
    assert!(refused.bytes > 0, "{refused:?}");
    assert!(refused.detail.contains("65536"), "{refused:?}");
    refused_logged(&holder);
    // Refused, not waited for: the questions' end was read and drawn.
    connection.wait_for_text("asked");

    let is_resumed =
        |notice: &hide_holder_protocol::Notice| notice.reason == NoticeReason::AnswersResumed;
    ask_until_resumed(&script, &mut connection, &release, |connection| {
        connection.notices.iter().any(is_resumed)
    });
    let resumed = connection.wait_notice(is_resumed);
    assert!(resumed.count >= 1, "{resumed:?}");
    assert!(resumed.bytes >= resumed.count, "{resumed:?}");
    assert_eq!(holder.logged("holder.answers_resumed"), 1);
    assert!(connection.closed.is_none(), "{:?}", connection.closed);
}

/// A node that does not accept notices, as one built before them does not,
/// is sent none and keeps its connection; the refusal is in the holder's
/// log.
#[test]
fn a_node_that_does_not_accept_notices_is_sent_none() {
    let home = Home::new();
    let (script, holder, release) = questioning(&home);
    let mut connection: Connection = holder.open();
    assert_eq!(connection.welcome.sends, vec![Capability::Screen]);
    script.go(1);
    refused_logged(&holder);
    connection.wait_for_text("asked");
    ask_until_resumed(&script, &mut connection, &release, |_| {
        holder.logged("holder.answers_resumed") == 1
    });
    wait_until("the end of the refusals in the holder's log", || {
        holder.logged("holder.answers_resumed") == 1
    });
    connection.drain();
    assert!(connection.notices.is_empty(), "{:?}", connection.notices);
    assert!(connection.closed.is_none(), "{:?}", connection.closed);
}
