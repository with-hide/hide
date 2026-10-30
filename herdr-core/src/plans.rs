//! A session's own plan (PRD hide-plan): the steps an agent declares for
//! its work with `hide plan`, kept by the core for the native session that
//! declared it (D-06) and drawn as a map (D-10).
//!
//! The graph rules are [`crate::dag`]'s, the ones a mission's plan follows
//! too (D-30); this module adds only what a session's plan has of its own:
//! the four statuses, the review the operator gives in conversation (D-04,
//! D-05), what a replan changed and why (D-07), and when it was last
//! updated. Hide runs nothing by the graph: it refuses a step done before
//! the steps it waits on, and computes the current step and the count
//! (D-03, D-08). The record is saved in the link store beside the session
//! record, by the link worker, and deleted with the session (D-30, B23).
//!
//! A step may be handed to another Hide agent (PRD hide-plan-delegate): the
//! step keeps the delegation's attempt and where it stands, and a step moves
//! to working or done only through [`judge`], the one place that holds the
//! order, the evidence its output asks for and the delegated child's report
//! (D-13, D-14), for `node` and `set` alike.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dag::{self, NODE_LIMIT};
use crate::links::ledger::SessionIdentity;
use crate::node_access::NodeLink;

/// Plans kept at once; past it the plan updated longest ago whose session
/// runs in no pane is let go with a diagnostic, as the session record lets
/// its oldest entries go past its own caps (engineering #15).
pub const PLAN_LIMIT: usize = 128;
/// Steps one step may wait on (D-23).
pub const DEPENDENCY_LIMIT: usize = 8;
const TITLE_LIMIT: usize = 300;
const LINE_LIMIT: usize = 600;
/// Bytes of a file path a step names as its evidence.
const PATH_LIMIT: usize = 1024;
/// Children a main's plans may have handed steps to at once, counting every
/// one that has not ended or gone to sleep (D-15, engineering #15).
pub const PLAN_DELEGATE_LIMIT: usize = 3;
/// How long a plan may go without an update before the prompt hint adds
/// the command that updates it (D-14, B17).
pub const STALE_AFTER_MS: u64 = 30 * 60 * 1000;

/// Where one step stands (D-03). A dropped step counts as finished for the
/// steps below it (D-26) and is left out of the count (D-25).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Planned,
    Working,
    Done,
    Dropped,
}

impl StepStatus {
    fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Dropped)
    }
}

/// What a step leaves behind, drawn as the tail of its card (D-02).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    Pr,
    File,
    None,
}

/// Whether the operator's review is still awaited (D-04): a plan put up
/// with `--ask` is proposed until the agent records the operator's answer
/// with `hide plan approve`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Review {
    #[default]
    Approved,
    Proposed,
}

/// What a finished step left, as its done recorded it after the check (PRD
/// hide-plan-delegate D-13, D-19): a pull request by repository and number,
/// or a file by its path below the session's checkout.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Evidence {
    /// `repo` is empty while the claim waits for its check.
    Pr {
        repo: String,
        number: u32,
    },
    File {
        path: String,
    },
}

impl Evidence {
    /// The same claim: a pull request by number, a file by path.
    fn same_claim(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Pr { number: a, .. }, Self::Pr { number: b, .. }) => a == b,
            (Self::File { path: a }, Self::File { path: b }) => a == b,
            _ => false,
        }
    }
}

/// Where a handed step's current attempt stands, as its child's letters and
/// its end say (D-07, D-22): working, its success report came, it sent a
/// `block`, or its pane closed for good without a report.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationState {
    Working,
    Reported,
    Blocked,
    Closed,
}

/// The agent a step was handed to (D-24): the attempt, the spawn that
/// started its child and the child once it registered, and where it stands.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Delegation {
    pub attempt: u32,
    /// The spawn record of the attempt, which names its child once bound.
    pub spawn: String,
    /// The child's name and agent kind, as the spawn asked for them.
    pub name: String,
    pub agent: String,
    /// The child's registration; none while it is still being started.
    #[serde(default)]
    pub agent_id: Option<String>,
    pub state: DelegationState,
    /// When the attempt's child sent its success report; a later attempt
    /// starts without one, so an earlier child's report never counts (B8).
    #[serde(default)]
    pub reported_at_unix_ms: Option<u64>,
    /// The main took the step back (직접 하기, 빼기, or a replan that left
    /// it out): the step is the main's own again, and the attempt stays so
    /// the next hand-off takes a new one (B17).
    #[serde(default)]
    pub released: bool,
    /// The ledger number of the last letter folded into `state`, so a fold
    /// after a restart neither repeats a letter nor loses one.
    #[serde(default)]
    pub seen: u64,
}

impl Delegation {
    /// Handed and not taken back: its child's report holds the step's done.
    pub fn active(&self) -> bool {
        !self.released
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Step {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub status: StepStatus,
    /// What the step does now, in one line.
    #[serde(default)]
    pub line: Option<String>,
    #[serde(default)]
    pub output: Option<Output>,
    /// The last `set` changed this step: a new title, a new step, or one
    /// the new plan left out, kept as dropped (B6).
    #[serde(default)]
    pub changed: bool,
    /// Why, as the agent said with `--reason` or `--ask`.
    #[serde(default)]
    pub change: Option<String>,
    /// The agent means to hand the step to another agent (맡길 예정, B3).
    #[serde(default)]
    pub delegate: bool,
    /// What its done left, checked when it was recorded (B10-B12); a step
    /// done before evidence existed has none (B15).
    #[serde(default)]
    pub evidence: Option<Evidence>,
    /// The agent the step was handed to, kept after its done so the plan
    /// still says who did it (D-32).
    #[serde(default)]
    pub delegation: Option<Delegation>,
}

impl dag::Node for Step {
    fn key(&self) -> &str {
        &self.key
    }
    fn depends_on(&self) -> &[String] {
        &self.depends_on
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Plan {
    pub session: SessionIdentity,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub review: Review,
    /// Why the operator is asked, when the agent said so.
    #[serde(default)]
    pub ask: Option<String>,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

impl Plan {
    /// The plan's instance (D-24), which a delegation's intent names: its
    /// first `set`'s time, unique among the session's plans because a plan
    /// that replaces a finished one is made later than it.
    pub fn id(&self) -> String {
        self.created_at_unix_ms.to_string()
    }
}

/// What `hide plan set` sends: the whole step list, each step with its key,
/// title, the keys it waits on and its status (D-02).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInput {
    pub nodes: Vec<StepInput>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StepInput {
    pub key: String,
    pub title: String,
    pub depends_on: Vec<String>,
    pub status: StepStatus,
    #[serde(default)]
    pub line: Option<String>,
    #[serde(default)]
    pub output: Option<Output>,
    /// 맡길 예정: the step is meant for another agent (B3).
    #[serde(default)]
    pub delegate: bool,
    /// A done step's pull request or file, checked as `node --pr`/`--file`
    /// would check it (B14).
    #[serde(default)]
    pub pr: Option<u32>,
    #[serde(default)]
    pub file: Option<String>,
}

impl dag::Node for StepInput {
    fn key(&self) -> &str {
        &self.key
    }
    fn depends_on(&self) -> &[String] {
        &self.depends_on
    }
}

/// One `hide plan` command (D-27). `set` and `node` change the caller's own
/// plan; `approve` and `drop` name another agent only to be refused, since
/// the operator answers a plan in its own conversation (D-05); `show` reads
/// the caller's plan or another agent's.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "verb", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanCommand {
    Set {
        plan: PlanInput,
        /// `--ask [why]`: put the plan up for the operator's review.
        #[serde(default)]
        ask: bool,
        #[serde(default)]
        why: Option<String>,
        #[serde(default)]
        reason: Option<String>,
    },
    Node {
        key: String,
        #[serde(default)]
        status: Option<StepStatus>,
        #[serde(default)]
        line: Option<String>,
        /// The pull request a done step made (`--pr <n>`, B10, B11).
        #[serde(default)]
        pr: Option<u32>,
        /// The file a done step wrote, below the checkout (`--file`, B12).
        #[serde(default)]
        file: Option<String>,
    },
    Approve {
        #[serde(default)]
        agent: Option<String>,
    },
    Drop {
        #[serde(default)]
        agent: Option<String>,
    },
    Show {
        #[serde(default)]
        agent: Option<String>,
    },
}

impl PlanCommand {
    pub fn mutates(&self) -> bool {
        !matches!(self, Self::Show { .. })
    }

    /// The bounds a daemon checks before the command reaches the core.
    pub fn valid(&self) -> bool {
        let agent_ok = |agent: &Option<String>| {
            agent.as_deref().is_none_or(|id| {
                !id.is_empty() && id.len() <= 64 && !id.chars().any(char::is_control)
            })
        };
        match self {
            Self::Set { why, reason, .. } => {
                // Too many steps is the core's `plan_too_large`, which says
                // what to do about it.
                valid_line(why.as_deref(), LINE_LIMIT) && valid_line(reason.as_deref(), LINE_LIMIT)
            }
            Self::Node {
                key,
                status,
                line,
                pr,
                file,
            } => {
                let evidence = pr.is_some() || file.is_some();
                dag::valid_key(key)
                    && (status.is_some() || line.is_some())
                    && valid_line(line.as_deref(), LINE_LIMIT)
                    // Evidence is what a done leaves, one of them.
                    && (!evidence || *status == Some(StepStatus::Done))
                    && !(pr.is_some() && file.is_some())
                    && pr.is_none_or(|number| number > 0)
                    && valid_line(file.as_deref(), PATH_LIMIT)
            }
            Self::Approve { agent } | Self::Drop { agent } | Self::Show { agent } => {
                agent_ok(agent)
            }
        }
    }
}

fn valid_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn valid_line(value: Option<&str>, limit: usize) -> bool {
    value.is_none_or(|value| valid_text(value, limit))
}

/// A refusal with where it was found: the code a caller branches on and
/// the step or field to fix, joined as the daemon's refusals are.
fn refused(code: &str, detail: impl std::fmt::Display) -> String {
    crate::coordination::refusal(code, &detail.to_string())
}

/// The step `steps` lets nothing be done before: the first step `key`
/// waits on that is neither done nor dropped.
fn blocker<'a>(steps: &'a [Step], step: &'a Step) -> Option<&'a str> {
    step.depends_on.iter().map(String::as_str).find(|key| {
        steps
            .iter()
            .find(|other| other.key == *key)
            .is_some_and(|other| !other.status.finished())
    })
}

/// The plan `command`'s `set` makes for `session` from `previous`, or why
/// it is refused (B1, B6, D-07). The plan it answers is judged by [`judge`]
/// before anything keeps it.
///
/// A plan whose steps are all finished is replaced whole, so new work
/// starts a new plan, with a new id. Otherwise a step the new list leaves
/// out is kept as dropped, and each step whose title is new or changed is
/// marked with the reason. The review stays as it was unless `ask` puts the
/// plan up again; a first plan is approved unless it asks. A step keeps its
/// evidence and its delegation across a replan (B14); a step left out, sent
/// back to planned or dropped is taken back from its child.
pub(crate) fn apply_set(
    previous: Option<&Plan>,
    session: &SessionIdentity,
    input: &PlanInput,
    ask: bool,
    why: Option<&str>,
    reason: Option<&str>,
    now: u64,
) -> Result<Plan, String> {
    if input.nodes.is_empty() {
        return Err(refused("plan_invalid", "nodes is empty"));
    }
    if input.nodes.len() > NODE_LIMIT {
        return Err(refused("plan_too_large", NODE_LIMIT));
    }
    let mut keys = BTreeSet::new();
    for step in &input.nodes {
        if dag::admit_key(&mut keys, &step.key).is_err() {
            return Err(refused("plan_invalid", format!("key {:?}", step.key)));
        }
        if !valid_text(&step.title, TITLE_LIMIT) {
            return Err(refused("plan_invalid", format!("{}: title", step.key)));
        }
        if !valid_line(step.line.as_deref(), LINE_LIMIT) {
            return Err(refused("plan_invalid", format!("{}: line", step.key)));
        }
        if step.depends_on.len() > DEPENDENCY_LIMIT {
            return Err(refused(
                "plan_invalid",
                format!(
                    "{}: depends_on names more than {DEPENDENCY_LIMIT}",
                    step.key
                ),
            ));
        }
        if (step.pr.is_some() || step.file.is_some())
            && (step.status != StepStatus::Done
                || (step.pr.is_some() && step.file.is_some())
                || step.pr == Some(0)
                || !valid_line(step.file.as_deref(), PATH_LIMIT))
        {
            return Err(refused(
                "plan_invalid",
                format!("{}: pr or file, one of them, on a done step", step.key),
            ));
        }
    }
    match dag::check_edges(&input.nodes) {
        Ok(()) => {}
        Err(dag::Refusal::Unknown { node, dependency }) => {
            return Err(refused(
                "plan_invalid",
                format!("{node}: depends_on {dependency:?} is no other step"),
            ));
        }
        Err(dag::Refusal::Cycle(node)) => {
            return Err(refused("plan_invalid", format!("{node}: depends_on loops")));
        }
        Err(dag::Refusal::Key(key)) => {
            return Err(refused("plan_invalid", format!("key {key:?}")));
        }
    }
    let replaced = previous.filter(|plan| finished(plan));
    let previous = previous.filter(|plan| !finished(plan));
    let change = why.or(reason).map(str::to_owned);
    let mut steps: Vec<Step> = input
        .nodes
        .iter()
        .map(|input| {
            let before =
                previous.and_then(|plan| plan.steps.iter().find(|step| step.key == input.key));
            let changed = previous.is_some() && before.is_none_or(|step| step.title != input.title);
            let claim = match (input.pr, &input.file) {
                (Some(number), _) => Some(Evidence::Pr {
                    repo: String::new(),
                    number,
                }),
                (None, Some(path)) => Some(Evidence::File { path: path.clone() }),
                (None, None) => None,
            };
            let kept = before.and_then(|step| step.evidence.clone());
            let evidence = match (claim, kept) {
                // The same pull request again keeps the repository its
                // check read.
                (Some(claim), Some(kept)) if claim.same_claim(&kept) => Some(kept),
                (Some(claim), _) => Some(claim),
                (None, kept) => kept,
            };
            Step {
                key: input.key.clone(),
                title: input.title.clone(),
                depends_on: input.depends_on.clone(),
                status: input.status,
                line: input.line.clone(),
                output: input.output,
                changed,
                change: if changed { change.clone() } else { None },
                delegate: input.delegate,
                evidence,
                delegation: before.and_then(|step| step.delegation.clone()),
            }
        })
        .collect();
    if let Some(previous) = previous {
        for gone in previous
            .steps
            .iter()
            .filter(|step| !keys.contains(step.key.as_str()))
        {
            if steps.len() >= NODE_LIMIT {
                return Err(refused("plan_too_large", NODE_LIMIT));
            }
            // Finished work stays finished, so the count never goes back;
            // anything else left out is dropped.
            let done = gone.status == StepStatus::Done;
            steps.push(Step {
                status: if done {
                    StepStatus::Done
                } else {
                    StepStatus::Dropped
                },
                // What it waited on may be gone too; a left-out step holds
                // nothing up and keeps no edge to a step that is not there,
                // nor, when done, to one the new list has not finished.
                depends_on: gone
                    .depends_on
                    .iter()
                    .filter(|key| {
                        input.nodes.iter().any(|node| {
                            node.key == **key && (!done || node.status == StepStatus::Done)
                        })
                    })
                    .cloned()
                    .collect(),
                changed: true,
                change: change.clone(),
                ..gone.clone()
            });
        }
    }
    for step in &mut steps {
        if matches!(step.status, StepStatus::Planned | StepStatus::Dropped) {
            release(step);
        }
    }
    Ok(Plan {
        session: session.clone(),
        steps,
        review: if ask {
            Review::Proposed
        } else {
            previous.map_or(Review::Approved, |plan| plan.review)
        },
        ask: if ask {
            why.map(str::to_owned)
        } else {
            previous.and_then(|plan| plan.ask.clone())
        },
        // A new plan never shares its id with the one it replaces.
        created_at_unix_ms: previous.map_or_else(
            || replaced.map_or(now, |plan| now.max(plan.created_at_unix_ms + 1)),
            |plan| plan.created_at_unix_ms,
        ),
        updated_at_unix_ms: now,
    })
}

/// Takes `step` back from its child: the step is the main's own again.
fn release(step: &mut Step) {
    if let Some(delegation) = step.delegation.as_mut() {
        delegation.released = true;
    }
}

/// `plan` with step `key` moved to `status`, its line set and its done's
/// evidence claimed, or why it is refused (B2). The same command again
/// changes nothing (`None`). The plan it answers is judged by [`judge`].
///
/// `--status working` on a step whose child is blocked or closed takes the
/// step back (직접 하기, B17), as `planned` and `dropped` take any handed
/// step back.
pub(crate) fn apply_node(
    plan: &Plan,
    key: &str,
    status: Option<StepStatus>,
    line: Option<&str>,
    claim: Option<Evidence>,
    now: u64,
) -> Result<Option<Plan>, String> {
    let index = plan
        .steps
        .iter()
        .position(|step| step.key == key)
        .ok_or_else(|| refused("plan_node_unknown", key))?;
    let step = &plan.steps[index];
    let mut next_step = step.clone();
    next_step.status = status.unwrap_or(step.status);
    next_step.line = line.map(str::to_owned).or_else(|| step.line.clone());
    if let Some(claim) = claim
        && !step
            .evidence
            .as_ref()
            .is_some_and(|kept| kept.same_claim(&claim))
    {
        next_step.evidence = Some(claim);
    }
    let taken_back = match status {
        Some(StepStatus::Planned | StepStatus::Dropped) => true,
        Some(StepStatus::Working) => step.delegation.as_ref().is_some_and(|delegation| {
            matches!(
                delegation.state,
                DelegationState::Blocked | DelegationState::Closed
            )
        }),
        _ => false,
    };
    if taken_back {
        release(&mut next_step);
    }
    if next_step == *step {
        return Ok(None);
    }
    let mut next = plan.clone();
    next.steps[index] = next_step;
    next.updated_at_unix_ms = now;
    Ok(Some(next))
}

/// What a change's evidence needs read off the lock before it is judged.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Asks {
    /// Pull request numbers, read in the session's checkout's repository.
    pub prs: BTreeSet<u32>,
    /// Paths below the session's checkout.
    pub files: BTreeSet<String>,
}

impl Asks {
    pub fn is_empty(&self) -> bool {
        self.prs.is_empty() && self.files.is_empty()
    }
}

/// What a pull request's check read (D-19).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrProof {
    /// Made from the session's branch or one of its plan children's, in
    /// `repo`.
    Yours {
        repo: String,
    },
    NotYours,
    /// GitHub could not be asked now; the same command later can.
    Unconfirmed,
}

/// What a file's check read on the session's machine (D-19).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileProof {
    Present,
    /// A link inside the checkout leads out of it.
    Outside,
    Missing,
    /// The machine could not be asked now.
    Unconfirmed,
}

/// The answers of [`Asks`], read off the lock.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Proofs {
    pub prs: BTreeMap<u32, PrProof>,
    pub files: BTreeMap<String, FileProof>,
}

/// Whether `after` may stand in place of `before` (D-13, D-14, B9-B14): the
/// one judgment every change of a step's status goes through, `node` and
/// `set` alike. A step newly working or done waits on no unfinished step; a
/// step newly done carries the evidence its output asks for and, while it
/// is handed to a child, that child's success report; and evidence new to
/// a step is checked off the lock. A done step that was already done keeps
/// what it had, so a plan made before these checks goes on as it was (B15).
///
/// Answers the reads still owed (`Asks`, empty when `after` is judged and
/// final) or the first refusal; on a refusal nothing of `after` is kept.
/// A step that is not done carries no evidence.
pub(crate) fn judge(
    before: Option<&Plan>,
    after: &mut Plan,
    proofs: &Proofs,
) -> Result<Asks, String> {
    let was = |key: &str| before.and_then(|plan| plan.steps.iter().find(|step| step.key == key));
    // A step newly working or done waits on nothing unfinished, and so does
    // a done step a replan gave another edge; a done step kept as it was is
    // not judged again, so moving a step back never locks the plan.
    if let Some((step, blocked)) = after
        .steps
        .iter()
        .filter(|step| match (step.status, was(&step.key)) {
            (StepStatus::Done, Some(prior)) => {
                prior.status != StepStatus::Done || prior.depends_on != step.depends_on
            }
            (StepStatus::Working, Some(prior)) => prior.status != StepStatus::Working,
            (status, None) => matches!(status, StepStatus::Done | StepStatus::Working),
            _ => false,
        })
        .find_map(|step| blocker(&after.steps, step).map(|key| (step, key)))
    {
        return Err(refused(
            "plan_order_refused",
            format!("{} waits on {blocked}", step.key),
        ));
    }
    let mut asks = Asks::default();
    for step in &after.steps {
        if step.status != StepStatus::Done {
            continue;
        }
        let prior = was(&step.key);
        if prior.is_none_or(|prior| prior.status != StepStatus::Done) {
            if step.delegation.as_ref().is_some_and(|delegation| {
                delegation.active() && delegation.reported_at_unix_ms.is_none()
            }) {
                return Err(refused("plan_report_missing", &step.key));
            }
            let needed = match step.output {
                Some(Output::Pr) => Some("--pr"),
                Some(Output::File) => Some("--file"),
                Some(Output::None) | None => None,
            };
            let carried = match &step.evidence {
                Some(Evidence::Pr { .. }) => Some("--pr"),
                Some(Evidence::File { .. }) => Some("--file"),
                None => None,
            };
            if let Some(needed) = needed
                && carried != Some(needed)
            {
                return Err(refused(
                    "plan_evidence_missing",
                    format!("{} {needed}", step.key),
                ));
            }
        }
        let new = step.evidence.as_ref().filter(|evidence| {
            prior
                .and_then(|prior| prior.evidence.as_ref())
                .is_none_or(|kept| !kept.same_claim(evidence))
        });
        match new {
            Some(Evidence::Pr { number, .. }) => match proofs.prs.get(number) {
                None => {
                    asks.prs.insert(*number);
                }
                Some(PrProof::Yours { .. }) => {}
                Some(PrProof::NotYours) => {
                    return Err(refused(
                        "plan_pr_not_yours",
                        format!("{} #{number}", step.key),
                    ));
                }
                Some(PrProof::Unconfirmed) => {
                    return Err(refused(
                        "plan_pr_unconfirmed",
                        format!("{} #{number}", step.key),
                    ));
                }
            },
            Some(Evidence::File { path }) => match proofs.files.get(path) {
                None => {
                    asks.files.insert(path.clone());
                }
                Some(FileProof::Present) => {}
                Some(FileProof::Outside) => {
                    return Err(refused("plan_file_outside", path));
                }
                Some(FileProof::Missing) => {
                    return Err(refused("plan_file_missing", path));
                }
                Some(FileProof::Unconfirmed) => {
                    return Err(refused("plan_file_unconfirmed", path));
                }
            },
            None => {}
        }
    }
    if !asks.is_empty() {
        return Ok(asks);
    }
    for step in &mut after.steps {
        if step.status != StepStatus::Done {
            step.evidence = None;
        } else if let Some(Evidence::Pr { repo, number }) = step.evidence.as_mut()
            && repo.is_empty()
            && let Some(PrProof::Yours { repo: read }) = proofs.prs.get(number)
        {
            repo.clone_from(read);
        }
    }
    Ok(asks)
}

/// `path`, as a step's `--file` names it, below the checkout: a relative
/// path is read from the checkout's root, an absolute one must be inside
/// it, and `.` and `..` are resolved by name. `None` for a path that leads
/// out of the checkout (B12).
pub fn file_below(root: &str, path: &str) -> Option<String> {
    let relative = if hide_platform::path::is_wire_absolute(path) {
        hide_platform::path::wire_relative(root, path)
            .ok()?
            .into_string()
    } else {
        path.to_owned()
    };
    let mut names: Vec<&str> = Vec::new();
    for name in relative.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                names.pop()?;
            }
            name => names.push(name),
        }
    }
    let joined = names.join("/");
    hide_platform::path::RelPath::parse(&joined)
        .ok()
        .filter(|relative| !relative.is_root())
        .map(hide_platform::path::RelPath::into_string)
}

/// What a `hide plan` command answers, or the reads its evidence needs off
/// the lock before it can be judged.
pub enum PlanTurn {
    Answered { answer: Value, changed: bool },
    Prove(PlanProof),
}

/// The reads a change's evidence needs (D-19), made on the caller's request
/// worker, never under the lock or on the owner thread: the pull requests
/// in the session's checkout's repository and the files below it, on the
/// session's own machine, a connected device's through its helper (B24).
pub struct PlanProof {
    pub(crate) link: Arc<dyn NodeLink>,
    /// The session's checkout, as its machine spells it.
    pub(crate) root: String,
    /// The branches the session's plan children were started on.
    pub(crate) branches: BTreeSet<String>,
    pub(crate) asks: Asks,
    /// For the log: the plan and the session.
    pub(crate) plan: String,
    pub(crate) session_id: String,
}

impl PlanProof {
    /// Reads every ask into `proofs`; an answer already there is not read
    /// again.
    pub fn run(&self, proofs: &mut Proofs) {
        use hide_node_link::catalog::PathFacts;
        use hide_node_link::protocol::Call;
        let facts = |paths: Vec<String>| -> Result<PathFacts, String> {
            crate::node_access::call_as(self.link.as_ref(), Call::PathFacts { paths }, PROOF_CALL)
                .map_err(|error| error.to_string())
        };
        let root_fact = |facts: &PathFacts| facts.paths.get(&self.root).cloned();
        if self
            .asks
            .prs
            .iter()
            .any(|number| !proofs.prs.contains_key(number))
        {
            let mut branches = self.branches.clone();
            // The session's own branch, as its checkout has it now: a pull
            // request made a moment ago from it is the session's own.
            match facts(vec![self.root.clone()]) {
                Ok(read) => branches.extend(
                    root_fact(&read)
                        .and_then(|fact| fact.repository)
                        .and_then(|place| place.branch),
                ),
                Err(reason) => crate::diagnostic!(json!({
                    "component": "plans", "kind": "plans.branch_unread",
                    "plan": self.plan, "session_id": self.session_id, "reason": reason,
                })),
            }
            let place = crate::github::Where::Checkout(std::path::PathBuf::from(&self.root));
            for number in &self.asks.prs {
                if proofs.prs.contains_key(number) {
                    continue;
                }
                let proof = match crate::github::pr_head(self.link.as_ref(), &place, *number) {
                    Ok(head) if !head.cross_repository && branches.contains(&head.branch) => {
                        PrProof::Yours {
                            repo: head.repository,
                        }
                    }
                    Ok(_) | Err(crate::github::PrHeadFailure::Missing) => PrProof::NotYours,
                    Err(crate::github::PrHeadFailure::Unread(reason)) => {
                        crate::diagnostic!(json!({
                            "component": "plans", "kind": "plans.pr_unconfirmed",
                            "plan": self.plan, "session_id": self.session_id,
                            "number": number, "reason": reason,
                        }));
                        PrProof::Unconfirmed
                    }
                };
                proofs.prs.insert(*number, proof);
            }
        }
        let files: Vec<&String> = self
            .asks
            .files
            .iter()
            .filter(|path| !proofs.files.contains_key(*path))
            .collect();
        if files.is_empty() {
            return;
        }
        let below = |path: &str| {
            hide_platform::path::RelPath::parse(path)
                .map(|relative| hide_platform::path::wire_join(&self.root, &relative))
        };
        let mut paths = vec![self.root.clone()];
        paths.extend(files.iter().filter_map(|path| below(path).ok()));
        let read = facts(paths);
        for path in files {
            let proof = match (&read, below(path)) {
                (Ok(read), Ok(full)) => match (root_fact(read), read.paths.get(&full)) {
                    (Some(root), Some(fact)) if fact.exists => {
                        // A link inside the checkout may lead out of it.
                        if hide_platform::path::wire_relative(&root.comparison, &fact.comparison)
                            .is_ok()
                        {
                            FileProof::Present
                        } else {
                            FileProof::Outside
                        }
                    }
                    (Some(_), Some(_)) => FileProof::Missing,
                    _ => FileProof::Unconfirmed,
                },
                (Ok(_), Err(_)) => FileProof::Outside,
                (Err(reason), _) => {
                    crate::diagnostic!(json!({
                        "component": "plans", "kind": "plans.file_unconfirmed",
                        "plan": self.plan, "session_id": self.session_id, "reason": reason,
                    }));
                    FileProof::Unconfirmed
                }
            };
            proofs.files.insert(path.clone(), proof);
        }
    }
}

/// How long one read of a step's evidence may take: one `gh` call or one
/// path read, each bounded on the node as well.
const PROOF_CALL: std::time::Duration = std::time::Duration::from_secs(20);

/// Every step is done or dropped (D-25).
pub fn finished(plan: &Plan) -> bool {
    plan.steps.iter().all(|step| step.status.finished())
}

/// What a plan says at a glance (D-08, D-25): the current step, the step
/// after it, and `done` of `total`, where a dropped step counts in neither.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Progress {
    pub current: Option<String>,
    pub next: Option<String>,
    pub done: u32,
    pub total: u32,
    pub finished: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlanView {
    /// The pane as the agent rows name it; empty in a session's record.
    pub pane_id: String,
    pub session: SessionIdentity,
    pub steps: Vec<StepView>,
    pub review: Review,
    pub ask: Option<String>,
    pub progress: Progress,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StepView {
    pub key: String,
    pub title: String,
    pub depends_on: Vec<String>,
    pub status: StepStatus,
    pub line: Option<String>,
    pub output: Option<Output>,
    pub changed: bool,
    pub change: Option<String>,
    /// A step it waits on is neither done nor dropped (B7's dashed card).
    pub waiting: bool,
    /// 맡길 예정 (B3).
    pub delegate: bool,
    pub evidence: Option<Evidence>,
    /// The child the step is handed to, or was when it finished; none for
    /// a step the main does itself or took back (B20).
    pub delegation: Option<DelegationView>,
}

/// A step's child as the screen draws it (B20): where its attempt stands,
/// who it is, and its pane and session while Hide knows them, so the
/// screen reads the child's own plan by its session (D-12, D-33).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DelegationView {
    pub state: DelegationState,
    pub attempt: u32,
    /// None while the child is still being started.
    pub agent_id: Option<String>,
    pub name: String,
    pub agent: String,
    pub pane_id: Option<String>,
    pub session: Option<SessionIdentity>,
}

/// Where a step's child is now: its pane and its session, when Hide knows
/// them.
pub type ChildPlace = (Option<String>, Option<SessionIdentity>);

/// What every surface draws of `plan`: the map, the chip and a mission
/// node's own plan read this one value (D-10, D-18). A child's pane and
/// session are the runtime's facts; the session record draws none.
pub fn view_of(plan: &Plan, pane_id: &str) -> PlanView {
    view_with(plan, pane_id, &|_| (None, None))
}

/// [`view_of`] with each handed step's child placed by `place`, which reads
/// the child's registration id.
pub fn view_with(plan: &Plan, pane_id: &str, place: &dyn Fn(&str) -> ChildPlace) -> PlanView {
    let steps = plan
        .steps
        .iter()
        .map(|step| StepView {
            key: step.key.clone(),
            title: step.title.clone(),
            depends_on: step.depends_on.clone(),
            status: step.status,
            line: step.line.clone(),
            output: step.output,
            changed: step.changed,
            change: step.change.clone(),
            waiting: step.status != StepStatus::Done
                && step.status != StepStatus::Dropped
                && step.depends_on.iter().any(|key| {
                    plan.steps.iter().any(|other| {
                        other.key == *key
                            && !matches!(other.status, StepStatus::Done | StepStatus::Dropped)
                    })
                }),
            delegate: step.delegate,
            evidence: step.evidence.clone(),
            delegation: step
                .delegation
                .as_ref()
                .filter(|delegation| delegation.active())
                .map(|delegation| {
                    let (pane_id, session) =
                        delegation.agent_id.as_deref().map_or((None, None), place);
                    DelegationView {
                        state: delegation.state,
                        attempt: delegation.attempt,
                        agent_id: delegation.agent_id.clone(),
                        name: delegation.name.clone(),
                        agent: delegation.agent.clone(),
                        pane_id,
                        session,
                    }
                }),
        })
        .collect();
    PlanView {
        pane_id: pane_id.to_owned(),
        session: plan.session.clone(),
        steps,
        review: plan.review,
        ask: plan.ask.clone(),
        progress: progress(plan),
        created_at_unix_ms: plan.created_at_unix_ms,
        updated_at_unix_ms: plan.updated_at_unix_ms,
    }
}

/// The current step is the topmost working one, otherwise the topmost
/// planned one (D-03); the next is the topmost planned step that is not it.
pub fn progress(plan: &Plan) -> Progress {
    let count = |pick: fn(StepStatus) -> bool| {
        u32::try_from(plan.steps.iter().filter(|step| pick(step.status)).count())
            .unwrap_or(u32::MAX)
    };
    let current = dag::topmost(&plan.steps, |step| step.status == StepStatus::Working)
        .or_else(|| dag::topmost(&plan.steps, |step| step.status == StepStatus::Planned));
    let next = dag::topmost(&plan.steps, |step| {
        step.status == StepStatus::Planned
            && Some(step.key.as_str()) != current.map(|c| c.key.as_str())
    });
    Progress {
        current: current.map(|step| step.key.clone()),
        next: next.map(|step| step.key.clone()),
        done: count(|status| status == StepStatus::Done),
        total: count(|status| status != StepStatus::Dropped),
        finished: finished(plan),
    }
}

/// The refusal's next step, which a hint or a refusal gives an agent as a
/// command it can run (D-27).
pub fn next_action(code: &str) -> &'static str {
    match code {
        "plan_invalid" => {
            "Fix the plan where the reason says: each node needs key, title, depends_on (other nodes' keys, no loop) and status planned|working|done|dropped; then run hide plan set again"
        }
        "plan_order_refused" => {
            "Finish or drop the step it waits on first: hide plan node <that key> --status done (or dropped), then run this again"
        }
        "plan_too_large" => {
            "A plan holds at most 64 steps; merge steps and run hide plan set again"
        }
        "plan_node_unknown" => {
            "Run hide plan show for the plan's keys, then run hide plan node <key> again"
        }
        "plan_missing" => {
            "This session has no plan; put one up with hide plan set --file <plan.json>"
        }
        "plan_not_yours" => {
            "A plan is changed only by its own session: send that agent a letter (hide request send <agent> ...) saying what the operator answered, and it runs hide plan approve or drop itself"
        }
        "plan_unavailable" => {
            "Hide cannot read its plan record right now; go on with the work and run the same command again in a minute"
        }
        "plan_session_unknown" => {
            "Hide does not know this pane's agent session yet; run the same command again after your next message"
        }
        "plan_full" => {
            "Hide keeps 128 plans and every one belongs to an agent on screen; finish or drop a plan you no longer need (hide plan drop), then run hide plan set again"
        }
        "plan_agent_unknown" => {
            "No running agent has that id or name; run hide agent list for its id and run the command again"
        }
        "plan_evidence_missing" => {
            "A step whose output is pr ends with its pull request and one whose output is file with its file: run hide plan node <key> --status done --pr <number> (or --file <path>)"
        }
        "plan_pr_not_yours" => {
            "That pull request was not made from this session's branch or a branch of a child it handed a step to; name the pull request this step made"
        }
        "plan_pr_unconfirmed" => {
            "Hide could not ask GitHub about the pull request just now; go on with the work and run the same command again in a minute"
        }
        "plan_file_outside" => {
            "Name a file inside this session's checkout, relative to its root, and run the command again"
        }
        "plan_file_missing" => {
            "No file is there yet; write it (and commit it where the step says so), then run the command again"
        }
        "plan_file_unconfirmed" => {
            "Hide could not ask the session's machine about the file just now; run the same command again in a minute"
        }
        "plan_report_missing" => {
            "The step is handed to a child and its current attempt has sent no report yet; wait for its report letter (hide inbox), or take the step back with hide plan node <key> --status working once it is blocked or closed"
        }
        "plan_review_pending" => {
            "The plan waits for the operator's review; hand a step to another agent once the operator approved it (hide plan approve)"
        }
        "plan_delegate_needs_parent" => {
            "A step is handed to your own child: run hide agent spawn --parent here --plan-node <key> ..."
        }
        "plan_candidate_refused" => {
            "Hand the step to an agent and model among Settings > Conductor's worker candidates (hide mission settings shows them), then run hide agent spawn again"
        }
        "plan_delegate_full" => {
            "Three children of your plans are working already; wait for one to report and finish its step, or do this step yourself"
        }
        "plan_delegate_not_permitted" => {
            "An agent a plan or a mission started does its own work and starts no agents; do the step yourself, or tell your parent with hide request send"
        }
        _ => "Check the hide plan command (hide plan help) and run it again",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionIdentity {
        SessionIdentity {
            device_id: "mac".into(),
            agent: "claude".into(),
            session_id: "s1".into(),
        }
    }

    fn step(key: &str, depends: &[&str], status: StepStatus) -> StepInput {
        StepInput {
            key: key.into(),
            title: format!("{key} title"),
            depends_on: depends.iter().map(|key| (*key).to_owned()).collect(),
            status,
            line: None,
            output: None,
            delegate: false,
            pr: None,
            file: None,
        }
    }

    /// `set` as the runtime runs it: made, then judged with `proofs`.
    fn set_with(
        previous: Option<&Plan>,
        nodes: Vec<StepInput>,
        proofs: &Proofs,
    ) -> Result<Plan, String> {
        let mut next = apply_set(
            previous,
            &session(),
            &PlanInput { nodes },
            false,
            None,
            Some("why"),
            10,
        )?;
        let asks = judge(previous.filter(|plan| !finished(plan)), &mut next, proofs)?;
        assert!(asks.is_empty(), "owed {asks:?}");
        Ok(next)
    }

    fn set(previous: Option<&Plan>, nodes: Vec<StepInput>, ask: bool) -> Result<Plan, String> {
        let mut next = apply_set(
            previous,
            &session(),
            &PlanInput { nodes },
            ask,
            None,
            Some("why"),
            10,
        )?;
        let asks = judge(
            previous.filter(|plan| !finished(plan)),
            &mut next,
            &Proofs::default(),
        )?;
        assert!(asks.is_empty(), "owed {asks:?}");
        Ok(next)
    }

    /// `node` as the runtime runs it: changed, then judged with `proofs`;
    /// the asks it still owes when it is not judged yet.
    fn node_with(
        plan: &Plan,
        key: &str,
        status: StepStatus,
        claim: Option<Evidence>,
        proofs: &Proofs,
    ) -> Result<Result<Option<Plan>, Asks>, String> {
        let Some(mut next) = apply_node(plan, key, Some(status), None, claim, 50)? else {
            return Ok(Ok(None));
        };
        let asks = judge(Some(plan), &mut next, proofs)?;
        Ok(if asks.is_empty() {
            Ok(Some(next))
        } else {
            Err(asks)
        })
    }

    fn node(plan: &Plan, key: &str, status: StepStatus) -> Result<Option<Plan>, String> {
        node_with(plan, key, status, None, &Proofs::default()).map(|judged| judged.unwrap())
    }

    fn output(mut step: StepInput, output: Output) -> StepInput {
        step.output = Some(output);
        step
    }

    fn yours(number: u32) -> Proofs {
        let mut proofs = Proofs::default();
        proofs.prs.insert(
            number,
            PrProof::Yours {
                repo: "acme/app".into(),
            },
        );
        proofs
    }

    fn handed(plan: &mut Plan, key: &str, state: DelegationState, reported: bool) {
        let step = plan.steps.iter_mut().find(|step| step.key == key).unwrap();
        step.status = StepStatus::Working;
        step.delegation = Some(Delegation {
            attempt: 1,
            spawn: "spawn-1".into(),
            name: "web".into(),
            agent: "codex".into(),
            agent_id: Some("agent-9".into()),
            state,
            reported_at_unix_ms: reported.then_some(40),
            released: false,
            seen: 0,
        });
    }

    #[test]
    fn a_step_starts_only_after_what_it_waits_on_by_node_and_by_set() {
        use StepStatus::*;
        let plan = set(
            None,
            vec![step("a", &[], Planned), step("b", &["a"], Planned)],
            false,
        )
        .unwrap();
        assert_eq!(
            node(&plan, "b", Working).unwrap_err(),
            crate::coordination::refusal("plan_order_refused", "b waits on a")
        );
        assert_eq!(
            set(
                Some(&plan),
                vec![step("a", &[], Planned), step("b", &["a"], Working)],
                false
            )
            .unwrap_err(),
            crate::coordination::refusal("plan_order_refused", "b waits on a")
        );
        let started = node(&plan, "a", Working).unwrap().unwrap();
        let done = node(&started, "a", Done).unwrap().unwrap();
        assert_eq!(
            node(&done, "b", Working).unwrap().unwrap().steps[1].status,
            Working
        );
        // Moving a finished step back never locks the steps already done.
        let both = node(&node(&done, "b", Working).unwrap().unwrap(), "b", Done)
            .unwrap()
            .unwrap();
        let back = node(&both, "a", Working).unwrap().unwrap();
        assert_eq!(
            node(&back, "a", Done).unwrap().unwrap().steps[0].status,
            Done
        );
    }

    #[test]
    fn a_pr_or_file_step_is_done_only_with_its_evidence_and_a_none_step_without() {
        use StepStatus::*;
        let plan = set(
            None,
            vec![
                output(step("spec", &[], Working), Output::File),
                output(step("pr", &[], Working), Output::Pr),
                output(step("check", &[], Working), Output::None),
                step("free", &[], Working),
            ],
            false,
        )
        .unwrap();
        assert_eq!(
            node(&plan, "pr", Done).unwrap_err(),
            crate::coordination::refusal("plan_evidence_missing", "pr --pr")
        );
        assert_eq!(
            node(&plan, "spec", Done).unwrap_err(),
            crate::coordination::refusal("plan_evidence_missing", "spec --file")
        );
        // A file is not a pull request's evidence.
        let wrong = Some(Evidence::File {
            path: "a.md".into(),
        });
        assert_eq!(
            node_with(&plan, "pr", Done, wrong, &Proofs::default()).unwrap_err(),
            crate::coordination::refusal("plan_evidence_missing", "pr --pr")
        );
        assert!(node(&plan, "check", Done).unwrap().is_some());
        assert!(node(&plan, "free", Done).unwrap().is_some());
        // The same refusal on set, which keeps the plan as it was.
        assert_eq!(
            set(
                Some(&plan),
                vec![output(step("pr", &[], Done), Output::Pr)],
                false
            )
            .unwrap_err(),
            crate::coordination::refusal("plan_evidence_missing", "pr --pr")
        );
    }

    #[test]
    fn a_pull_request_is_checked_off_the_lock_and_kept_with_its_repository() {
        use StepStatus::*;
        let plan = set(
            None,
            vec![output(step("pr", &[], Working), Output::Pr)],
            false,
        )
        .unwrap();
        let claim = || {
            Some(Evidence::Pr {
                repo: String::new(),
                number: 12,
            })
        };
        // First the read is asked for, and nothing is kept.
        let asks = node_with(&plan, "pr", Done, claim(), &Proofs::default())
            .unwrap()
            .unwrap_err();
        assert_eq!(asks.prs, BTreeSet::from([12]));
        let mut proofs = Proofs::default();
        proofs.prs.insert(12, PrProof::NotYours);
        assert_eq!(
            node_with(&plan, "pr", Done, claim(), &proofs).unwrap_err(),
            crate::coordination::refusal("plan_pr_not_yours", "pr #12")
        );
        proofs.prs.insert(12, PrProof::Unconfirmed);
        assert_eq!(
            node_with(&plan, "pr", Done, claim(), &proofs).unwrap_err(),
            crate::coordination::refusal("plan_pr_unconfirmed", "pr #12")
        );
        let done = node_with(&plan, "pr", Done, claim(), &yours(12))
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            done.steps[0].evidence,
            Some(Evidence::Pr {
                repo: "acme/app".into(),
                number: 12
            })
        );
        // Asked again with the same pull request, nothing is read again.
        assert_eq!(
            node_with(&done, "pr", Done, claim(), &Proofs::default()).unwrap(),
            Ok(None)
        );
    }

    #[test]
    fn a_file_is_kept_only_when_its_machine_found_it_inside_the_checkout() {
        use StepStatus::*;
        let plan = set(
            None,
            vec![output(step("spec", &[], Working), Output::File)],
            false,
        )
        .unwrap();
        let claim = || {
            Some(Evidence::File {
                path: "docs/spec.md".into(),
            })
        };
        for (proof, code) in [
            (FileProof::Missing, "plan_file_missing"),
            (FileProof::Outside, "plan_file_outside"),
            (FileProof::Unconfirmed, "plan_file_unconfirmed"),
        ] {
            let mut proofs = Proofs::default();
            proofs.files.insert("docs/spec.md".into(), proof);
            assert_eq!(
                node_with(&plan, "spec", Done, claim(), &proofs).unwrap_err(),
                crate::coordination::refusal(code, "docs/spec.md")
            );
        }
        let mut proofs = Proofs::default();
        proofs
            .files
            .insert("docs/spec.md".into(), FileProof::Present);
        let done = node_with(&plan, "spec", Done, claim(), &proofs)
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(done.steps[0].evidence, claim());
    }

    #[test]
    fn a_file_path_is_read_below_the_checkout_and_never_out_of_it() {
        let root = "/work/app";
        assert_eq!(
            file_below(root, "docs/spec.md").as_deref(),
            Some("docs/spec.md")
        );
        assert_eq!(
            file_below(root, "./docs/../spec.md").as_deref(),
            Some("spec.md")
        );
        assert_eq!(
            file_below(root, "/work/app/docs/spec.md").as_deref(),
            Some("docs/spec.md")
        );
        assert_eq!(file_below(root, "../other/spec.md"), None);
        assert_eq!(file_below(root, "/work/app-other/spec.md"), None);
        assert_eq!(file_below(root, "/work/app"), None);
        assert_eq!(
            file_below("C:/work/app", "C:/work/app/a.md").as_deref(),
            Some("a.md")
        );
    }

    #[test]
    fn a_handed_step_is_done_only_after_its_current_child_reported() {
        use StepStatus::*;
        let mut plan = set(None, vec![step("web", &[], Planned)], false).unwrap();
        handed(&mut plan, "web", DelegationState::Working, false);
        assert_eq!(
            node(&plan, "web", Done).unwrap_err(),
            crate::coordination::refusal("plan_report_missing", "web")
        );
        handed(&mut plan, "web", DelegationState::Reported, true);
        let done = node(&plan, "web", Done).unwrap().unwrap();
        // The child stays named on the finished step (D-32).
        assert!(done.steps[0].delegation.as_ref().unwrap().active());
        // A pr step handed to a child also needs the child's pull request.
        let mut pr = set(
            None,
            vec![output(step("pr", &[], Planned), Output::Pr)],
            false,
        )
        .unwrap();
        handed(&mut pr, "pr", DelegationState::Reported, true);
        assert_eq!(
            node(&pr, "pr", Done).unwrap_err(),
            crate::coordination::refusal("plan_evidence_missing", "pr --pr")
        );
    }

    #[test]
    fn taking_a_blocked_or_closed_step_back_makes_it_the_mains_own() {
        use StepStatus::*;
        let mut plan = set(None, vec![step("web", &[], Planned)], false).unwrap();
        handed(&mut plan, "web", DelegationState::Working, false);
        // While the child works, working again is no hand-back.
        assert_eq!(node(&plan, "web", Working).unwrap(), None);
        handed(&mut plan, "web", DelegationState::Blocked, false);
        let mine = node(&plan, "web", Working).unwrap().unwrap();
        assert!(!mine.steps[0].delegation.as_ref().unwrap().active());
        assert!(view_of(&mine, "p").steps[0].delegation.is_none());
        // Done by the main itself needs no report.
        assert!(node(&mine, "web", Done).unwrap().is_some());
        handed(&mut plan, "web", DelegationState::Working, false);
        let dropped = node(&plan, "web", Dropped).unwrap().unwrap();
        assert!(!dropped.steps[0].delegation.as_ref().unwrap().active());
    }

    #[test]
    fn a_replan_keeps_evidence_and_hand_offs_and_a_step_done_before_evidence_stays_done() {
        use StepStatus::*;
        assert_eq!(
            set_with(
                None,
                vec![
                    output(step("spec", &[], Done), Output::File),
                    step("web", &["spec"], Planned),
                ],
                &Proofs::default(),
            )
            .unwrap_err(),
            crate::coordination::refusal("plan_evidence_missing", "spec --file")
        );
        // A plan from before evidence: its done step carries none (B15).
        let mut old = set(
            None,
            vec![step("spec", &[], Done), step("web", &["spec"], Planned)],
            false,
        )
        .unwrap();
        old.steps[0].output = Some(Output::File);
        handed(&mut old, "web", DelegationState::Working, false);
        let mut spec = output(step("spec", &[], Done), Output::File);
        spec.title = "spec title".into();
        let replanned = set(
            Some(&old),
            vec![
                spec,
                step("web", &["spec"], Working),
                step("ship", &["web"], Planned),
            ],
            false,
        )
        .unwrap();
        assert_eq!(replanned.steps[0].status, Done);
        assert_eq!(replanned.steps[0].evidence, None);
        assert!(replanned.steps[1].delegation.as_ref().unwrap().active());
        // A set naming the pull request of a step done with one keeps it.
        let pr = set(
            None,
            vec![output(step("pr", &[], Working), Output::Pr)],
            false,
        )
        .unwrap();
        let mut done_pr = output(step("pr", &[], Done), Output::Pr);
        done_pr.pr = Some(7);
        let done = set_with(
            Some(&pr),
            vec![done_pr.clone(), step("next", &["pr"], Planned)],
            &yours(7),
        )
        .unwrap();
        let mut again = done_pr;
        again.pr = None;
        let kept = set(
            Some(&done),
            vec![again, step("next", &["pr"], Working)],
            false,
        )
        .unwrap();
        assert_eq!(
            kept.steps[0].evidence,
            Some(Evidence::Pr {
                repo: "acme/app".into(),
                number: 7
            })
        );
    }

    #[test]
    fn a_plan_that_replaces_a_finished_one_has_its_own_id() {
        use StepStatus::*;
        let done = set(None, vec![step("a", &[], Done)], false).unwrap();
        let mut next = apply_set(
            Some(&done),
            &session(),
            &PlanInput {
                nodes: vec![step("b", &[], Planned)],
            },
            false,
            None,
            None,
            done.created_at_unix_ms,
        )
        .unwrap();
        judge(None, &mut next, &Proofs::default()).unwrap();
        assert_ne!(next.id(), done.id());
    }

    /// A session's checkout on branch `main-branch`, where pull request 1 is
    /// from it, 2 from a child's branch, 3 from another branch, 4 from a
    /// fork, 5 is not there and 6 cannot be asked about; `docs/spec.md` is
    /// there, `docs/link.md` leads out of the checkout.
    struct Checkout;

    impl crate::node_access::NodeLink for Checkout {
        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            _timeout: std::time::Duration,
        ) -> Result<crate::node_access::LinkAnswer, hide_node_link::LinkError> {
            use hide_node_link::catalog::{PathFact, PathFacts, RepositoryPlace};
            use hide_node_link::gh::{GhAnswer, GithubFailureCategory};
            use hide_node_link::protocol::Call;
            let answer = match call {
                Call::PathFacts { paths } => {
                    let fact = |path: &str| {
                        let (comparison, exists) = match path {
                            "/work/app" => ("/work/app", true),
                            "/work/app/docs/spec.md" => ("/work/app/docs/spec.md", true),
                            "/work/app/docs/link.md" => ("/etc/passwd", true),
                            other => (other, false),
                        };
                        PathFact {
                            comparison: comparison.into(),
                            exists,
                            project: None,
                            project_unread: None,
                            repository: (path == "/work/app").then(|| RepositoryPlace {
                                root: "/work/app".into(),
                                main_root: "/work/app".into(),
                                branch: Some("main-branch".into()),
                                head_oid: None,
                            }),
                        }
                    };
                    serde_json::to_value(PathFacts {
                        paths: paths
                            .iter()
                            .map(|path| (path.clone(), fact(path)))
                            .collect(),
                        ..PathFacts::default()
                    })
                    .unwrap()
                }
                Call::Gh { args, .. } => {
                    let head = |branch: &str, fork: bool| GhAnswer::Output {
                        stdout: json!({
                            "headRefName": branch, "isCrossRepository": fork,
                            "url": "https://github.com/acme/app/pull/1",
                        })
                        .to_string(),
                    };
                    let failed = |reason: &str| GhAnswer::Failed {
                        category: GithubFailureCategory::NetworkOrRateLimit,
                        reason: reason.into(),
                    };
                    serde_json::to_value(match args[2].as_str() {
                        "1" => head("main-branch", false),
                        "2" => head("child-branch", false),
                        "3" => head("someone-else", false),
                        "4" => head("main-branch", true),
                        "5" => failed(
                            "GraphQL: Could not resolve to a PullRequest with the number of 5.",
                        ),
                        _ => failed("error connecting to api.github.com"),
                    })
                    .unwrap()
                }
                other => panic!("not asked: {other:?}"),
            };
            Ok(answer.into())
        }
    }

    #[test]
    fn a_pull_request_is_the_sessions_by_its_branch_and_a_file_by_where_its_machine_finds_it() {
        let proof = PlanProof {
            link: Arc::new(Checkout),
            root: "/work/app".into(),
            branches: BTreeSet::from(["child-branch".to_owned()]),
            asks: Asks {
                prs: BTreeSet::from([1, 2, 3, 4, 5, 6]),
                files: BTreeSet::from([
                    "docs/spec.md".to_owned(),
                    "docs/gone.md".to_owned(),
                    "docs/link.md".to_owned(),
                ]),
            },
            plan: "1000".into(),
            session_id: "s1".into(),
        };
        let mut proofs = Proofs::default();
        proof.run(&mut proofs);
        let yours = PrProof::Yours {
            repo: "acme/app".into(),
        };
        assert_eq!(proofs.prs[&1], yours, "made from the session's branch");
        assert_eq!(proofs.prs[&2], yours, "made from a child's branch");
        assert_eq!(proofs.prs[&3], PrProof::NotYours);
        assert_eq!(
            proofs.prs[&4],
            PrProof::NotYours,
            "a fork's branch of the same name"
        );
        assert_eq!(proofs.prs[&5], PrProof::NotYours, "no such pull request");
        assert_eq!(proofs.prs[&6], PrProof::Unconfirmed);
        assert_eq!(proofs.files["docs/spec.md"], FileProof::Present);
        assert_eq!(proofs.files["docs/gone.md"], FileProof::Missing);
        assert_eq!(proofs.files["docs/link.md"], FileProof::Outside);
    }

    #[test]
    fn a_first_plan_is_approved_unless_it_asks_and_a_replan_keeps_the_review() {
        use StepStatus::*;
        let first = set(None, vec![step("a", &[], Planned)], false).unwrap();
        assert_eq!(first.review, Review::Approved);
        let asked = set(None, vec![step("a", &[], Planned)], true).unwrap();
        assert_eq!(asked.review, Review::Proposed);
        let replanned = set(Some(&asked), vec![step("a", &[], Working)], false).unwrap();
        assert_eq!(replanned.review, Review::Proposed);
        assert!(!replanned.steps[0].changed);
    }

    #[test]
    fn a_replan_marks_new_renamed_and_left_out_steps_with_the_reason() {
        use StepStatus::*;
        let first = set(
            None,
            vec![
                step("a", &[], Done),
                step("b", &["a"], Planned),
                step("c", &["b"], Planned),
            ],
            false,
        )
        .unwrap();
        let mut renamed = step("b", &["a"], Working);
        renamed.title = "b renamed".into();
        let next = set(
            Some(&first),
            vec![step("a", &[], Done), renamed, step("d", &["b"], Planned)],
            false,
        )
        .unwrap();
        let marks: Vec<(&str, bool, Option<&str>, StepStatus)> = next
            .steps
            .iter()
            .map(|step| {
                (
                    step.key.as_str(),
                    step.changed,
                    step.change.as_deref(),
                    step.status,
                )
            })
            .collect();
        assert_eq!(
            marks,
            [
                ("a", false, None, Done),
                ("b", true, Some("why"), Working),
                ("d", true, Some("why"), Planned),
                ("c", true, Some("why"), Dropped),
            ]
        );
        assert_eq!(next.review, Review::Approved);
    }

    #[test]
    fn a_replan_that_leaves_out_finished_steps_keeps_them_done_and_the_count() {
        use StepStatus::*;
        let first = set(
            None,
            vec![
                step("a", &[], Done),
                step("b", &["a"], Done),
                step("c", &["b"], Planned),
            ],
            false,
        )
        .unwrap();
        // The agent sends only what is left, with its first step not done.
        let next = set(Some(&first), vec![step("c", &[], Working)], false).unwrap();
        let kept: Vec<(&str, StepStatus, bool)> = next
            .steps
            .iter()
            .map(|step| (step.key.as_str(), step.status, step.changed))
            .collect();
        assert_eq!(
            kept,
            [("c", Working, false), ("a", Done, true), ("b", Done, true)]
        );
        let count = progress(&next);
        assert_eq!((count.done, count.total), (2, 3));
    }

    #[test]
    fn a_finished_plan_is_replaced_whole_by_the_next_set() {
        use StepStatus::*;
        let done = set(None, vec![step("a", &[], Done)], false).unwrap();
        let next = set(Some(&done), vec![step("b", &[], Planned)], false).unwrap();
        assert_eq!(next.steps.len(), 1);
        assert!(!next.steps[0].changed);
    }

    #[test]
    fn a_step_done_before_what_it_waits_on_is_refused_and_a_dropped_one_holds_nothing() {
        use StepStatus::*;
        assert_eq!(
            set(
                None,
                vec![step("a", &[], Planned), step("b", &["a"], Done)],
                false
            )
            .unwrap_err(),
            crate::coordination::refusal("plan_order_refused", "b waits on a")
        );
        let plan = set(
            None,
            vec![step("a", &[], Planned), step("b", &["a"], Planned)],
            false,
        )
        .unwrap();
        assert_eq!(
            node(&plan, "b", Done).unwrap_err(),
            crate::coordination::refusal("plan_order_refused", "b waits on a")
        );
        let dropped = node(&plan, "a", Dropped).unwrap().unwrap();
        let done = node(&dropped, "b", Done).unwrap().unwrap();
        assert_eq!(node(&done, "b", Done).unwrap(), None);
    }

    #[test]
    fn an_invalid_plan_says_where() {
        use StepStatus::*;
        assert_eq!(
            set(None, vec![step("a", &["z"], Planned)], false).unwrap_err(),
            crate::coordination::refusal("plan_invalid", "a: depends_on \"z\" is no other step")
        );
        assert_eq!(
            set(
                None,
                vec![step("a", &["b"], Planned), step("b", &["a"], Planned)],
                false
            )
            .unwrap_err(),
            crate::coordination::refusal("plan_invalid", "a: depends_on loops")
        );
        let many = (0..65)
            .map(|n| step(&format!("s{n}"), &[], Planned))
            .collect();
        assert_eq!(
            set(None, many, false).unwrap_err(),
            crate::coordination::refusal("plan_too_large", "64")
        );
    }

    #[test]
    fn the_count_leaves_dropped_steps_out_and_the_current_step_is_the_topmost_working_one() {
        use StepStatus::*;
        let plan = set(
            None,
            vec![
                step("spec", &[], Done),
                step("core", &["spec"], Planned),
                step("web", &["spec"], Working),
                step("docs", &["spec"], Dropped),
                step("ship", &["core", "web"], Planned),
            ],
            false,
        )
        .unwrap();
        assert_eq!(
            progress(&plan),
            Progress {
                current: Some("web".into()),
                next: Some("core".into()),
                done: 1,
                total: 4,
                finished: false,
            }
        );
    }
}
