//! One connection per pair of machines, whoever dials and however often
//! (D-62; iroh#4639, #4600).

use hide_peer::loopback::Net;
use hide_peer::peers::PeerEvent;
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::{CloseCode, StreamKind};

use crate::{Machine, granted, pair, within};

#[tokio::test]
async fn a_machine_with_a_live_connection_is_not_dialed_again() {
    let net = Net::new();
    let host = Machine::new(&net, "windows-pc");
    let guest = Machine::new(&net, "macbook");
    pair(&host, &guest);

    // Two callers at once share one dial, and a later one takes the
    // connection that is there.
    let to = host.addr();
    let (first, second) = tokio::join!(guest.peers.dial(&to), guest.peers.dial(&to));
    let third = guest.peers.dial(&to).await.unwrap();
    assert_eq!(first.unwrap().id(), third.id());
    assert_eq!(second.unwrap().id(), third.id());
    assert_eq!(net.dials(), 1);
}

#[tokio::test]
async fn two_machines_that_dial_each_other_at_once_end_with_one_connection() {
    let net = Net::new();
    let mut mac = Machine::new(&net, "macbook");
    let mut pc = Machine::new(&net, "windows-pc");
    pair(&pc, &mac);
    pair(&mac, &pc);

    let (to_pc, to_mac) = (pc.addr(), mac.addr());
    let (from_mac, from_pc) = tokio::join!(mac.peers.dial(&to_pc), pc.peers.dial(&to_mac));
    from_mac.expect("the Mac ends with a connection");
    from_pc.expect("the PC ends with a connection");

    // Whichever dials established, both machines settle on the same one.
    let link = within("one connection on both machines", async {
        loop {
            if let (Some(ours), Some(theirs)) =
                (mac.peers.link(&pc.key()), pc.peers.link(&mac.key()))
                && ours.id() == theirs.id()
            {
                assert_ne!(ours.dialed(), theirs.dialed(), "one side dialed it");
                return ours;
            }
            tokio::select! {
                event = mac.events.recv() => { event.expect("the Mac's events"); }
                event = pc.events.recv() => { event.expect("the PC's events"); }
            }
        }
    })
    .await;

    // It is a working connection in both directions.
    granted(&mut mac, pc.key(), GrantState::Active).await;
    granted(&mut pc, mac.key(), GrantState::Active).await;
    link.open(StreamKind::Management).await.unwrap();
    pc.peers
        .link(&mac.key())
        .unwrap()
        .open(StreamKind::Management)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_machine_that_dials_again_replaces_its_old_connection() {
    let net = Net::new();
    let mut host = Machine::new(&net, "windows-pc");
    let guest = Machine::new(&net, "macbook");
    pair(&host, &guest);
    let old = guest.peers.dial(&host.addr()).await.unwrap();
    let guest_key = guest.key();
    host.event("the first connection", |event| {
        matches!(event, PeerEvent::Connected { key, .. } if *key == guest_key).then_some(())
    })
    .await;

    // The guest's daemon starts again on the same state folder, so under
    // the same key, while the host still holds the old connection as live.
    let mut restarted = Machine::in_dir(&net, "macbook", guest.dir.clone());
    assert_eq!(restarted.key(), guest_key);
    let new = restarted.peers.dial(&host.addr()).await.unwrap();
    assert_ne!(new.id(), old.id());

    // The host keeps the new one and closes the old one as replaced; it
    // does not close the new one as a duplicate.
    host.event("the old connection's end", |event| {
        matches!(event, PeerEvent::Disconnected { key, code: CloseCode::Replaced } if *key == guest_key)
            .then_some(())
    })
    .await;
    assert_eq!(
        within("the old connection", old.closed()).await,
        CloseCode::Replaced
    );
    assert_eq!(host.peers.link(&guest_key).unwrap().id(), new.id());
    granted(&mut restarted, host.key(), GrantState::Active).await;
    new.open(StreamKind::Management)
        .await
        .expect("the new connection works");
}

#[tokio::test]
async fn a_guest_dials_again_after_its_connection_dropped() {
    let net = Net::new();
    let host = Machine::new(&net, "windows-pc");
    let mut guest = Machine::new(&net, "macbook");
    pair(&host, &guest);
    let host_key = host.key();
    let connected = |event: &PeerEvent| {
        matches!(event, PeerEvent::Connected { key, .. } if *key == host_key).then_some(())
    };

    let _kept = guest.peers.keep(host_key);
    guest.event("the first connection", connected).await;
    let first = guest.peers.link(&host_key).unwrap().id();

    // The connection is lost. The guest waits its first redial interval
    // (one second) and connects again by itself.
    host.peers
        .link(&guest.key())
        .unwrap()
        .close(CloseCode::Lost);
    guest
        .event("the drop", |event| {
            matches!(event, PeerEvent::Disconnected { key, code: CloseCode::Lost } if *key == host_key)
                .then_some(())
        })
        .await;
    guest
        .event("the connection after the drop", connected)
        .await;
    assert_ne!(guest.peers.link(&host_key).unwrap().id(), first);
    assert_eq!(net.dials(), 2, "one dial per connection");
}
