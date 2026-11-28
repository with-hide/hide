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
    let conn = guest
        .dial(&host.addr(), hide_peer::DialBound::start())
        .await
        .unwrap();
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
    let quiet = within(
        "the silent machine's connection",
        silent.dial(&host.addr(), hide_peer::DialBound::start()),
    )
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

/// A hello of a build that speaks only the device connection number
/// `number`, from a machine named `device`.
fn of_number(number: u32, device: &str, update: bool) -> Vec<u8> {
    let mut theirs = Hello::ours("9.0.0-by-hand", device, true);
    theirs.link = hello::LinkRange {
        oldest: number,
        newest: number,
    };
    theirs.update = update;
    theirs.encode().unwrap()
}

/// A number later than any this build speaks, and one earlier.
const LATER: u32 = hello::NEWEST_LINK + 8;
const EARLIER: u32 = hello::OLDEST_LINK - 1;

/// Dials `host` by hand with `ours` as the hello and waits for the
/// connection's end.
async fn dial_with(
    guest: &hide_peer::loopback::Loopback,
    host: &Machine,
    ours: &[u8],
) -> CloseCode {
    let conn = guest
        .dial(&host.addr(), hide_peer::DialBound::start())
        .await
        .unwrap();
    let (mut send, _recv) = conn.open(StreamKind::Hello.priority()).await.unwrap();
    send.write_all(&stream::encode_header(StreamKind::Hello, 0))
        .await
        .unwrap();
    send.write_all(ours).await.unwrap();
    send.flush().await.unwrap();
    within("the connection's end", conn.closed()).await
}

/// Two builds that share no number end their connection at the first
/// exchange, and that exchange is where one asks the other to update
/// (D-40, B22): the host hears it from a guest that holds an active grant
/// there, and from nobody else.
#[tokio::test]
async fn an_update_is_asked_for_in_the_first_exchange_by_a_guest_with_an_active_grant() {
    let net = Net::new();
    let mut host = Machine::new(&net, "windows-pc");
    let dir = tempfile::tempdir().unwrap();
    let (key, _) = MachineKey::load_or_create(&layout::machine_key(dir.path()), "by-hand").unwrap();
    let guest = net.join(key);
    let grant = host
        .peers
        .grant_pending(guest.key(), "by-hand", "By hand", Via::Link)
        .unwrap();
    host.peers.confirm(&guest.key(), &grant.id).unwrap();

    // Asked by nobody: the host only learns it is the one behind.
    let quiet = of_number(LATER, "by-hand", false);
    assert_eq!(
        dial_with(&guest, &host, &quiet).await,
        CloseCode::UpdateNeeded
    );
    let behind = host
        .event("the update notice", |event| match event {
            PeerEvent::UpdateNeeded { behind, .. } => Some(*behind),
            PeerEvent::UpdateRequested { .. } => panic!("no update was asked for"),
            _ => None,
        })
        .await;
    assert_eq!(behind, hello::Behind::Ours);

    // Asked by the guest that holds the grant: heard, with the notice.
    let asking = of_number(LATER, "by-hand", true);
    assert_eq!(
        dial_with(&guest, &host, &asking).await,
        CloseCode::UpdateNeeded
    );
    let asked = host
        .event("the update request", |event| match event {
            PeerEvent::UpdateRequested { key } => Some(*key),
            _ => None,
        })
        .await;
    assert_eq!(asked, guest.key());

    // Paused, the same guest asks and is not heard: the next event of the
    // kind is the one a later active ask makes.
    host.peers.pause(&guest.key()).unwrap();
    assert_eq!(
        dial_with(&guest, &host, &asking).await,
        CloseCode::UpdateNeeded
    );
    host.peers.resume(&guest.key()).unwrap();
    let stranger = net.join(MachineKey::generate().unwrap());
    assert_eq!(
        dial_with(&stranger, &host, &of_number(LATER, "stranger", true)).await,
        CloseCode::UpdateNeeded
    );
    assert_eq!(
        dial_with(&guest, &host, &asking).await,
        CloseCode::UpdateNeeded
    );
    let mut asked = Vec::new();
    while let Ok(event) = host.events.try_recv() {
        if let PeerEvent::UpdateRequested { key } = event {
            asked.push(key);
        }
    }
    assert_eq!(asked, [guest.key()]);
}

/// The machine that asks says so once, in the first exchange of its next
/// connection to that host, and only to a host it is an active guest of.
#[tokio::test]
async fn an_update_request_is_said_once_to_a_host_this_machine_is_an_active_guest_of() {
    use hide_peer::store::{Host, KnownState};

    let net = Net::new();
    let guest = Machine::new(&net, "macbook");
    let host = net.join(MachineKey::generate().unwrap());
    let to_host = hide_peer::protocol::PeerAddr::new(host.key(), Vec::new());
    assert!(guest.peers.ask_update(&host.key()).is_err());
    guest
        .peers
        .known(|known| {
            known.put(Host {
                key: host.key(),
                device: "by-hand".to_owned(),
                name: "By hand".to_owned(),
                chosen_name: None,
                grant: "g1".to_owned(),
                state: KnownState::Active,
                candidates: Vec::new(),
                started: None,
                ssh: None,
            })
        })
        .unwrap();
    guest.peers.ask_update(&host.key()).unwrap();

    // The host, by hand: reads the guest's hello and answers with one of
    // an earlier number, so it is the one behind.
    let answer = |host: hide_peer::loopback::Loopback| async move {
        let conn = host.accept().await.unwrap();
        let (mut send, mut recv) = conn.accept().await.unwrap();
        let mut header = [0u8; stream::HEADER_BYTES];
        recv.read_exact(&mut header).await.unwrap();
        send.write_all(&stream::encode_answer(Status::Accepted, 0))
            .await
            .unwrap();
        send.write_all(&of_number(EARLIER, "by-hand", false))
            .await
            .unwrap();
        send.flush().await.unwrap();
        let mut prefix = [0u8; hello::PREFIX_BYTES];
        recv.read_exact(&mut prefix).await.unwrap();
        let mut body = vec![0u8; hello::body_length(prefix).expect("a hello")];
        recv.read_exact(&mut body).await.unwrap();
        (host, Hello::decode(&body).unwrap().update)
    };
    let (dialed, (host, asked)) = tokio::join!(guest.peers.dial(&to_host), answer(host));
    assert!(
        matches!(
            dialed,
            Err(hide_peer::peers::DialError::UpdateNeeded(
                hello::Behind::Theirs
            ))
        ),
        "{:?}",
        dialed.err()
    );
    assert!(asked, "the first exchange after the ask says it");
    let (_, (_, again)) = tokio::join!(guest.peers.dial(&to_host), answer(host));
    assert!(!again, "an ask is said once");
}
