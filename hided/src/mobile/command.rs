//! What `hide phone` asks this machine's daemon (PRD device-connect-iroh
//! D-70, B61): the phones paired with this computer, removing one, and a
//! pairing code for a computer with no window to open Settings > Mobile in.
//!
//! Asked over the bootstrap socket (`pane_auth`), one ask per connection. A
//! program in a Hide pane may list and remove; only a terminal outside the
//! panes pairs, since a pairing lets a phone in (D-53). A bot lists the
//! phones in the whole Hide's mode and pairs or removes none, as it does no
//! machine (`crate::pairing`, mission-87). The answer to `pair` carries the
//! link the QR encodes, which `phone_cli` draws and never prints.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Mobile, phones};
use crate::pairing::Caller;

/// How long `pair` waits for Mobile to reach the relay (or Tailscale) before
/// it can show a code: a relay socket opens in well under this, and a
/// network that does not answer by then is not going to while a person
/// waits at the terminal.
const EXPOSED_WITHIN: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "verb", rename_all = "snake_case", deny_unknown_fields)]
pub enum Ask {
    List,
    /// A phone by its id or its name.
    Remove {
        phone: String,
    },
    Pair,
    /// Where the code `pair` answered stands, named by its [`code_id`].
    PairStatus {
        code_id: String,
    },
    /// Ends the code `pair` answered, when the command will not show it.
    PairEnd {
        code_id: String,
    },
    /// The interface language the operator chose, which `pair` words its
    /// lines in; `null` for none, which the command resolves with its own
    /// system language as every screen does (docs/LOCALIZATION.md).
    Language,
}

pub fn refused(code: &str) -> Value {
    json!({"ok": false, "reason": code, "next_action": next_action(code)})
}

/// What the person does about a refusal `code`.
pub fn next_action(code: &str) -> &'static str {
    match code {
        crate::pairing::DEVICE_CONNECT_OFF => {
            "This build does not offer hide phone yet: pair a phone from Settings > Mobile"
        }
        "phone_pair_needs_terminal" => {
            "hide phone pair draws a QR code for a person to scan: run it in a terminal window"
        }
        "phone_window_small" => "Make this terminal window bigger and run hide phone pair again",
        "phone_pair_ended" => {
            "The QR expired or was replaced before a phone scanned it: run hide phone pair again"
        }
        "phone_pair_from_pane" => {
            "Ask the operator to run hide phone pair in a terminal outside Hide's panes"
        }
        "phone_limit" => {
            "Four phones are paired with this computer: remove one with hide phone remove <phone> first"
        }
        "phone_unknown" => "No phone of that id or name is paired: hide phone list shows them",
        "phone_ambiguous" => "More than one phone has that name: remove it by its id",
        "mobile_unreachable" => {
            "Hide could not reach its relay from this computer: check the network and run hide phone pair again"
        }
        "mobile_failed" => {
            "Mobile could not be set up on this computer: hide status and Settings > Mobile say why"
        }
        // A bot's refusal, another screen's, or one no phone code names.
        code => crate::pairing::next_action(code),
    }
}

impl Ask {
    /// What a bot's mode has to grant for this ask (mission-87): reading the
    /// phones is the whole Hide's, as `hide device list` is; pairing and
    /// removing one is the operator's alone, as adding or removing a machine
    /// is, and so is the code a pairing shows.
    fn bot_need(&self) -> hide_core::access::Need {
        match self {
            Self::List | Self::Language => hide_core::access::Need::All,
            _ => hide_core::access::Need::Operator,
        }
    }
}

impl Mobile {
    /// Answers one `hide phone` ask.
    pub async fn answer_cli(self: &Arc<Self>, ask: Ask, caller: Caller) -> Value {
        match caller {
            Caller::Operator => {}
            Caller::Pane if ask != Ask::Pair => {}
            Caller::Pane => return refused("phone_pair_from_pane"),
            // The bootstrap socket names no other screen; one that did would
            // be refused here, as `crate::pairing` refuses it.
            Caller::OtherScreen => return refused("device_from_other_screen"),
            Caller::Bot(mode) => {
                if let Err(code) = hide_core::access::admit(mode, ask.bot_need()) {
                    return refused(code);
                }
            }
        }
        match ask {
            Ask::Language => json!({
                "ok": true,
                "interface_language": self.projection.borrow().interface_language,
            }),
            Ask::List => self.cli_list(),
            Ask::Remove { phone } => self.cli_remove(&phone),
            Ask::Pair => self.cli_pair().await,
            Ask::PairStatus { code_id } => self.cli_pair_status(&code_id),
            Ask::PairEnd { code_id: wanted } => {
                let ended = {
                    let mut inner = self.lock();
                    let shown = inner
                        .phones
                        .code()
                        .is_some_and(|(code, _)| code_id(code) == wanted);
                    if shown {
                        inner.phones.clear_code();
                    }
                    shown
                };
                if ended {
                    self.publish();
                }
                json!({"ok": true})
            }
        }
    }

    /// The phones as Settings > Mobile lists them, from the frame it draws.
    fn cli_list(&self) -> Value {
        let frame = self.subscribe_frame().borrow().clone();
        let phones = frame.get("phones").cloned().unwrap_or_else(|| json!([]));
        json!({
            "ok": true,
            "enabled": frame.get("enabled").cloned().unwrap_or(json!(false)),
            "phones": phones,
            "max_phones": phones::MAX_PHONES,
        })
    }

    fn cli_remove(&self, wanted: &str) -> Value {
        let found = {
            let inner = self.lock();
            let by_id = inner.phones.current().find(|phone| phone.id == wanted);
            match by_id {
                Some(phone) => Ok((phone.id.clone(), phone.name.clone())),
                None => {
                    let mut named = inner.phones.current().filter(|phone| phone.name == wanted);
                    match (named.next(), named.next()) {
                        (Some(phone), None) => Ok((phone.id.clone(), phone.name.clone())),
                        (Some(_), Some(_)) => Err("phone_ambiguous"),
                        (None, _) => Err("phone_unknown"),
                    }
                }
            }
        };
        match found {
            Ok((id, name)) => {
                self.revoke(&id, "revoked_from_cli");
                json!({"ok": true, "phone_id": id, "name": name})
            }
            Err(code) => refused(code),
        }
    }

    /// Turns Mobile on when it is off, waits for it to be reachable, and
    /// issues a code, which voids any earlier one as Settings' QR button
    /// does. The answer carries the QR's link and when the code expires.
    async fn cli_pair(self: &Arc<Self>) -> Value {
        let (enabled, full) = {
            let inner = self.lock();
            (
                inner.settings.enabled,
                inner.phones.current().count() >= phones::MAX_PHONES,
            )
        };
        if full {
            return refused("phone_limit");
        }
        let mut frames = self.subscribe_frame();
        if !enabled
            && let Err(message) = self.handle_event(
                0,
                &json!({"kind": "mobile_enable", "payload": {"enabled": true}}),
            )
        {
            hide_core::diagnostic!(json!({
                "component": "mobile_pairing", "kind": "cli.enable_failed", "message": message,
            }));
            return refused("mobile_failed");
        }
        let reached = tokio::time::timeout(EXPOSED_WITHIN, async {
            loop {
                let exposure = frames.borrow_and_update()["exposure"]
                    .as_str()
                    .unwrap_or("off")
                    .to_owned();
                match exposure.as_str() {
                    "exposed" => return Ok(()),
                    "failed" | "blocked" | "foreign" => return Err("mobile_failed"),
                    _ => {}
                }
                if frames.changed().await.is_err() {
                    return Err("mobile_failed");
                }
            }
        })
        .await;
        match reached {
            Ok(Ok(())) => {}
            Ok(Err(code)) => return refused(code),
            Err(_) => return refused("mobile_unreachable"),
        }
        if let Err(message) = self.handle_event(0, &json!({"kind": "mobile_show_code"})) {
            hide_core::diagnostic!(json!({
                "component": "mobile_pairing", "kind": "cli.code_refused", "message": message,
            }));
            return refused("mobile_unreachable");
        }
        let frame = self.subscribe_frame().borrow().clone();
        let shown = self.lock().phones.code().map(|(code, _)| code_id(code));
        match (frame["qr"].as_str(), shown) {
            (Some(qr), Some(code_id)) => json!({
                "ok": true,
                "qr": qr,
                "code_id": code_id,
                "name": self.mac_name(),
            }),
            // The relay dropped between the wait and the code, which takes
            // the code with it.
            _ => refused("mobile_unreachable"),
        }
    }

    /// `paired` with the phone's name once a phone paired with the code
    /// `code_id` names, `waiting` while that code is live, and `ended` once
    /// it expired or a newer code, Mobile switching off or the last Settings >
    /// Mobile closing voided it.
    fn cli_pair_status(&self, wanted: &str) -> Value {
        let inner = self.lock();
        if let Some((spent, name)) = &inner.paired_code
            && spent == wanted
        {
            return json!({"ok": true, "state": "paired", "name": name});
        }
        let live = inner
            .phones
            .code()
            .is_some_and(|(code, expiry)| code_id(code) == wanted && super::now_ms() < expiry);
        json!({"ok": true, "state": if live { "waiting" } else { "ended" }})
    }
}

/// What names a pairing code to the command that shows it, without the code:
/// the start of its SHA-256. Two codes never share it, though two issued in
/// the same millisecond share an expiry.
pub fn code_id(code: &str) -> String {
    phones::credential_hash(code)[..16].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mobile(directory: &std::path::Path) -> Arc<Mobile> {
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        super::super::test_mobile(directory, &gate)
    }

    /// Exposes Mobile on the relay and issues a code, as `pair` does once the
    /// relay answers, without a relay to answer: the code and its id.
    fn exposed_code(mobile: &Mobile) -> (String, String) {
        let mut inner = mobile.lock();
        inner.settings.enabled = true;
        inner.exposure = super::super::Exposure::Relay;
        let (code, _) = inner.phones.new_code(super::super::now_ms());
        let id = code_id(&code);
        (code, id)
    }

    #[tokio::test]
    async fn pair_status_follows_the_code_it_was_given() {
        let directory = tempfile::tempdir().unwrap();
        let mobile = mobile(directory.path());
        let (code, id) = exposed_code(&mobile);
        let status = |mobile: &Arc<Mobile>, id: &str| mobile.cli_pair_status(id)["state"].clone();
        assert_eq!(status(&mobile, &id), "waiting");
        assert_eq!(status(&mobile, "0123456789abcdef"), "ended");
        let (phone, _) = mobile.pair(&code, "Pixel").unwrap();
        let paired = mobile.cli_pair_status(&id);
        assert_eq!(paired["state"], "paired");
        assert_eq!(paired["name"], phone.name);

        // A newer code voids the one the command showed, even one issued
        // in the same millisecond.
        let (_, first) = exposed_code(&mobile);
        let (_, second) = exposed_code(&mobile);
        assert_eq!(status(&mobile, &first), "ended");
        assert_eq!(status(&mobile, &second), "waiting");
        // Ending the code the command did not show ends that code alone.
        let _ = mobile
            .answer_cli(Ask::PairEnd { code_id: first }, Caller::Operator)
            .await;
        assert_eq!(status(&mobile, &second), "waiting");
        let _ = mobile
            .answer_cli(
                Ask::PairEnd {
                    code_id: second.clone(),
                },
                Caller::Operator,
            )
            .await;
        assert_eq!(status(&mobile, &second), "ended");
    }

    #[tokio::test]
    async fn remove_takes_an_id_or_a_name_that_only_one_phone_has() {
        let directory = tempfile::tempdir().unwrap();
        let mobile = mobile(directory.path());
        let mut paired = Vec::new();
        for _ in 0..3 {
            let (code, _) = exposed_code(&mobile);
            paired.push(mobile.pair(&code, "iPhone").unwrap().0);
        }
        // Each pairing of the same kind of phone is named apart.
        assert_eq!(
            paired
                .iter()
                .map(|phone| phone.name.as_str())
                .collect::<Vec<_>>(),
            ["iPhone", "iPhone 2", "iPhone 3"]
        );
        assert_eq!(mobile.cli_remove("Galaxy")["reason"], "phone_unknown");
        let removed = mobile.cli_remove("iPhone 2");
        assert_eq!(removed["phone_id"], paired[1].id.as_str());
        let removed = mobile.cli_remove(&paired[0].id);
        assert_eq!(removed["name"], "iPhone");
        let listed = mobile.cli_list();
        let names: Vec<&str> = listed["phones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|phone| phone["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["iPhone 3"]);
    }

    #[tokio::test]
    async fn only_a_terminal_outside_the_panes_pairs() {
        let directory = tempfile::tempdir().unwrap();
        let mobile = mobile(directory.path());
        let answer = mobile.answer_cli(Ask::Pair, Caller::Pane).await;
        assert_eq!(answer["reason"], "phone_pair_from_pane");
        // The language its refusal is worded in is anyone's to read: none chosen yet.
        let language = mobile.answer_cli(Ask::Language, Caller::Pane).await;
        assert_eq!(language, json!({"ok": true, "interface_language": null}));
        // Nothing was turned on for it.
        assert!(!mobile.lock().settings.enabled);
    }

    /// mission-87: a bot reads the phones in the whole Hide's mode, and
    /// pairs or removes none in either mode, with the bot's own refusal.
    #[tokio::test]
    async fn a_bot_lists_phones_and_pairs_or_removes_none() {
        use hide_core::access::{Mode, NOT_OWN_WORK, OPERATOR_ONLY};
        let directory = tempfile::tempdir().unwrap();
        let mobile = mobile(directory.path());
        let (code, _) = exposed_code(&mobile);
        mobile.pair(&code, "iPhone").unwrap();
        let (_, id) = exposed_code(&mobile);

        let listed = mobile.answer_cli(Ask::List, Caller::Bot(Mode::All)).await;
        assert_eq!(listed["ok"], true);
        assert_eq!(listed["phones"][0]["name"], "iPhone");
        let own = mobile.answer_cli(Ask::List, Caller::Bot(Mode::Own)).await;
        assert_eq!(own["reason"], NOT_OWN_WORK);
        assert_eq!(
            own["next_action"],
            hide_core::access::NOT_OWN_WORK_NEXT_ACTION
        );
        for mode in [Mode::All, Mode::Own] {
            for ask in [
                Ask::Pair,
                Ask::Remove {
                    phone: "iPhone".to_owned(),
                },
                Ask::PairStatus {
                    code_id: id.clone(),
                },
                Ask::PairEnd {
                    code_id: id.clone(),
                },
            ] {
                let answer = mobile.answer_cli(ask, Caller::Bot(mode)).await;
                assert_eq!(answer["reason"], OPERATOR_ONLY, "{mode:?}");
            }
        }
        // The phone is still paired and the shown code still stands.
        assert_eq!(mobile.cli_list()["phones"].as_array().unwrap().len(), 1);
        assert_eq!(mobile.cli_pair_status(&id)["state"], "waiting");
    }

    /// The new-phone notices owed to the phones already paired stay as many
    /// as a computer keeps phones: past that the oldest owed one is dropped
    /// (PRD device-connect-iroh D-70).
    #[tokio::test]
    async fn owed_new_phone_notices_stay_within_the_phones_a_computer_keeps() {
        let directory = tempfile::tempdir().unwrap();
        let mobile = mobile(directory.path());
        let mut paired = Vec::new();
        for index in 0..phones::MAX_PHONES {
            let (code, _) = exposed_code(&mobile);
            paired.push(mobile.pair(&code, &format!("Phone {index}")).unwrap().0);
        }
        for phone in &paired {
            mobile.owe_paired_notice(phone.clone());
        }
        let mut one_more = paired[0].clone();
        one_more.id = "one-more".to_owned();
        mobile.owe_paired_notice(one_more);
        let owed: Vec<String> = mobile
            .lock()
            .paired_notices
            .iter()
            .map(|phone| phone.id.clone())
            .collect();
        assert_eq!(owed.len(), phones::MAX_PHONES);
        assert_eq!(owed[0], paired[1].id);
        assert_eq!(owed.last().unwrap(), "one-more");
    }

    /// D-70 (security pre-review O2): a new phone is told once to each
    /// connected computer that may open this one, whether or not anyone is
    /// at it, as the phone's name and nothing of the code or its keys, with
    /// no window here and no phone paired before it to hear it.
    #[tokio::test]
    async fn a_new_phone_is_told_to_each_computer_that_may_open_this_one() {
        use crate::desktop_notices::{Computers, DesktopNotices};
        use hide_peer::protocol::core::CoreMessage;
        #[derive(Default)]
        struct Recording(std::sync::Mutex<Vec<(String, CoreMessage)>>);
        impl Computers for Recording {
            fn tell(&self, key: &str, message: CoreMessage) -> bool {
                self.0.lock().unwrap().push((key.to_owned(), message));
                true
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let desktop = Arc::new(DesktopNotices::new(true));
        let computers = Arc::new(Recording::default());
        desktop.set_computers(Arc::clone(&computers) as Arc<dyn Computers>);
        // Two computers that may open this one told their seat; no one is at either.
        for key in ["macbook", "studio"] {
            desktop.peer_seat(
                key,
                hide_core::notice_place::Seat {
                    input: None,
                    shows: false,
                    phone: None,
                },
            );
        }
        let mobile = super::super::test_mobile_with(directory.path(), &gate, desktop);
        let (code, _) = exposed_code(&mobile);
        let (phone, _) = mobile.pair(&code, "Pixel").unwrap();
        mobile.tell_paired(std::slice::from_ref(&phone));

        let mut told = std::mem::take(&mut *computers.0.lock().unwrap());
        told.sort_by(|a, b| a.0.cmp(&b.0));
        let paired = CoreMessage::PhonePaired {
            phone: phone.name.clone(),
        };
        assert_eq!(
            told,
            [
                ("macbook".to_owned(), paired.clone()),
                ("studio".to_owned(), paired)
            ]
        );
        // What crosses to the other computer is the name alone.
        let wire = serde_json::to_value(&told[0].1).unwrap();
        assert_eq!(wire, json!({"t": "phone_paired", "phone": phone.name}));
        assert!(!wire.to_string().contains(&code));
    }

    /// The loop that sends owed new-phone notices is owned to the end: with
    /// the batch it took still waiting for a blocking thread, `shutdown` does
    /// not return, and it returns once that batch and the loop have finished
    /// (PRD device-connect-iroh D-70).
    #[test]
    fn shutdown_waits_for_the_batch_of_new_phone_notices_in_flight() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        runtime.block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
            let mobile = super::super::test_mobile_on_network(directory.path(), &gate);
            let (code, _) = exposed_code(&mobile);
            let (phone, _) = mobile.pair(&code, "Pixel").unwrap();
            mobile.owe_paired_notice(phone);
            let observing = mobile
                .resident_task
                .lock()
                .unwrap()
                .as_ref()
                .expect("Mobile runs its resident loops")
                .abort_handle();
            // The one blocking thread is held, so the batch the loop takes waits behind it.
            let (release, held) = std::sync::mpsc::channel::<()>();
            let (started, holding) = tokio::sync::oneshot::channel();
            let holder = tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                let _ = held.recv_timeout(Duration::from_secs(30));
            });
            holding.await.unwrap();
            // The loop may start its sleep after the clock moved, so the
            // clock moves one interval at a time until it takes the notice.
            for _ in 0..10 {
                tokio::time::advance(super::super::OBSERVE_INTERVAL).await;
                for _ in 0..100 {
                    if mobile.lock().paired_notices.is_empty() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                if mobile.lock().paired_notices.is_empty() {
                    break;
                }
            }
            assert!(
                mobile.lock().paired_notices.is_empty(),
                "the loop took the owed notice"
            );
            let stopping = tokio::spawn({
                let mobile = Arc::clone(&mobile);
                async move { mobile.shutdown().await }
            });
            for _ in 0..100 {
                tokio::task::yield_now().await;
            }
            assert!(
                !stopping.is_finished(),
                "shutdown waits for the batch in flight"
            );
            release.send(()).unwrap();
            holder.await.unwrap();
            stopping.await.unwrap();
            assert!(observing.is_finished());
        });
    }

    #[tokio::test]
    async fn a_full_computer_refuses_before_it_turns_anything_on() {
        let directory = tempfile::tempdir().unwrap();
        let mobile = mobile(directory.path());
        for _ in 0..phones::MAX_PHONES {
            let (code, _) = exposed_code(&mobile);
            mobile.pair(&code, "iPhone").unwrap();
        }
        mobile.lock().phones.clear_code();
        let answer = mobile.answer_cli(Ask::Pair, Caller::Operator).await;
        assert_eq!(answer["reason"], "phone_limit");
        assert!(mobile.lock().phones.code().is_none());
    }

    #[test]
    fn asks_read_as_the_command_sends_them() {
        let ask = |value: Value| serde_json::from_value::<Ask>(value).ok();
        assert_eq!(ask(json!({"verb": "list"})), Some(Ask::List));
        assert_eq!(ask(json!({"verb": "language"})), Some(Ask::Language));
        assert_eq!(
            ask(json!({"verb": "remove", "phone": "iPhone"})),
            Some(Ask::Remove {
                phone: "iPhone".into()
            })
        );
        assert_eq!(
            ask(json!({"verb": "pair_status", "code_id": "ab"})),
            Some(Ask::PairStatus {
                code_id: "ab".into()
            })
        );
        assert_eq!(ask(json!({"verb": "show_link"})), None);
    }
}
