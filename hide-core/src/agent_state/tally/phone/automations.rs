//! What a phone sees of the automations (PRD automations B34, B35, D-20):
//! the core's `automations` section, trimmed to the list and each
//! automation's run records the phone's tab draws, and the automations the
//! core counts as the operator's turn, which the phone's 내 차례 draws: those
//! whose pause a person fixes (`agent_scope.automation_asks`, D-12) and the
//! once ones whose run waits to be opened (`agent_scope.automation_looks`,
//! mission-104 O-4). It carries no folder, no task text,
//! no agent's answer, no pane and no branch; of the agent it carries only the
//! kind a usage-limit skip names. Only the structs below reach the wire, so a
//! field the core adds to an automation stays on the Mac until it is named
//! here.
//!
//! How a schedule, a next run or a record reads is the web's own rule
//! (`web/src/automationWords.ts`, shared with the desktop), run on the phone
//! over this value.

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
    /// The listed automations paused for a reason a person fixes, the oldest
    /// pause first, as the core counts them in Needs You (D-12, B23).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub asks: Vec<String>,
    /// The listed once automations whose run ended and waits for the
    /// operator to open it, the one that ended first first, as the core
    /// counts them in Needs You (mission-104 O-4).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub looks: Vec<String>,
    /// The book before could not be read and was set aside: one line above
    /// the list. The file it was set aside under stays on the Mac.
    #[serde(skip_serializing_if = "is_false")]
    pub unreadable: bool,
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
    /// A once automation whose one time came and whose run for it ended
    /// (mission-104 O-3).
    pub done: bool,
    /// The run whose end waits in Needs You (mission-104 O-4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub look_run: Option<u64>,
    pub unseen: usize,
    /// What the runs its records keep spent together (B39).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_spend: Option<crate::automations::RecentSpend>,
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
    /// From `at_unix_ms` to `last_unix_ms`, `count` times in a row.
    Skipped {
        at_unix_ms: u64,
        last_unix_ms: u64,
        count: u32,
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
    /// What its conversation spent (B39); absent until the ledger read it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spend: Option<crate::automations::Spend>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

impl PhoneAutomations {
    /// The phone's automations from the core's section, with `asks` and
    /// `looks` the core's `automation_asks` and `automation_looks` in their
    /// order; none before the core restored its book. Each names only an
    /// automation the frame lists.
    pub fn project(
        section: Option<&AutomationsSnapshot>,
        asks: &[String],
        looks: &[String],
    ) -> Self {
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
        let listed = |ids: &[String]| -> Vec<String> {
            ids.iter()
                .filter(|id| items.iter().any(|item| &item.id == *id))
                .cloned()
                .collect()
        };
        let (asks, looks) = (listed(asks), listed(looks));
        Self {
            items,
            omitted,
            zone: section.zone.clone(),
            asks,
            looks,
            unreadable: section.unreadable.is_some(),
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
        done: automation.done,
        look_run: automation.look_run,
        unseen: automation.unseen,
        recent_spend: automation.recent_spend,
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
        RecordSnapshot::Skipped {
            at_unix_ms,
            last_unix_ms,
            count,
            reason,
        } => PhoneRecord::Skipped {
            at_unix_ms: *at_unix_ms,
            last_unix_ms: *last_unix_ms,
            count: *count,
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
        spend: run.spend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automations::{AgentChoice, Folder, PauseReason, Unreadable};
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
            spend: None,
            answer: None,
            results: Vec::new(),
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
                path: "/srv/secret/hide/automations/a".into(),
                default: true,
                worktree: false,
            },
            paused: None,
            running: false,
            done: false,
            look_run: None,
            unseen: 1,
            recent_spend: None,
            records: (0..records as u64).rev().map(run).collect(),
        }
    }

    fn section(items: Vec<AutomationSnapshot>) -> AutomationsSnapshot {
        AutomationsSnapshot {
            items,
            unseen: 0,
            last_choice: None,
            zone: Some("Asia/Seoul".into()),
            own_folder_root: Some("/srv/secret/hide/automations".into()),
            unreadable: None,
        }
    }

    /// The phone gets the list, the records and the asks, read as the core
    /// decided, and none of the folder, the task, a pane or a branch; of the
    /// agent only the kind a usage-limit skip names.
    #[test]
    fn the_phone_gets_the_list_and_records_and_nothing_private() {
        let mut paused = automation("b", 1);
        paused.records.insert(
            0,
            RecordSnapshot::Skipped {
                at_unix_ms: 9,
                last_unix_ms: 9,
                count: 1,
                reason: SkipReason::UsageLimit {
                    agent: "claude".into(),
                },
            },
        );
        paused.paused = Some(Pause {
            reason: PauseReason::LoginExpired,
            at_unix_ms: 7,
        });
        paused.next_run_at_unix_ms = None;
        let mut book = section(vec![automation("a", 2), paused]);
        book.unreadable = Some(Unreadable {
            file: "secret-book.json".into(),
            at_unix_ms: 3,
        });
        let projected =
            PhoneAutomations::project(Some(&book), &["b".into(), "unlisted".into()], &[]);
        assert!(
            projected.unreadable,
            "the line above the list, without its file"
        );
        assert_eq!(projected.omitted, 0);
        assert_eq!(projected.items.len(), 2);
        let first = &projected.items[0];
        assert_eq!(first.schedule, schedule(), "the schedule as it reads");
        assert_eq!(first.next_run_at_unix_ms, Some(42));
        assert_eq!(first.records.len(), 2);
        assert!(matches!(&first.records[0], PhoneRecord::Run(run) if run.number == 1));
        assert_eq!(
            projected
                .find("b")
                .unwrap()
                .paused
                .map(|pause| pause.reason),
            Some(PauseReason::LoginExpired)
        );
        assert_eq!(projected.zone.as_deref(), Some("Asia/Seoul"));
        assert_eq!(
            projected.asks,
            ["b"],
            "an ask names a listed automation only"
        );
        assert!(matches!(
            &projected.find("b").unwrap().records[0],
            PhoneRecord::Skipped { reason: SkipReason::UsageLimit { agent }, .. } if agent == "claude"
        ));
        let sent = serde_json::to_string(&projected).unwrap();
        for private in ["secret", "folder", "task", "pane", "branch"] {
            assert!(
                !sent.contains(private),
                "{private} reached the phone: {sent}"
            );
        }
    }

    #[test]
    fn records_past_the_phone_cap_are_counted_and_the_newest_kept() {
        let projected =
            PhoneAutomations::project(Some(&section(vec![automation("a", 50)])), &[], &[]);
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
        let projected = PhoneAutomations::project(Some(&section(many)), &[], &[]);
        assert_eq!(projected.items.len(), MAX_AUTOMATIONS);
        assert_eq!(projected.omitted, 3);
        let mut items = vec![phone_automation(&automation("a", 20)); 4];
        let one = wire_len(&items[0]) + 1;
        assert_eq!(fit(&mut items, one * 2 + 1), 2);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn no_section_is_an_empty_list() {
        assert_eq!(
            PhoneAutomations::project(None, &["a".into()], &[]),
            PhoneAutomations::default()
        );
    }

    /// B39: the phone's run rows and automation carry what the runs spent,
    /// as the core read it.
    #[test]
    fn the_phone_gets_what_each_run_spent() {
        let spend = crate::automations::Spend {
            tokens: 12_300,
            micro_usd: 420_000,
            unpriced: false,
        };
        let mut item = automation("a", 2);
        if let RecordSnapshot::Run(run) = &mut item.records[0] {
            run.spend = Some(spend);
        }
        item.recent_spend = Some(crate::automations::RecentSpend { spend, runs: 1 });
        let phone = serde_json::to_value(PhoneAutomations::project(
            Some(&section(vec![item])),
            &[],
            &[],
        ))
        .unwrap();
        let item = &phone["items"][0];
        assert_eq!(
            item["records"][0]["spend"],
            json!({"tokens": 12_300, "micro_usd": 420_000, "unpriced": false})
        );
        assert_eq!(item["records"][1].get("spend"), None);
        assert_eq!(item["recent_spend"]["runs"], 1);
    }
}
