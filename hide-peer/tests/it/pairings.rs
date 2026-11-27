//! Two operators pair their machines, and take the pairing apart again
//! (PRD B1-B7, B25-B27, B50, B51).
//!
//! Each machine is a real [`Pairings`] over the loopback wire; a test plays
//! the two operators through what their screens would show ([`View`]) and
//! reads the outcome from the records and from what then opens.

use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use hide_kit::layout;
use hide_peer::link::{PairLink, Secret};
use hide_peer::loopback::{Loopback, Net};
use hide_peer::pairing::{
    AcceptError, AskKind, JoinError, JoinFailure, JoinPhase, Me, Pairings, RequestError,
    RequestState, View,
};
use hide_peer::peers::{OpenError, PeerEvent, Peers};
use hide_peer::protocol::frame::MAX_MESSAGE;
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::pairing::{ConfirmState, PairRefusal, Pairing, Revoke};
use hide_peer::protocol::{PeerKey, StreamKind};
use hide_peer::store::{KnownState, StoreError, Via};
use hide_peer::wire::{read_message, write_message};
use hide_peer::words::{self, Nonce, WordList};
use hide_peer::{Clock, limits};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

use crate::pairing::read;
use crate::{Machine, granted, runtime_clock, within};

/// A machine with its operator's view of the pairings.
struct Desk {
    machine: Machine,
    pairings: Pairings<Loopback>,
    view: watch::Receiver<View>,
    saved: watch::Receiver<u64>,
}

impl Desk {
    fn new(net: &Arc<Net>, device: &'static str, name: &str) -> Self {
        Self::on(Machine::new(net, device), name)
    }

    /// A machine whose records and timers follow the test runtime's time.
    fn timed(net: &Arc<Net>, device: &'static str, name: &str) -> Self {
        let dir = Arc::new(tempfile::tempdir().unwrap());
        Self::on(Machine::with_clock(net, device, dir, runtime_clock()), name)
    }

    fn on(mut machine: Machine, name: &str) -> Self {
        let me = Me {
            device: machine.name.to_owned(),
            name: name.to_owned(),
        };
        let pairings = Pairings::start(machine.peers.clone(), me).unwrap();
        // The pairing and revoke streams are the pairings'; every other
        // stream stays the machine's to accept.
        let (others, kept) = mpsc::channel(8);
        let mut incoming = std::mem::replace(&mut machine.incoming, kept);
        let serving = pairings.clone();
        tokio::spawn(async move {
            while let Some(stream) = incoming.recv().await {
                if let Some(other) = serving.serve(stream)
                    && others.send(other).await.is_err()
                {
                    return;
                }
            }
        });
        Self {
            view: pairings.view(),
            saved: pairings.saved(),
            machine,
            pairings,
        }
    }

    fn key(&self) -> PeerKey {
        self.machine.key()
    }

    fn peers(&self) -> &Peers<Loopback> {
        &self.machine.peers
    }

    /// What this machine's screen comes to show.
    async fn shows<T>(&mut self, what: &str, mut pick: impl FnMut(&View) -> Option<T>) -> T {
        within(what, async {
            loop {
                let picked = pick(&self.view.borrow_and_update());
                if let Some(picked) = picked {
                    return picked;
                }
                self.view.changed().await.expect("the pairings' view");
            }
        })
        .await
    }

    /// Waits until this machine's records say `reached`.
    async fn saves(&mut self, what: &str, reached: impl Fn(&Peers<Loopback>) -> bool) {
        within(what, async {
            loop {
                self.saved.borrow_and_update();
                if reached(&self.machine.peers) {
                    return;
                }
                self.saved.changed().await.expect("the pairings' records");
            }
        })
        .await
    }

    fn host_state(&self, host: &PeerKey) -> Option<KnownState> {
        self.peers().host(host).map(|host| host.state)
    }

    fn grants(&self, guest: &PeerKey) -> GrantState {
        self.peers().granting(guest).state
    }

    async fn join_ends(&mut self) -> (JoinPhase, Option<JoinFailure>) {
        self.shows("the end of the pairing", |view| {
            let join = view.join.as_ref()?;
            matches!(
                join.phase,
                JoinPhase::Paired | JoinPhase::Waiting | JoinPhase::Failed
            )
            .then_some((join.phase, join.failure))
        })
        .await
    }
}

/// Both operators do their part: the guest pastes the host's link and says
/// the words match, the host allows.
async fn pair_by_link(host: &mut Desk, guest: &mut Desk) {
    let offer = host.pairings.share(WordList::English).unwrap();
    guest.pairings.join(&offer.link).unwrap();
    let ask = host
        .shows("the guest's request", |view| view.asks.first().cloned())
        .await;
    guest
        .shows("the words", |view| view.join.as_ref()?.words)
        .await;
    assert!(guest.pairings.compare(true));
    assert!(host.pairings.answer(ask.id, true));
    assert_eq!(guest.join_ends().await, (JoinPhase::Paired, None));
    guest.pairings.dismiss_join();
    granted(&mut guest.machine, host.key(), GrantState::Active).await;
}

#[tokio::test]
async fn a_link_pairs_two_machines_once_both_operators_agreed() {
    let net = Net::new();
    let mut pc = Desk::new(&net, "windows-pc", "Windows PC");
    let mut mac = Desk::new(&net, "macbook", "MacBook");

    let offer = pc.pairings.share(WordList::Korean).unwrap();
    assert_eq!(pc.view.borrow().offer.as_ref().unwrap().link, offer.link);
    mac.pairings.join(&offer.link).unwrap();

    // Both screens show who the other is and the same four words.
    let ask = pc
        .shows("the guest's request", |view| view.asks.first().cloned())
        .await;
    let (host_name, words) = mac
        .shows("the words on the guest", |view| {
            let join = view.join.as_ref()?;
            Some((join.name.clone()?, join.words?))
        })
        .await;
    assert_eq!(
        (ask.name.as_str(), ask.device.as_str(), ask.kind, ask.key),
        ("MacBook", "macbook", AskKind::Link, mac.key())
    );
    assert_eq!(host_name, "Windows PC");
    assert_eq!(ask.words, Some(words), "the same words in the same order");
    assert!(pc.view.borrow().offer.is_none(), "the link is used up");

    // The guest alone saying yes makes nothing.
    assert!(mac.pairings.compare(true));
    mac.shows("the wait for the host", |view| {
        (view.join.as_ref()?.phase == JoinPhase::Settling).then_some(())
    })
    .await;
    assert_eq!(pc.grants(&mac.key()), GrantState::None);
    assert_eq!(mac.host_state(&pc.key()), None);

    assert!(pc.pairings.answer(ask.id, true));
    assert_eq!(mac.join_ends().await, (JoinPhase::Paired, None));

    // Each side holds the other in its record, and the guest's streams open.
    assert_eq!(pc.grants(&mac.key()), GrantState::Active);
    let host = mac.peers().host(&pc.key()).unwrap();
    assert_eq!(
        (host.state, host.device.as_str(), host.name.as_str()),
        (KnownState::Active, "windows-pc", "Windows PC")
    );
    granted(&mut mac.machine, pc.key(), GrantState::Active).await;
    let link = mac.peers().link(&pc.key()).unwrap();
    link.open(StreamKind::Management).await.unwrap();
    assert!(pc.view.borrow().asks.is_empty());

    // The link opens nothing a second time, and the machine it paired is
    // not paired twice.
    let mut other = Desk::new(&net, "other", "Other");
    other.pairings.join(&offer.link).unwrap();
    assert_eq!(
        other.join_ends().await,
        (JoinPhase::Failed, Some(JoinFailure::NoOffer))
    );
    let again = pc.pairings.share(WordList::English).unwrap();
    assert_eq!(mac.pairings.join(&again.link), Err(JoinError::Already));
    assert_eq!(pc.pairings.join(&again.link), Err(JoinError::Own));
}

/// The words come from one value of each side, and a guest is bound to its
/// own before it sees the host's (PRD D-17): the host's operator is shown
/// the words of the value the guest committed to, and a guest that opens
/// another value, chosen with the host's in hand, is refused before anyone
/// is asked.
#[tokio::test]
async fn a_guest_cannot_choose_its_value_after_it_saw_the_hosts() {
    let net = Net::new();
    let mut pc = Desk::new(&net, "windows-pc", "Windows PC");
    let guest = Machine::new(&net, "macbook");
    let conn = guest.peers.dial(&pc.machine.addr()).await.unwrap();
    let committed = Nonce::from_bytes([1; 32]);
    let commit = words::commitment(&pc.key(), &guest.key(), &committed);
    // What a guest sends with a link: the secret and its commitment.
    let pair = |link: &str| Pairing::Pair {
        secret: PairLink::parse(link).unwrap().secret.to_hex().to_string(),
        device: "macbook".into(),
        name: "MacBook".into(),
        commit: commit.clone(),
    };

    // A guest that opens what it committed to: the operator is asked, with
    // the words of both values.
    let offer = pc.pairings.share(WordList::English).unwrap();
    let mut pairing = conn.open(StreamKind::Pairing).await.unwrap();
    write_message(&mut pairing.send, &pair(&offer.link))
        .await
        .unwrap();
    let Pairing::Asking { nonce, .. } = read(&mut pairing).await else {
        panic!("the host's value");
    };
    let hosts = Nonce::from_hex(&nonce).expect("the host's value");
    assert!(pc.view.borrow().asks.is_empty(), "nobody is asked yet");
    let reveal = Pairing::Reveal {
        nonce: committed.to_hex().to_string(),
    };
    write_message(&mut pairing.send, &reveal).await.unwrap();
    let ask = pc
        .shows("the guest's request", |view| view.asks.first().cloned())
        .await;
    assert_eq!(
        ask.words,
        Some(words::of(
            WordList::English,
            &pc.key(),
            &guest.key(),
            &committed,
            &hosts
        ))
    );
    // One value for one `pair`: a second one is not read for the words.
    let second = Pairing::Reveal {
        nonce: Nonce::from_bytes([2; 32]).to_hex().to_string(),
    };
    write_message(&mut pairing.send, &second).await.unwrap();
    assert!(pc.pairings.answer(ask.id, false));
    assert_eq!(
        read(&mut pairing).await,
        Pairing::Refused {
            reason: PairRefusal::Declined
        }
    );
    assert_eq!(pc.view.borrow().asks.len(), 0);

    // The same guest on a new link, now choosing its value once it holds
    // the host's: the value it opens is not the committed one.
    let offer = pc.pairings.share(WordList::English).unwrap();
    write_message(&mut pairing.send, &pair(&offer.link))
        .await
        .unwrap();
    let Pairing::Asking { nonce, .. } = read(&mut pairing).await else {
        panic!("the host's value");
    };
    assert_ne!(
        nonce,
        hosts.to_hex().as_str(),
        "the host draws its value anew for each pairing"
    );
    let chosen = Pairing::Reveal {
        nonce: Nonce::from_bytes([2; 32]).to_hex().to_string(),
    };
    write_message(&mut pairing.send, &chosen).await.unwrap();
    assert_eq!(
        read(&mut pairing).await,
        Pairing::Refused {
            reason: PairRefusal::NoOffer
        }
    );
    let view = pc.view.borrow();
    assert!(view.asks.is_empty(), "nobody was asked");
    assert!(view.offer.is_none(), "the link is spent");
}

/// A guest that commits to no value cannot be held to one, so it is not
/// answered with the host's: its link is spent like one with a wrong secret.
#[tokio::test]
async fn a_pair_that_commits_to_no_value_spends_the_link() {
    let net = Net::new();
    let pc = Desk::new(&net, "windows-pc", "Windows PC");
    let guest = Machine::new(&net, "macbook");
    let conn = guest.peers.dial(&pc.machine.addr()).await.unwrap();
    let mut pairing = conn.open(StreamKind::Pairing).await.unwrap();
    let spelled = words::commitment(&pc.key(), &guest.key(), &Nonce::from_bytes([1; 32]));
    for commit in [
        String::new(),
        spelled.to_uppercase(),
        spelled[..62].to_owned(),
    ] {
        let offer = pc.pairings.share(WordList::English).unwrap();
        let pair = Pairing::Pair {
            secret: PairLink::parse(&offer.link)
                .unwrap()
                .secret
                .to_hex()
                .to_string(),
            device: "macbook".into(),
            name: "MacBook".into(),
            commit,
        };
        write_message(&mut pairing.send, &pair).await.unwrap();
        assert_eq!(
            read(&mut pairing).await,
            Pairing::Refused {
                reason: PairRefusal::NoOffer
            }
        );
        let view = pc.view.borrow();
        assert!(view.asks.is_empty(), "nobody was asked");
        assert!(view.offer.is_none(), "the link is spent");
    }
}

/// The guest's words are those of the first value the host sent: a host
/// that sends another once it holds the guest's value, one it could have
/// searched for, moves nothing on the guest's screen (PRD D-17).
#[tokio::test]
async fn the_guests_words_stay_those_of_the_hosts_first_value() {
    let net = Net::new();
    let mut mac = Desk::new(&net, "macbook", "MacBook");
    // The host is played by hand.
    let mut host = Machine::new(&net, "windows-pc");
    let link = PairLink {
        host: host.key(),
        device: "windows-pc".into(),
        relay: None,
        addrs: Vec::new(),
        secret: Secret::from_bytes([5; 32]),
    };
    mac.pairings.join(&link.text().unwrap()).unwrap();
    let mut pairing = host.accepted(StreamKind::Pairing).await;
    let Pairing::Pair { commit, .. } = read(&mut pairing).await else {
        panic!("the guest's pair");
    };
    let asking = |nonce: [u8; 32]| Pairing::Asking {
        device: "windows-pc".into(),
        name: "Windows PC".into(),
        words: WordList::English.name().into(),
        nonce: Nonce::from_bytes(nonce).to_hex().to_string(),
    };
    write_message(&mut pairing.send, &asking([3; 32]))
        .await
        .unwrap();
    let Pairing::Reveal { nonce } = read(&mut pairing).await else {
        panic!("the guest's value");
    };
    let guests = Nonce::from_hex(&nonce).expect("the guest's value");
    assert!(
        words::opens(&commit, &host.key(), &mac.key(), &guests),
        "the guest opens what it committed to"
    );
    let first = words::of(
        WordList::English,
        &host.key(),
        &mac.key(),
        &guests,
        &Nonce::from_bytes([3; 32]),
    );
    let shown = mac
        .shows("the words", |view| view.join.as_ref()?.words)
        .await;
    assert_eq!(shown, first);

    // Another value of the host's, then its grant. The guest reads them in
    // order, so its confirm says both were read.
    write_message(&mut pairing.send, &asking([4; 32]))
        .await
        .unwrap();
    let granted = Pairing::Granted {
        grant: "grant-1".into(),
        device: "windows-pc".into(),
        name: "Windows PC".into(),
    };
    write_message(&mut pairing.send, &granted).await.unwrap();
    assert!(mac.pairings.compare(true));
    assert!(matches!(
        read(&mut pairing).await,
        Pairing::Confirm { grant } if grant == "grant-1"
    ));
    let words = mac.view.borrow().join.as_ref().and_then(|join| join.words);
    assert_eq!(words, Some(first), "the words did not move");
}

#[tokio::test]
async fn either_operator_saying_no_leaves_nothing_and_a_new_link_starts_over() {
    let net = Net::new();
    let mut pc = Desk::new(&net, "windows-pc", "Windows PC");
    let mut mac = Desk::new(&net, "macbook", "MacBook");

    // The guest's operator says the words differ, after the host allowed.
    let offer = pc.pairings.share(WordList::English).unwrap();
    mac.pairings.join(&offer.link).unwrap();
    let ask = pc
        .shows("the guest's request", |view| view.asks.first().cloned())
        .await;
    mac.shows("the words", |view| view.join.as_ref()?.words)
        .await;
    assert!(pc.pairings.answer(ask.id, true));
    pc.saves("the pending grant", |peers| {
        peers.granting(&ask.key).state == GrantState::Pending
    })
    .await;
    assert!(mac.pairings.compare(false));
    mac.shows("the closed window", |view| {
        view.join.is_none().then_some(())
    })
    .await;
    // The host's pending grant is taken back; it never opened anything.
    pc.saves("the grant taken back", |peers| {
        peers.granting(&ask.key).state == GrantState::Revoked
    })
    .await;
    assert_eq!(mac.host_state(&pc.key()), None);

    // The host's operator says no.
    let offer = pc.pairings.share(WordList::English).unwrap();
    mac.pairings.join(&offer.link).unwrap();
    let ask = pc
        .shows("the guest's request", |view| view.asks.first().cloned())
        .await;
    assert!(pc.pairings.answer(ask.id, false));
    assert_eq!(
        mac.join_ends().await,
        (JoinPhase::Failed, Some(JoinFailure::Declined))
    );
    assert_eq!(mac.host_state(&pc.key()), None);
    assert!(!pc.pairings.answer(ask.id, true), "answered once");

    // A link whose secret is wrong ends the offer: the right link is then
    // refused too.
    let offer = pc.pairings.share(WordList::English).unwrap();
    let at = offer.link.find("&s=").unwrap() + 3;
    let mut forged = offer.link.clone();
    forged.replace_range(at..at + 64, &"0".repeat(64));
    mac.pairings.join(&forged).unwrap();
    assert_eq!(
        mac.join_ends().await,
        (JoinPhase::Failed, Some(JoinFailure::NoOffer))
    );
    assert!(pc.view.borrow().offer.is_none());
    assert!(
        pc.view.borrow().asks.is_empty(),
        "the operator is not asked"
    );
    mac.pairings.join(&offer.link).unwrap();
    assert_eq!(
        mac.join_ends().await,
        (JoinPhase::Failed, Some(JoinFailure::NoOffer))
    );

    // A new link starts over.
    pair_by_link(&mut pc, &mut mac).await;
    assert_eq!(pc.grants(&mac.key()), GrantState::Active);
}

#[tokio::test(start_paused = true)]
async fn two_minutes_without_an_answer_end_a_pairing_and_nine_end_a_link() {
    let net = Net::new();
    let mut pc = Desk::timed(&net, "windows-pc", "Windows PC");
    let mut mac = Desk::timed(&net, "macbook", "MacBook");

    let offer = pc.pairings.share(WordList::English).unwrap();
    mac.pairings.join(&offer.link).unwrap();
    pc.shows("the guest's request", |view| view.asks.first().cloned())
        .await;
    mac.shows("the words", |view| view.join.as_ref()?.words)
        .await;
    // Nobody answers.
    tokio::time::sleep(limits::ANSWER_WITHIN + Duration::from_secs(1)).await; // wait-ok: the two minutes an answer may take run out, on the test's paused clock
    let join = mac.view.borrow().join.clone().unwrap();
    assert_eq!(
        (join.phase, join.failure, join.words),
        (JoinPhase::Failed, Some(JoinFailure::Unanswered), None)
    );
    assert!(pc.view.borrow().asks.is_empty());
    assert_eq!(pc.grants(&mac.key()), GrantState::None);
    assert_eq!(mac.host_state(&pc.key()), None);

    // A link nobody used is over after nine minutes.
    let offer = pc.pairings.share(WordList::English).unwrap();
    tokio::time::sleep(limits::OFFER_FOR - Duration::from_secs(1)).await; // wait-ok: one second before the link's nine minutes end, on the test's paused clock
    assert!(pc.view.borrow().offer.is_some());
    tokio::time::sleep(Duration::from_secs(2)).await; // wait-ok: the link's nine minutes are over, on the test's paused clock
    assert!(pc.view.borrow().offer.is_none());
    mac.pairings.join(&offer.link).unwrap();
    assert_eq!(
        mac.join_ends().await,
        (JoinPhase::Failed, Some(JoinFailure::NoOffer))
    );
}

#[tokio::test]
async fn a_full_host_refuses_before_its_operator_is_asked() {
    let net = Net::new();
    let pc = Desk::new(&net, "windows-pc", "Windows PC");
    let mut mac = Desk::new(&net, "macbook", "MacBook");
    for index in 0..limits::MAX_GRANTS as u8 {
        let guest = PeerKey::from_bytes([index + 1; 32]);
        pc.peers()
            .grant_pending(guest, "guest", "Guest", Via::Link)
            .unwrap();
    }
    let offer = pc.pairings.share(WordList::English).unwrap();
    mac.pairings.join(&offer.link).unwrap();
    assert_eq!(
        mac.join_ends().await,
        (JoinPhase::Failed, Some(JoinFailure::HostFull))
    );
    assert!(pc.view.borrow().asks.is_empty());
}

#[tokio::test]
async fn a_guest_that_cannot_save_its_side_is_not_connected() {
    let net = Net::new();
    let mut pc = Desk::new(&net, "windows-pc", "Windows PC");
    let mut mac = Desk::new(&net, "macbook", "MacBook");
    // The guest's record cannot be written: a folder is in its place.
    let known = layout::peer_known(mac.machine.dir.path());
    std::fs::create_dir_all(known.join("in-the-way")).unwrap();

    let offer = pc.pairings.share(WordList::English).unwrap();
    mac.pairings.join(&offer.link).unwrap();
    let ask = pc
        .shows("the guest's request", |view| view.asks.first().cloned())
        .await;
    mac.shows("the words", |view| view.join.as_ref()?.words)
        .await;
    assert!(mac.pairings.compare(true));
    assert!(pc.pairings.answer(ask.id, true));
    assert_eq!(
        mac.join_ends().await,
        (JoinPhase::Failed, Some(JoinFailure::Unreachable))
    );
    // The host's pending grant never became one that opens anything.
    pc.saves("the grant taken back", |peers| {
        peers.granting(&ask.key).state == GrantState::Revoked
    })
    .await;
}

#[tokio::test]
async fn a_pairing_over_ssh_settles_over_the_connection() {
    let net = Net::new();
    let pc = Desk::new(&net, "windows-pc", "Windows PC");
    let mut mac = Desk::new(&net, "macbook", "MacBook");

    // The target saves a pending grant and says who it is; asked again
    // with the same request, it says the same.
    let accepted = pc
        .pairings
        .accept_ssh("req-1", mac.key(), "macbook", "MacBook", Via::Ssh)
        .unwrap();
    let again = pc
        .pairings
        .accept_ssh("req-1", mac.key(), "macbook", "MacBook", Via::Ssh)
        .unwrap();
    assert_eq!(accepted, again);
    assert_eq!(
        (
            accepted.key,
            accepted.device.as_str(),
            accepted.name.as_str()
        ),
        (pc.key(), "windows-pc", "Windows PC")
    );
    assert_eq!(pc.grants(&mac.key()), GrantState::Pending);
    assert!(matches!(
        pc.pairings
            .accept_ssh("req-2", pc.key(), "windows-pc", "Windows PC", Via::Ssh),
        Err(AcceptError::Invalid)
    ));

    // The guest saves what it was told, connects and confirms on its own.
    mac.pairings
        .adopt(accepted.clone(), Vec::new(), None)
        .unwrap();
    mac.saves("the confirmed pairing", |peers| {
        peers
            .host(&accepted.key)
            .is_some_and(|host| host.state == KnownState::Active)
    })
    .await;
    assert_eq!(pc.grants(&mac.key()), GrantState::Active);
    let via = pc
        .peers()
        .grants(|grants, now| grants.list(now).map(|grant| grant.via).collect::<Vec<_>>());
    assert_eq!(via, [Via::Ssh]);
    granted(&mut mac.machine, pc.key(), GrantState::Active).await;

    // The same answer adopted again changes nothing; another pairing with
    // a machine already connected is refused, and there is one grant.
    mac.pairings.adopt(accepted, Vec::new(), None).unwrap();
    let second = pc
        .pairings
        .accept_ssh("req-3", mac.key(), "macbook", "MacBook", Via::Ssh)
        .unwrap();
    assert!(matches!(
        mac.pairings.adopt(second, Vec::new(), None),
        Err(AcceptError::Already)
    ));
    assert_eq!(pc.grants(&mac.key()), GrantState::Active);
}

#[tokio::test]
async fn a_grant_the_host_no_longer_holds_shows_as_pair_again_and_a_new_link_takes_its_place() {
    let net = Net::new();
    let mut pc = Desk::new(&net, "windows-pc", "Windows PC");
    let mut mac = Desk::new(&net, "macbook", "MacBook");

    // The pending grant is gone before the guest's confirm arrives.
    let accepted = pc
        .pairings
        .accept_ssh("req-1", mac.key(), "macbook", "MacBook", Via::Ssh)
        .unwrap();
    pc.pairings.remove_guest(&mac.key()).unwrap();
    mac.pairings.adopt(accepted, Vec::new(), None).unwrap();
    let host = pc.key();
    mac.saves("the dropped pairing", |peers| {
        peers
            .host(&host)
            .is_some_and(|host| host.state == KnownState::Unpaired)
    })
    .await;
    assert_eq!(pc.grants(&mac.key()), GrantState::Revoked);

    // Pairing again goes on in the same record.
    pair_by_link(&mut pc, &mut mac).await;
    assert_eq!(mac.host_state(&pc.key()), Some(KnownState::Active));
    assert_eq!(mac.peers().hosts().len(), 1);

    // The host removes the guest: it is cut off at once, shows as a host
    // to pair with again, and is not dialed any more.
    let mut open = mac
        .peers()
        .link(&pc.key())
        .unwrap()
        .open(StreamKind::Terminal)
        .await
        .unwrap();
    pc.pairings.remove_guest(&mac.key()).unwrap();
    let mut byte = [0u8; 1];
    let ended = within("the guest's stream", open.recv.read(&mut byte)).await;
    assert_eq!(
        ended.map_err(|error| error.kind()),
        Err(ErrorKind::ConnectionAborted)
    );
    mac.saves("the removal heard", |peers| {
        peers
            .host(&host)
            .is_some_and(|host| host.state == KnownState::Unpaired)
    })
    .await;
    mac.machine
        .event("the connection's end", |event| {
            matches!(event, PeerEvent::Disconnected { key, .. } if *key == host).then_some(())
        })
        .await;
    let dials = net.dials();
    mac.peers().network_changed();
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        net.dials(),
        dials,
        "a host that dropped the pairing is not dialed"
    );

    // Removing it here then forgets it at once: there is nothing to take
    // back from the host.
    mac.pairings.remove_host(&pc.key()).unwrap();
    assert_eq!(mac.host_state(&pc.key()), None);
}

#[tokio::test]
async fn the_other_direction_is_asked_over_the_connection_and_each_direction_stops_on_its_own() {
    let net = Net::new();
    let mut pc = Desk::new(&net, "windows-pc", "Windows PC");
    let mut mac = Desk::new(&net, "macbook", "MacBook");
    // The Mac opens the PC.
    pair_by_link(&mut pc, &mut mac).await;
    let connection = mac.peers().link(&pc.key()).unwrap().id();

    // Only the machine that is opened may ask to open the other.
    assert_eq!(mac.pairings.request(pc.key()), Err(RequestError::NotAGuest));
    pc.pairings.request(mac.key()).unwrap();
    let ask = mac
        .shows("the PC's request", |view| view.asks.first().cloned())
        .await;
    assert_eq!(
        (ask.kind, ask.words, ask.name.as_str(), ask.key),
        (AskKind::Request, None, "Windows PC", pc.key())
    );
    assert_eq!(pc.pairings.request(mac.key()), Err(RequestError::Waiting));
    assert!(mac.pairings.answer(ask.id, true));
    let guest = mac.key();
    pc.saves("the PC as the Mac's guest", |peers| {
        peers
            .host(&guest)
            .is_some_and(|host| host.state == KnownState::Active)
    })
    .await;
    pc.shows("the request's end", |view| {
        view.requests.is_empty().then_some(())
    })
    .await;
    granted(&mut pc.machine, mac.key(), GrantState::Active).await;
    assert_eq!(pc.pairings.request(mac.key()), Err(RequestError::Already));
    // Both directions, one connection.
    let back = pc.peers().link(&mac.key()).unwrap();
    assert_eq!(back.id(), connection);

    let forth = mac.peers().link(&pc.key()).unwrap();
    let mut mac_opened = forth.open(StreamKind::Terminal).await.unwrap();
    let mut pc_opened = back.open(StreamKind::Terminal).await.unwrap();
    let mut mac_accepted = mac.machine.accepted(StreamKind::Terminal).await;

    // The Mac stops using the PC: what it opened there ends and nothing
    // opens, while the PC goes on using the Mac over the same connection.
    mac.pairings.pause_host(&pc.key()).unwrap();
    let mut byte = [0u8; 1];
    let ended = within("the Mac's stream", mac_opened.recv.read(&mut byte)).await;
    assert_eq!(
        ended.map_err(|error| error.kind()),
        Err(ErrorKind::ConnectionAborted)
    );
    assert!(matches!(
        forth.open(StreamKind::Terminal).await,
        Err(OpenError::NoGrant(_))
    ));
    pc_opened.send.write_all(b"x").await.unwrap();
    pc_opened.send.flush().await.unwrap();
    within("the PC's stream", mac_accepted.recv.read_exact(&mut byte))
        .await
        .unwrap();
    assert_eq!(mac.peers().link(&pc.key()).unwrap().id(), connection);

    // Turned on again, it opens under what the PC grants.
    mac.pairings.resume_host(&pc.key()).unwrap();
    forth.open(StreamKind::Terminal).await.unwrap();

    // The Mac removes "the PC opens this Mac": the PC's side shows as a
    // host to pair with again, and the Mac still opens the PC.
    mac.pairings.remove_guest(&pc.key()).unwrap();
    pc.saves("the removal heard", |peers| {
        peers
            .host(&guest)
            .is_some_and(|host| host.state == KnownState::Unpaired)
    })
    .await;
    assert!(matches!(
        back.open(StreamKind::Terminal).await,
        Err(OpenError::NoGrant(_))
    ));
    forth.open(StreamKind::Terminal).await.unwrap();
    assert_eq!(mac.peers().link(&pc.key()).unwrap().id(), connection);
    assert!(matches!(
        pc.pairings.pause_host(&mac.key()),
        Err(StoreError::Unknown)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_request_the_other_way_says_how_it_ended() {
    let net = Net::new();
    let mut pc = Desk::timed(&net, "windows-pc", "Windows PC");
    let mut mac = Desk::timed(&net, "macbook", "MacBook");
    pair_by_link(&mut pc, &mut mac).await;
    let guest = mac.key();
    let ended = |view: &View| {
        view.requests
            .iter()
            .find(|request| request.key == guest && request.state != RequestState::Waiting)
            .map(|request| request.state)
    };

    pc.pairings.request(guest).unwrap();
    let ask = mac
        .shows("the PC's request", |view| view.asks.first().cloned())
        .await;
    assert!(mac.pairings.answer(ask.id, false));
    assert_eq!(pc.shows("declined", ended).await, RequestState::Declined);

    // Asked again, and nobody answers for two minutes.
    pc.pairings.request(guest).unwrap();
    mac.shows("the PC's request", |view| view.asks.first().cloned())
        .await;
    tokio::time::sleep(limits::ANSWER_WITHIN + Duration::from_secs(1)).await; // wait-ok: the two minutes an answer may take run out, on the test's paused clock
    assert_eq!(ended(&pc.view.borrow()), Some(RequestState::Unanswered));
    assert!(mac.view.borrow().asks.is_empty());

    // The guest is not connected.
    mac.peers().close().await;
    pc.machine
        .event("the guest leaving", |event| {
            matches!(event, PeerEvent::Disconnected { key, .. } if *key == guest).then_some(())
        })
        .await;
    pc.pairings.request(guest).unwrap();
    assert_eq!(
        pc.shows("unreachable", ended).await,
        RequestState::Unreachable
    );
    assert_eq!(pc.host_state(&guest), None, "nothing was made");
    pc.pairings.dismiss_request(&guest);
    assert!(pc.view.borrow().requests.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_removal_reaches_a_host_that_was_off_and_a_second_one_forgets_it_here() {
    let net = Net::new();
    let mut pc = Desk::timed(&net, "windows-pc", "Windows PC");
    let mut mini = Desk::timed(&net, "mac-mini", "Mac mini");
    let mut mac = Desk::timed(&net, "macbook", "MacBook");
    pair_by_link(&mut pc, &mut mac).await;
    pair_by_link(&mut mini, &mut mac).await;
    let (pc_key, mini_key, guest) = (pc.key(), mini.key(), mac.key());

    // Both hosts are off when the guest removes them.
    pc.peers().close().await;
    mini.peers().close().await;
    for host in [pc_key, mini_key] {
        mac.machine
            .event("the host going off", |event| {
                matches!(event, PeerEvent::Disconnected { key, .. } if *key == host).then_some(())
            })
            .await;
    }
    mac.pairings.remove_host(&pc_key).unwrap();
    mac.pairings.remove_host(&mini_key).unwrap();
    assert_eq!(mac.host_state(&pc_key), Some(KnownState::RevokePending));

    // Removed a second time, the Mac mini is forgotten here; what it holds
    // is its own operator's to remove.
    mac.pairings.remove_host(&mini_key).unwrap();
    assert_eq!(mac.host_state(&mini_key), None);

    // The PC comes back: the removal reaches it, and only then is it
    // forgotten here.
    let dir = Arc::clone(&pc.machine.dir);
    let back = Machine::with_clock(&net, "windows-pc", dir, Clock::system());
    let mut pc = Desk::on(back, "Windows PC");
    assert_eq!(pc.grants(&guest), GrantState::Active);
    mac.saves("the host forgotten", |peers| peers.host(&pc_key).is_none())
        .await;
    pc.saves("the grant removed", |peers| {
        peers.granting(&guest).state == GrantState::Revoked
    })
    .await;
}

/// B27: a removal of a grant the two have since replaced by pairing again
/// takes nothing. The host answers it as done, so whoever kept sending it
/// stops, and the new grant, its connection and its streams stay.
#[tokio::test]
async fn a_late_removal_of_an_old_grant_leaves_the_pairing_made_since() {
    let net = Net::new();
    let mut pc = Desk::new(&net, "windows-pc", "Windows PC");
    let mut mac = Desk::new(&net, "macbook", "MacBook");
    let (host, guest) = (pc.key(), mac.key());
    pair_by_link(&mut pc, &mut mac).await;
    let old = mac.peers().host(&host).unwrap().grant;

    // The host removes the guest, and the two pair again: a new grant.
    pc.pairings.remove_guest(&guest).unwrap();
    mac.saves("the removal heard", |peers| {
        peers
            .host(&host)
            .is_some_and(|host| host.state == KnownState::Unpaired)
    })
    .await;
    mac.machine
        .event("the connection's end", |event| {
            matches!(event, PeerEvent::Disconnected { key, .. } if *key == host).then_some(())
        })
        .await;
    pair_by_link(&mut pc, &mut mac).await;
    let new = mac.peers().host(&host).unwrap().grant;
    assert_ne!(old, new);
    let held = |pc: &Desk| {
        pc.peers()
            .grants(|grants, _| grants.current(&guest).map(|grant| grant.id.clone()))
    };
    assert_eq!(held(&pc), Some(new.clone()));

    // The guest works over the new pairing.
    let link = mac.peers().link(&host).unwrap();
    let mut terminal = link.open(StreamKind::Terminal).await.unwrap();
    let mut there = pc.machine.accepted(StreamKind::Terminal).await;

    // The removal of the old grant, sent before the two paired again,
    // arrives only now.
    let mut revoking = link.open(StreamKind::Revoke).await.unwrap();
    let revoke = Revoke::Revoke { grant: old.clone() };
    write_message(&mut revoking.send, &revoke).await.unwrap();
    let body = within("the answer", read_message(&mut revoking.recv, MAX_MESSAGE))
        .await
        .unwrap()
        .expect("an answer");
    assert_eq!(
        Revoke::parse(&body).unwrap(),
        Some(Revoke::Revoked { grant: old })
    );

    // It took nothing: the new grant stands on both machines, and the
    // stream opened under it still carries the guest's keys.
    assert_eq!(held(&pc), Some(new));
    assert_eq!(pc.grants(&guest), GrantState::Active);
    assert_eq!(mac.host_state(&host), Some(KnownState::Active));
    terminal.send.write_all(b"k").await.unwrap();
    let mut byte = [0u8; 1];
    within("the guest's key", there.recv.read_exact(&mut byte))
        .await
        .unwrap();
    assert_eq!(&byte, b"k");
}

#[tokio::test(start_paused = true)]
async fn the_seventh_confirm_or_removal_in_a_minute_goes_unanswered() {
    let net = Net::new();
    let pc = Desk::timed(&net, "windows-pc", "Windows PC");
    let stranger = Machine::new(&net, "stranger");
    let link = stranger.peers.dial(&pc.machine.addr()).await.unwrap();

    // Neither stream needs a grant, so anyone who reaches the host can
    // send these: each is answered the same, six times a minute.
    let mut pairing = link.open(StreamKind::Pairing).await.unwrap();
    for _ in 0..limits::RATE_PER_WINDOW {
        let confirm = Pairing::Confirm {
            grant: "nope".into(),
        };
        write_message(&mut pairing.send, &confirm).await.unwrap();
        assert!(matches!(
            read(&mut pairing).await,
            Pairing::Confirmed {
                state: ConfirmState::Unknown,
                ..
            }
        ));
    }
    let confirm = Pairing::Confirm {
        grant: "nope".into(),
    };
    write_message(&mut pairing.send, &confirm).await.unwrap();
    let ended = within(
        "the stream's end",
        read_message(&mut pairing.recv, MAX_MESSAGE),
    )
    .await;
    assert!(matches!(ended, Ok(None)), "the seventh is not answered");

    let mut revoking = link.open(StreamKind::Revoke).await.unwrap();
    for _ in 0..limits::RATE_PER_WINDOW {
        let revoke = Revoke::Revoke {
            grant: "nope".into(),
        };
        write_message(&mut revoking.send, &revoke).await.unwrap();
        let body = within("the answer", read_message(&mut revoking.recv, MAX_MESSAGE))
            .await
            .unwrap()
            .expect("an answer");
        assert_eq!(
            Revoke::parse(&body).unwrap(),
            Some(Revoke::Revoked {
                grant: "nope".into()
            })
        );
    }
    let revoke = Revoke::Revoke {
        grant: "nope".into(),
    };
    write_message(&mut revoking.send, &revoke).await.unwrap();
    let ended = within(
        "the stream's end",
        read_message(&mut revoking.recv, MAX_MESSAGE),
    )
    .await;
    assert!(matches!(ended, Ok(None)));

    // A minute later the same machine is answered again.
    tokio::time::sleep(limits::RATE_WINDOW).await; // wait-ok: the rate window passes, on the test's paused clock
    let mut pairing = link.open(StreamKind::Pairing).await.unwrap();
    let confirm = Pairing::Confirm {
        grant: "nope".into(),
    };
    write_message(&mut pairing.send, &confirm).await.unwrap();
    assert!(matches!(
        read(&mut pairing).await,
        Pairing::Confirmed { .. }
    ));
}
