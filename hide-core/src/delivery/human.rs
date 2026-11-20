//! The approved human cause, an inactivity warning its observer did not
//! confirm, takes the durable store reservation and the existing external
//! channels. No retry can follow a consumed reservation. A letter that ends
//! with no recipient tells only its sender (PRD letters-missions-simplify
//! B7), never the operator.
use super::ledger::Ledger;
use super::watch::Watch;
use super::worker::{HumanNotice, HumanNoticeKind};

const NOTICE_LIMIT: usize = 8;

pub(super) fn claim(ledger: &mut Ledger, now: u64) -> Vec<HumanNotice> {
    let mut notices = Vec::new();
    // A pane observer that has not looked at its first warning of a quiet
    // episode an hour on: the operator is told once per episode. One the
    // observer saw is its own call, and code-owned observers handle their
    // own escalation.
    for index in 0..ledger.watches.len() {
        if notices.len() == NOTICE_LIMIT {
            break;
        }
        let watch = &ledger.watches[index];
        let Some(first) = watch.first_warning_at_unix_ms else {
            continue;
        };
        if watch.parent.code_owned()
            || watch.operator_notified
            || now.saturating_sub(first) < super::SECOND_WARNING_MS
            || observer_saw(ledger, watch, first)
            // Another observer of the same target and episode told it.
            || ledger.watches.iter().any(|other| {
                other.operator_notified
                    && other.target.same_identity(&watch.target)
                    && other.last_activity_at_unix_ms == watch.last_activity_at_unix_ms
            })
        {
            continue;
        }
        notices.push(HumanNotice {
            id: watch.id.clone(),
            actor: watch.target.clone(),
            kind: HumanNoticeKind::ObserverUnconfirmed,
            recipient: watch.parent.name.clone(),
            about: watch.target.name.clone(),
        });
        ledger.watches[index].operator_notified = true;
    }
    notices
}

/// Whether `watch`'s observer looked at its status cell about the target
/// since the first warning made at `first` (`runtime::status_cells`).
pub(crate) fn observer_saw(ledger: &Ledger, watch: &Watch, first: u64) -> bool {
    let observer = crate::runtime::status_recipient(Some(ledger), &watch.parent);
    ledger
        .status_seen
        .get(&observer)
        .and_then(|seen| seen.get(&crate::runtime::status_subject_child(&watch.target.pane_id)))
        .is_some_and(|revision| *revision >= first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{Actor, INACTIVITY_MS, SECOND_WARNING_MS, watch};

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
            engine: None,
            status_changed_at_unix_ms: activity,
            session_modified_at_unix_ms: None,
            failure: None,
            gone: false,
            inactivity_ms: None,
            asking: false,
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
        assert!(ledger.letters.is_empty(), "a warning is no letter");
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        assert!(claim(&mut restored, due + SECOND_WARNING_MS).is_empty());
    }

    /// An observer that looked at its warning cell decides for itself; the
    /// operator is told only of one that did not look within the hour.
    #[test]
    fn a_warning_its_observer_saw_notifies_no_one() {
        let mut ledger = Ledger::default();
        let parent = actor("parent");
        warn(&mut ledger, &parent, &actor("target"), 10);
        let first = ledger.watches[0].first_warning_at_unix_ms.unwrap();
        let observer = crate::runtime::status_recipient(Some(&ledger), &parent);
        ledger.status_seen.insert(
            observer,
            [(crate::runtime::status_subject_child("target"), first)].into(),
        );
        assert!(claim(&mut ledger, 10 + INACTIVITY_MS + SECOND_WARNING_MS).is_empty());
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
            engine: None,
            status_changed_at_unix_ms: activity,
            session_modified_at_unix_ms: None,
            failure: None,
            gone: false,
            inactivity_ms: None,
            asking: false,
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
