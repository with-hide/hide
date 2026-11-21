//! Letters and reads between two cores (PRD device-connect-iroh D-72, B66):
//! a MacBook that opens a Mac mini, each a core of its own with its own
//! ledger. What the daemon carries between them is handed over here by the
//! test, message for message: the outbox of one, the other's answer, and a
//! read asked through the link.
//!
//! An agent of the machine that opens writes to an agent of the other and
//! gets its reply, each letter once; the machine that is only opened
//! answers and reports but writes nothing first; a letter the other core
//! refuses ends with its reason and blocks nothing behind it. A letter is
//! between agents: a pane is no recipient, and a sender Hide holds no
//! registration for writes none (PRD letters-missions-simplify D-21).

use super::access_letters::Fixture;
use super::*;
use crate::core_calls::{Agent, Asked, CoreLink, Letter, Outcome, Party, take};
use crate::delivery::Command;
use crate::delivery::ledger::State;
use crate::delivery::worker::Effect;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};

const MAC: &str = "macbook";
const MINI: &str = "mac-mini";
const WITHIN: Duration = Duration::from_secs(5);

fn mac_key() -> String {
    "a1".repeat(32)
}

fn mini_key() -> String {
    "b2".repeat(32)
}

/// The other core, reached as the daemon reaches it: what an agent asks of
/// it is prepared there as its machine's and run.
struct Wire {
    there: std::sync::Weak<Mutex<Runtime>>,
    /// How the other core knows this one.
    here: String,
    moved: AtomicUsize,
}

impl CoreLink for Wire {
    fn call(
        &self,
        _: &str,
        from: &Party,
        asked: &Asked,
        within: Duration,
    ) -> Result<Value, String> {
        let there = self.there.upgrade().ok_or("machine_unavailable")?;
        let call = there
            .lock()
            .unwrap()
            .prepare_core_call(&self.here, from, asked.clone())?;
        call.run(within)
    }

    fn outbox_moved(&self) {
        self.moved.fetch_add(1, Ordering::SeqCst);
    }
}

struct Pair {
    mac: Fixture,
    mini: Fixture,
    mac_wire: Arc<Wire>,
}

impl Pair {
    /// The MacBook opens the Mac mini, and the two are connected.
    fn start() -> Self {
        Self::of(Fixture::start(), Fixture::start())
    }

    /// [`Pair::start`], of two cores that already run.
    fn of(mac: Fixture, mini: Fixture) -> Self {
        let wire = |here: &Fixture, there: &Fixture, key: String| {
            let wire = Arc::new(Wire {
                there: Arc::downgrade(&there.runtime),
                here: key,
                moved: AtomicUsize::new(0),
            });
            here.runtime.lock().unwrap().set_core_link(wire.clone());
            wire
        };
        let mac_wire = wire(&mac, &mini, mac_key());
        wire(&mini, &mac, mini_key());
        let pair = Self {
            mac,
            mini,
            mac_wire,
        };
        pair.paired(true, false, true);
        pair
    }

    /// Tells both cores the pairing, as the daemon does: the MacBook opens
    /// the Mac mini when `mac_opens`, the Mac mini the MacBook when
    /// `mini_opens`.
    fn paired(&self, mac_opens: bool, mini_opens: bool, connected: bool) {
        let told = |fixture: &Fixture, key: String, device: &str, name: &str, opens, opened_by| {
            let event = serde_json::to_vec(&json!({
                "schema_version": SCHEMA_VERSION,
                "kind": "peer_machines",
                "payload": { "machines": [{
                    "key": key, "device_id": device, "name": name,
                    "opens": if opens { json!("active") } else { Value::Null },
                    "opened_by": if opened_by { json!("active") } else { Value::Null },
                    "connected": connected, "path": if connected { json!("direct") } else { Value::Null },
                    "reason": null, "update": null,
                }]},
            }))
            .unwrap();
            fixture.runtime.lock().unwrap().dispatch_json(&event);
        };
        told(
            &self.mac,
            mini_key(),
            MINI,
            "Mac mini",
            mac_opens,
            mini_opens,
        );
        told(&self.mini, mac_key(), MAC, "MacBook", mini_opens, mac_opens);
    }

    /// Hands what waits in `from`'s outbox to `to` and each answer back,
    /// as the connection does; the answers.
    fn carry(from: &Fixture, to: &Fixture, to_key: &str, from_key: &str) -> Vec<Outcome> {
        let outbox = from.runtime.lock().unwrap().core_outbox(to_key);
        outbox
            .settled(WITHIN)
            .into_iter()
            .map(|letter| {
                let id = letter.id.clone();
                let prepared = to
                    .runtime
                    .lock()
                    .unwrap()
                    .prepare_core_letter(from_key, letter);
                let outcome = take(prepared, WITHIN).unwrap();
                let settle =
                    from.runtime
                        .lock()
                        .unwrap()
                        .core_letter_answered(to_key, &id, outcome.clone());
                if let Some(settle) = settle {
                    settle.run(WITHIN).unwrap();
                }
                outcome
            })
            .collect()
    }

    fn mac_to_mini(&self) -> Vec<Outcome> {
        Self::carry(&self.mac, &self.mini, &mini_key(), &mac_key())
    }

    fn mini_to_mac(&self) -> Vec<Outcome> {
        Self::carry(&self.mini, &self.mac, &mac_key(), &mini_key())
    }
}

fn letters(fixture: &Fixture) -> Vec<crate::delivery::ledger::Letter> {
    let guard = fixture.runtime.lock().unwrap();
    guard.delivery_state().unwrap().letters.clone()
}

fn letter(fixture: &Fixture, id: &str) -> crate::delivery::ledger::Letter {
    letters(fixture)
        .into_iter()
        .find(|letter| letter.id == id)
        .unwrap_or_else(|| panic!("no letter {id}"))
}

fn send(target: &str, intent: &str, kind: &str) -> Command {
    Command::Send {
        target: target.into(),
        intent: intent.into(),
        body: format!("{intent}: please look"),
        kind: kind.into(),
        now: false,
        force: false,
    }
}

fn agent_there(device: &str, id: &str) -> String {
    format!("core:{device}:agent:{id}")
}

/// The fixture's worker as its core names it to another, by the name it
/// registered under.
fn agent(fixture: &Fixture) -> Agent {
    let guard = fixture.runtime.lock().unwrap();
    let ledger = guard.delivery_state().unwrap();
    let record = ledger
        .agents
        .iter()
        .find(|record| record.id == fixture.ids["worker"])
        .unwrap();
    Agent {
        id: record.id.clone(),
        name: record.wire_name().to_owned(),
    }
}

/// The worker as the one that asks a read or a start.
fn worker(fixture: &Fixture) -> Party {
    agent(fixture).into()
}

/// What `fixture`'s core hands machine `key` now.
fn outbox(fixture: &Fixture, key: &str) -> Vec<Letter> {
    let outbox = fixture.runtime.lock().unwrap().core_outbox(key);
    outbox.settled(WITHIN)
}

/// The cells Hide keeps for the fixture's worker, by subject and line.
fn cells(fixture: &Fixture) -> Vec<(String, String)> {
    let guard = fixture.runtime.lock().unwrap();
    let ledger = guard.delivery_state().unwrap();
    guard
        .status_cells(Some(&ledger))
        .into_iter()
        .filter(|addressed| addressed.recipient == fixture.ids["worker"])
        .map(|addressed| (addressed.cell.subject, addressed.cell.line))
        .collect()
}

/// The name the worker of the machine called `machine` goes by on the other
/// core.
fn named_there(machine: &str, fixture: &Fixture) -> String {
    format!("hide:core:{machine}:{}", agent(fixture).name)
}

/// B66: the letter waits in the sender's ledger, the other core keeps it
/// under the sending machine's name, and the reply names the sender's own
/// letter. Each is handed over once however often the connection asks.
#[test]
fn an_agent_writes_to_another_cores_agent_and_gets_its_reply_each_once() {
    let pair = Pair::start();
    let there = agent_there(MINI, &pair.mini.ids["worker"]);

    let sent = pair
        .mac
        .agent("worker", send(&there, "ask", "request"))
        .unwrap();
    let sent_id = sent["id"].as_str().unwrap().to_owned();
    assert_eq!(sent["recipient"]["pane_id"], format!("hide:{there}"));
    assert_eq!(letter(&pair.mac, &sent_id).state, State::Pending);
    assert_eq!(
        pair.mac_wire.moved.load(Ordering::SeqCst),
        1,
        "the daemon is told a letter waits to go"
    );
    let again = pair
        .mac
        .agent("worker", send(&there, "ask", "request"))
        .unwrap();
    assert_eq!(
        again["id"], sent["id"],
        "the same intent is the same letter"
    );

    assert_eq!(
        pair.mac_to_mini(),
        [Outcome::Taken {
            recipient: agent(&pair.mini)
        }]
    );
    let kept = letters(&pair.mini);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        kept[0].sender.name,
        named_there("MacBook", &pair.mac),
        "the letter says which machine wrote it"
    );
    assert_eq!(kept[0].recipient.pane_id, "worker");
    assert_eq!(kept[0].core_id.as_deref(), Some(sent_id.as_str()));
    assert_eq!(kept[0].state, State::Pending, "it waits for its recipient");
    let delivered = letter(&pair.mac, &sent_id);
    assert_eq!(delivered.state, State::Delivered);
    assert!(
        delivered.waiting_answer,
        "the request still waits for its answer"
    );
    assert_eq!(
        delivered.recipient.name,
        named_there("Mac mini", &pair.mini)
    );
    assert!(
        pair.mac_to_mini().is_empty(),
        "a taken letter is not sent again"
    );
    let again = pair
        .mac
        .agent("worker", send(&there, "ask", "request"))
        .unwrap();
    assert_eq!(
        again["id"], sent["id"],
        "and the same send is still that letter"
    );

    let reply = pair
        .mini
        .agent(
            "worker",
            Command::Reply {
                id: kept[0].id.clone(),
                intent: "answer".into(),
                body: "done".into(),
                now: false,
            },
        )
        .unwrap();
    let reply_id = reply["id"].as_str().unwrap();
    assert_eq!(
        pair.mini_to_mac(),
        [Outcome::Taken {
            recipient: agent(&pair.mac)
        }]
    );
    let answered = letters(&pair.mac);
    let answer = answered
        .iter()
        .find(|letter| letter.kind == "reply")
        .expect("the reply");
    assert_eq!(answer.reply_to.as_deref(), Some(sent_id.as_str()));
    assert_eq!(answer.sender.name, named_there("Mac mini", &pair.mini));
    assert_eq!(answer.recipient.pane_id, "worker");
    assert_eq!(answer.body, "done");
    assert!(!letter(&pair.mac, &sent_id).waiting_answer);
    assert_eq!(letter(&pair.mini, reply_id).state, State::Delivered);
    assert!(pair.mini_to_mac().is_empty());
    assert_eq!(answered.len(), 2, "the letter and its one reply");
}

/// B66: with no connection the letter stays in the sender's ledger, which
/// gives it no deadline, and when the two connect it arrives once, even
/// when the first answer never came back.
#[test]
fn a_letter_written_while_the_other_machine_is_away_waits_and_arrives_once() {
    let pair = Pair::start();
    pair.paired(true, false, false);
    let there = agent_there(MINI, &pair.mini.ids["worker"]);
    let sent = pair
        .mac
        .agent("worker", send(&there, "later", "request"))
        .unwrap();
    let sent_id = sent["id"].as_str().unwrap();
    assert_eq!(letter(&pair.mac, sent_id).state, State::Pending);

    pair.paired(true, false, true);
    // The Mac mini keeps the letter, and its answer is lost on the way.
    let waiting = outbox(&pair.mac, &mini_key());
    assert_eq!(waiting.len(), 1);
    let prepared = pair
        .mini
        .runtime
        .lock()
        .unwrap()
        .prepare_core_letter(&mac_key(), waiting[0].clone());
    assert!(matches!(take(prepared, WITHIN), Ok(Outcome::Taken { .. })));
    assert_eq!(letter(&pair.mac, sent_id).state, State::Pending);
    // Asked again, it is the letter the Mac mini already keeps.
    assert_eq!(pair.mac_to_mini().len(), 1);
    assert_eq!(letters(&pair.mini).len(), 1, "it arrived once");
    assert_eq!(letter(&pair.mac, sent_id).state, State::Delivered);
}

/// D-72, B66: the machine that is only opened asked for nothing here, so
/// its agents write nothing first, whichever core is asked; it reports the
/// result of a start this machine's agent asked it for, to that agent
/// only. With grants both ways both write.
#[test]
fn a_machine_that_is_only_opened_reports_a_start_and_writes_nothing_first() {
    let pair = Pair::start();
    let mac_worker = agent_there(MAC, &pair.mac.ids["worker"]);
    assert_eq!(
        pair.mini
            .agent("worker", send(&mac_worker, "first", "request"))
            .unwrap_err(),
        "machine_not_permitted",
        "refused where it is written"
    );
    assert!(letters(&pair.mini).is_empty());

    // The MacBook's core refuses it whoever carried it there.
    let wire = |kind: &str, intent: &str, to: &str| Letter {
        id: format!("letter-{intent}"),
        from: agent(&pair.mini),
        to: to.into(),
        reply_to: None,
        kind: kind.into(),
        intent: intent.into(),
        body: "the work is done".into(),
        now: false,
    };
    let asked = |letter: Letter| {
        let prepared = pair
            .mac
            .runtime
            .lock()
            .unwrap()
            .prepare_core_letter(&mini_key(), letter);
        take(prepared, WITHIN).unwrap()
    };
    let refused = Outcome::Refused {
        reason: "machine_not_permitted".into(),
    };
    let mac_id = pair.mac.ids["worker"].clone();
    assert_eq!(asked(wire("request", "first", &mac_id)), refused);
    assert_eq!(
        asked(wire("report", "result", &mac_id)),
        refused,
        "a report of nothing this machine asked for is a first letter"
    );

    // The MacBook's worker asks the Mac mini for a start: the store
    // answers with the agent it asks as.
    let (client, origin, worker_record) = {
        let guard = pair.mac.runtime.lock().unwrap();
        let ledger = guard.delivery_state().unwrap();
        let record = ledger
            .agents
            .iter()
            .find(|record| record.id == mac_id)
            .unwrap()
            .clone();
        (
            guard.delivery_client.clone().unwrap(),
            record.actor.clone(),
            record,
        )
    };
    let asked_as = client
        .submit(
            Effect::CoreStart {
                origin,
                key: mini_key(),
                registers: None,
            },
            WITHIN,
        )
        .unwrap();
    assert_eq!(asked_as["id"], mac_id.as_str());
    // The ledger then remembers as many starts as it can, every asker
    // still alive: one more is refused before it leaves, and the report of
    // the first, a mission that ran through all of them, is still taken.
    // An asker with no registration is registered with its start.
    let asker = |n: usize| {
        let actor = crate::delivery::Actor {
            pane_id: format!("w9:p{n}"),
            name: format!("asker-{n}"),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: crate::engine::tokens::session_digest(&format!("asker-{n}")),
        };
        crate::coordination::AgentRecord {
            id: String::new(),
            name: Some(actor.name.clone()),
            session: format!("asker-{n}"),
            instance: format!("terminal-asker-{n}"),
            pane: actor.pane_id.clone(),
            actor,
            ..worker_record.clone()
        }
    };
    let start = |n: usize| {
        let record = asker(n);
        client.submit(
            Effect::CoreStart {
                origin: record.actor.clone(),
                key: mini_key(),
                registers: Some(Box::new(record)),
            },
            WITHIN,
        )
    };
    for n in 1..crate::delivery::ledger::CORE_STARTS {
        start(n).unwrap();
    }
    assert_eq!(start(0).unwrap_err(), "capacity");
    // An agent Hide holds no registration for asks no start.
    assert_eq!(
        client
            .submit(
                Effect::CoreStart {
                    origin: asker(0).actor,
                    key: mini_key(),
                    registers: None,
                },
                WITHIN,
            )
            .unwrap_err(),
        "sender_not_registered"
    );
    assert_eq!(
        asked(wire("report", "result", &mac_id)),
        Outcome::Taken {
            recipient: agent(&pair.mac)
        }
    );
    let report = &letters(&pair.mac)[0];
    assert_eq!(report.kind, "report");
    assert_eq!(report.sender.name, named_there("Mac mini", &pair.mini));
    assert_eq!(report.recipient.pane_id, "worker");
    assert_eq!(
        asked(wire("request", "first", &mac_id)),
        refused,
        "a start lets its result in, not a question"
    );
    assert_eq!(
        asked(wire("report", "result-other", &pair.mac.ids["other"])),
        refused,
        "and only to the agent that asked"
    );

    pair.paired(true, true, true);
    pair.mini
        .agent("worker", send(&mac_worker, "first", "request"))
        .unwrap();
    assert_eq!(
        pair.mini_to_mac(),
        [Outcome::Taken {
            recipient: agent(&pair.mac)
        }]
    );
}

/// B65: the report of a mission closed on the Mac mini goes to the
/// MacBook's agent that asked for its Conductor, under the Mac mini's
/// name. Written while the MacBook is away it waits, the same close asked
/// again is the same letter, and it arrives once when the MacBook is back.
#[test]
fn a_closed_missions_report_reaches_the_agent_that_asked_once_its_machine_is_back() {
    let pair = Pair::start();
    let mac_id = pair.mac.ids["worker"].clone();
    // The MacBook's worker asked the Mac mini for a Conductor: the MacBook
    // keeps who asked, and the Mac mini registered the asker as the origin
    // of what it started, as a start does.
    let (client, asker) = {
        let guard = pair.mac.runtime.lock().unwrap();
        let ledger = guard.delivery_state().unwrap();
        let asker = ledger
            .agents
            .iter()
            .find(|record| record.id == mac_id)
            .unwrap()
            .actor
            .clone();
        (guard.delivery_client.clone().unwrap(), asker)
    };
    client
        .submit(
            Effect::CoreStart {
                origin: asker,
                key: mini_key(),
                registers: None,
            },
            WITHIN,
        )
        .unwrap();
    let (client, authority, origin) = {
        let guard = pair.mini.runtime.lock().unwrap();
        let origin = guard.core_actor(guard.peer_on(MAC).unwrap(), &worker(&pair.mac));
        let authority = crate::delivery::worker::Authority {
            caller: crate::delivery::Actor::core_caller(&mac_key(), &origin.pane_id),
            context: guard.core_context(&origin),
            also_reads: Vec::new(),
            proven: crate::delivery::worker::Proven::Now,
            registers: Vec::new(),
            written: None,
        };
        (guard.delivery_client.clone().unwrap(), authority, origin)
    };
    crate::coordination::executor::register_code_owned(&client, &authority, &origin).unwrap();

    // The letter the close owes an origin on the MacBook, sent as its
    // Conductor sends any letter.
    let report = || {
        crate::runtime::mission_report::letter(
            agent_there(MAC, &mac_id),
            "mission-1",
            "Fix the bug",
            crate::missions::MissionStatus::Done,
            Some("Merged as #12."),
        )
    };
    pair.paired(true, false, false);
    let sent = pair.mini.agent("worker", report()).unwrap();
    let sent_id = sent["id"].as_str().unwrap().to_owned();
    assert_eq!(letter(&pair.mini, &sent_id).state, State::Pending);
    assert_eq!(
        pair.mini.agent("worker", report()).unwrap()["id"],
        sent_id.as_str(),
        "the same close asked again"
    );

    pair.paired(true, false, true);
    assert_eq!(
        pair.mini_to_mac(),
        [Outcome::Taken {
            recipient: agent(&pair.mac)
        }]
    );
    assert!(pair.mini_to_mac().is_empty(), "it is handed over once");
    let arrived = letters(&pair.mac);
    assert_eq!(arrived.len(), 1);
    assert_eq!(arrived[0].kind, "report");
    assert_eq!(arrived[0].sender.name, named_there("Mac mini", &pair.mini));
    assert_eq!(arrived[0].recipient.pane_id, "worker");
    assert_eq!(
        arrived[0].body,
        "Mission mission-1 \"Fix the bug\" closed as done.\n\nMerged as #12."
    );
    assert_eq!(letter(&pair.mini, &sent_id).state, State::Delivered);
}

/// D-72: a letter the other core does not keep is answered with why, ends
/// in its sender's ledger with that reason, and its sender is told once by
/// a cell about it; the letter behind it goes through.
#[test]
fn a_letter_the_other_core_refuses_ends_with_its_reason_and_its_sender_is_told() {
    let pair = Pair::start();
    let lost = pair
        .mac
        .agent(
            "worker",
            send(&agent_there(MINI, "nobody"), "lost", "request"),
        )
        .unwrap();
    let lost_id = lost["id"].as_str().unwrap().to_owned();
    let kept = pair
        .mac
        .agent(
            "worker",
            send(
                &agent_there(MINI, &pair.mini.ids["worker"]),
                "kept",
                "request",
            ),
        )
        .unwrap();
    assert!(cells(&pair.mac).is_empty());
    assert_eq!(
        pair.mac_to_mini(),
        [
            Outcome::Refused {
                reason: "target_unavailable".into()
            },
            Outcome::Taken {
                recipient: agent(&pair.mini)
            },
        ]
    );
    assert!(
        pair.mac_to_mini().is_empty(),
        "a refused letter is not asked again"
    );
    let ended = letter(&pair.mac, &lost_id);
    assert_eq!(ended.state, State::NoRecipient);
    assert!(!ended.waiting_answer);
    assert_eq!(
        ended.bell_failure.map(|failure| failure.code).as_deref(),
        Some("target_unavailable")
    );
    assert_eq!(
        letter(&pair.mac, kept["id"].as_str().unwrap()).state,
        State::Delivered
    );
    assert_eq!(letters(&pair.mini).len(), 1);

    let told = cells(&pair.mac);
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0].0, format!("letter:{lost_id}"));
    for part in [lost_id.as_str(), "target_unavailable", "not delivered"] {
        assert!(told[0].1.contains(part), "{part}: {}", told[0].1);
    }
    // The answer coming once more moves nothing and tells no one again.
    let again = pair.mac.runtime.lock().unwrap().core_letter_answered(
        &mini_key(),
        &lost_id,
        Outcome::Refused {
            reason: "capacity".into(),
        },
    );
    assert!(again.is_none());
    assert_eq!(cells(&pair.mac), told);
}

/// PRD letters-missions-simplify D-21: a letter between two machines names
/// an agent's registration on each side. A pane of the other machine is no
/// recipient, the other core takes a letter for a registration only, and a
/// letter whose agent ended with no agent after it ends for its sender the
/// same way however often it is asked.
#[test]
fn a_letter_between_machines_names_registrations_and_no_pane() {
    let pair = Pair::start();
    assert_eq!(
        pair.mac
            .agent(
                "worker",
                send(&format!("core:{MINI}:pane:worker"), "pane", "request")
            )
            .unwrap_err(),
        "recipient_not_agent",
        "refused where it is written"
    );
    assert!(letters(&pair.mac).is_empty());

    // The other core takes a pane's id for no agent, whoever carried it,
    // and a registration's id for its agent.
    pair.paired(true, true, true);
    let wire = |id: &str, to: &str| Letter {
        id: id.into(),
        from: agent(&pair.mini),
        to: to.into(),
        reply_to: None,
        kind: "request".into(),
        intent: format!("intent-{id}"),
        body: "please look".into(),
        now: false,
    };
    let asked = |letter: Letter| {
        let prepared = pair
            .mac
            .runtime
            .lock()
            .unwrap()
            .prepare_core_letter(&mini_key(), letter);
        take(prepared, WITHIN).unwrap()
    };
    assert_eq!(
        asked(wire("letter-1", "w1:p9")),
        Outcome::Refused {
            reason: "target_unavailable".into()
        }
    );
    let other = pair.mac.ids["other"].clone();
    let to_other = asked(wire("letter-2", &other));
    let Outcome::Taken { recipient } = &to_other else {
        panic!("{to_other:?}");
    };
    assert_eq!(recipient.id, other);
    let kept = letters(&pair.mac);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].to.as_deref(), Some(other.as_str()));
    assert_eq!(
        kept[0].from, None,
        "another machine's agent has no registration here"
    );

    // The same letter asked again is answered as it is kept, and once its
    // agent ended with no agent after it, as ended.
    assert_eq!(asked(wire("letter-2", &other)), to_other);
    pair.mac
        .agent(
            "other",
            Command::Agents {
                command: crate::coordination::Command::End {
                    id: other.clone(),
                    actor: None,
                    keep_pane: true,
                },
            },
        )
        .unwrap();
    assert_eq!(
        asked(wire("letter-2", &other)),
        Outcome::Refused {
            reason: "no_recipient".into()
        }
    );
    assert_eq!(letters(&pair.mac).len(), 1, "kept once");
}

/// B66: an agent of the machine that opens reads the other machine's
/// agents through that machine's core; the machine that is only opened
/// reads nothing, typing into a pane is no machine's, and a machine that is
/// away is not asked.
#[test]
fn an_agent_reads_another_cores_agents_and_the_opened_machine_reads_none() {
    use crate::coordination::Command as Agents;
    use crate::pane_command::PaneCommand;
    let pair = Pair::start();
    let list = |machine: &str| Command::Agents {
        command: Agents::List {
            live: false,
            machine: Some(machine.into()),
        },
    };
    let listed = pair.mac.agent("worker", list(MINI)).unwrap();
    let mut ids: Vec<&str> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    ids.sort();
    let mut there: Vec<&str> = pair.mini.ids.values().map(String::as_str).collect();
    there.sort();
    assert_eq!(ids, there, "the Mac mini's own agents");
    let shown = pair
        .mac
        .agent(
            "worker",
            Command::Agents {
                command: Agents::Show {
                    id: agent_there(MINI, &pair.mini.ids["other"]),
                },
            },
        )
        .unwrap();
    assert_eq!(shown["id"], pair.mini.ids["other"]);

    assert_eq!(
        pair.mini.agent("worker", list(MAC)).unwrap_err(),
        "machine_not_permitted"
    );
    let forged = pair.mac.runtime.lock().unwrap().prepare_core_call(
        &mini_key(),
        &worker(&pair.mini),
        Asked::Delivery(Box::new(list(MAC))),
    );
    assert_eq!(forged.err().as_deref(), Some("machine_not_permitted"));

    let pane = |command: PaneCommand| {
        let context = crate::runtime::delivery::tests::authority(&crate::delivery::Actor {
            pane_id: "worker".into(),
            name: "worker".into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: None,
        })
        .context;
        pair.mac
            .runtime
            .lock()
            .unwrap()
            .prepare_pane_command(crate::node::TEST_NODE, "worker", &context, command)
            .map(drop)
    };
    let pane_there = format!("core:{MINI}:pane:worker");
    assert_eq!(
        pane(PaneCommand::Send {
            pane: pane_there.clone(),
            text: Some("hello".into()),
            keys: Vec::new(),
            enter: true,
            paste: false,
        }),
        Err("machine_not_permitted".into())
    );
    assert_eq!(
        pane(PaneCommand::Read {
            pane: pane_there.clone(),
            lines: 10
        }),
        Ok(()),
        "a read is the other core's to answer"
    );

    pair.paired(true, false, false);
    assert_eq!(
        pair.mac.agent("worker", list(MINI)).unwrap_err(),
        "machine_unavailable"
    );
    assert_eq!(
        pane(PaneCommand::Read {
            pane: pane_there,
            lines: 10
        }),
        Err("machine_unavailable".into())
    );
}

/// A device id is what a machine says of itself, so a second connection may
/// say the one a paired machine says. What was stored for one connection
/// stays its own: the other is handed none of its letters, settles and
/// answers none, its agents are other identities, and a `core:` name two
/// connections claim names neither.
#[test]
fn two_connections_that_say_one_device_id_share_no_letter_answer_or_identity() {
    let pair = Pair::start();
    let there = agent_there(MINI, &pair.mini.ids["worker"]);
    let sent = pair
        .mac
        .agent("worker", send(&there, "ask", "request"))
        .unwrap();
    let sent_id = sent["id"].as_str().unwrap().to_owned();

    // A second connection that says the Mac mini's device id, paired both
    // ways and connected.
    let other_key = "c3".repeat(32);
    let machine = |key: &str| {
        json!({
            "key": key, "device_id": MINI, "name": "Mac mini",
            "opens": "active", "opened_by": "active",
            "connected": true, "path": "direct", "reason": null, "update": null,
        })
    };
    let event = serde_json::to_vec(&json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "peer_machines",
        "payload": { "machines": [machine(&mini_key()), machine(&other_key)] },
    }))
    .unwrap();
    pair.mac.runtime.lock().unwrap().dispatch_json(&event);
    let outbox = |key: &str| outbox(&pair.mac, key).len();

    // The letter kept for the first connection goes to it alone, and the
    // other's "taken" or "refused" moves nothing of it.
    assert_eq!((outbox(&other_key), outbox(&mini_key())), (0, 1));
    for outcome in [
        Outcome::Taken {
            recipient: agent(&pair.mini),
        },
        Outcome::Refused {
            reason: "capacity".into(),
        },
    ] {
        let settle = pair
            .mac
            .runtime
            .lock()
            .unwrap()
            .core_letter_answered(&other_key, &sent_id, outcome);
        if let Some(settle) = settle {
            settle.run(WITHIN).unwrap();
        }
    }
    assert_eq!(letter(&pair.mac, &sent_id).state, State::Pending);
    assert_eq!(outbox(&mini_key()), 1, "it still waits to go");

    // Only the connection it was written to answers it.
    let wire = |id: &str, intent: &str, reply_to: Option<&str>| Letter {
        id: id.into(),
        from: agent(&pair.mini),
        to: pair.mac.ids["worker"].clone(),
        reply_to: reply_to.map(str::to_owned),
        kind: if reply_to.is_some() {
            "reply"
        } else {
            "request"
        }
        .into(),
        intent: intent.into(),
        body: "done".into(),
        now: false,
    };
    let asked = |key: &str, letter: Letter| {
        let prepared = pair
            .mac
            .runtime
            .lock()
            .unwrap()
            .prepare_core_letter(key, letter);
        take(prepared, WITHIN).unwrap()
    };
    assert_eq!(
        asked(&other_key, wire("letter-5", "answer", Some(&sent_id))),
        Outcome::Refused {
            reason: "letter_unavailable".into()
        }
    );
    assert!(matches!(
        asked(&mini_key(), wire("letter-5", "answer", Some(&sent_id))),
        Outcome::Taken { .. }
    ));

    // The same agent id on the two connections is two identities.
    for (key, id, intent) in [
        (mini_key(), "letter-6", "one"),
        (other_key.clone(), "letter-7", "two"),
    ] {
        assert!(matches!(
            asked(&key, wire(id, intent, None)),
            Outcome::Taken { .. }
        ));
    }
    let kept = letters(&pair.mac);
    let from = |intent: &str| {
        kept.iter()
            .find(|letter| letter.intent == intent)
            .unwrap()
            .sender
            .clone()
    };
    assert_eq!(from("one").pane_id, from("two").pane_id);
    assert!(!from("one").same_identity(&from("two")));
    assert!(from("one").core_of(&mini_key()) && from("two").core_of(&other_key));

    // A name two connections claim names neither.
    assert_eq!(
        pair.mac
            .agent("worker", send(&there, "again", "request"))
            .unwrap_err(),
        "device_ambiguous"
    );
}

/// D-72: what another machine's agent asks is judged again where it
/// applies. A grant taken back after a first letter, or a step of a start,
/// was prepared refuses it there, while the other direction still stands.
#[test]
fn a_grant_taken_back_after_an_ask_was_prepared_refuses_it_where_it_applies() {
    let pair = Pair::start();
    // Each machine opens the other.
    pair.paired(true, true, true);
    let first = || Letter {
        id: "letter-9".into(),
        from: agent(&pair.mini),
        to: pair.mac.ids["worker"].clone(),
        reply_to: None,
        kind: "request".into(),
        intent: "first".into(),
        body: "please look".into(),
        now: false,
    };
    let prepare = || {
        pair.mac
            .runtime
            .lock()
            .unwrap()
            .prepare_core_letter(&mini_key(), first())
    };
    let prepared = prepare();
    assert!(prepared.is_ok());
    // A step of a start the Mac mini's agent asks for here: its own record
    // as the origin of what it starts.
    let (client, authority, origin) = {
        let guard = pair.mac.runtime.lock().unwrap();
        let origin = guard.core_actor(guard.peer_on(MINI).unwrap(), &worker(&pair.mini));
        let authority = crate::delivery::worker::Authority {
            caller: crate::delivery::Actor::core_caller(&mini_key(), &origin.pane_id),
            context: guard.core_context(&origin),
            also_reads: Vec::new(),
            proven: crate::delivery::worker::Proven::Now,
            registers: Vec::new(),
            written: None,
        };
        (guard.delivery_client.clone().unwrap(), authority, origin)
    };
    let registers =
        || crate::coordination::executor::register_code_owned(&client, &authority, &origin);

    // The MacBook takes back the Mac mini's grant and still opens the Mac
    // mini itself.
    pair.paired(true, false, true);
    assert_eq!(
        take(prepared, WITHIN).unwrap(),
        Outcome::Refused {
            reason: "machine_not_permitted".into()
        }
    );
    assert!(letters(&pair.mac).is_empty());
    assert_eq!(registers().unwrap_err(), "machine_not_permitted");

    // Granted again, the same asks apply.
    pair.paired(true, true, true);
    assert!(matches!(
        take(prepare(), WITHIN).unwrap(),
        Outcome::Taken { .. }
    ));
    registers().unwrap();
}

/// PRD device-connect-iroh D-71, D-72 over the state folders released
/// builds wrote. What this build is the first to keep of another machine in
/// the ledger is written by its own code beside what the release left: a
/// letter that waits for an agent there, under that machine's connection id
/// and the agent's address; a letter kept from there, under the id its own
/// core gave it; and the start an agent asked of it. The next start is the
/// same ledger: its conversions find nothing to do and write nothing, the
/// store reads every record as it was written, the letter still goes out,
/// the kept one is answered as kept and the start's report is still let in.
#[test]
fn what_a_ledger_keeps_of_another_machine_over_a_released_state_folder_is_the_same_at_the_next_start()
 {
    use crate::node_migration::upgrade_tests::{fixtures, second_start, started_at};
    let carried = |pair: &Pair, letter: Letter| {
        let prepared = pair
            .mac
            .runtime
            .lock()
            .unwrap()
            .prepare_core_letter(&mini_key(), letter);
        take(prepared, WITHIN).unwrap()
    };
    let folders = fixtures();
    assert!(!folders.is_empty(), "no released state folder was found");
    for folder in folders {
        let tag = folder.file_name().unwrap().to_string_lossy().into_owned();
        let root = tempfile::tempdir().unwrap();
        // Where the fixture's core keeps its state.
        started_at(&folder, &root.path().join("state"));
        let pair = Pair::of(Fixture::start_in(root), Fixture::start());
        let released = letters(&pair.mac).len();
        let mac_id = pair.mac.ids["worker"].clone();
        let mini_id = pair.mini.ids["worker"].clone();

        pair.mac
            .agent(
                "worker",
                send(&agent_there(MINI, &mini_id), "waits", "request"),
            )
            .unwrap_or_else(|refusal| panic!("{tag}: {refusal}"));
        let (client, origin) = {
            let guard = pair.mac.runtime.lock().unwrap();
            let ledger = guard.delivery_state().unwrap();
            let record = ledger.agents.iter().find(|record| record.id == mac_id);
            (
                guard.delivery_client.clone().unwrap(),
                record.unwrap().actor.clone(),
            )
        };
        client
            .submit(
                Effect::CoreStart {
                    origin,
                    key: mini_key(),
                    registers: None,
                },
                WITHIN,
            )
            .unwrap_or_else(|refusal| panic!("{tag}: {refusal}"));
        let report = |id: &str, intent: &str| Letter {
            id: id.into(),
            from: agent(&pair.mini),
            to: mac_id.clone(),
            reply_to: None,
            kind: "report".into(),
            intent: intent.into(),
            body: "the work is done".into(),
            now: false,
        };
        let (first, second) = (report("letter-70", "result"), report("letter-71", "more"));
        let taken = Outcome::Taken {
            recipient: agent(&pair.mac),
        };
        assert_eq!(carried(&pair, first.clone()), taken, "{tag}");

        let Pair { mac, mini, .. } = pair;
        let before = (*mac.runtime.lock().unwrap().delivery_state().unwrap()).clone();
        let [waiting, kept] = &before.letters[released..] else {
            panic!("{tag}: {:?}", &before.letters[released..]);
        };
        assert!(waiting.recipient.core_of(&mini_key()), "{tag}");
        assert_eq!(
            waiting.recipient.core_address().map(|address| address.id),
            Some(mini_id.as_str()),
            "{tag}"
        );
        assert!(kept.sender.core_of(&mini_key()), "{tag}");
        assert_eq!(kept.core_id.as_deref(), Some("letter-70"), "{tag}");
        let starts: Vec<(&str, &str)> = before
            .core_starts
            .iter()
            .map(|start| (start.origin.as_str(), start.key.as_str()))
            .collect();
        assert_eq!(starts, [(mac_id.as_str(), mini_key().as_str())], "{tag}");

        // The next start, as `hided` runs it before the core reads the
        // folder, and the ledger as the store then reads it.
        let root = mac.stop();
        let state = root.path().join("state");
        assert_eq!(second_start(&state), Vec::<String>::new(), "{tag}");
        let after = crate::delivery::store::load(&hide_kit::layout::delivery_store(&state))
            .unwrap_or_else(|refusal| panic!("{tag}: {refusal}"));
        assert_eq!(after.letters, before.letters, "{tag}");
        assert_eq!(after.agents, before.agents, "{tag}");
        assert_eq!(after.core_starts, before.core_starts, "{tag}");

        // The core that starts over it is the same to the other machine.
        let pair = Pair::of(Fixture::start_in(root), mini);
        assert_eq!(pair.mac.ids["worker"], mac_id, "{tag}");
        let leaving = outbox(&pair.mac, &mini_key());
        assert_eq!(leaving.len(), 1, "{tag}: {leaving:?}");
        assert_eq!(leaving[0].id, waiting.id, "{tag}");
        assert_eq!(leaving[0].to, mini_id, "{tag}");
        assert_eq!(leaving[0].from, agent(&pair.mac), "{tag}");
        assert_eq!(
            pair.mac_to_mini(),
            [Outcome::Taken {
                recipient: agent(&pair.mini)
            }],
            "{tag}"
        );
        assert_eq!(carried(&pair, first), taken, "{tag}");
        assert_eq!(
            letters(&pair.mac).len(),
            before.letters.len(),
            "{tag}: kept once"
        );
        assert_eq!(carried(&pair, second), taken, "{tag}");
    }
}
