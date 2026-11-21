//! An event reaches the guest's product once, whatever is lost on the way
//! (D-54, D-66).
//!
//! The host's part is played by hand: a box of numbered events that it
//! sends again from where it last heard an acknowledgement. The guest's
//! part is the real inbox record on the real management stream.

use hide_kit::layout;
use hide_peer::PeerStream;
use hide_peer::inbox::{Applied, Apply, Inbox, Recorded};
use hide_peer::loopback::{LoopConn, Net};
use hide_peer::protocol::event::Event;
use hide_peer::protocol::frame::MAX_MESSAGE;
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::manage::Manage;
use hide_peer::protocol::{CloseCode, StreamKind};
use hide_peer::wire::{read_message, write_message};
use serde_json::json;

use crate::{Machine, granted, pair, within};

fn letter(seq: u64) -> Event {
    Event {
        v: 1,
        outbox: "box-1".into(),
        seq,
        kind: "letter".into(),
        key: format!("intent-{seq}"),
        at_unix_ms: 1_795_000_000_000 + seq,
        body: json!({ "to": "agent", "text": format!("letter {seq}") }),
    }
}

/// The product: a letter is taken once per intent, as the core's ledger
/// does.
#[derive(Default)]
struct Letters {
    taken: Vec<String>,
    /// How often the record handed an event over, whatever its intent.
    applies: usize,
}

impl Apply for Letters {
    fn apply(&mut self, event: &Event) -> Applied {
        self.applies += 1;
        if !self.taken.contains(&event.key) {
            self.taken.push(event.key.clone());
        }
        Applied::Done
    }
}

async fn next(stream: &mut PeerStream<LoopConn>) -> Manage {
    let body = within(
        "a management message",
        read_message(&mut stream.recv, MAX_MESSAGE),
    )
    .await
    .unwrap()
    .expect("the stream is open");
    Manage::parse(&body)
        .unwrap()
        .expect("a message this build knows")
}

/// The guest's side of one event: record it, acknowledge it, then apply.
async fn receive(
    stream: &mut PeerStream<LoopConn>,
    inbox: &mut Inbox,
    letters: &mut Letters,
) -> Recorded {
    let Manage::Event(event) = next(stream).await else {
        panic!("the host sends events");
    };
    let outbox = event.outbox.clone();
    let recorded = inbox.record(event).unwrap();
    if let Recorded::New { seq } | Recorded::Again { seq } = recorded {
        write_message(&mut stream.send, &Manage::EventAck { outbox, seq })
            .await
            .unwrap();
    }
    inbox.apply_recorded(letters).unwrap();
    recorded
}

#[tokio::test]
async fn an_event_resent_after_a_lost_acknowledgement_is_applied_once() {
    let net = Net::new();
    let mut host = Machine::new(&net, "windows-pc");
    let mut guest = Machine::new(&net, "macbook");
    pair(&host, &guest);
    let inbox_dir = layout::peer_inbox(guest.dir.path(), &host.key());
    let mut inbox = Inbox::open(&inbox_dir).unwrap();
    let mut letters = Letters::default();

    // First connection: the guest says where it stands and takes two
    // letters.
    let link = guest.peers.dial(&host.addr()).await.unwrap();
    granted(&mut guest, host.key(), GrantState::Active).await;
    let mut to_host = link.open(StreamKind::Management).await.unwrap();
    let (outbox, seq) = inbox.position();
    let resume = Manage::Resume {
        outbox: outbox.to_owned(),
        seq,
    };
    write_message(&mut to_host.send, &resume).await.unwrap();
    let mut to_guest = host.accepted(StreamKind::Management).await;
    assert_eq!(
        next(&mut to_guest).await,
        Manage::Resume {
            outbox: String::new(),
            seq: 0
        }
    );
    for seq in [1, 2] {
        write_message(&mut to_guest.send, &Manage::Event(letter(seq)))
            .await
            .unwrap();
        assert_eq!(
            receive(&mut to_host, &mut inbox, &mut letters).await,
            Recorded::New { seq }
        );
    }
    // The host hears the first acknowledgement, and the connection is lost
    // before it reads the second.
    assert_eq!(
        next(&mut to_guest).await,
        Manage::EventAck {
            outbox: "box-1".into(),
            seq: 1
        }
    );
    link.close(CloseCode::Lost);
    within("the connection's end", link.closed()).await;
    drop((to_host, to_guest));

    // The guest's daemon also stops here and starts again: what it knows
    // is what its record holds.
    drop(inbox);
    let mut inbox = Inbox::open(&inbox_dir).unwrap();
    assert_eq!(inbox.position(), ("box-1", 2));

    // Second connection. The host sends again from the last acknowledgement
    // it heard, so letter 2 arrives a second time, then letter 3.
    let link = guest.peers.dial(&host.addr()).await.unwrap();
    granted(&mut guest, host.key(), GrantState::Active).await;
    let mut to_host = link.open(StreamKind::Management).await.unwrap();
    let mut to_guest = host.accepted(StreamKind::Management).await;
    for seq in [2, 3] {
        write_message(&mut to_guest.send, &Manage::Event(letter(seq)))
            .await
            .unwrap();
    }
    assert_eq!(
        receive(&mut to_host, &mut inbox, &mut letters).await,
        Recorded::Again { seq: 2 }
    );
    assert_eq!(
        receive(&mut to_host, &mut inbox, &mut letters).await,
        Recorded::New { seq: 3 }
    );
    // Both are acknowledged, so the host's box empties.
    for seq in [2, 3] {
        assert_eq!(
            next(&mut to_guest).await,
            Manage::EventAck {
                outbox: "box-1".into(),
                seq
            }
        );
    }
    assert_eq!(letters.taken, ["intent-1", "intent-2", "intent-3"]);
    assert_eq!(
        letters.applies, 3,
        "the record itself hands each event over once"
    );
}
