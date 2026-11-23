//! A grant change moves one direction's generation and nothing else
//! (D-62, D-66).

use std::io::ErrorKind;

use hide_peer::loopback::Net;
use hide_peer::peers::{OpenError, PeerEvent};
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::{CloseCode, StreamKind};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Machine, granted, pair, within};

#[tokio::test]
async fn a_pause_ends_that_direction_s_streams_and_leaves_the_other_direction_and_the_connection() {
    let net = Net::new();
    let mut mac = Machine::new(&net, "macbook");
    let mut pc = Machine::new(&net, "windows-pc");
    // Each is a guest of the other: one connection, two grants.
    pair(&pc, &mac);
    pair(&mac, &pc);
    let link = mac.peers.dial(&pc.addr()).await.unwrap();
    granted(&mut mac, pc.key(), GrantState::Active).await;
    granted(&mut pc, mac.key(), GrantState::Active).await;
    let back = pc
        .peers
        .link(&mac.key())
        .expect("one connection serves both directions");
    assert_eq!(back.id(), link.id());

    // Under the PC's grant the Mac opens a stream; under the Mac's, the PC.
    let mut mac_opened = link.open(StreamKind::NodeLink).await.unwrap();
    let mut pc_accepted = pc.accepted(StreamKind::NodeLink).await;
    let mut pc_opened = back.open(StreamKind::NodeLink).await.unwrap();
    let mut mac_accepted = mac.accepted(StreamKind::NodeLink).await;
    let before = pc.peers.granting(&mac.key()).generation;

    // The PC pauses the Mac. Reads parked on that direction's stream end on
    // both machines, with nothing sent on it.
    let parked = tokio::spawn(async move {
        let mut byte = [0u8; 1];
        mac_opened
            .recv
            .read(&mut byte)
            .await
            .map_err(|error| error.kind())
    });
    pc.peers.pause(&mac.key()).unwrap();
    assert_eq!(
        within("the Mac's parked read", parked).await.unwrap(),
        Err(ErrorKind::ConnectionAborted)
    );
    let mut byte = [0u8; 1];
    let ended = within(
        "the PC's end of that stream",
        pc_accepted.recv.read(&mut byte),
    )
    .await;
    assert_eq!(
        ended.map_err(|error| error.kind()),
        Err(ErrorKind::ConnectionAborted)
    );
    assert!(!pc_accepted.send.holds());

    // The other direction still carries bytes, on the same connection.
    pc_opened.send.write_all(b"k").await.unwrap();
    within(
        "a byte in the other direction",
        mac_accepted.recv.read_exact(&mut byte),
    )
    .await
    .unwrap();
    assert_eq!(&byte, b"k");
    assert_eq!(mac.peers.link(&pc.key()).unwrap().id(), link.id());

    // The Mac is told why, and opens nothing until the PC resumes.
    granted(&mut mac, pc.key(), GrantState::Paused).await;
    assert!(matches!(
        link.open(StreamKind::NodeLink).await,
        Err(OpenError::NoGrant(GrantState::Paused))
    ));
    // The PC lets go of the stream the pause ended, which frees its place.
    drop(pc_accepted);
    pc.peers.resume(&mac.key()).unwrap();
    granted(&mut mac, pc.key(), GrantState::Active).await;
    let reopened = link
        .open(StreamKind::NodeLink)
        .await
        .expect("the resumed grant opens streams again");
    let after = pc.peers.granting(&mac.key()).generation;
    assert!(
        after > before,
        "the resumed grant runs under a generation never used before"
    );

    // The same resume sent again (a retry, a second press) is no change:
    // the stream running under the grant goes on.
    pc.peers.resume(&mac.key()).unwrap();
    assert_eq!(pc.peers.granting(&mac.key()).generation, after);
    assert!(reopened.send.holds());
}

/// The guest was allowed nine minutes into its connection and never
/// confirmed. The pending grant kept the connection past the ten minutes a
/// connection without a grant gets, and no longer than the grant itself
/// waits.
#[tokio::test(start_paused = true)]
async fn a_connection_kept_by_a_pending_grant_closes_when_that_grant_stops_waiting() {
    use std::sync::Arc;
    use std::time::Duration;

    use hide_peer::store::{Host, KnownState, Via};

    use crate::runtime_clock;

    const MINUTE: Duration = Duration::from_secs(60);
    let net = Net::new();
    let dir = || Arc::new(tempfile::tempdir().unwrap());
    let host = Machine::with_clock(&net, "windows-pc", dir(), runtime_clock());
    let guest = Machine::with_clock(&net, "macbook", dir(), runtime_clock());
    let link = guest.peers.dial(&host.addr()).await.unwrap();

    tokio::time::advance(9 * MINUTE).await;
    let pending = host
        .peers
        .grant_pending(guest.key(), guest.name, guest.name, Via::Link)
        .unwrap();
    // The guest saved its side and never sends its confirm: its own end
    // has a reason to stay, so only the host's limit can end this.
    guest
        .peers
        .known(|known| {
            known.put(Host {
                key: host.key(),
                device: host.name.to_owned(),
                name: host.name.to_owned(),
                grant: pending.id.clone(),
                state: KnownState::Confirming,
                candidates: Vec::new(),
                started: None,
            })
        })
        .unwrap();

    // Minute 18: past the ten minutes, and the grant still waits.
    tokio::time::advance(9 * MINUTE).await;
    assert_eq!(host.peers.granting(&guest.key()).state, GrantState::Pending);
    assert_eq!(
        host.peers.link(&guest.key()).map(|live| live.id()),
        Some(link.id()),
        "the pending grant keeps its guest connected while it waits"
    );

    // Minute 19: the grant stopped waiting, unconfirmed.
    tokio::time::advance(MINUTE).await;
    assert_eq!(
        within("the connection's end", link.closed()).await,
        CloseCode::NoGrant
    );
    assert_eq!(host.peers.granting(&guest.key()).state, GrantState::None);
}

#[tokio::test]
async fn a_connection_with_no_grant_left_is_closed() {
    let net = Net::new();
    let mut host = Machine::new(&net, "windows-pc");
    let mut guest = Machine::new(&net, "macbook");
    pair(&host, &guest);
    let link = guest.peers.dial(&host.addr()).await.unwrap();
    granted(&mut guest, host.key(), GrantState::Active).await;

    // The host removes the guest. Nothing is granted in the other
    // direction, so the connection has no reason left.
    host.peers.revoke(&guest.key(), None).unwrap();
    assert_eq!(
        within("the connection's end", link.closed()).await,
        CloseCode::NoGrant
    );
    let guest_key = guest.key();
    host.event("the host's own end of it", |event| {
        matches!(event, PeerEvent::Disconnected { key, code: CloseCode::NoGrant } if *key == guest_key)
            .then_some(())
    })
    .await;
    assert!(host.peers.link(&guest.key()).is_none());
    assert_eq!(host.peers.identity(&guest.key()), None);
}

#[tokio::test]
async fn one_kind_at_its_cap_is_refused_and_frees_its_place_when_a_stream_ends() {
    let net = Net::new();
    let mut host = Machine::new(&net, "windows-pc");
    let mut guest = Machine::new(&net, "macbook");
    pair(&host, &guest);
    let link = guest.peers.dial(&host.addr()).await.unwrap();
    granted(&mut guest, host.key(), GrantState::Active).await;

    // One key stream per connection.
    let first = link.open(StreamKind::Keys).await.unwrap();
    let accepted = host.accepted(StreamKind::Keys).await;
    assert!(matches!(
        link.open(StreamKind::Keys).await,
        Err(OpenError::Refused(
            hide_peer::protocol::stream::Status::Limit
        ))
    ));
    // Another kind is not held up by it.
    link.open(StreamKind::Terminal).await.unwrap();

    // The place is the accepting side's: it is free once that side let the
    // stream go.
    drop(accepted);
    drop(first);
    link.open(StreamKind::Keys)
        .await
        .expect("the ended stream's place is free");
}

/// A paired machine renamed since its pairing is shown by its new name from
/// its next connection, on both sides, with no pairing again; one that says
/// no name keeps the name it paired under.
#[tokio::test]
async fn a_paired_machine_is_named_by_what_it_calls_itself_from_its_next_connection() {
    let net = Net::new();
    let mut mini = Machine::calling_itself(&net, "mini", "Mac mini");
    let mut mac = Machine::calling_itself(&net, "macbook", "hoyeon의 MacBook Pro");
    let plain = Machine::new(&net, "plain");
    pair(&mini, &mac);
    pair(&mini, &plain);
    mac.peers.dial(&mini.addr()).await.unwrap();
    granted(&mut mac, mini.key(), GrantState::Active).await;
    let mac_key = mac.key();
    mini.event("the Mac connected", |event| {
        matches!(event, PeerEvent::Connected { key, .. } if *key == mac_key).then_some(())
    })
    .await;
    let name_of = |machine: &Machine, key| {
        machine
            .peers
            .grants(|grants, _| grants.current(&key).map(|grant| grant.name.clone()))
    };
    assert_eq!(
        name_of(&mini, mac.key()).as_deref(),
        Some("hoyeon의 MacBook Pro")
    );
    assert_eq!(
        mac.peers.host(&mini.key()).map(|host| host.name).as_deref(),
        Some("Mac mini")
    );

    plain.peers.dial(&mini.addr()).await.unwrap();
    let plain_key = plain.key();
    mini.event("the plain machine connected", |event| {
        matches!(event, PeerEvent::Connected { key, .. } if *key == plain_key).then_some(())
    })
    .await;
    assert_eq!(name_of(&mini, plain.key()).as_deref(), Some("plain"));
}
