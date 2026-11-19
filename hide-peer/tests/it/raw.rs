//! The contract seen from a peer that speaks it by hand: the first
//! exchange, a stream's header and the answer to it (D-62, D-65).

use hide_kit::layout;
use hide_peer::limits;
use hide_peer::loopback::{LoopConn, Net};
use hide_peer::peers::PeerEvent;
use hide_peer::protocol::hello::{self, GrantState, Hello, HelloFrame};
use hide_peer::protocol::stream::{self, Status};
use hide_peer::protocol::{CloseCode, StreamKind};
use hide_peer::store::Via;
use hide_peer::wire::read_message;
use hide_peer::{MachineKey, PeerConn, PeerTransport};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Machine, within};

/// Opens a stream that names `tag` and `generation` and returns the answer.
async fn open(conn: &LoopConn, tag: u8, generation: u64) -> (Status, u64) {
    let (mut send, mut recv) = conn.open(0).await.unwrap();
    let mut header = stream::encode_header(StreamKind::Management, generation);
    header[0] = tag;
    send.write_all(&header).await.unwrap();
    let mut answer = [0u8; stream::ANSWER_BYTES];
    within("the answer to a stream", recv.read_exact(&mut answer))
        .await
        .unwrap();
    stream::decode_answer(answer).expect("an answer of the contract")
}

#[tokio::test]
async fn a_stream_is_judged_by_its_kind_and_the_generation_it_names() {
    let net = Net::new();
    let host = Machine::new(&net, "windows-pc");
    let dir = tempfile::tempdir().unwrap();
    let (key, _) = MachineKey::load_or_create(&layout::machine_key(dir.path()), "by-hand").unwrap();
    let guest = net.join(key);
    let grant = host
        .peers
        .grant_pending(guest.key(), "by-hand", "By hand", Via::Link)
        .unwrap();
    host.peers.confirm(&guest.key(), &grant.id).unwrap();
    let generation = host.peers.granting(&guest.key()).generation;

    // The first exchange: the header of the hello stream and one hello each
    // way, then the host says what it grants.
    let conn = guest.dial(&host.addr()).await.unwrap();
    let (mut send, mut recv) = conn.open(StreamKind::Hello.priority()).await.unwrap();
    send.write_all(&stream::encode_header(StreamKind::Hello, 0))
        .await
        .unwrap();
    send.write_all(
        &Hello::ours("0.0.1-by-hand", "by-hand", false)
            .encode()
            .unwrap(),
    )
    .await
    .unwrap();
    let mut answer = [0u8; stream::ANSWER_BYTES];
    within("the hello stream's answer", recv.read_exact(&mut answer))
        .await
        .unwrap();
    assert_eq!(stream::decode_answer(answer), Some((Status::Accepted, 0)));
    let mut prefix = [0u8; hello::PREFIX_BYTES];
    recv.read_exact(&mut prefix).await.unwrap();
    let mut body = vec![0u8; hello::body_length(prefix).expect("a hello")];
    recv.read_exact(&mut body).await.unwrap();
    assert_eq!(Hello::decode(&body).unwrap().device, "windows-pc");
    let frame = within("the grant frame", read_message(&mut recv, hello::MAX_HELLO))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        HelloFrame::parse(&frame).unwrap(),
        Some(HelloFrame::Grant {
            state: GrantState::Active,
            generation
        })
    );

    // A stream under the generation that holds is accepted; one that names
    // an earlier grant is told the generation that holds now.
    assert_eq!(
        open(&conn, StreamKind::Management.tag(), generation)
            .await
            .0,
        Status::Accepted
    );
    assert_eq!(
        open(&conn, StreamKind::NodeLink.tag(), generation - 1).await,
        (Status::Stale, generation)
    );
    assert_eq!(
        open(&conn, StreamKind::NodeLink.tag(), 0).await.0,
        Status::Stale
    );
    // A kind this build does not know, and a second hello stream.
    assert_eq!(open(&conn, 0xee, generation).await.0, Status::UnknownKind);
    assert_eq!(
        open(&conn, StreamKind::Hello.tag(), 0).await.0,
        Status::Limit
    );
    // Pairing needs no grant.
    assert_eq!(
        open(&conn, StreamKind::Pairing.tag(), 0).await.0,
        Status::Accepted
    );

    // Paused: the generation the guest still holds opens nothing.
    host.peers.pause(&guest.key()).unwrap();
    let frame = within(
        "the pause's frame",
        read_message(&mut recv, hello::MAX_HELLO),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        HelloFrame::parse(&frame).unwrap(),
        Some(HelloFrame::Grant {
            state: GrantState::Paused,
            generation: 0
        })
    );
    assert_eq!(
        open(&conn, StreamKind::NodeLink.tag(), generation).await,
        (Status::NoGrant, 0)
    );
    drop(send);
}

/// A machine proves its key to the host and then never sends its hello,
/// which anyone who can reach the host can do. Another machine is admitted
/// meanwhile, before any of the time the silent one is given has passed.
#[tokio::test(start_paused = true)]
async fn a_peer_that_never_sends_its_hello_holds_up_no_other_machine() {
    let net = Net::new();
    let mut host = Machine::new(&net, "windows-pc");
    let guest = Machine::new(&net, "macbook");
    let silent = net.join(MachineKey::generate().unwrap());
    let quiet = within("the silent machine's connection", silent.dial(&host.addr()))
        .await
        .unwrap();

    let started = tokio::time::Instant::now();
    guest
        .peers
        .dial(&host.addr())
        .await
        .expect("the other machine is admitted");
    let admitted = guest.key();
    host.event("the other machine's connection", |event| {
        matches!(event, PeerEvent::Connected { key, .. } if *key == admitted).then_some(())
    })
    .await;
    assert!(
        started.elapsed() < limits::HELLO_WITHIN,
        "admitted while the host still waited for the silent machine's hello"
    );

    // The host was waiting for that hello all along: its time ends it.
    tokio::time::advance(limits::HELLO_WITHIN).await;
    assert_eq!(
        within("the silent connection's end", quiet.closed()).await,
        CloseCode::Protocol
    );
}
