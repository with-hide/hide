//! `hide skill` / `hide mcp` in the runtime (PRD skill-mcp-manager B31-B33,
//! D-19, D-29): a command becomes store requests of its own, and an install,
//! an update or an MCP added or changed waits for the operator's confirm,
//! which stands as the asking agent's person's turn in 내 차례
//! (`status.store_asks`, laid on its row as a `HumanKind::Install` turn)
//! until the operator answers it on the screen or the command stops waiting.
//!
//! Nothing here touches a file: each step is a store request the kit's
//! workers run (`kit_store`), and the daemon waits on the run's answer off
//! the owner thread (`store_command::Prepared::run`).

use std::collections::BTreeMap;
use std::sync::mpsc::{Sender, channel};

use hide_agent_hooks::mcp::Server;
use hide_kit::store::{
    ItemKind, ItemReport, SentKeys, SourceSpec, StoreChange, StoreItem, StoreRefusal, StoreReport,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Runtime;
use crate::agent_profiles::AnswerState;
use crate::coordination::human_turn::HumanKind;
use crate::model::{DeclaredHumanTurn, StoreAnswer};
use crate::store_command::{
    BUSY, CONFIRM_WAIT, DECLINED, INVALID, NOT_FOUND, PASS_WAIT, Prepared, REFUSED, StoreChanged,
    StoreCommand, StoreLine, StoreList, StoreShown, UNANSWERED, UNAVAILABLE, Waiting,
};

/// The most commands waiting at once; past it a command is refused, since
/// each one is a caller still waiting on its answer (engineering rule 15).
const RUN_LIMIT: usize = 16;

/// One install or update waiting on the operator's confirm, as the screen
/// draws it: the source's preview or the update's diff are in the
/// machine's store report under the ids named here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StoreAskSnapshot {
    /// The confirm's id, which `store_ask_answer` names and the asking
    /// agent's person's turn carries.
    pub id: String,
    pub device_id: String,
    /// The asking agent's pane.
    pub pane_id: String,
    pub since_unix_ms: u64,
    #[serde(flatten)]
    pub what: StoreAskWhat,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "what", rename_all = "snake_case")]
pub enum StoreAskWhat {
    /// Install `names` of the preview `preview` (B6).
    Install { preview: String, names: Vec<String> },
    /// Take the update the diff `diff` shows (B23).
    Update { item: String, diff: String },
    /// Add this MCP server; the operator types its keys (B9, B33).
    AddMcp { server: Server },
    /// Change the MCP item's server; the operator types its keys.
    SetMcp { item: String, server: Server },
}

/// The operator's answer to a confirm (`store_ask_answer`).
#[derive(Debug, Deserialize)]
pub(super) struct StoreAskAnswerPayload {
    pub(super) id: String,
    pub(super) confirm: bool,
    /// The skills the operator left ticked, for an install.
    #[serde(default)]
    pub(super) names: Option<Vec<String>>,
    /// The key values the operator typed, for an MCP.
    #[serde(default)]
    pub(super) keys: SentKeys,
}

/// What a command does once its store work answers.
#[derive(PartialEq)]
enum Plan {
    /// One request whose outcome is the answer.
    Plain {
        item: Option<String>,
    },
    /// Read the source, then ask for `names` (every new skill when empty).
    Install {
        source: SourceSpec,
        names: Vec<String>,
    },
    /// Read the update, then ask.
    Update {
        item: String,
    },
    AddMcp {
        server: Server,
    },
    SetMcp {
        item: String,
        server: Server,
    },
}

enum Stage {
    /// The source or the update is being read; `ended` once its request
    /// answered and the report holding it is in.
    Reading {
        request: String,
        ended: bool,
    },
    Confirming(StoreAskSnapshot),
    Applying {
        request: String,
    },
}

struct Run {
    device_id: String,
    pane_id: String,
    plan: Plan,
    stage: Stage,
    reply: Sender<Result<Value, String>>,
}

/// The commands waiting, by run id.
#[derive(Default)]
pub(super) struct StoreRuns {
    runs: BTreeMap<String, Run>,
    seq: u64,
}

fn answer_value(answer: &impl Serialize) -> Value {
    serde_json::to_value(answer).expect("a store answer serializes")
}

fn refused(code: &str, detail: impl AsRef<str>) -> String {
    crate::coordination::refusal(code, detail.as_ref())
}

fn store_refused(refusal: &StoreRefusal) -> String {
    let cause = serde_json::to_value(refusal.cause)
        .ok()
        .and_then(|cause| cause.as_str().map(str::to_owned))
        .unwrap_or_default();
    refused(REFUSED, format!("{cause}: {}", refusal.reason))
}

impl Runtime {
    /// `hide skill` / `hide mcp` for a caller in `pane_id` on `device`:
    /// a read is answered at once from the machine's last store report; a
    /// change is queued and waited on off the lock.
    pub(crate) fn prepare_store_command(
        &mut self,
        device: &str,
        pane_id: &str,
        command: StoreCommand,
    ) -> Result<Prepared, String> {
        let context = self
            .worker_context
            .clone()
            .ok_or_else(|| UNAVAILABLE.to_owned())?;
        let machine = match command.machine() {
            Some(name) => self.machine_named(name)?,
            None => device.to_owned(),
        };
        let ready = |answer| Prepared {
            answer: Waiting::Ready(answer),
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
        };
        let report = self.kit_view(&machine).store;
        let Some(report) = report else {
            return Ok(ready(Err(refused(
                NOT_FOUND,
                format!("{machine} has not reported its store yet"),
            ))));
        };
        let item_of = |kind: ItemKind, name: &str| -> Result<&ItemReport, String> {
            let id = StoreItem::id_of(kind, name);
            report
                .items
                .iter()
                .find(|item| item.id == id)
                .ok_or_else(|| refused(NOT_FOUND, format!("no {} named {name}", kind.code())))
        };
        let (plan, changes) = match command {
            StoreCommand::List { kind, .. } => {
                return Ok(ready(Ok(answer_value(&StoreList {
                    columns: report.columns.get(&kind).cloned().unwrap_or_default(),
                    items: report
                        .items
                        .iter()
                        .filter(|item| item.kind == kind)
                        .map(StoreLine::from)
                        .collect(),
                    machine,
                    kind,
                }))));
            }
            StoreCommand::Show { kind, name, .. } => {
                let item = item_of(kind, &name);
                return Ok(ready(item.map(|item| {
                    answer_value(&StoreShown {
                        machine: machine.clone(),
                        item: item.clone(),
                    })
                })));
            }
            StoreCommand::Set {
                kind,
                name,
                agent,
                on,
                ..
            } => {
                let item = match item_of(kind, &name) {
                    Ok(item) => item.id.clone(),
                    Err(code) => return Ok(ready(Err(code))),
                };
                let column = match agent {
                    None => None,
                    Some(agent) => match column_of(&report, kind, &agent) {
                        Some(column) => Some(column),
                        None => {
                            return Ok(ready(Err(refused(
                                INVALID,
                                format!("no {} column for {agent}", kind.code()),
                            ))));
                        }
                    },
                };
                (
                    Plan::Plain {
                        item: Some(item.clone()),
                    },
                    vec![StoreChange::Set { item, column, on }],
                )
            }
            StoreCommand::Remove { kind, name, .. } => {
                let item = match item_of(kind, &name) {
                    Ok(item) => item.id.clone(),
                    Err(code) => return Ok(ready(Err(code))),
                };
                (
                    Plan::Plain {
                        item: Some(item.clone()),
                    },
                    vec![StoreChange::Delete { item }],
                )
            }
            StoreCommand::Add { source, names, .. } => (
                Plan::Install {
                    source: source.clone(),
                    names,
                },
                vec![StoreChange::Inspect { source }],
            ),
            StoreCommand::Update { name, .. } => {
                let item = match item_of(ItemKind::Skill, &name) {
                    Ok(item) => item.id.clone(),
                    Err(code) => return Ok(ready(Err(code))),
                };
                (
                    Plan::Update { item: item.clone() },
                    vec![StoreChange::PreviewUpdate { item }],
                )
            }
            StoreCommand::AddMcp { name, reach, .. } => {
                if item_of(ItemKind::Mcp, &name).is_ok() {
                    return Ok(ready(Err(refused(
                        REFUSED,
                        format!("name_taken: an item named {name} is already in the store"),
                    ))));
                }
                (
                    Plan::AddMcp {
                        server: Server { name, reach },
                    },
                    Vec::new(),
                )
            }
            StoreCommand::UpdateMcp { name, reach, .. } => {
                let item = match item_of(ItemKind::Mcp, &name) {
                    Ok(item) => item.id.clone(),
                    Err(code) => return Ok(ready(Err(code))),
                };
                (
                    Plan::SetMcp {
                        item,
                        server: Server { name, reach },
                    },
                    Vec::new(),
                )
            }
        };
        let wait = match plan {
            Plan::Plain { .. } => PASS_WAIT,
            _ => CONFIRM_WAIT,
        };
        // The same command again from the same pane, as an agent whose tool
        // gave up on the first one runs it, waits on the run already
        // standing instead of raising a second confirm (engineering rule
        // 11); the first waiter is told, so it takes nothing down.
        if let Some((run_id, run)) =
            self.store_runs.runs.iter_mut().find(|(_, run)| {
                run.device_id == machine && run.pane_id == pane_id && run.plan == plan
            })
        {
            let (reply, receiver) = channel();
            let _ = std::mem::replace(&mut run.reply, reply).send(Err(refused(
                UNANSWERED,
                "the same command ran again and waits on this one's confirm",
            )));
            return Ok(Prepared {
                answer: Waiting::On {
                    run: run_id.clone(),
                    receiver,
                    wait,
                },
                runtime: context.runtime,
                notifier: context.notifier,
            });
        }
        if self.store_runs.runs.len() >= RUN_LIMIT {
            return Ok(ready(Err(refused(
                BUSY,
                format!("{RUN_LIMIT} store commands are waiting already"),
            ))));
        }
        self.store_runs.seq += 1;
        let run_id = format!(
            "cli-{}-{}",
            super::kit_store::kit_store_now_ms(),
            self.store_runs.seq
        );
        let (reply, receiver) = channel();
        let stage = match &plan {
            Plan::Plain { .. } => Stage::Applying {
                request: run_id.clone(),
            },
            Plan::Install { .. } | Plan::Update { .. } => Stage::Reading {
                request: format!("{run_id}-read"),
                ended: false,
            },
            Plan::AddMcp { server } => Stage::Confirming(self.store_ask(
                &run_id,
                &machine,
                pane_id,
                StoreAskWhat::AddMcp {
                    server: server.clone(),
                },
            )),
            Plan::SetMcp { item, server } => Stage::Confirming(self.store_ask(
                &run_id,
                &machine,
                pane_id,
                StoreAskWhat::SetMcp {
                    item: item.clone(),
                    server: server.clone(),
                },
            )),
        };
        let request = match &stage {
            Stage::Applying { request } | Stage::Reading { request, .. } => Some(request.clone()),
            Stage::Confirming(_) => None,
        };
        self.store_runs.runs.insert(
            run_id.clone(),
            Run {
                device_id: machine.clone(),
                pane_id: pane_id.to_owned(),
                plan,
                stage,
                reply,
            },
        );
        match request {
            Some(request) => {
                let refusal = self.store_request_refusal(&machine, &request, changes.len());
                self.start_store_request(&machine, request, changes, refusal);
            }
            None => {
                self.store_asks_moved();
            }
        }
        Ok(Prepared {
            answer: Waiting::On {
                run: run_id,
                receiver,
                wait,
            },
            runtime: context.runtime,
            notifier: context.notifier,
        })
    }

    fn store_ask(
        &self,
        id: &str,
        device_id: &str,
        pane_id: &str,
        what: StoreAskWhat,
    ) -> StoreAskSnapshot {
        StoreAskSnapshot {
            id: id.to_owned(),
            device_id: device_id.to_owned(),
            pane_id: pane_id.to_owned(),
            since_unix_ms: super::kit_store::kit_store_now_ms(),
            what,
        }
    }

    /// A store request ended: the command whose step it was goes on, or
    /// ends with the store's refusal.
    pub(super) fn store_request_ended(&mut self, request_id: &str) {
        let Some(run_id) = self.store_runs.runs.iter().find_map(|(id, run)| {
            matches!(&run.stage,
                Stage::Reading { request, .. } | Stage::Applying { request } if request == request_id)
            .then(|| id.clone())
        }) else {
            return;
        };
        let Some(answer) = self
            .snapshot
            .status
            .store_answers
            .iter()
            .find(|answer| answer.request_id == request_id)
            .cloned()
        else {
            return;
        };
        let refusal = answer_refusal(&answer);
        let Some(run) = self.store_runs.runs.get_mut(&run_id) else {
            return;
        };
        match (&mut run.stage, refusal) {
            (_, Some(refusal)) => self.finish_store_run(&run_id, Err(refusal)),
            (Stage::Reading { ended, .. }, None) => *ended = true,
            (Stage::Applying { .. }, None) => {
                let item = match &run.plan {
                    Plan::Plain { item } => item.clone(),
                    Plan::Install { .. } => None,
                    Plan::Update { item } | Plan::SetMcp { item, .. } => Some(item.clone()),
                    Plan::AddMcp { server } => Some(StoreItem::id_of(ItemKind::Mcp, &server.name)),
                };
                let value = answer_value(&StoreChanged {
                    machine: run.device_id.clone(),
                    item,
                    outcomes: answer.outcomes,
                });
                self.finish_store_run(&run_id, Ok(value));
            }
            (Stage::Confirming(_), None) => {}
        }
    }

    /// The machine's new report is in: each command whose source or update
    /// it read asks the operator now. True when an ask stands up.
    pub(super) fn store_report_landed(&mut self, device_id: &str) -> bool {
        let ready = self
            .store_runs
            .runs
            .iter()
            .filter(|(_, run)| {
                run.device_id == device_id
                    && matches!(run.stage, Stage::Reading { ended: true, .. })
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return false;
        }
        let report = self.kit_view(device_id).store.unwrap_or_default();
        for run_id in ready {
            let Some(run) = self.store_runs.runs.get(&run_id) else {
                continue;
            };
            let what = match &run.plan {
                Plan::Install { source, names } => install_ask(&report, source, names),
                Plan::Update { item } => report
                    .diffs
                    .iter()
                    .filter(|diff| &diff.item == item)
                    .max_by_key(|diff| diff.at_unix_ms)
                    .map(|diff| StoreAskWhat::Update {
                        item: item.clone(),
                        diff: diff.id.clone(),
                    })
                    .ok_or_else(|| refused(REFUSED, "not_found: the update could not be read")),
                Plan::Plain { .. } | Plan::AddMcp { .. } | Plan::SetMcp { .. } => continue,
            };
            match what {
                Ok(what) => {
                    let ask = self.store_ask(&run_id, device_id, &run.pane_id.clone(), what);
                    if let Some(run) = self.store_runs.runs.get_mut(&run_id) {
                        run.stage = Stage::Confirming(ask);
                    }
                }
                Err(refusal) => self.finish_store_run(&run_id, Err(refusal)),
            }
        }
        self.store_asks_moved()
    }

    /// The operator answered a confirm (`store_ask_answer`): [취소] ends the
    /// command declined; [설치] queues the change with what the confirm
    /// holds, the skills left ticked and the keys typed.
    pub(super) fn answer_store_ask(&mut self, payload: StoreAskAnswerPayload) -> bool {
        let StoreAskAnswerPayload {
            id,
            confirm,
            names,
            keys,
        } = payload;
        let Some(run) = self.store_runs.runs.get(&id) else {
            crate::diagnostic!(json!({
                "component": "store",
                "kind": "ask.answer_stale",
                "ask_id": id,
            }));
            return false;
        };
        let Stage::Confirming(ask) = &run.stage else {
            return false;
        };
        if !confirm {
            self.finish_store_run(&id, Err(DECLINED.to_owned()));
            return true;
        }
        let change = match &ask.what {
            StoreAskWhat::Install {
                preview,
                names: asked,
            } => {
                let names = names
                    .map(|names| {
                        names
                            .into_iter()
                            .filter(|name| asked.contains(name))
                            .collect()
                    })
                    .unwrap_or_else(|| asked.clone());
                if names.is_empty() {
                    self.finish_store_run(&id, Err(DECLINED.to_owned()));
                    return true;
                }
                StoreChange::Install {
                    preview: preview.clone(),
                    names,
                }
            }
            StoreAskWhat::Update { item, diff } => {
                let report = self.kit_view(&run.device_id).store.unwrap_or_default();
                match report.diffs.iter().find(|shown| &shown.id == diff) {
                    Some(diff) => StoreChange::Update {
                        item: item.clone(),
                        commit: diff.commit.clone(),
                    },
                    None => {
                        self.finish_store_run(
                            &id,
                            Err(refused(REFUSED, "not_found: the update shown is gone")),
                        );
                        return true;
                    }
                }
            }
            StoreAskWhat::AddMcp { server } => StoreChange::AddMcp {
                server: server.clone(),
                keys,
                from: None,
            },
            StoreAskWhat::SetMcp { item, server } => StoreChange::SetMcp {
                item: item.clone(),
                server: server.clone(),
                keys,
            },
        };
        let device_id = run.device_id.clone();
        let request = format!("{id}-apply");
        if let Some(run) = self.store_runs.runs.get_mut(&id) {
            run.stage = Stage::Applying {
                request: request.clone(),
            };
        }
        self.store_asks_moved();
        let refusal = self.store_request_refusal(&device_id, &request, 1);
        self.start_store_request(&device_id, request, vec![change], refusal);
        true
    }

    /// The command stopped waiting: its confirm comes down. True when the
    /// snapshot changed.
    pub(crate) fn abandon_store_run(&mut self, run_id: &str) -> bool {
        if self.store_runs.runs.remove(run_id).is_none() {
            return false;
        }
        crate::diagnostic!(json!({
            "component": "store",
            "kind": "command.abandoned",
            "run_id": run_id,
        }));
        self.store_asks_moved();
        true
    }

    fn finish_store_run(&mut self, run_id: &str, answer: Result<Value, String>) {
        let Some(run) = self.store_runs.runs.remove(run_id) else {
            return;
        };
        if let Err(code) = &answer {
            crate::diagnostic!(json!({
                "component": "store",
                "kind": "command.refused",
                "run_id": run_id,
                "device_id": run.device_id,
                "code": crate::coordination::split_refusal(code).0,
            }));
        }
        let _ = run.reply.send(answer);
        if matches!(run.stage, Stage::Confirming(_)) {
            self.store_asks_moved();
        }
    }

    /// A device that went ends the commands waiting on it.
    pub(super) fn forget_store_runs(&mut self, device_id: &str) {
        let gone = self
            .store_runs
            .runs
            .iter()
            .filter(|(_, run)| run.device_id == device_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for run_id in gone {
            self.finish_store_run(
                &run_id,
                Err(refused(UNAVAILABLE, format!("{device_id}'s link ended"))),
            );
        }
    }

    /// The confirms standing now went up or came down: the snapshot lists
    /// them, and each lays its turn on the asking agent's row. True when
    /// either changed.
    fn store_asks_moved(&mut self) -> bool {
        let asks = self
            .store_runs
            .runs
            .values()
            .filter_map(|run| match &run.stage {
                Stage::Confirming(ask) => Some(ask.clone()),
                Stage::Reading { .. } | Stage::Applying { .. } => None,
            })
            .collect::<Vec<_>>();
        let listed = asks != self.snapshot.status.store_asks;
        self.snapshot.status.store_asks = asks;
        let laid = self.lay_store_asks();
        listed || laid
    }

    /// The person's turns the rows carry: each agent's own declaration from
    /// the ledger, else the confirm it waits on. True when they moved, with
    /// the lineage and the read state laid again.
    pub(super) fn lay_store_asks(&mut self) -> bool {
        let mut laid = self.human_turns_declared.clone();
        for ask in &self.snapshot.status.store_asks {
            let place = match &ask.what {
                StoreAskWhat::Install { names, .. } => names.join(", "),
                StoreAskWhat::Update { item, .. } | StoreAskWhat::SetMcp { item, .. } => item
                    .split_once(':')
                    .map_or(item.as_str(), |(_, name)| name)
                    .to_owned(),
                StoreAskWhat::AddMcp { server } => server.name.clone(),
            };
            laid.entry(ask.pane_id.clone())
                .or_insert_with(|| DeclaredHumanTurn {
                    id: ask.id.clone(),
                    kind: HumanKind::Install,
                    place: crate::coordination::human_turn::place_of(&place)
                        .unwrap_or_else(|_| ask.id.clone()),
                    device: self.device_label(&ask.device_id),
                    since_unix_ms: ask.since_unix_ms,
                });
        }
        if laid == self.human_turns_laid {
            return false;
        }
        self.human_turns_laid = laid;
        self.refresh_agent_lineage();
        self.refresh_pane_read_state();
        true
    }
}

/// The store column `agent` names: a column's own id, or one of the agents
/// it reaches.
fn column_of(report: &StoreReport, kind: ItemKind, agent: &str) -> Option<String> {
    let columns = report.columns.get(&kind)?;
    columns
        .iter()
        .find(|column| column.id == agent)
        .or_else(|| {
            columns
                .iter()
                .find(|column| column.agents.iter().any(|one| one == agent))
        })
        .map(|column| column.id.clone())
}

/// The confirm of an install: the newest preview of `source`, with the
/// skills named, or every one not in the store yet.
fn install_ask(
    report: &StoreReport,
    source: &SourceSpec,
    names: &[String],
) -> Result<StoreAskWhat, String> {
    let preview = report
        .previews
        .iter()
        .filter(|preview| &preview.source == source)
        .max_by_key(|preview| preview.at_unix_ms)
        .ok_or_else(|| refused(REFUSED, "not_found: the source could not be read"))?;
    let names = if names.is_empty() {
        preview
            .skills
            .iter()
            .filter(|skill| !skill.taken)
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>()
    } else {
        for name in names {
            match preview.skills.iter().find(|skill| &skill.name == name) {
                None => {
                    return Err(refused(
                        INVALID,
                        format!("{name} is not a skill of this source"),
                    ));
                }
                Some(skill) if skill.taken => {
                    return Err(refused(
                        REFUSED,
                        format!("name_taken: an item named {name} is already in the store"),
                    ));
                }
                Some(_) => {}
            }
        }
        names.to_vec()
    };
    if names.is_empty() {
        return Err(refused(
            REFUSED,
            "name_taken: every skill of this source is in the store already",
        ));
    }
    Ok(StoreAskWhat::Install {
        preview: preview.id.clone(),
        names,
    })
}

/// Why an ended request refuses its command, if it does: it never ran, or
/// one of its changes was refused.
fn answer_refusal(answer: &StoreAnswer) -> Option<String> {
    if answer.state == AnswerState::Refused {
        return Some(refused(
            UNAVAILABLE,
            "the machine's store could not run the change",
        ));
    }
    answer
        .outcomes
        .iter()
        .find_map(|outcome| outcome.refused.as_ref())
        .map(store_refused)
}
