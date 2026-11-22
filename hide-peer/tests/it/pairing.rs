//! Pairing over the pairing stream settles in two steps (D-30).

use hide_peer::loopback::Net;
use hide_peer::peers::OpenError;
use hide_peer::protocol::StreamKind;
use hide_peer::protocol::frame::MAX_MESSAGE;
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::pairing::{ConfirmState, Pairing};
use hide_peer::store::{Host, KnownState, Via};
use hide_peer::wire::{read_message, write_message};

use crate::{Machine, granted, within};

pub async fn read(stream: &mut hide_peer::PeerStream<hide_peer::loopback::LoopConn>) -> Pairing {
    let body = within(
        "a pairing message",
        read_message(&mut stream.recv, MAX_MESSAGE),
    )
    .await
    .unwrap()
    .expect("the stream is open");
    Pairing::parse(&body)
        .unwrap()
        .expect("a pairing message this build knows")
}

#[tokio::test]
async fn a_guest_runs_nothing_until_both_sides_settled() {
    let net = Net::new();
    let mut host = Machine::new(&net, "windows-pc");
    let mut guest = Machine::new(&net, "macbook");

    // Without a grant a machine connects, and opens hello and pairing only.
    let link = guest.peers.dial(&host.addr()).await.unwrap();
    assert!(matches!(
        link.open(StreamKind::Management).await,
        Err(OpenError::NoGrant(_))
    ));
    assert_eq!(guest.peers.identity(&host.key()), None);

    let mut asking = link.open(StreamKind::Pairing).await.unwrap();
    let pair = Pairing::Pair {
        secret: "5e".repeat(32),
        device: "macbook".into(),
        name: "MacBook".into(),
        commit: "c0".repeat(32),
    };
    write_message(&mut asking.send, &pair).await.unwrap();

    // The host's side: the operator allows it, and a pending grant is saved
    // before anything is answered.
    let mut answering = host.accepted(StreamKind::Pairing).await;
    let Pairing::Pair { device, name, .. } = read(&mut answering).await else {
        panic!("the guest asks first");
    };
    let grant = host
        .peers
        .grant_pending(guest.key(), &device, &name, Via::Link)
        .unwrap();
    let granted_message = Pairing::Granted {
        grant: grant.id.clone(),
        device: "windows-pc".into(),
        name: "Windows PC".into(),
    };
    write_message(&mut answering.send, &granted_message)
        .await
        .unwrap();

    // A pending grant opens nothing, on either side's say-so.
    let Pairing::Granted {
        grant: id,
        device,
        name,
    } = read(&mut asking).await
    else {
        panic!("the host answers the ask");
    };
    assert!(matches!(
        link.open(StreamKind::Management).await,
        Err(OpenError::NoGrant(_))
    ));
    assert_eq!(host.peers.granting(&guest.key()).state, GrantState::Pending);

    // The guest's operator says the words match: its side is saved, then
    // the confirm goes out.
    guest
        .peers
        .known(|known| {
            known.put(Host {
                key: host.key(),
                device,
                name,
                grant: id.clone(),
                state: KnownState::Confirming,
                candidates: Vec::new(),
                started: None,
            })
        })
        .unwrap();
    write_message(&mut asking.send, &Pairing::Confirm { grant: id.clone() })
        .await
        .unwrap();
    let Pairing::Confirm { grant: confirmed } = read(&mut answering).await else {
        panic!("the guest confirms");
    };
    let state = host.peers.confirm(&guest.key(), &confirmed).unwrap();
    assert_eq!(state, ConfirmState::Active);
    write_message(
        &mut answering.send,
        &Pairing::Confirmed {
            grant: confirmed,
            state,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read(&mut asking).await,
        Pairing::Confirmed {
            state: ConfirmState::Active,
            ..
        }
    ));
    guest
        .peers
        .known(|known| known.set_state(&host.key(), KnownState::Active))
        .unwrap();

    // Now, and only now, the guest's streams open, on the same connection.
    granted(&mut guest, host.key(), GrantState::Active).await;
    let mut management = link.open(StreamKind::Management).await.unwrap();
    let mut served = host.accepted(StreamKind::Management).await;
    assert_eq!(guest.peers.link(&host.key()).unwrap().id(), link.id());

    // Each side names the other by both ids, from its own record.
    let as_host_sees = host.peers.identity(&guest.key()).unwrap();
    assert_eq!(
        (as_host_sees.key(), as_host_sees.device()),
        (&guest.key(), "macbook")
    );
    assert_eq!(
        guest.peers.identity(&host.key()).unwrap().device(),
        "windows-pc"
    );

    // The answer to the confirm was lost and the guest sends it again: the
    // same answer, and the stream opened under the grant is not disturbed.
    assert_eq!(
        host.peers.confirm(&guest.key(), &id).unwrap(),
        ConfirmState::Active
    );
    write_message(&mut management.send, &"still here")
        .await
        .unwrap();
    let body = within(
        "the management stream",
        read_message(&mut served.recv, MAX_MESSAGE),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(body, br#""still here""#);
}
