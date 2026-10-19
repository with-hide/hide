//! The two approved human causes share the durable store reservation and
//! existing external channels. No retry can follow a consumed reservation.
use super::ledger::{Ledger, State};
use super::worker::{HumanNotice, HumanNoticeKind};

const NOTICE_LIMIT: usize = 8;

pub(super) fn claim(ledger: &mut Ledger, now: u64) -> Vec<HumanNotice> {
    let mut notices = Vec::new();
    for index in 0..ledger.letters.len() {
        if notices.len() == NOTICE_LIMIT {
            break;
        }
        let letter = &ledger.letters[index];
        let Some(warning) = &letter.watch_warning else {
            continue;
        };
        // Code-owned observers handle their own escalation.
        if letter.recipient.code_owned()
            || warning.ordinal != 1
            || now.saturating_sub(letter.created_at_unix_ms) < super::SECOND_WARNING_MS
            || matches!(letter.state, State::Acknowledged | State::Cancelled)
            || ledger.letters.iter().any(|answer| {
                answer.reply_to.as_deref() == Some(&letter.id)
                    && answer.sender.same_identity(&letter.recipient)
            })
            || !ledger.watches.iter().any(|watch| {
                watch.target.same_identity(&warning.target)
                    && watch.parent.same_identity(&letter.recipient)
                    && watch.last_activity_at_unix_ms == warning.activity_at_unix_ms
            })
            || ledger
                .letters
                .iter()
                .filter_map(|letter| letter.watch_warning.as_ref())
                .any(|receipt| {
                    receipt.parent_notified
                        && receipt.target.same_identity(&warning.target)
                        && receipt.activity_at_unix_ms == warning.activity_at_unix_ms
                })
        {
            continue;
        }
        let target = warning.target.clone();
        let activity = warning.activity_at_unix_ms;
        notices.push(HumanNotice {
            id: letter.id.clone(),
            actor: target.clone(),
            kind: HumanNoticeKind::ObserverUnconfirmed,
            recipient: letter.recipient.name.clone(),
            about: target.name.clone(),
        });
        for receipt in ledger
            .letters
            .iter_mut()
            .filter_map(|letter| letter.watch_warning.as_mut())
        {
            if receipt.target.same_identity(&target) && receipt.activity_at_unix_ms == activity {
                receipt.parent_notified = true;
            }
        }
    }
    for letter in &mut ledger.letters {
        if notices.len() == NOTICE_LIMIT {
            break;
        }
        if letter.state == State::Undelivered && !letter.human_notified {
            letter.human_notified = true;
            notices.push(HumanNotice {
                id: letter.id.clone(),
                actor: letter.sender.clone(),
                kind: HumanNoticeKind::LetterUndelivered,
                recipient: letter.recipient.name.clone(),
                about: String::new(),
            });
        }
    }
    notices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{Actor, Command, INACTIVITY_MS, SECOND_WARNING_MS, mailbox, watch};

    fn actor(name: &str) -> Actor {
        Actor {
            pane_id: name.into(),
            name: name.into(),
            kind: "codex".into(),
            device_id: "local".into(),
            session: Some(format!("{name}-session")),
        }
    }

    fn warn(ledger: &mut Ledger, parent: &Actor, target: &Actor, activity: u64) -> watch::Watch {
        let started = watch::start(ledger, parent, target, activity).unwrap();
        let reading = watch::Reading {
            id: started.id.clone(),
            status: "idle".into(),
            status_available: true,
            state_change_seq: Some(activity),
            status_changed_at_unix_ms: activity,
            session_modified_at_unix_ms: None,
            failure: None,
            gone: false,
            inactivity_ms: None,
            asking: false,
            blocked: false,
        };
        watch::tick(ledger, &[reading], activity + INACTIVITY_MS).unwrap();
        started
    }

    #[test]
    fn first_warning_waits_sixty_minutes_and_shared_target_claim_survives_restart() {
        let mut ledger = Ledger::default();
        let target = actor("target");
        warn(&mut ledger, &actor("parent"), &target, 10);
        warn(&mut ledger, &actor("other-parent"), &target, 10);
        let due = 10 + INACTIVITY_MS + SECOND_WARNING_MS;
        assert!(claim(&mut ledger, due - 1).is_empty());
        let notices = claim(&mut ledger, due);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].actor.pane_id, "target");
        assert_eq!(notices[0].kind, HumanNoticeKind::ObserverUnconfirmed);
        assert_eq!(
            (notices[0].recipient.as_str(), notices[0].about.as_str()),
            ("parent", "target")
        );
        assert!(
            notices[0]
                .english_body()
                .ends_with(&format!("hide request show {}.", notices[0].id)),
            "the Herdr notification names the command an agent runs"
        );
        assert_eq!(ledger.letters.len(), 2);
        assert!(
            ledger.letters.iter().all(|letter| letter
                .watch_warning
                .as_ref()
                .unwrap()
                .parent_notified)
        );
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        assert!(claim(&mut restored, due + SECOND_WARNING_MS).is_empty());
        restored.watches.clear();
        warn(&mut restored, &actor("parent"), &target, 10);
        assert!(claim(&mut restored, due + SECOND_WARNING_MS).is_empty());
    }

    #[test]
    fn acknowledged_cancelled_and_replied_warnings_do_not_notify_people() {
        for action in ["ack", "cancel", "reply"] {
            let mut ledger = Ledger::default();
            let parent = actor("parent");
            warn(&mut ledger, &parent, &actor("target"), 10);
            let id = ledger.letters[0].id.clone();
            let command = match action {
                "ack" => Command::Ack { id },
                "cancel" => Command::Cancel { id },
                _ => Command::Reply {
                    id,
                    intent: "checked".into(),
                    body: "I checked the target".into(),
                },
            };
            mailbox::apply(&mut ledger, &parent, None, &command, 20).unwrap();
            assert!(
                claim(&mut ledger, 10 + INACTIVITY_MS + SECOND_WARNING_MS).is_empty(),
                "{action}"
            );
        }
    }

    #[test]
    fn activity_resets_the_episode_and_second_warning_has_no_new_human_schedule() {
        let mut ledger = Ledger::default();
        let parent = actor("parent");
        let target = actor("target");
        let started = warn(&mut ledger, &parent, &target, 10);
        let due = 10 + INACTIVITY_MS + SECOND_WARNING_MS;
        assert_eq!(claim(&mut ledger, due).len(), 1);
        let reading = |activity| watch::Reading {
            id: started.id.clone(),
            status: "idle".into(),
            status_available: true,
            state_change_seq: Some(activity),
            status_changed_at_unix_ms: activity,
            session_modified_at_unix_ms: None,
            failure: None,
            gone: false,
            inactivity_ms: None,
            asking: false,
            blocked: false,
        };
        watch::tick(&mut ledger, &[reading(10)], due).unwrap();
        assert_eq!(ledger.watches[0].warning_count, 2);
        assert!(claim(&mut ledger, due + SECOND_WARNING_MS).is_empty());
        let activity = due + SECOND_WARNING_MS + 1;
        watch::tick(&mut ledger, &[reading(activity)], activity).unwrap();
        assert!(claim(&mut ledger, activity).is_empty());
        watch::tick(&mut ledger, &[reading(activity)], activity + INACTIVITY_MS).unwrap();
        assert_eq!(
            claim(&mut ledger, activity + INACTIVITY_MS + SECOND_WARNING_MS).len(),
            1
        );
    }
}
