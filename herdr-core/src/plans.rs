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

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::dag::{self, NODE_LIMIT};
use crate::links::ledger::SessionIdentity;

/// Plans kept at once; past it the plan updated longest ago whose session
/// runs in no pane is let go with a diagnostic, as the session record lets
/// its oldest entries go past its own caps (engineering #15).
pub const PLAN_LIMIT: usize = 128;
/// Steps one step may wait on (D-23).
pub const DEPENDENCY_LIMIT: usize = 8;
const TITLE_LIMIT: usize = 300;
const LINE_LIMIT: usize = 600;
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
            Self::Set {
                plan, why, reason, ..
            } => {
                plan.nodes.len() <= NODE_LIMIT
                    && valid_line(why.as_deref(), LINE_LIMIT)
                    && valid_line(reason.as_deref(), LINE_LIMIT)
            }
            Self::Node { key, status, line } => {
                dag::valid_key(key)
                    && (status.is_some() || line.is_some())
                    && valid_line(line.as_deref(), LINE_LIMIT)
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
/// it is refused (B1, B6, D-07).
///
/// A plan whose steps are all finished is replaced whole, so new work
/// starts a new plan. Otherwise a step the new list leaves out is kept as
/// dropped, and each step whose title is new or changed is marked with the
/// reason. The review stays as it was unless `ask` puts the plan up again;
/// a first plan is approved unless it asks.
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
    let previous = previous.filter(|plan| !finished(plan));
    let change = why.or(reason).map(str::to_owned);
    let mut steps: Vec<Step> = input
        .nodes
        .iter()
        .map(|input| {
            let before =
                previous.and_then(|plan| plan.steps.iter().find(|step| step.key == input.key));
            let changed = previous.is_some() && before.is_none_or(|step| step.title != input.title);
            Step {
                key: input.key.clone(),
                title: input.title.clone(),
                depends_on: input.depends_on.clone(),
                status: input.status,
                line: input.line.clone(),
                output: input.output,
                changed,
                change: if changed { change.clone() } else { None },
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
            steps.push(Step {
                status: StepStatus::Dropped,
                // What it waited on may be gone too; a dropped step holds
                // nothing up and keeps no edge to a step that is not there.
                depends_on: gone
                    .depends_on
                    .iter()
                    .filter(|key| keys.contains(key.as_str()))
                    .cloned()
                    .collect(),
                changed: true,
                change: change.clone(),
                ..gone.clone()
            });
        }
    }
    if let Some((step, blocked)) = steps
        .iter()
        .filter(|step| step.status == StepStatus::Done)
        .find_map(|step| blocker(&steps, step).map(|key| (step, key)))
    {
        return Err(refused(
            "plan_order_refused",
            format!("{} waits on {blocked}", step.key),
        ));
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
        created_at_unix_ms: previous.map_or(now, |plan| plan.created_at_unix_ms),
        updated_at_unix_ms: now,
    })
}

/// `plan` with step `key` moved to `status` and its line set, or why it is
/// refused (B2). The same command again changes nothing (`None`).
pub(crate) fn apply_node(
    plan: &Plan,
    key: &str,
    status: Option<StepStatus>,
    line: Option<&str>,
    now: u64,
) -> Result<Option<Plan>, String> {
    let index = plan
        .steps
        .iter()
        .position(|step| step.key == key)
        .ok_or_else(|| refused("plan_node_unknown", key))?;
    let step = &plan.steps[index];
    if status == Some(StepStatus::Done)
        && step.status != StepStatus::Done
        && let Some(blocked) = blocker(&plan.steps, step)
    {
        return Err(refused(
            "plan_order_refused",
            format!("{key} waits on {blocked}"),
        ));
    }
    let status = status.unwrap_or(step.status);
    let line = line.map(str::to_owned).or_else(|| step.line.clone());
    if status == step.status && line == step.line {
        return Ok(None);
    }
    let mut next = plan.clone();
    next.steps[index].status = status;
    next.steps[index].line = line;
    next.updated_at_unix_ms = now;
    Ok(Some(next))
}

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
    #[serde(flatten)]
    pub step: Step,
    /// A step it waits on is neither done nor dropped (B7's dashed card).
    pub waiting: bool,
}

/// What every surface draws of `plan`: the map, the chip and a mission
/// node's own plan read this one value (D-10, D-18).
pub fn view_of(plan: &Plan, pane_id: &str) -> PlanView {
    let steps = plan
        .steps
        .iter()
        .map(|step| StepView {
            waiting: step.status != StepStatus::Done
                && step.status != StepStatus::Dropped
                && step.depends_on.iter().any(|key| {
                    plan.steps.iter().any(|other| {
                        other.key == *key
                            && !matches!(other.status, StepStatus::Done | StepStatus::Dropped)
                    })
                }),
            step: step.clone(),
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
        }
    }

    fn set(previous: Option<&Plan>, nodes: Vec<StepInput>, ask: bool) -> Result<Plan, String> {
        apply_set(
            previous,
            &session(),
            &PlanInput { nodes },
            ask,
            None,
            Some("why"),
            10,
        )
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
            apply_node(&plan, "b", Some(Done), None, 20).unwrap_err(),
            crate::coordination::refusal("plan_order_refused", "b waits on a")
        );
        let dropped = apply_node(&plan, "a", Some(Dropped), None, 20)
            .unwrap()
            .unwrap();
        let done = apply_node(&dropped, "b", Some(Done), None, 30)
            .unwrap()
            .unwrap();
        assert_eq!(apply_node(&done, "b", Some(Done), None, 40).unwrap(), None);
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
