//! The runtime half of a node's finish lines (PRD mission-map-git-criteria
//! D-12, D-13, D-16, D-21, D-23): who may record which line, the commit a
//! check command is compared with, the gate on a node's done and on a
//! mission's close, and the CI GitHub reports for a node's pull request.
//!
//! Everything here runs under the lock and reads only what the core holds:
//! the ledger's registrations, the catalog's checkouts and the last GitHub
//! answer. Nothing asks a machine or GitHub.

use serde_json::{Value, json};

use super::Runtime;
use crate::delivery::Actor;
use crate::mission_criteria::{CheckKind, CriterionKind, Recorded, Role};
use crate::mission_events::{EventKind, MissionEvent};
use crate::missions::{
    DECIDED_LIMIT, Decided, Mission, MissionEnd, MissionNode, MissionStatus, NodeKind, NodePatch,
    NodeStatus,
};
use crate::model::{
    CheckoutSnapshot, GithubProjectSnapshot, PullRequestBadge, PullRequestChecks,
    PullRequestSnapshot,
};

/// Whether `pull` is merged, so its head is the work that reached main.
fn merged(pull: &PullRequestSnapshot) -> bool {
    pull.badge == PullRequestBadge::Merged
}

impl Runtime {
    /// The pull request list of the mission's project as the last GitHub
    /// read that was not stale has it.
    pub(super) fn mission_pulls(&self, mission: &Mission) -> Option<&GithubProjectSnapshot> {
        let github = if mission.device_id == self.node.as_str() {
            Some(&self.github)
        } else {
            self.device_github.get(&mission.device_id)
        };
        github?
            .project(&mission.project)
            .filter(|project| !project.status.stale && project.pull_requests_read)
    }

    /// The commit the node's work folder is on now (B7, D-16): the head the
    /// catalog reads for the checkout its check command ran in, whoever
    /// works the node now, or, once its pull request is merged, that pull
    /// request's head, which is the work that reached main. None when Hide
    /// cannot read it, and then no check passes.
    fn node_head(
        &self,
        mission: &Mission,
        node: &MissionNode,
        pulls: Option<&GithubProjectSnapshot>,
    ) -> Option<String> {
        if let Some(pull) = node.pr.and_then(|number| pull_in(pulls, number))
            && merged(pull)
        {
            return pull.head_oid.clone();
        }
        let folder = node.folder.as_deref()?;
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == mission.device_id)
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| checkout.path == folder)
            .and_then(|checkout| checkout.head_sha().map(str::to_owned))
    }

    /// The checkout whose tabs hold `pane` on `machine`.
    fn pane_checkout(&self, machine: &str, pane: &str) -> Option<&CheckoutSnapshot> {
        let raw = |id: &str| {
            id.rsplit_once(":pane:")
                .map_or(id, |(_, raw)| raw)
                .to_owned()
        };
        let pane = raw(pane);
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == machine)
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| {
                checkout
                    .tabs
                    .iter()
                    .any(|tab| tab.panes.iter().any(|row| raw(&row.id) == pane))
            })
    }

    /// The keys of the lines that keep `node` from done (B9), with the
    /// mission's last pull request list. A pull request whose CI GitHub has
    /// not reported is not known to pass, so it counts as `ci` until a read
    /// says it has none.
    pub(super) fn node_unmet(&self, mission: &Mission, node: &MissionNode) -> Vec<String> {
        self.node_unmet_in(mission, node, self.mission_pulls(mission))
    }

    /// `node_unmet` against `pulls`, a list GitHub may have answered just
    /// now.
    fn node_unmet_in(
        &self,
        mission: &Mission,
        node: &MissionNode,
        pulls: Option<&GithubProjectSnapshot>,
    ) -> Vec<String> {
        let head = self.node_head(mission, node, pulls);
        let mut out: Vec<String> = crate::mission_criteria::unmet(&node.criteria, head.as_deref())
            .into_iter()
            .map(str::to_owned)
            .collect();
        let has_ci = node
            .criteria
            .iter()
            .any(|line| line.kind == CriterionKind::Ci);
        if let Some(number) = node.pr
            && !has_ci
        {
            let known = pull_in(pulls, number).is_some_and(|pull| {
                pull.checks == PullRequestChecks::None
                    // A merged pull request's checks are no longer read.
                    || (merged(pull) && pull.checks == PullRequestChecks::Unknown)
            });
            if !known {
                out.push("ci".into());
            }
        }
        out
    }

    /// The node's view of each line now: a check command passed for another
    /// commit, or for a folder Hide cannot read, shows open (B7, B32).
    pub(super) fn shown_criteria(
        &self,
        mission: &Mission,
        node: &MissionNode,
    ) -> Vec<crate::mission_criteria::Criterion> {
        let head = self.node_head(mission, node, self.mission_pulls(mission));
        node.criteria
            .iter()
            .map(|line| {
                let mut line = line.clone();
                line.state = line.shown(head.as_deref());
                line
            })
            .collect()
    }

    /// Whether the gate holds `node` (D-12): a mission made before criteria
    /// and an external node, which Hide finishes itself, are not held.
    fn gated(mission: &Mission, node: &MissionNode) -> bool {
        !mission.exempt && node.kind != NodeKind::External
    }

    /// The gate on `node` as a command would leave it (D-12, B9): done only
    /// with every line met, a node already done included, against the pull
    /// request and worker the same command names.
    pub(super) fn gate_node_done(
        &self,
        mission: &Mission,
        node: &MissionNode,
    ) -> Result<(), String> {
        if node.status != NodeStatus::Done || !Self::gated(mission, node) {
            return Ok(());
        }
        let unmet = self.node_unmet(mission, node);
        if unmet.is_empty() {
            return Ok(());
        }
        Err(crate::coordination::refusal(
            "mission_criteria_unmet",
            &format!("{}: {}", node.key, unmet.join(", ")),
        ))
    }

    /// The lines that keep each done node of `mission` from being done
    /// now, against `pulls` (B7, B9): a new commit, a changed check command
    /// or a checker's new verdict after the node was done opens it again.
    fn done_unmet(&self, mission: &Mission, pulls: Option<&GithubProjectSnapshot>) -> Vec<String> {
        mission
            .nodes
            .iter()
            .filter(|node| node.status == NodeStatus::Done && Self::gated(mission, node))
            .filter_map(|node| {
                let unmet = self.node_unmet_in(mission, node, pulls);
                (!unmet.is_empty()).then(|| format!("{}: {}", node.key, unmet.join(", ")))
            })
            .collect()
    }

    /// What keeps `mission` from ending as done now, against `pulls` (B10):
    /// a node neither done nor dropped, or a done node whose lines no longer
    /// hold. The Conductor's close as done or merge_wait and both closes a
    /// waiting mission makes by itself, on its pull requests merged (B41) or
    /// its branches in main (B18), ask this one rule; a mission made before
    /// criteria is not held (D-19).
    pub(super) fn close_held(
        &self,
        mission: &Mission,
        pulls: Option<&GithubProjectSnapshot>,
    ) -> Option<String> {
        if mission.exempt {
            return None;
        }
        let open: Vec<&str> = mission
            .nodes
            .iter()
            .filter(|node| !matches!(node.status, NodeStatus::Done | NodeStatus::Dropped))
            .map(|node| node.key.as_str())
            .collect();
        if !open.is_empty() {
            return Some(crate::coordination::refusal(
                "mission_nodes_unfinished",
                &open.join(", "),
            ));
        }
        let unmet = self.done_unmet(mission, pulls);
        (!unmet.is_empty())
            .then(|| crate::coordination::refusal("mission_criteria_unmet", &unmet.join("; ")))
    }

    /// The gate on the Conductor's close: a cancel is never held.
    pub(super) fn gate_close(&self, mission: &Mission, end: MissionEnd) -> Result<(), String> {
        if end == MissionEnd::Cancelled {
            return Ok(());
        }
        self.close_held(mission, self.mission_pulls(mission))
            .map_or(Ok(()), Err)
    }

    /// The node `patch` would leave, with the lines that rest on what it
    /// changes made current (B7, B8): a pull request named now takes its CI
    /// from the last list, and another worker runs the check command again.
    pub(super) fn prospective_node(
        &self,
        mission: &Mission,
        node: &MissionNode,
        patch: &NodePatch,
        now: u64,
    ) -> Result<MissionNode, String> {
        let mut next = crate::missions::apply_patch(node, patch, now)?;
        if mission.exempt {
            return Ok(next);
        }
        if next.pr != node.pr {
            next.criteria.retain(|line| line.kind != CriterionKind::Ci);
            if let Some(project) = self.mission_pulls(mission) {
                read_node_ci(&mut next, project, now);
            }
        }
        if let (Some(before), Some(after)) = (&node.agent_id, &next.agent_id)
            && before != after
        {
            next.folder = None;
            for line in &mut next.criteria {
                if line.kind == CriterionKind::Check && line.skip.is_none() {
                    *line = crate::mission_criteria::Criterion::new(
                        line.key.clone(),
                        CriterionKind::Check,
                    );
                }
            }
        }
        Ok(next)
    }

    /// Whether registration `id` is `actor`'s.
    fn registration_is(&self, id: Option<&str>, actor: &Actor) -> bool {
        let Some(id) = id else { return false };
        self.delivery_state().is_ok_and(|ledger| {
            ledger
                .agents
                .iter()
                .any(|record| record.id == id && record.actor.same_identity(actor))
        })
    }

    /// The mission a line command from `actor` acts on (B5): the one it
    /// names, or the open mission it conducts or whose node `node` it works
    /// or checks.
    fn line_mission(
        &self,
        actor: &Actor,
        named: Option<&str>,
        node: &str,
    ) -> Result<String, String> {
        let concerns = |mission: &Mission| {
            mission.conductor.actor.same_identity(actor)
                || mission.node(node).is_some_and(|row| {
                    self.registration_is(row.agent_id.as_deref(), actor)
                        || row.criteria.iter().any(|line| {
                            line.checkers.iter().any(|checker| {
                                self.registration_is(checker.agent_id.as_deref(), actor)
                            })
                        })
                })
        };
        let open = || {
            self.missions
                .book
                .missions
                .iter()
                .filter(|mission| mission.status.open())
        };
        let mission = match named {
            Some(id) => self.missions.book.get(id).ok_or("mission_unknown")?,
            None => match open().find(|mission| concerns(mission)) {
                Some(mission) => mission,
                // A node another agent owns is someone else's line.
                None if open().any(|mission| mission.node(node).is_some()) => {
                    return Err("mission_criterion_not_yours".into());
                }
                None => return Err("mission_none".into()),
            },
        };
        match mission.status {
            MissionStatus::Cancelled => return Err("mission_cancelled".into()),
            MissionStatus::Done => return Err("mission_closed".into()),
            _ => {}
        }
        if !concerns(mission) {
            return Err("mission_criterion_not_yours".into());
        }
        Ok(mission.id.clone())
    }

    /// `hide mission record` (B5-B7, B13): the owner's result for one line.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_criterion(
        &mut self,
        actor: &Actor,
        named: Option<&str>,
        node: &str,
        key: &str,
        result: (bool, String, Option<String>, bool),
        now: u64,
    ) -> Result<(Value, bool), String> {
        let (pass, evidence, commit, rerun) = result;
        let evidence = evidence.trim().to_owned();
        if evidence.is_empty() {
            return Err("mission_evidence_missing".into());
        }
        let evidence = Some(evidence);
        let id = self.line_mission(actor, named, node)?;
        let mission = self.missions.book.get(&id).ok_or("mission_unknown")?;
        let index = mission
            .nodes
            .iter()
            .position(|row| row.key == node)
            .ok_or("mission_node_unknown")?;
        let row = &mission.nodes[index];
        let at = row
            .criteria
            .iter()
            .position(|line| line.key == key)
            .ok_or("mission_criterion_unknown")?;
        let line = &row.criteria[at];
        let conductor = mission.conductor.actor.same_identity(actor);
        let worker = self.registration_is(row.agent_id.as_deref(), actor);
        // The Conductor certifies a node it works itself: one that never
        // had a worker (D-23 as the mission's Conductor read it).
        let never_worked = row.agent_id.is_none()
            && !row
                .criteria
                .iter()
                .any(|line| line.by.as_ref().is_some_and(|by| by.role == Role::Worker));
        let by = |role: Role, agent_id: Option<String>| Recorded {
            role,
            name: actor.name.clone(),
            agent_id,
            at_unix_ms: now,
        };
        let mut next = row.clone();
        let effect = match line.kind {
            CriterionKind::Review | CriterionKind::Verify => {
                let slot = line
                    .checkers
                    .iter()
                    .position(|checker| self.registration_is(checker.agent_id.as_deref(), actor))
                    .ok_or("mission_criterion_not_yours")?;
                let agent_id = line.checkers[slot].agent_id.clone();
                let target = &mut next.criteria[at];
                let changed = target.record_checker(slot, pass, evidence, now)?;
                if changed {
                    target.by = Some(by(Role::Checker, agent_id));
                }
                crate::mission_criteria::Effect {
                    changed,
                    spent: false,
                }
            }
            CriterionKind::Ci => return Err("mission_criterion_not_yours".into()),
            CriterionKind::Own | CriterionKind::Check | CriterionKind::Defect => {
                let recorder = if worker {
                    by(Role::Worker, row.agent_id.clone())
                } else if conductor && never_worked {
                    by(Role::Conductor, mission.conductor.agent_id.clone())
                } else {
                    return Err("mission_criterion_not_yours".into());
                };
                if line.kind == CriterionKind::Check {
                    // The check is the folder it ran in: its head later is
                    // what the line is held against (D-16).
                    let folder = self
                        .pane_checkout(&actor.device_id, &actor.pane_id)
                        .filter(|_| actor.device_id == mission.device_id)
                        .map(|checkout| checkout.path.clone())
                        .ok_or("mission_checkout_unknown")?;
                    let moved = next.folder.as_deref() != Some(folder.as_str());
                    next.folder = Some(folder);
                    let mut effect =
                        next.criteria[at].record(pass, evidence, commit, rerun, recorder)?;
                    effect.changed |= moved;
                    effect
                } else {
                    next.criteria[at].record(pass, evidence, commit, rerun, recorder)?
                }
            }
        };
        if !effect.changed {
            return Ok((json!({"mission": self.mission_view_of(&id)}), false));
        }
        let mut events = Vec::new();
        // A check failing again on the commit its rerun was for sends the
        // node back to work (B13).
        if effect.spent && back_to_work(next.status) {
            let before = next.clone();
            next = crate::missions::apply_patch(
                &next,
                &NodePatch {
                    status: Some(NodeStatus::Working),
                    ..NodePatch::default()
                },
                now,
            )?;
            events.extend(super::mission_events::node_events(&before, &next, now));
        }
        let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
        target.nodes[index] = next;
        target.updated_at_unix_ms = now;
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.criterion_recorded",
            "mission": id, "node": node, "line": key, "pass": pass,
        }));
        self.save_missions();
        self.record_mission_events(&id, events);
        Ok((json!({"mission": self.mission_view_of(&id)}), true))
    }

    /// `hide mission skip` (B11, B12): the Conductor goes past a line with
    /// its reason; the check command and the CI only on an answered
    /// decision that named the node.
    pub(super) fn skip_criterion(
        &mut self,
        actor: &Actor,
        named: Option<&str>,
        node: &str,
        key: &str,
        (reason, decision): (String, Option<u64>),
        now: u64,
    ) -> Result<(Value, bool), String> {
        let id = self.line_mission(actor, named, node)?;
        let mission = self.missions.book.get(&id).ok_or("mission_unknown")?;
        if !mission.conductor.actor.same_identity(actor) {
            return Err("mission_not_conductor".into());
        }
        let reason = reason.trim().to_owned();
        if reason.is_empty() {
            return Err("mission_skip_reason_missing".into());
        }
        if reason.chars().any(char::is_control) {
            return Err("mission_text_invalid".into());
        }
        let index = mission
            .nodes
            .iter()
            .position(|row| row.key == node)
            .ok_or("mission_node_unknown")?;
        let at = mission.nodes[index]
            .criteria
            .iter()
            .position(|line| line.key == key)
            .ok_or("mission_criterion_unknown")?;
        let line = &mission.nodes[index].criteria[at];
        let answered = |key: u64| {
            mission
                .decided
                .iter()
                .any(|decided| decided.key == key && decided.node == node)
        };
        if line.protected() && !decision.is_some_and(answered) {
            return Err(crate::coordination::refusal(
                "mission_criteria_protected",
                &format!("{node}: {key}"),
            ));
        }
        // A decision is kept only where it was needed, so the line says
        // `operator 결정` only when it was one.
        let decision = decision.filter(|_| line.protected());
        let by = Recorded {
            role: Role::Conductor,
            name: actor.name.clone(),
            agent_id: mission.conductor.agent_id.clone(),
            at_unix_ms: now,
        };
        let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
        let changed = target.nodes[index].criteria[at].skip(reason.clone(), decision, by);
        if !changed {
            return Ok((json!({"mission": self.mission_view_of(&id)}), false));
        }
        target.updated_at_unix_ms = now;
        let mut event = MissionEvent::new(now, EventKind::Skipped)
            .node(node)
            .line(Some(&reason));
        event.criterion = Some(key.to_owned());
        self.save_missions();
        self.record_mission_events(&id, vec![event]);
        Ok((json!({"mission": self.mission_view_of(&id)}), true))
    }

    /// `hide mission defect` (B21): a later node found a defect in `node`,
    /// which works again with the defect as a line.
    pub(super) fn add_mission_defect(
        &mut self,
        actor: &Actor,
        id: &str,
        node: &str,
        (found_by, text): (String, String),
        now: u64,
    ) -> Result<(Value, bool), String> {
        let mission = self.missions.book.get(id).ok_or("mission_unknown")?;
        if mission.node(&found_by).is_none() || found_by == node {
            return Err("mission_node_unknown".into());
        }
        let index = mission
            .nodes
            .iter()
            .position(|row| row.key == node)
            .ok_or("mission_node_unknown")?;
        let row = &mission.nodes[index];
        if row.kind == NodeKind::External {
            return Err(crate::coordination::refusal(
                "mission_external_invalid",
                node,
            ));
        }
        // The same defect from the same node while it is still open is the
        // same command again.
        if row.criteria.iter().any(|line| {
            line.kind == CriterionKind::Defect
                && line.text.as_deref() == Some(text.as_str())
                && line.found_by.as_deref() == Some(found_by.as_str())
                && line.state != crate::mission_criteria::CriterionState::Pass
        }) {
            return Ok((json!({"mission": self.mission_view_of(id)}), false));
        }
        let before = row.clone();
        let mut next = row.clone();
        crate::mission_criteria::add_defect(
            &mut next.criteria,
            text.clone(),
            found_by.clone(),
            Recorded {
                role: Role::Conductor,
                name: actor.name.clone(),
                agent_id: mission.conductor.agent_id.clone(),
                at_unix_ms: now,
            },
        )?;
        next.reopened = next.reopened.saturating_add(1);
        if next.status != NodeStatus::Working {
            next = crate::missions::apply_patch(
                &next,
                &NodePatch {
                    status: Some(NodeStatus::Working),
                    ..NodePatch::default()
                },
                now,
            )?;
        }
        let mut event = MissionEvent::new(now, EventKind::Reopened)
            .node(node)
            .line(Some(&text));
        event.count = Some(u32::from(next.reopened));
        event.criterion = Some(found_by);
        let mut events = vec![event];
        events.extend(
            super::mission_events::node_events(&before, &next, now)
                .into_iter()
                .filter(|event| event.kind != EventKind::Node || before.status != next.status),
        );
        let target = self.missions.book.get_mut(id).ok_or("mission_unknown")?;
        target.nodes[index] = next;
        target.updated_at_unix_ms = now;
        self.save_missions();
        self.record_mission_events(id, events);
        Ok((json!({"mission": self.mission_view_of(id)}), true))
    }

    /// The node with registration `id` bound to its next free `kind`
    /// checker of the same agent (B3, B4); the node's own worker is never
    /// its checker.
    pub(super) fn bind_checker(
        &self,
        node: &MissionNode,
        kind: CheckKind,
        id: &str,
    ) -> Result<MissionNode, String> {
        let ledger = self
            .delivery_state()
            .map_err(|_| "mission_checker_unknown")?;
        let record = ledger
            .agents
            .iter()
            .find(|record| record.id == id)
            .ok_or("mission_checker_unknown")?;
        let worker = node
            .agent_id
            .as_deref()
            .and_then(|worker| ledger.agents.iter().find(|row| row.id == worker));
        if node.agent_id.as_deref() == Some(id)
            || worker.is_some_and(|worker| worker.actor.same_identity(&record.actor))
        {
            return Err(crate::coordination::refusal(
                "mission_checker_is_worker",
                &node.key,
            ));
        }
        let mut next = node.clone();
        let line = next
            .criteria
            .iter_mut()
            .find(|line| line.kind == kind.kind())
            .ok_or("mission_checker_unknown")?;
        if line
            .checkers
            .iter()
            .any(|checker| checker.agent_id.as_deref() == Some(id))
        {
            return Ok(next);
        }
        let agent = hide_agent_adapter::canonical_kind(&record.actor.kind);
        let slot = line
            .checkers
            .iter_mut()
            .find(|checker| {
                checker.agent_id.is_none()
                    && hide_agent_adapter::canonical_kind(&checker.worker.agent) == agent
            })
            .ok_or("mission_checker_unknown")?;
        slot.agent_id = Some(id.to_owned());
        Ok(next)
    }

    /// Whether registration `id` checks one of `node`'s lines, so it cannot
    /// become the node's worker (B4).
    pub(super) fn checks_node(node: &MissionNode, id: &str) -> bool {
        node.criteria.iter().any(|line| {
            line.checkers
                .iter()
                .any(|checker| checker.agent_id.as_deref() == Some(id))
        })
    }
}

/// Whether a node whose check or CI spent its rerun goes back to work
/// (B13): one waiting, in review or not yet started; a node the operator
/// holds, or one done, failed or dropped, stays as it is.
pub(super) fn back_to_work(status: NodeStatus) -> bool {
    matches!(
        status,
        NodeStatus::Planned | NodeStatus::Review | NodeStatus::Waiting
    )
}

fn pull_in(pulls: Option<&GithubProjectSnapshot>, number: u32) -> Option<&PullRequestSnapshot> {
    pulls?
        .pull_requests
        .iter()
        .find(|pull| pull.number == number)
}

/// Keeps a question the operator answered that named a node, so a skip of
/// that node's check command or CI can rest on it (B12).
pub(super) fn note_decided(mission: &mut Mission, key: u64, node: Option<&str>) {
    let Some(node) = node.filter(|_| key != 0) else {
        return;
    };
    if mission
        .decided
        .iter()
        .any(|decided| decided.key == key && decided.node == node)
    {
        return;
    }
    mission.decided.push(Decided {
        key,
        node: node.to_owned(),
    });
    if mission.decided.len() > DECIDED_LIMIT {
        let over = mission.decided.len() - DECIDED_LIMIT;
        mission.decided.drain(..over);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.decided_dropped",
            "mission": mission.id, "count": over, "cap": DECIDED_LIMIT,
        }));
    }
}

/// The CI GitHub reports for each node's pull request in `project` (B8,
/// B13): a line where the pull request has checks, none where it has none,
/// unchanged where the read does not say. Answers the nodes whose rerun
/// failed again, which go back to work.
pub(super) fn read_mission_ci(
    mission: &mut Mission,
    project: &crate::model::GithubProjectSnapshot,
    now: u64,
) -> (bool, Vec<usize>) {
    if mission.exempt {
        return (false, Vec::new());
    }
    let mut changed = false;
    let mut spent = Vec::new();
    for (at, node) in mission.nodes.iter_mut().enumerate() {
        let effect = read_node_ci(node, project, now);
        changed |= effect.changed;
        if effect.spent {
            spent.push(at);
        }
    }
    (changed, spent)
}

/// The CI `project` reports for `node`'s pull request: a line where it has
/// checks, none where it has none, unchanged where the list does not say.
fn read_node_ci(
    node: &mut MissionNode,
    project: &GithubProjectSnapshot,
    now: u64,
) -> crate::mission_criteria::Effect {
    let mut out = crate::mission_criteria::Effect::default();
    let Some(pull) = node.pr.and_then(|number| pull_in(Some(project), number)) else {
        return out;
    };
    let line = node
        .criteria
        .iter()
        .position(|line| line.kind == CriterionKind::Ci);
    match (pull.checks, line, pull.head_oid.as_deref()) {
        (PullRequestChecks::None, Some(line), _) => {
            node.criteria.remove(line);
            out.changed = true;
        }
        (PullRequestChecks::None | PullRequestChecks::Unknown, ..) | (_, _, None) => {}
        (checks, line, Some(head)) => {
            let line = match line {
                Some(line) => line,
                None => {
                    // The CI line goes after the check command, before the
                    // reviews.
                    let place = node
                        .criteria
                        .iter()
                        .position(|line| {
                            matches!(
                                line.kind,
                                CriterionKind::Review
                                    | CriterionKind::Verify
                                    | CriterionKind::Defect
                            )
                        })
                        .unwrap_or(node.criteria.len());
                    node.criteria.insert(
                        place,
                        crate::mission_criteria::Criterion::new("ci", CriterionKind::Ci),
                    );
                    out.changed = true;
                    place
                }
            };
            let effect = node.criteria[line].read_ci(head, checks, now);
            out.changed |= effect.changed;
            out.spent = effect.spent;
        }
    }
    out
}
