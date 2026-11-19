//! What a phone sees of the automations (PRD automations B34, B35, D-20):
//! the core's `automations` section, trimmed to the list and each
//! automation's run records the phone's tab draws. It carries no folder, no
//! task text, no agent's answer, no pane and no branch: only the structs
//! below reach the wire, so a field the core adds to an automation stays on
//! the Mac until it is named here.
//!
//! How a schedule, a next run or a record reads is the web's own rule
//! (`web/src/mobile/automationModel.ts`), run on the phone over this value.

use serde::Serialize;

use super::{LIST_BUDGET, wire_len};
use crate::automations::{
    AutomationSnapshot, AutomationsSnapshot, MissedReason, Pause, RecordSnapshot, RunSnapshot,
    RunState, Schedule, SkipReason, Trigger,
};

/// How many automations one frame carries, in the core's order; the rest are
/// counted in `omitted` (engineering #15).
pub const MAX_AUTOMATIONS: usize = 64;
/// How many records of one automation the phone lists, the newest first; the
/// rest are counted in its `records_omitted` and stay on the desktop.
pub const MAX_RECORDS: usize = 20;
/// The longest name kept, in characters.
const NAME_CHARS: usize = 200;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PhoneAutomations {
    pub items: Vec<PhoneAutomation>,
    /// Automations past the caps, which no frame carries.
    #[serde(skip_serializing_if = "is_zero")]
    pub omitted: usize,
    /// The zone the schedules run in, by its IANA name: the phone words a
    /// time in it, as the schedule reads (B6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhoneAutomation {
    pub id: String,
    pub name: String,
    /// The choice its schedule reads as (B6), which the phone words.
    pub schedule: Schedule,
    /// None while it is paused.
    pub next_run_at_unix_ms: Option<u64>,
    /// Why it does not run on schedule, and since when; none while it is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<Pause>,
    /// A run goes now: [지금 실행] is off (B19).
    pub running: bool,
    pub unseen: usize,
    /// Newest first.
    pub records: Vec<PhoneRecord>,
    /// Older records the phone does not list.
    #[serde(skip_serializing_if = "is_zero")]
    pub records_omitted: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PhoneRecord {
    Run(PhoneRun),
    Skipped {
        at_unix_ms: u64,
        reason: SkipReason,
    },
    Missed {
        first_unix_ms: u64,
        last_unix_ms: u64,
        count: u32,
        reason: MissedReason,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhoneRun {
    pub number: u64,
    pub trigger: Trigger,
    pub started_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at_unix_ms: Option<u64>,
    pub state: RunState,
    pub attempt: u32,
    /// Why the last attempt failed or the run ended, as the core's code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub seen: bool,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

impl PhoneAutomations {
    /// The phone's automations from the core's section; none before the core
    /// restored its book.
    pub fn project(section: Option<&AutomationsSnapshot>) -> Self {
        let Some(section) = section else {
            return Self::default();
        };
        let mut omitted = section.items.len().saturating_sub(MAX_AUTOMATIONS);
        let mut items: Vec<PhoneAutomation> = section
            .items
            .iter()
            .take(MAX_AUTOMATIONS)
            .map(phone_automation)
            .collect();
        omitted += fit(&mut items, LIST_BUDGET);
        Self {
            items,
            omitted,
            zone: section.zone.clone(),
        }
    }

    pub fn find(&self, id: &str) -> Option<&PhoneAutomation> {
        self.items.iter().find(|automation| automation.id == id)
    }
}

/// Keeps the items within `budget` bytes of JSON, dropping the last ones
/// first, and answers how many it dropped.
fn fit(items: &mut Vec<PhoneAutomation>, budget: usize) -> usize {
    let mut total = 0;
    let kept = items
        .iter()
        .take_while(|item| {
            total += wire_len(item) + 1;
            total <= budget
        })
        .count();
    let dropped = items.len() - kept;
    items.truncate(kept);
    dropped
}

fn phone_automation(automation: &AutomationSnapshot) -> PhoneAutomation {
    PhoneAutomation {
        id: automation.id.clone(),
        name: automation.name.chars().take(NAME_CHARS).collect(),
        schedule: automation.reads_as.clone(),
        next_run_at_unix_ms: automation.next_run_at_unix_ms,
        paused: automation.paused,
        running: automation.running,
        unseen: automation.unseen,
        records: automation
            .records
            .iter()
            .take(MAX_RECORDS)
            .map(phone_record)
            .collect(),
        records_omitted: automation.records.len().saturating_sub(MAX_RECORDS),
    }
}

fn phone_record(record: &RecordSnapshot) -> PhoneRecord {
    match record {
        RecordSnapshot::Run(run) => PhoneRecord::Run(phone_run(run)),
        RecordSnapshot::Skipped { at_unix_ms, reason } => PhoneRecord::Skipped {
            at_unix_ms: *at_unix_ms,
            reason: reason.clone(),
        },
        RecordSnapshot::Missed {
            first_unix_ms,
            last_unix_ms,
            count,
            reason,
        } => PhoneRecord::Missed {
            first_unix_ms: *first_unix_ms,
            last_unix_ms: *last_unix_ms,
            count: *count,
            reason: *reason,
        },
    }
}

fn phone_run(run: &RunSnapshot) -> PhoneRun {
    PhoneRun {
        number: run.number,
        trigger: run.trigger,
        started_at_unix_ms: run.started_at_unix_ms,
        ended_at_unix_ms: run.ended_at_unix_ms,
        state: run.state,
        attempt: run.attempt,
        reason: run.reason.clone(),
        seen: run.seen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automations::{AgentChoice, Folder, PauseReason};
    use serde_json::json;

    fn schedule() -> Schedule {
        serde_json::from_value(json!({"kind": "weekdays", "time": "09:00"})).unwrap()
    }

    fn run(number: u64) -> RecordSnapshot {
        RecordSnapshot::Run(RunSnapshot {
            number,
            trigger: Trigger::Manual,
            due_unix_ms: None,
            started_at_unix_ms: number * 1000,
            ended_at_unix_ms: Some(number * 1000 + 500),
            state: RunState::Succeeded,
            attempt: 1,
            reason: None,
            pane_id: Some("secret-pane".into()),
            opened: false,
            resumable: true,
            conversation_missing: false,
            branch: Some("secret-branch".into()),
            seen: false,
        })
    }

    fn automation(id: &str, records: usize) -> AutomationSnapshot {
        AutomationSnapshot {
            id: id.into(),
            name: format!("automation {id}"),
            task: "secret task words".into(),
            schedule: serde_json::from_value(json!({"kind": "cron", "expr": "0 9 * * 1-5"}))
                .unwrap(),
            reads_as: schedule(),
            next_run_at_unix_ms: Some(42),
            agent: AgentChoice {
                kind: "claude".into(),
                profile: None,
                model: None,
                effort: None,
            },
            folder: Folder {
                path: "/Users/secret/hide/automations/a".into(),
                default: true,
                worktree: false,
            },
            paused: None,
            running: false,
            unseen: 1,
            records: (0..records as u64).rev().map(run).collect(),
        }
    }

    fn section(items: Vec<AutomationSnapshot>) -> AutomationsSnapshot {
        AutomationsSnapshot {
            items,
            unseen: 0,
            last_choice: None,
            zone: Some("Asia/Seoul".into()),
            own_folder_root: Some("/Users/secret/hide/automations".into()),
        }
    }

    /// The phone gets the list and the records, read as the core decided, and
    /// none of the folder, the task, a pane or a branch.
    #[test]
    fn the_phone_gets_the_list_and_records_and_nothing_private() {
        let mut paused = automation("b", 1);
        paused.paused = Some(Pause {
            reason: PauseReason::LoginExpired,
            at_unix_ms: 7,
        });
        paused.next_run_at_unix_ms = None;
        let projected = PhoneAutomations::project(Some(&section(vec![automation("a", 2), paused])));
        assert_eq!(projected.omitted, 0);
        assert_eq!(projected.items.len(), 2);
        let first = &projected.items[0];
        assert_eq!(first.schedule, schedule(), "the schedule as it reads");
        assert_eq!(first.next_run_at_unix_ms, Some(42));
        assert_eq!(first.records.len(), 2);
        assert!(matches!(&first.records[0], PhoneRecord::Run(run) if run.number == 1));
        assert_eq!(
            projected.find("b").unwrap().paused.map(|pause| pause.reason),
            Some(PauseReason::LoginExpired)
        );
        assert_eq!(projected.zone.as_deref(), Some("Asia/Seoul"));
        let sent = serde_json::to_string(&projected).unwrap();
        for private in ["secret", "folder", "task", "pane", "branch"] {
            assert!(!sent.contains(private), "{private} reached the phone: {sent}");
        }
    }

    #[test]
    fn records_past_the_phone_cap_are_counted_and_the_newest_kept() {
        let projected = PhoneAutomations::project(Some(&section(vec![automation("a", 50)])));
        let item = &projected.items[0];
        assert_eq!(item.records.len(), MAX_RECORDS);
        assert_eq!(item.records_omitted, 50 - MAX_RECORDS);
        assert!(matches!(&item.records[0], PhoneRecord::Run(run) if run.number == 49));
    }

    #[test]
    fn automations_past_the_count_or_the_bytes_are_counted() {
        let many = (0..MAX_AUTOMATIONS + 3)
            .map(|index| automation(&index.to_string(), 0))
            .collect();
        let projected = PhoneAutomations::project(Some(&section(many)));
        assert_eq!(projected.items.len(), MAX_AUTOMATIONS);
        assert_eq!(projected.omitted, 3);
        let mut items = vec![phone_automation(&automation("a", 20)); 4];
        let one = wire_len(&items[0]) + 1;
        assert_eq!(fit(&mut items, one * 2 + 1), 2);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn no_section_is_an_empty_list() {
        assert_eq!(PhoneAutomations::project(None), PhoneAutomations::default());
    }
}
