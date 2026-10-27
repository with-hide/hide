//! Each machine's install kit as the runtime holds it (PRD device-parity):
//! what the last check of each machine found, the kit work each machine runs
//! next, and the operator's Reinstall.
//!
//! This Mac's kit runs on its own worker (`crate::kit::KitPump`); a device's
//! runs on its helper, called from a worker that exists while that device
//! has work queued (`crate::kit::spawn_device_worker`). Nothing here touches
//! a file or a process: a worker takes its job under the lock, runs it with
//! the lock released, and hands the report back.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hide_kit::{ComponentId, KitReport, Scope};

use super::Runtime;
use super::hosts::HostPhase;
use crate::model::{CodexDaemonOffSnapshot, KitSnapshot, KitUnavailable};
use crate::node_access::NodeLink;

/// How often this Mac's kit is read again while Settings is on screen, so a
/// part the operator removed by hand shows up without a relaunch.
const LOCAL_STATUS_INTERVAL: Duration = Duration::from_secs(5);

/// What a machine's kit worker is asked to do next.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum KitJob {
    Apply(Scope),
    Status,
}

impl KitJob {
    /// Two requests for one machine as one: an install answers a read too,
    /// and a Reinstall does everything the connection pass does.
    fn merge(self, later: KitJob) -> KitJob {
        match (self, later) {
            (KitJob::Apply(before), KitJob::Apply(after)) => {
                KitJob::Apply(merge_scopes(Some(before), after))
            }
            (KitJob::Apply(scope), KitJob::Status) | (KitJob::Status, KitJob::Apply(scope)) => {
                KitJob::Apply(scope)
            }
            (KitJob::Status, KitJob::Status) => KitJob::Status,
        }
    }
}

fn merge_scopes(before: Option<Scope>, after: Scope) -> Scope {
    match before {
        Some(before) => before.merge(after),
        None => after,
    }
}

/// A limit failure names its subject without copying or hashing an
/// arbitrarily large external identifier under the runtime lock.
fn diagnostic_identity(value: &str) -> serde_json::Value {
    let mut end = value.len().min(128);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    serde_json::json!({
        "prefix": &value[..end],
        "bytes": value.len(),
        "truncated": end != value.len(),
    })
}

/// What a device's kit worker does on one call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DeviceKitWork {
    Job(KitJob),
    /// The device is being removed from Hide: its kit comes off (D-16).
    Remove,
}

/// One kit call a device's worker makes, with what it needs from the
/// runtime taken under the lock.
pub(crate) struct DeviceKitCall {
    pub(crate) work: DeviceKitWork,
    pub(crate) channel: Arc<dyn NodeLink>,
    pub(crate) cli_dir: String,
    pub(crate) herdr_socket: Option<String>,
    pub(crate) preserve_herdr_server: bool,
    pub(crate) retirement_projects: Vec<String>,
    /// The run of the shared-server turn-off this call carries, if any.
    pub(crate) daemon_off_run: Option<u64>,
    pub(crate) connect_attempt: Option<u64>,
}

/// What a device's kit call answered.
pub(crate) enum DeviceKitAnswer {
    Report(Result<KitReport, String>),
    Removed(Result<hide_node_link::protocol::KitRemoved, String>),
}

/// Where one device stands with the first-run agent choice in this run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FirstRunChoice {
    /// Its record waits for the choice and this run has not sent it.
    Waiting,
    /// This run sent it; a device that still waits after that is reported.
    Sent,
}

/// Why Hide's kit does not run on a device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum KitDeclined {
    /// The operator has not allowed Hide's helper there.
    NoConsent,
    /// The consent was given to an older build's kit; Hide asks again. What
    /// that build put there may still be in place.
    ConsentOutdated,
    /// This build has no helper for the device's platform.
    Unsupported,
}

impl KitDeclined {
    fn cause(&self) -> KitUnavailable {
        match self {
            Self::NoConsent => KitUnavailable::NoConsent,
            Self::ConsentOutdated => KitUnavailable::ConsentOutdated,
            Self::Unsupported => KitUnavailable::UnsupportedPlatform,
        }
    }

    /// Whether Hide has certainly put no hook there: never allowed, or a
    /// platform it has no helper for. An outdated consent may leave an
    /// older build's hooks in place, so that answer stays unknown.
    pub(super) fn installed_nothing(&self) -> bool {
        !matches!(self, Self::ConsentOutdated)
    }
}

/// What a removal leaves on a device whose helper is not connected (B24).
const LEFT_ON_DEVICE: [&str; 3] = ["Hide's hook entries", "Hide's hide link", "the helper root"];

impl Runtime {
    /// Registered roots and the catalog's known checkouts, on this device only.
    /// This copies paths; the kit inspects their run states outside the lock.
    pub(crate) fn retirement_projects(&self, device_id: &str) -> Vec<String> {
        let mut paths = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .filter(|registration| registration.device_id == device_id)
            .map(|registration| registration.path.clone())
            .collect::<Vec<_>>();
        let projects = self.snapshot.navigator.workspaces.iter().chain(
            self.snapshot
                .status
                .remote
                .iter()
                .filter(|status| status.target_id == device_id)
                .filter_map(|status| status.session.as_ref())
                .flat_map(|session| session.workspaces.iter()),
        );
        for project in
            projects.filter(|project| project.device_id == device_id && project.registered)
        {
            paths.extend(
                project
                    .checkouts
                    .iter()
                    .map(|checkout| checkout.path.clone()),
            );
        }
        paths.sort();
        paths.dedup();
        paths
    }

    /// This Mac cannot run the kit at all, and its row says why (B11); a
    /// first-run Continue waiting for that pass learns it did not run.
    /// `detail` is what went wrong in the operator's terms; it goes to the
    /// log, and the row words `cause`.
    pub(crate) fn set_local_kit_unavailable(&mut self, cause: KitUnavailable, detail: &str) {
        let node = self.node.clone();
        if self.set_kit_state(node.as_str(), KitSnapshot::unavailable(cause)) {
            crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": "kit.unavailable",
                "device_id": node.as_str(),
                "cause": cause.as_str(),
                "detail": detail,
            }));
        }
        self.settle_first_run_apply();
    }

    /// The worker exists: the launch pass is its first job (B1, B10).
    pub(crate) fn queue_local_kit_launch(&mut self) {
        self.local_kit_pending = Some(Scope::automatic());
        let node = self.node.clone();
        self.set_kit_busy(node.as_str());
    }

    /// The next job for this Mac's kit worker: a queued install first, else
    /// a re-read while Settings is on screen; off screen nothing is read.
    pub(crate) fn take_local_kit_job(&mut self, now: Instant) -> Option<KitJob> {
        if let Some(scope) = self.local_kit_pending.take() {
            return Some(KitJob::Apply(scope));
        }
        if std::mem::take(&mut self.local_kit_check_requested)
            || self.settings_observed() && self.local_kit_next_status.is_none_or(|next| now >= next)
        {
            self.local_kit_next_status = Some(now + LOCAL_STATUS_INTERVAL);
            return Some(KitJob::Status);
        }
        None
    }

    /// A Settings tab showing the kit opened: this Mac and every connected
    /// device are read once. A machine that cannot run the kit has nothing
    /// to read, and one with work queued answers with that work. True when a
    /// machine's `checking` flipped, so the caller announces it: a slow read
    /// over a device must show as under way before it answers (B10).
    pub(super) fn request_kit_check(&mut self) -> bool {
        let mut changed = false;
        if self
            .kit_states
            .get(self.node.as_str())
            .is_none_or(|state| state.unavailable.is_none())
        {
            self.local_kit_check_requested = true;
            let node = self.node.clone();
            changed |= self.set_kit_checking(node.as_str());
        }
        let ready = self
            .device_hosts
            .iter()
            .filter(|(_, host)| matches!(host.phase, HostPhase::Ready { .. }))
            .map(|(device_id, _)| device_id.clone())
            .collect::<Vec<_>>();
        for device_id in ready {
            if !self.device_kit_pending.contains_key(&device_id)
                && !self.device_kit_running.contains(&device_id)
            {
                self.queue_device_kit(&device_id, KitJob::Status);
                changed |= self.set_kit_checking(&device_id);
            }
        }
        changed
    }

    /// Has `machine`'s kit read once more, for a session that newly wears
    /// the update badge (`sidebar::update_badged_sessions`): an agent started
    /// again, perhaps after its update, which the read settles. A device
    /// that is not connected, or has kit work queued or running, answers
    /// with that work instead.
    pub(super) fn request_machine_kit_read(&mut self, machine: &str) {
        if machine == self.node.as_str() {
            self.local_kit_check_requested = true;
            return;
        }
        let ready = self
            .device_hosts
            .get(machine)
            .is_some_and(|host| matches!(host.phase, HostPhase::Ready { .. }));
        if ready
            && !self.device_kit_pending.contains_key(machine)
            && !self.device_kit_running.contains(machine)
        {
            self.queue_device_kit(machine, KitJob::Status);
        }
    }

    /// A read asked with Check again is on its way: the machine's row shows
    /// it until an answer lands, report or failure (B10). It publishes only
    /// the flip, so a re-read every few seconds adds nothing while Settings
    /// is open. True when the snapshot changed.
    fn set_kit_checking(&mut self, device_id: &str) -> bool {
        let mut state = self.kit_state(device_id);
        state.checking = true;
        state.check_failed = None;
        self.set_kit_state(device_id, state)
    }

    fn diagnose_unknown_kit_agents(&mut self, device_id: &str, report: &KitReport) {
        // Reports are external input. Bound both the count and retained text;
        // crossing either bound is an explicit diagnostic, never growth.
        const LIMIT: usize = 128;
        const ID_BYTES: usize = 512;
        for agent in &report.agents {
            if hide_agent_adapter::adapter(&agent.id).is_some() {
                continue;
            }
            if self
                .unknown_kit_agents
                .iter()
                .any(|(device, id)| device == device_id && id == &agent.id)
            {
                continue;
            }
            if device_id.len() > ID_BYTES
                || agent.id.len() > ID_BYTES
                || self.unknown_kit_agents.len() >= LIMIT
            {
                if !self.unknown_kit_agents_limit_reported {
                    self.unknown_kit_agents_limit_reported = true;
                    crate::diagnostic!(serde_json::json!({
                        "component": "kit", "kind": "unknown_agent.diagnostic_limit",
                        "limit": LIMIT, "id_bytes": ID_BYTES,
                        "device_id": diagnostic_identity(device_id),
                        "agent_id": diagnostic_identity(&agent.id),
                    }));
                }
                continue;
            }
            let key = (device_id.to_owned(), agent.id.clone());
            if self.unknown_kit_agents.insert(key) {
                crate::diagnostic!(serde_json::json!({
                    "component": "kit", "kind": "unknown_agent.omitted",
                    "device_id": device_id, "agent_id": agent.id,
                }));
            }
        }
    }

    /// The kit's own sentence for each part and piece whose state or cause
    /// changed since the machine's last report: the screen words the cause,
    /// and this is where the path, the version or the io error is kept.
    fn diagnose_kit_reasons(&self, device_id: &str, report: &KitReport) {
        let before = self.kit_states.get(device_id);
        for part in &report.components {
            let Some(reason) = part.reason.as_deref() else {
                continue;
            };
            let unchanged = before.is_some_and(|before| {
                before.components.iter().any(|known| {
                    known.id == part.id && known.state == part.state && known.why == part.why
                })
            });
            if !unchanged {
                crate::diagnostic!(serde_json::json!({
                    "component": "kit",
                    "kind": "part.reason",
                    "device_id": device_id,
                    "part": part.id.code(),
                    "state": part.state,
                    "cause": part.why.as_ref().map(|why| why.cause),
                    "reason": reason,
                }));
            }
        }
        for agent in &report.agents {
            let known =
                before.and_then(|before| before.agents.iter().find(|row| row.id == agent.id));
            let pieces = [
                (
                    "skill",
                    Some(&agent.skill),
                    known.map(|row| Some(&row.skill)),
                ),
                (
                    "hook",
                    agent.hook.as_ref(),
                    known.map(|row| row.hook.as_ref()),
                ),
                (
                    "herdr",
                    agent.herdr.as_ref(),
                    known.map(|row| row.herdr.as_ref()),
                ),
            ];
            for (name, piece, known) in pieces {
                let Some(piece) = piece else {
                    continue;
                };
                let Some(reason) = piece.reason.as_deref() else {
                    continue;
                };
                let unchanged = known
                    .flatten()
                    .is_some_and(|known| known.state == piece.state && known.why == piece.why);
                if !unchanged {
                    crate::diagnostic!(serde_json::json!({
                        "component": "kit",
                        "kind": "part.reason",
                        "device_id": device_id,
                        "agent": agent.id,
                        "piece": name,
                        "state": piece.state,
                        "cause": piece.why.as_ref().map(|why| why.cause),
                        "reason": reason,
                    }));
                }
            }
        }
    }

    /// Stores what a check or an install found on one machine. A report that
    /// lands while an install is queued for that machine keeps it busy.
    pub(crate) fn ingest_kit_report(&mut self, device_id: &str, report: &KitReport) -> bool {
        self.diagnose_unknown_kit_agents(device_id, report);
        // The retired labels plugin is no part the operator acts on; what was
        // taken out, and what stayed for the next pass, goes to the log
        // (PRD labels-in-hided D-12, design principle 13).
        let retirement = &report.labels_retirement;
        if !retirement.is_empty() {
            crate::diagnostic!(serde_json::json!({
                "component": "labels",
                "kind": if retirement.failures.is_empty() { "plugin.retired" } else { "plugin.retire_incomplete" },
                "device_id": device_id,
                "removed": retirement.removed,
                "failures": retirement.failures,
            }));
        }
        // What an older layout left (the standalone hcoord plugin, folders
        // `~/.hide` replaced) is no part either (PRD hide-home-layout D-13).
        let legacy = &report.legacy_retirement;
        if !legacy.is_empty() {
            crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": if legacy.failures.is_empty() { "legacy.retired" } else { "legacy.retire_incomplete" },
                "device_id": device_id,
                "removed": legacy.removed,
                "failures": legacy.failures,
            }));
        }
        let mut snapshot = KitSnapshot::from_report(report);
        // A status read has no server update outcome. Keep the actionable
        // failure until a later install confirms what happened to Herdr.
        if report.herdr.is_none() {
            snapshot.herdr_start_failed = self
                .kit_states
                .get(device_id)
                .is_some_and(|state| state.herdr_start_failed);
        }
        self.diagnose_kit_reasons(device_id, report);
        self.install_updated_agents(device_id, &snapshot);
        snapshot.busy = self.kit_install_queued(device_id);
        // A refusal reaches the operator once per change, not on every read
        // while Settings is open: the kit's own words go to the log, and the
        // first run opens with it (`decide_agent_onboarding`).
        let refusal_new = snapshot.refused.is_some()
            && self
                .kit_states
                .get(device_id)
                .and_then(|state| state.refused.as_ref())
                != snapshot.refused.as_ref();
        if let Some(refusal) = report.refused.as_ref().filter(|_| refusal_new) {
            crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": "kit.refused",
                "device_id": device_id,
                "cause": refusal.cause,
                "path": refusal.path,
                "variable": refusal.variable,
                "detail": refusal.detail,
            }));
        }
        let mut changed = self.decide_agent_onboarding(device_id, report, refusal_new);
        if let Some(update) = &report.herdr {
            changed |= self.ingest_device_herdr_update(device_id, update);
        }
        // A daemon answer the kit could not read reads as no answer; the log
        // keeps it when it appears or changes, not on every read (B13).
        let unreadable_before = self
            .kit_states
            .get(device_id)
            .and_then(|state| state.codex_daemon_unreadable.as_deref());
        if let Some(reason) = report.codex_daemon_unreadable.as_deref()
            && unreadable_before != Some(reason)
        {
            crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": "codex_daemon.unreadable",
                "device_id": device_id,
                "reason": reason,
            }));
        }
        // Every Codex start on this machine is refused while its Codex is
        // unread (`codex_launch::CAPABILITY_UNKNOWN`); why goes to the log
        // when it appears or changes, as the cause and exit code alone.
        let unread_before = self
            .kit_states
            .get(device_id)
            .and_then(|state| state.codex_capability_unread);
        if let Some(failure) = report.codex_capability_unread
            && unread_before != Some(failure)
        {
            crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": "codex_capability_unread",
                "device_id": device_id,
                "agent": "codex",
                "failure": failure,
            }));
        }
        // Only the pass that carried the operator's request answers it; a
        // read that lands before or after keeps what the last one said.
        match &report.codex_daemon_off {
            Some(outcome) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "kit",
                    "kind": "codex_daemon_off.answered",
                    "device_id": device_id,
                    "outcome": outcome,
                }));
            }
            None => {
                snapshot.codex_daemon_off = self
                    .kit_states
                    .get(device_id)
                    .and_then(|state| state.codex_daemon_off);
            }
        }
        changed |= self.set_kit_state(device_id, snapshot);
        if device_id == self.node.as_str() {
            changed |= self.settle_first_run_apply();
        }
        changed
    }

    /// An agent that is on, whose copy was too old in the machine's last
    /// report and is new enough in this one, gets the pieces it still lacks
    /// on the pass this queues, without a press: a read only judges, and
    /// the next install would otherwise wait for a launch or a Reinstall
    /// (docs/agent-hooks.md, Updating an agent that is too old). A pass that
    /// installed them leaves nothing to repair, so it queues nothing more.
    fn install_updated_agents(&mut self, device_id: &str, snapshot: &KitSnapshot) {
        let Some(before) = self.kit_states.get(device_id) else {
            return;
        };
        let updated = snapshot
            .agents
            .iter()
            .filter(|agent| {
                agent.enabled
                    && agent.update.is_none()
                    && agent.needs_attention()
                    && before
                        .agents
                        .iter()
                        .any(|old| old.id == agent.id && old.update.is_some())
            })
            .map(|agent| agent.id.clone())
            .collect::<Vec<_>>();
        if updated.is_empty() {
            return;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "kit",
            "kind": "kit.agent_updated",
            "device_id": device_id,
            "agents": updated,
        }));
        self.queue_kit_scope(device_id, Scope::automatic());
    }

    /// What a machine's kit report says about the first-run agent choice.
    ///
    /// The kit's record is the one place that knows whether a machine was
    /// asked and answered (`held_for_onboarding`); `ui_state.agent_onboarding`
    /// follows it for this Mac and keeps the answer given, which is the choice
    /// a device that connects later receives. This Mac's record waiting asks;
    /// a record that never waited is an existing install, which is never
    /// asked and keeps what it has on as the saved choice. A device whose own
    /// record waits, once this Mac has answered, gets that choice sent once
    /// per run by its own detection; before that nothing is installed there.
    fn decide_agent_onboarding(
        &mut self,
        device_id: &str,
        report: &KitReport,
        refusal_new: bool,
    ) -> bool {
        use crate::model::AgentOnboarding;
        // A pass that stopped before changing anything says nothing about the
        // record. On a Mac no pass has ever answered for, it is still the
        // first run: the wizard opens with a refusal that is new, so the
        // operator learns why nothing was set up (design principle 13), and
        // stays closed once they closed it until the refusal changes or Hide
        // starts again.
        if report.refused.is_some() {
            let opened = refusal_new
                && device_id == self.node.as_str()
                && self.snapshot.ui_state.agent_onboarding.is_none()
                && self.open_first_run();
            if opened {
                self.persist_ui_state();
            }
            return opened;
        }
        // A helper that predates agents lists none and says nothing about the
        // record either.
        if report.agents.is_empty() {
            return false;
        }
        if device_id != self.node.as_str() {
            if !report.held_for_onboarding {
                self.device_first_run_choice.remove(device_id);
                return false;
            }
            self.device_first_run_choice
                .entry(device_id.to_owned())
                .or_insert(FirstRunChoice::Waiting);
            if self.snapshot.ui_state.agent_onboarding == Some(AgentOnboarding::Done) {
                self.send_first_run_choice(device_id);
            }
            return false;
        }
        let before = self.snapshot.ui_state.agent_onboarding;
        let after = if report.held_for_onboarding {
            // An Apply just pressed has its install queued here: a report that
            // was already on its way says the old thing and must not bring
            // the question back for a moment.
            if before == Some(AgentOnboarding::Done) && self.local_kit_pending.is_some() {
                return false;
            }
            AgentOnboarding::Pending
        } else {
            AgentOnboarding::Done
        };
        // A record that waits is a first run: the wizard opens with the
        // question as its first step (PRD first-run-onboarding D-08).
        let opened = after == AgentOnboarding::Pending && self.open_first_run();
        if before == Some(after) {
            if opened {
                self.persist_ui_state();
            }
            return opened;
        }
        self.snapshot.ui_state.agent_onboarding = Some(after);
        if after == AgentOnboarding::Done && before != Some(AgentOnboarding::Done) {
            // Never asked, or answered outside this app: what it has on today
            // is the choice a device added later receives, so that device
            // gets no less than before the choice existed.
            self.snapshot.ui_state.agent_onboarding_agents = report
                .agents
                .iter()
                .filter(|agent| agent.enabled)
                .map(|agent| agent.id.clone())
                .collect();
            self.send_first_run_choice_to_waiting_devices();
        }
        self.persist_ui_state();
        true
    }

    /// Sends the saved choice to a device that waits for it, once per run;
    /// a second report from the same device that still waits is a failure
    /// to answer, logged and not retried (engineering rules 11 and 15).
    fn send_first_run_choice(&mut self, device_id: &str) {
        if self.device_first_run_choice.get(device_id) == Some(&FirstRunChoice::Sent) {
            crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": "first_run_choice.unanswered",
                "device_id": device_id,
            }));
            return;
        }
        self.device_first_run_choice
            .insert(device_id.to_owned(), FirstRunChoice::Sent);
        let agents = self.snapshot.ui_state.agent_onboarding_agents.clone();
        self.queue_device_kit(
            device_id,
            KitJob::Apply(Scope::first_run(agents.iter().map(String::as_str))),
        );
    }

    /// A choice that was queued for a device and did not run is unsent again.
    fn reopen_first_run_choice(&mut self, device_id: &str) {
        if let Some(state) = self.device_first_run_choice.get_mut(device_id) {
            *state = FirstRunChoice::Waiting;
        }
    }

    fn send_first_run_choice_to_waiting_devices(&mut self) {
        let waiting = self
            .device_first_run_choice
            .iter()
            .filter(|(_, state)| **state == FirstRunChoice::Waiting)
            .map(|(device_id, _)| device_id.clone())
            .collect::<Vec<_>>();
        for device_id in waiting {
            self.send_first_run_choice(&device_id);
        }
    }

    /// The wizard's first step was continued with `agents` on (PRD
    /// first-run-onboarding B10, B45). While this Mac's record waits it is the
    /// first-run answer: the agents left on are switched on here and on every
    /// device that waits for it, each by its own detection, and the choice is
    /// kept for devices that connect later. After that, from a step reopened
    /// by a failed apply or by Help > Set up Hide, it switches each agent
    /// installed here the way Settings > Agents does, so only a changed agent
    /// moves. A press while the last one's pass still runs is that intent.
    pub(super) fn apply_agent_onboarding(&mut self, agents: Vec<String>, finish: bool) -> bool {
        if let Some(unknown) = agents
            .iter()
            .find(|agent| hide_kit::agents::adapter(agent).is_none())
        {
            self.set_error(
                "kit.unknown_agent",
                format!("{unknown} is not an agent Hide knows"),
                false,
            );
            return true;
        }
        if self.first_run_apply_in_flight() {
            return false;
        }
        let mut agents = agents;
        agents.sort();
        agents.dedup();
        let answer =
            self.snapshot.ui_state.agent_onboarding == Some(crate::model::AgentOnboarding::Pending);
        let node = self.node.clone();
        let view = self.kit_view(node.as_str());
        if view.refused.is_some() {
            // The pass stopped before changing anything, so the step drew no
            // tile and there is no choice to record: Try again runs the same
            // pass, which checks again what stopped it, and its answer brings
            // the tiles or the refusal back.
            crate::diagnostic!(serde_json::json!({
                "component": "first_run",
                "kind": "first_run.retry_refused",
                "answer": answer,
            }));
            return self.queue_kit_scope(node.as_str(), Scope::automatic());
        }
        if let Some(reason) = view.unavailable {
            // Nothing can be written here, and the step may not even have
            // drawn a tile, so the press records and queues nothing: the first
            // run's question stays open and the agents are read again (B12).
            // The step itself says why, so nothing reaches a banner (B44).
            crate::diagnostic!(serde_json::json!({
                "component": "first_run",
                "kind": "first_run.apply_kit_unavailable",
                "answer": answer,
                "reason": reason,
            }));
            self.local_kit_check_requested = true;
            return true;
        }
        // The next newly connected device receives the most recently accepted
        // choice, including a choice revised in Help after first run ended.
        let preference_changed = self.snapshot.ui_state.agent_onboarding_agents != agents;
        if preference_changed {
            self.snapshot.ui_state.agent_onboarding_agents = agents.clone();
            self.persist_ui_state();
        }
        let changed = if answer {
            self.finish_agent_onboarding(agents.clone())
        } else {
            self.switch_agents_here(&agents)
        };
        preference_changed | changed | self.wait_for_first_run_apply(agents, finish, answer)
    }

    /// Switches every agent installed on this Mac to what `on` says, through
    /// the per-agent path Settings > Agents uses; the caller has checked that
    /// the kit can run here.
    fn switch_agents_here(&mut self, on: &[String]) -> bool {
        let node = self.node.clone();
        let view = self.kit_view(node.as_str());
        // Decided before anything is queued: a queued switch marks the row
        // busy, which would make every later agent of the same press read as
        // undecided and be sent again.
        let moves: Vec<(String, bool)> = view
            .agents
            .iter()
            .filter(|row| row.availability == hide_kit::Availability::Available)
            .filter_map(|row| {
                let wanted = on.contains(&row.id);
                let now = self
                    .queued_agent_choice(node.as_str(), &row.id)
                    .unwrap_or(row.enabled);
                (wanted != now || wanted && row.needs_attention()).then(|| (row.id.clone(), wanted))
            })
            .collect();
        let mut changed = false;
        for (agent, wanted) in moves {
            changed |= self.request_kit_agent_set(node.as_str(), &agent, wanted);
        }
        changed
    }

    /// Ends the first-run choice with `agents` on; only a pending choice can
    /// end, so a second Apply or a stale client changes nothing
    /// (engineering rule 11). The choice reads as made at once; this Mac's
    /// record confirms it, and says otherwise (the question comes back) when
    /// it could not be saved.
    pub(super) fn finish_agent_onboarding(&mut self, agents: Vec<String>) -> bool {
        use crate::model::AgentOnboarding;
        if self.snapshot.ui_state.agent_onboarding != Some(AgentOnboarding::Pending) {
            return false;
        }
        self.snapshot.ui_state.agent_onboarding = Some(AgentOnboarding::Done);
        self.snapshot.ui_state.agent_onboarding_agents = agents.clone();
        self.persist_ui_state();
        let node = self.node.clone();
        self.queue_kit_scope(
            node.as_str(),
            Scope::first_run(agents.iter().map(String::as_str)),
        );
        self.send_first_run_choice_to_waiting_devices();
        true
    }

    fn kit_install_queued(&self, device_id: &str) -> bool {
        if device_id == self.node.as_str() {
            self.local_kit_pending.is_some()
        } else {
            matches!(
                self.device_kit_pending.get(device_id),
                Some(KitJob::Apply(_))
            )
        }
    }

    /// The operator pressed Reinstall on a machine's row: every part that is
    /// outdated, missing, removed or failed is installed again, and the parts
    /// in place are not touched (B8, B9).
    pub(super) fn request_kit_reinstall(
        &mut self,
        device_id: &str,
        only: Option<&[ComponentId]>,
        only_agents: Option<&[String]>,
    ) -> bool {
        if device_id != self.node.as_str() && !self.device_registration_exists(device_id) {
            self.set_error(
                "kit.unknown_machine",
                format!("Device {device_id} is not registered"),
                false,
            );
            return true;
        }
        let state = self.kit_view(device_id);
        if let Some(cause) = state.unavailable {
            self.set_error("kit.unavailable", cause.as_str(), false);
            return true;
        }
        // A failed bootstrap may have no pane or terminal services. Retry
        // the complete connection, including upload, services and routes.
        if state.herdr_start_failed
            && device_id != self.node.as_str()
            && self
                .link_origin(device_id)
                .is_some_and(crate::model::LinkOrigin::core_redials)
        {
            if self.device_host_connecting(device_id) {
                return false;
            }
            self.close_device_host(device_id, "retrying Herdr bootstrap");
            return self.retry_device_host_now(device_id);
        }
        // A refused pass (or this node's start failure) needs another
        // automatic pass even when each individual kit part is installed.
        if state.refused.is_some() || state.herdr_start_failed {
            if !state.busy
                && self
                    .link_origin(device_id)
                    .is_some_and(crate::model::LinkOrigin::core_redials)
            {
                self.begin_device_connect(device_id);
                if let Some(HostPhase::Ready { platform, .. }) =
                    self.device_hosts.get(device_id).map(|host| &host.phase)
                {
                    let platform = platform.clone();
                    if let Some((os, arch)) = platform.split_once(' ') {
                        self.device_connect_platform(device_id, os, arch);
                    }
                }
            }
            return self.queue_kit_scope(device_id, Scope::automatic());
        }
        // Naming parts or agents narrows the repair to what is named.
        let named = only.is_some() || only_agents.is_some();
        let parts = state
            .components
            .iter()
            .filter(|part| part.state.needs_attention())
            .filter(|part| match only {
                Some(only) => only.contains(&part.id),
                None => !named,
            })
            .map(|part| part.id)
            .collect::<Vec<_>>();
        let agents = state
            .agents
            .iter()
            .filter(|agent| agent.needs_attention())
            .filter(|agent| match only_agents {
                Some(only) => only.contains(&agent.id),
                None => !named,
            })
            .map(|agent| agent.id.as_str())
            .collect::<Vec<_>>();
        // A second press after the first one repaired everything is the same
        // intent, already met (engineering rule 11).
        if parts.is_empty() && agents.is_empty() {
            return false;
        }
        let scope = Scope::reinstall(parts).merge(Scope::agents(agents, []));
        self.queue_kit_scope(device_id, scope)
    }

    /// The operator switched an agent on or off from its row (issue #517).
    /// An agent already where the switch puts it, or that is not set up on
    /// that machine, is the same intent, already met (engineering rule 11).
    pub(super) fn request_kit_agent_set(
        &mut self,
        device_id: &str,
        agent: &str,
        enabled: bool,
    ) -> bool {
        if hide_kit::agents::adapter(agent).is_none() {
            self.set_error(
                "kit.unknown_agent",
                format!("{agent} is not an agent Hide knows"),
                false,
            );
            return true;
        }
        if device_id != self.node.as_str() && !self.device_registration_exists(device_id) {
            self.set_error(
                "kit.unknown_machine",
                format!("Device {device_id} is not registered"),
                false,
            );
            return true;
        }
        let state = self.kit_view(device_id);
        if let Some(cause) = state.unavailable {
            self.set_error("kit.unavailable", cause.as_str(), false);
            return true;
        }
        let Some(now) = state.agents.iter().find(|row| row.id == agent) else {
            return false;
        };
        // The latest intent wins (engineering rule 11): what is queued for
        // this agent decides, and while an install runs the last report is
        // older than that install, so it decides nothing.
        let already = match self.queued_agent_choice(device_id, agent) {
            Some(queued) => queued == enabled,
            None if state.busy => false,
            None if enabled => {
                // Nothing to switch on where the agent is not set up; an agent
                // that is on and has nothing to repair is met.
                now.availability != hide_kit::Availability::Available
                    || (now.enabled && !now.needs_attention())
            }
            None => !now.enabled,
        };
        if enabled && now.availability != hide_kit::Availability::Available {
            return false;
        }
        if already {
            return false;
        }
        let scope = if enabled {
            Scope::agents([agent], [])
        } else {
            Scope::agents([], [agent])
        };
        self.queue_kit_scope(device_id, scope)
    }

    /// What the work still queued for a machine says about one agent's
    /// switch: `Some(true)` on, `Some(false)` off, `None` when it says nothing.
    fn queued_agent_choice(&self, device_id: &str, agent: &str) -> Option<bool> {
        let scope = if device_id == self.node.as_str() {
            self.local_kit_pending.as_ref()
        } else {
            match self.device_kit_pending.get(device_id) {
                Some(KitJob::Apply(scope)) => Some(scope),
                _ => None,
            }
        }?;
        if scope.agent_off.contains(agent) {
            Some(false)
        } else if scope.agent_on.contains(agent) {
            Some(true)
        } else {
            None
        }
    }

    /// The operator confirmed turning Codex's shared server off on a machine,
    /// from a not connected pane's popover (PRD settings-cleanup B27;
    /// codex-daemon-apply D-11, B1-B3): the pass turns autostart off and
    /// stops the daemon that is running. The same request while one is
    /// running is the same intent, and a machine whose Codex starts no shared
    /// server and has none answering has nothing to turn off (engineering
    /// rule 11). The answer comes back in the report of the pass that carried
    /// it, as `codex_daemon_off`.
    pub(super) fn request_codex_daemon_disable(&mut self, device_id: &str) -> bool {
        if device_id != self.node.as_str() && !self.device_registration_exists(device_id) {
            self.set_error(
                "kit.unknown_machine",
                format!("Device {device_id} is not registered"),
                false,
            );
            return true;
        }
        let state = self.kit_view(device_id);
        if let Some(cause) = state.unavailable {
            self.set_error("kit.unavailable", cause.as_str(), false);
            return true;
        }
        if state.codex_daemon_off == Some(CodexDaemonOffSnapshot::Pending) {
            return false;
        }
        if !state.shares_codex_server() {
            crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": "codex_daemon_off.ignored",
                "device_id": device_id,
                "codex_daemon_on": state.codex_daemon_on,
                "codex_daemon_running": state.codex_daemon_running,
            }));
            return false;
        }
        // The confirmation named what disconnects now. A stop held for a
        // later helper connection would end sessions it never named, so a
        // device takes it only over the connection it was confirmed on
        // (design principle 13: the refusal goes to the log).
        if device_id != self.node.as_str()
            && !matches!(
                self.device_hosts.get(device_id).map(|host| &host.phase),
                Some(HostPhase::Ready { host, .. }) if host.closed_reason().is_none()
            )
        {
            crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": "codex_daemon_off.refused",
                "device_id": device_id,
                "reason": "helper_not_connected",
            }));
            return false;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "kit",
            "kind": "codex_daemon_off.requested",
            "device_id": device_id,
        }));
        self.queue_kit_scope(device_id, Scope::codex_daemon_off());
        let mut state = self.kit_state(device_id);
        state.codex_daemon_off = Some(CodexDaemonOffSnapshot::Pending);
        self.set_kit_state(device_id, state);
        true
    }

    /// A request that cannot be answered, because the helper it waits for
    /// is gone or its call never ran, ends as a failure the pane can say.
    fn end_codex_daemon_off_unanswered(&mut self, device_id: &str) {
        let mut state = self.kit_state(device_id);
        if state.codex_daemon_off == Some(CodexDaemonOffSnapshot::Pending) {
            state.codex_daemon_off = Some(CodexDaemonOffSnapshot::Failed {
                reason: crate::model::CodexDaemonOffFailure::Unreachable,
            });
            self.set_kit_state(device_id, state);
        }
    }

    /// Queues a Reinstall of `parts` on one machine.
    pub(super) fn queue_kit_reinstall(&mut self, device_id: &str, parts: Vec<ComponentId>) -> bool {
        self.queue_kit_scope(device_id, Scope::reinstall(parts))
    }

    fn queue_kit_scope(&mut self, device_id: &str, scope: Scope) -> bool {
        if device_id != self.node.as_str() {
            self.queue_device_kit(device_id, KitJob::Apply(scope));
            return true;
        }
        let merged = merge_scopes(self.local_kit_pending.take(), scope);
        self.local_kit_pending = Some(merged);
        let node = self.node.clone();
        self.set_kit_busy(node.as_str());
        true
    }

    /// Queues kit work for a device and starts its worker when its helper is
    /// connected. A device that is not connected yet is asked to connect,
    /// and the work runs when it does; its row is busy only while the work
    /// can actually run.
    pub(super) fn queue_device_kit(&mut self, device_id: &str, job: KitJob) {
        let merged = match self.device_kit_pending.remove(device_id) {
            Some(before) => before.merge(job),
            None => job,
        };
        let installs = matches!(merged, KitJob::Apply(_));
        self.device_kit_pending.insert(device_id.to_owned(), merged);
        let reachable = |runtime: &Runtime| {
            matches!(
                runtime.device_hosts.get(device_id).map(|host| &host.phase),
                Some(HostPhase::Ready { .. } | HostPhase::Connecting)
            )
        };
        if !reachable(self) {
            self.start_device_host(device_id);
        }
        if installs && reachable(self) {
            self.set_kit_busy(device_id);
        }
        self.start_device_kit(device_id);
    }

    /// Starts the device's kit worker when it has work, its helper is
    /// connected and no worker runs for it yet.
    pub(super) fn start_device_kit(&mut self, device_id: &str) {
        let removal = self.device_kit_removals.contains_key(device_id);
        let queued = self.device_kit_pending.contains_key(device_id)
            && matches!(
                self.device_hosts.get(device_id).map(|host| &host.phase),
                Some(HostPhase::Ready { .. })
            );
        if self.device_kit_running.contains(device_id) || !(removal || queued) {
            return;
        }
        if !self.spawn_device_kit_worker(device_id) {
            self.fail_device_connect_install(device_id);
            self.clear_kit_busy(device_id);
        }
    }

    /// One worker per device, so its kit calls run one at a time and a
    /// removal never runs beside an install on the same helper.
    fn spawn_device_kit_worker(&mut self, device_id: &str) -> bool {
        let Some(context) = self.worker_context.clone() else {
            return false;
        };
        self.device_kit_running.insert(device_id.to_owned());
        match crate::kit::spawn_device_worker(
            context.runtime,
            context.notifier,
            device_id.to_owned(),
        ) {
            Ok(()) => true,
            Err(error) => {
                self.device_kit_running.remove(device_id);
                crate::diagnostic!(serde_json::json!({
                    "component": "kit",
                    "kind": "worker.spawn_failed",
                    "device_id": device_id,
                    "error": error.to_string(),
                }));
                false
            }
        }
    }

    /// The device is being removed: with its helper connected, Hide's parts
    /// come off it on the device's kit worker, after any kit call already
    /// running there, and the helper connection closes after that. Without
    /// one, or while another registration still reaches the same account,
    /// what stays on the device is recorded (B23, B24). Answers why the kit
    /// stays when the removal was not started.
    pub(super) fn queue_device_kit_removal(
        &mut self,
        registration: &crate::model::DeviceRegistration,
    ) -> Result<(), &'static str> {
        let device_id = registration.id.as_str();
        self.device_kit_pending.remove(device_id);
        self.kit_states.remove(device_id);
        let left = |reason: &'static str| {
            crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": "device.kit_left",
                "device_id": device_id,
                "reason": reason,
                "left": LEFT_ON_DEVICE,
            }));
            Err(reason)
        };
        if !registration.origin.removes_kit() {
            return left("the machine runs Hide itself and connects to the core again");
        }
        if self.registration_sharing_account(registration).is_some() {
            return left("another registered device reaches the same account on that machine");
        }
        let channel =
            self.device_hosts.get_mut(device_id).and_then(|host| {
                match std::mem::replace(&mut host.phase, HostPhase::NotAllowed) {
                    HostPhase::Ready { host: channel, .. } if channel.closed_reason().is_none() => {
                        Some(channel)
                    }
                    other => {
                        host.phase = other;
                        None
                    }
                }
            });
        let Some(channel) = channel else {
            return left("its helper was not connected");
        };
        self.device_kit_removals.insert(
            device_id.to_owned(),
            DeviceKitCall {
                work: DeviceKitWork::Remove,
                channel: Arc::clone(&channel),
                cli_dir: registration
                    .host_consent
                    .as_ref()
                    .and_then(|consent| consent.cli_dir.clone())
                    .unwrap_or_else(|| self.host_cli_dir.clone()),
                herdr_socket: self.device_kit_socket(registration),
                preserve_herdr_server: registration.herdr_session.is_some(),
                retirement_projects: Vec::new(),
                daemon_off_run: None,
                connect_attempt: None,
            },
        );
        self.device_kit_removing.insert(device_id.to_owned());
        if !self.device_kit_running.contains(device_id) && !self.spawn_device_kit_worker(device_id)
        {
            self.device_kit_removals.remove(device_id);
            self.device_kit_removing.remove(device_id);
            channel.close("device removed");
            return left("the kit worker could not start");
        }
        Ok(())
    }

    /// Whether Hide is still taking its kit off a removed device; a new
    /// connection to it waits, so the removal never deletes what the new
    /// connection installs.
    pub(super) fn device_kit_removing(&self, device_id: &str) -> bool {
        self.device_kit_removing.contains(device_id)
    }

    /// The device's next kit call, or `None` when its worker should stop:
    /// nothing is queued, or the helper is no longer connected (the work
    /// then waits for the next connection).
    pub(crate) fn take_device_kit_call(&mut self, device_id: &str) -> Option<DeviceKitCall> {
        if let Some(removal) = self.device_kit_removals.remove(device_id) {
            return Some(removal);
        }
        let channel = match self.device_hosts.get(device_id).map(|host| &host.phase) {
            Some(HostPhase::Ready { host, .. }) if host.closed_reason().is_none() => {
                Arc::clone(host)
            }
            _ => {
                self.device_kit_running.remove(device_id);
                self.clear_kit_busy(device_id);
                return None;
            }
        };
        let registration = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.id == device_id)
            .cloned();
        let (Some(registration), Some(job)) =
            (registration, self.device_kit_pending.remove(device_id))
        else {
            self.device_kit_running.remove(device_id);
            return None;
        };
        // A node that dials in has no consent, so it links `hide` where a
        // device would by default, in its own home.
        let herdr_socket = self.device_kit_socket(&registration);
        let cli_dir = registration
            .host_consent
            .and_then(|consent| consent.cli_dir)
            .unwrap_or_else(|| self.host_cli_dir.clone());
        let daemon_off_run =
            matches!(&job, KitJob::Apply(scope) if scope.codex_daemon_off).then(|| {
                self.codex_daemon_off_runs += 1;
                self.codex_daemon_off_running
                    .insert(device_id.to_owned(), self.codex_daemon_off_runs);
                self.codex_daemon_off_runs
            });
        Some(DeviceKitCall {
            connect_attempt: matches!(&job, KitJob::Apply(_))
                .then(|| self.claim_device_connect_kit(device_id))
                .flatten(),
            work: DeviceKitWork::Job(job),
            channel,
            cli_dir,
            herdr_socket,
            preserve_herdr_server: registration.herdr_session.is_some(),
            retirement_projects: self.retirement_projects(device_id),
            daemon_off_run,
        })
    }

    fn device_kit_socket(&self, registration: &crate::model::DeviceRegistration) -> Option<String> {
        if registration.herdr_session.is_some() {
            self.remote_connections
                .get(&registration.id)
                .and_then(|connection| connection.transport.device().cached_herdr_socket())
        } else {
            registration.herdr_socket_path.clone().or_else(|| {
                self.remote_connections
                    .get(&registration.id)
                    .and_then(|connection| connection.transport.device().cached_herdr_socket())
            })
        }
    }

    /// Stores what a device's kit call answered. A call that failed keeps
    /// what the device last reported; a device never read says why it could
    /// not be, until the next connection or Settings reads it again.
    pub(crate) fn ingest_device_kit_answer(
        &mut self,
        device_id: &str,
        answer: DeviceKitAnswer,
        daemon_off_run: Option<u64>,
    ) -> bool {
        // Only the turn-off that still runs is settled by its answer; one the
        // device was forgotten under, or that a newer run replaced, settled
        // already, and its answer changes nothing now.
        let current = daemon_off_run.is_some()
            && self.codex_daemon_off_running.get(device_id) == daemon_off_run.as_ref();
        if current {
            self.codex_daemon_off_running.remove(device_id);
        }
        let answer = match answer {
            DeviceKitAnswer::Report(Ok(mut report)) if daemon_off_run.is_some() && !current => {
                report.codex_daemon_off = None;
                DeviceKitAnswer::Report(Ok(report))
            }
            answer => answer,
        };
        if let DeviceKitAnswer::Report(_) = &answer
            && (!self.device_registration_exists(device_id) || self.device_kit_removing(device_id))
        {
            // The device went while the call ran: its kit state went with it,
            // and a late answer must not bring it back for a device added
            // again under the same id.
            return false;
        }
        match answer {
            DeviceKitAnswer::Removed(removed) => self.ingest_device_kit_removal(device_id, removed),
            DeviceKitAnswer::Report(Ok(report)) => self.ingest_kit_report(device_id, &report),
            DeviceKitAnswer::Report(Err(reason)) => {
                // The call never ran to a report, so the choice it carried
                // was not delivered: the next report that waits sends it again.
                self.reopen_first_run_choice(device_id);
                crate::diagnostic!(serde_json::json!({
                    "component": "kit",
                    "kind": "device.call_failed",
                    "device_id": device_id,
                    "reason": reason,
                }));
                let mut state = self.kit_state(device_id);
                state.busy = self.kit_install_queued(device_id);
                state.checking = false;
                state.check_failed = Some(crate::model::KitCheckFailure::ReadFailed);
                if state.components.is_empty() {
                    state.unavailable = Some(KitUnavailable::Unreadable);
                }
                // Only the call that carried the current request ends it.
                if current && state.codex_daemon_off == Some(CodexDaemonOffSnapshot::Pending) {
                    state.codex_daemon_off = Some(CodexDaemonOffSnapshot::Failed {
                        reason: crate::model::CodexDaemonOffFailure::Unreachable,
                    });
                }
                self.set_kit_state(device_id, state)
            }
        }
    }

    /// Records what a removal took off a device and what stayed (B23, B24).
    /// A device added again while it ran connects now.
    fn ingest_device_kit_removal(
        &mut self,
        device_id: &str,
        removed: Result<hide_node_link::protocol::KitRemoved, String>,
    ) -> bool {
        match removed {
            Ok(removed) => crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": "device.kit_removed",
                "device_id": device_id,
                "components": removed.kit.components.iter().map(|(id, outcome)| {
                    serde_json::json!({ "id": id.code(), "outcome": outcome })
                }).collect::<Vec<_>>(),
                // The agents' hooks and skill stubs, a failed one included.
                "agents": removed.kit.agents.iter().map(|(piece, outcome)| {
                    serde_json::json!({ "piece": piece, "outcome": outcome })
                }).collect::<Vec<_>>(),
                "helper_root": removed.helper_root,
            })),
            Err(reason) => crate::diagnostic!(serde_json::json!({
                "component": "kit",
                "kind": "device.kit_left",
                "device_id": device_id,
                "reason": reason,
                "left": LEFT_ON_DEVICE,
            })),
        }
        self.device_kit_removing.remove(device_id);
        self.device_registration_exists(device_id) && self.start_device_host(device_id)
    }

    /// The helper connection a device's turn-off was confirmed on ended
    /// before its worker took it: the request leaves the queue, never sent
    /// on a later connection, and ends as unreachable so the popover can try
    /// again (PRD codex-daemon-apply B11). Other queued work stays for the
    /// next connection.
    pub(super) fn drop_queued_codex_daemon_off(&mut self, device_id: &str) {
        let Some(KitJob::Apply(scope)) = self.device_kit_pending.get(device_id) else {
            return;
        };
        if !scope.codex_daemon_off {
            return;
        }
        let rest = Scope {
            codex_daemon_off: false,
            ..scope.clone()
        };
        // A turn-off alone leaves nothing to run; the next connection queues
        // its own pass.
        if rest.is_automatic() {
            self.device_kit_pending.remove(device_id);
            self.clear_kit_busy(device_id);
        } else {
            self.device_kit_pending
                .insert(device_id.to_owned(), KitJob::Apply(rest));
        }
        crate::diagnostic!(serde_json::json!({
            "component": "kit",
            "kind": "codex_daemon_off.dropped",
            "device_id": device_id,
            "reason": "the helper connection ended before the request ran",
        }));
        self.end_codex_daemon_off_unanswered(device_id);
    }

    /// Queued work cannot run now: the row stops saying it is working.
    pub(super) fn clear_kit_busy(&mut self, device_id: &str) {
        let mut state = self.kit_state(device_id);
        if state.busy || state.checking {
            state.busy = false;
            state.checking = false;
            self.set_kit_state(device_id, state);
        }
        if device_id == self.node.as_str() {
            self.settle_memory_hook_preparation();
        }
    }

    /// The device's consent was withdrawn or the device is going away:
    /// nothing queued for it runs.
    pub(super) fn forget_device_kit_work(&mut self, device_id: &str) {
        self.forget_device_connect(device_id);
        self.device_kit_pending.remove(device_id);
        self.codex_daemon_off_running.remove(device_id);
        self.end_codex_daemon_off_unanswered(device_id);
        self.reopen_first_run_choice(device_id);
        self.clear_kit_busy(device_id);
        self.update_badges.remove(device_id);
    }

    /// The hook parts of this Mac that a Reinstall would repair, for
    /// Memory's "update hooks".
    pub(super) fn local_hook_parts_to_repair(&self) -> Vec<ComponentId> {
        self.kit_states
            .get(self.node.as_str())
            .map(|state| {
                state
                    .components
                    .iter()
                    .filter(|part| {
                        matches!(
                            part.id,
                            ComponentId::ClaudeCodeHook | ComponentId::CodexHook
                        ) && part.state.needs_attention()
                    })
                    .map(|part| part.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How many of each agent's sessions run on one machine now (PRD
    /// settings-cleanup B16, B19).
    ///
    /// Only the sessions that exist now are counted: the walk is over the
    /// panes the snapshot holds, so a closed session leaves the count on the
    /// next pass and nothing accumulates per session (D-23). A sleeping agent
    /// has ended its process, so its pane holds no running session. The work
    /// is one pass over the machine's panes each time the agent lineage pass
    /// or a device's session republishes, not a per-tick cost.
    fn agent_sessions(&self, device_id: &str) -> std::collections::BTreeMap<&'static str, u32> {
        let remote = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == device_id)
            .and_then(|status| status.session.as_ref());
        let (workspaces, agents) = if device_id == self.node.as_str() {
            (
                &self.snapshot.navigator.workspaces,
                &self.snapshot.navigator.agents,
            )
        } else if let Some(session) = remote {
            (&session.workspaces, &session.agents)
        } else {
            return std::collections::BTreeMap::new();
        };
        let mut sessions: std::collections::BTreeMap<&'static str, u32> =
            hide_agent_adapter::ADAPTERS
                .iter()
                .filter(|adapter| {
                    adapter
                        .session
                        .is_some_and(|format| format.has_session_file())
                })
                .map(|adapter| (adapter.id, 0))
                .collect();
        let panes = workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter());
        for pane in panes {
            let local_pane = !crate::agent_hooks::is_remote_pane(&pane.id);
            if (device_id == self.node.as_str()) != local_pane {
                continue;
            }
            let Some(agent) = agents
                .iter()
                .find(|agent| agent.pane_id == pane.id && agent.sleep.is_none())
            else {
                continue;
            };
            let Some(count) = hide_agent_adapter::adapter(&agent.agent_kind)
                .and_then(|adapter| sessions.get_mut(adapter.id))
            else {
                continue;
            };
            *count += 1;
        }
        sessions
    }

    /// Puts [`Self::agent_sessions`] on a kit snapshot's agent rows; an agent
    /// whose sessions Hide does not read keeps `None`.
    fn fill_agent_sessions(&self, device_id: &str, kit: &mut KitSnapshot) {
        let sessions = self.agent_sessions(device_id);
        for agent in &mut kit.agents {
            agent.sessions = sessions.get(agent.id.as_str()).copied();
        }
    }

    /// Recounts every machine's sessions onto its kit rows after the panes
    /// changed; returns whether any count moved.
    pub(super) fn refresh_agent_sessions(&mut self) -> bool {
        let ids = self
            .snapshot
            .navigator
            .devices
            .iter()
            .map(|device| device.id.clone())
            .collect::<Vec<_>>();
        let mut changed = false;
        for id in ids {
            let sessions = self.agent_sessions(&id);
            let Some(device) = self
                .snapshot
                .navigator
                .devices
                .iter_mut()
                .find(|device| device.id == id)
            else {
                continue;
            };
            for agent in &mut device.kit.agents {
                let next = sessions.get(agent.id.as_str()).copied();
                if agent.sessions != next {
                    agent.sessions = next;
                    changed = true;
                }
            }
        }
        changed
    }

    /// A machine's kit as its row shows it. A device the helper may not be
    /// installed on, or whose platform this build does not carry, installs
    /// nothing and says why (B17, B21); otherwise its last report stands.
    /// What a device's node did about its Herdr server after a kit pass:
    /// one log record for every outcome (PRD device-herdr-update B12), and
    /// the device row's line while the server stays older than the pin (B8).
    /// An outcome that could not tell the server's Herdr keeps the last
    /// known one.
    fn ingest_device_herdr_update(
        &mut self,
        device_id: &str,
        update: &hide_kit::device_herdr::HerdrUpdate,
    ) -> bool {
        use hide_kit::device_herdr::HerdrUpdate;
        crate::diagnostic!(update.record(device_id));
        let behind = match update {
            HerdrUpdate::Failed { .. } => return false,
            HerdrUpdate::Behind {
                server_version,
                version,
                cause,
                ..
            } => Some(crate::model::HerdrBehindSnapshot {
                server_version: server_version.clone(),
                version: version.clone(),
                cause: *cause,
            }),
            HerdrUpdate::NoServer { .. }
            | HerdrUpdate::Started { .. }
            | HerdrUpdate::Current { .. }
            | HerdrUpdate::Kept { .. }
            | HerdrUpdate::HandedOff { .. } => None,
        };
        if self.device_herdr_behind.get(device_id) == behind.as_ref() {
            return false;
        }
        match behind {
            Some(behind) => self
                .device_herdr_behind
                .insert(device_id.to_owned(), behind),
            None => self.device_herdr_behind.remove(device_id),
        };
        self.refresh_device_snapshots()
    }

    pub(super) fn kit_view(&self, device_id: &str) -> KitSnapshot {
        let mut view = self.kit_view_without_sessions(device_id);
        self.fill_agent_sessions(device_id, &mut view);
        view
    }

    fn kit_view_without_sessions(&self, device_id: &str) -> KitSnapshot {
        if device_id == self.node.as_str() {
            return self.kit_state(device_id);
        }
        let mut view = match self.device_kit_declined(device_id) {
            Some(declined) => KitSnapshot::unavailable(declined.cause()),
            None => self.kit_state(device_id),
        };
        view.shares_account_with = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.id == device_id)
            .and_then(|registration| self.registration_sharing_account(registration))
            .map(|other| other.label.clone());
        view
    }

    /// Why Hide's kit does not run on a device now, if it does not.
    pub(super) fn device_kit_declined(&self, device_id: &str) -> Option<KitDeclined> {
        let host = self.host_snapshot(device_id);
        match (host.consent.as_str(), host.state.as_str()) {
            ("none", _) => Some(KitDeclined::NoConsent),
            ("outdated", _) => Some(KitDeclined::ConsentOutdated),
            (_, "unsupported") => Some(KitDeclined::Unsupported),
            _ => None,
        }
    }

    /// Another registration that reaches the same account on the same
    /// machine: its consent bound the same user and host key, as two
    /// registrations for two Herdr servers there do. The hooks, the `hide`
    /// link and the helper root belong to that account, not to one
    /// registration, so removing one of them leaves those for the other.
    fn registration_sharing_account(
        &self,
        registration: &crate::model::DeviceRegistration,
    ) -> Option<&crate::model::DeviceRegistration> {
        fn account(registration: &crate::model::DeviceRegistration) -> Option<(&str, &str)> {
            let identity = registration.host_consent.as_ref()?.identity.as_ref()?;
            Some((&identity.user, &identity.host_key_sha256))
        }
        let own = account(registration)?;
        self.snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|other| other.id != registration.id && account(other) == Some(own))
    }

    /// What a Codex start on this machine passes about the shared daemon,
    /// from its kit's last report (PRD overview-request-view D-20).
    pub(super) fn codex_daemon(&self, device_id: &str) -> crate::codex_launch::CodexDaemon {
        self.kit_states
            .get(device_id)
            .map(crate::codex_launch::CodexDaemon::from_kit)
            .unwrap_or_default()
    }

    /// [`Self::codex_daemon`] for the machine a pane id belongs to.
    pub(super) fn codex_daemon_for_pane(&self, pane_id: &str) -> crate::codex_launch::CodexDaemon {
        let device = pane_id
            .strip_prefix("remote:")
            .and_then(|rest| rest.split_once(":pane:"))
            .map_or(self.node.as_str(), |(device, _)| device);
        self.codex_daemon(device)
    }

    pub(super) fn kit_state(&self, device_id: &str) -> KitSnapshot {
        self.kit_states.get(device_id).cloned().unwrap_or_default()
    }

    fn set_kit_busy(&mut self, device_id: &str) {
        let mut state = self.kit_state(device_id);
        state.busy = true;
        self.set_kit_state(device_id, state);
    }

    pub(super) fn set_kit_state(&mut self, device_id: &str, snapshot: KitSnapshot) -> bool {
        if self.kit_states.get(device_id) == Some(&snapshot) {
            return false;
        }
        let judged_before = self.kit_states.get(device_id).map(pane_judgement_inputs);
        let judged_now = pane_judgement_inputs(&snapshot);
        self.kit_states.insert(device_id.to_owned(), snapshot);
        self.refresh_device_snapshots();
        // A device's agent panes are judged against its kit.
        if device_id != self.node.as_str() {
            self.refresh_device_catalog(device_id);
        } else if judged_before.as_ref() != Some(&judged_now) {
            // A Codex pane's reason for not being connected follows the
            // shared server (its setting, or a daemon that still answers), and
            // an OpenCode, Grok or Cursor pane's count follows its agent's
            // hook piece, so
            // the panes are judged again at once.
            self.sync_pane_lineage();
        }
        true
    }
}

/// What of a machine's kit the judgement of its panes reads: the Codex
/// shared server, the state of each agent's hook piece and the update its
/// copy needs, for an agent that is on.
type PaneJudgementInputs = (
    bool,
    Vec<(
        String,
        Option<hide_kit::ComponentState>,
        Option<hide_kit::AgentUpdate>,
    )>,
);

fn pane_judgement_inputs(kit: &KitSnapshot) -> PaneJudgementInputs {
    (
        kit.shares_codex_server(),
        kit.agents
            .iter()
            .map(|agent| {
                (
                    agent.id.clone(),
                    agent.hook.as_ref().map(|hook| hook.state),
                    agent.update.clone().filter(|_| agent.enabled),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown(id: &str) -> hide_kit::AgentReport {
        hide_kit::AgentReport {
            id: id.into(),
            label: id.into(),
            availability: hide_kit::Availability::Available,
            enabled: false,
            chosen: false,
            skill: hide_kit::PieceReport {
                state: hide_kit::ComponentState::Installed,
                reason: None,
                why: None,
                location: None,
            },
            hook: None,
            herdr: None,
            update: None,
            doc_url: String::new(),
        }
    }

    #[test]
    fn an_unknown_helper_agent_is_diagnosed_once_for_each_device() {
        let mut runtime = super::super::tests::runtime();
        let report = KitReport {
            agents: vec![unknown("future-agent")],
            ..Default::default()
        };
        let (_, records) = crate::diagnostics::capture(|| {
            for device in ["device-a", "device-a", "device-b", "device-b"] {
                runtime.diagnose_unknown_kit_agents(device, &report);
            }
        });
        let unknown: Vec<_> = records
            .iter()
            .filter(|record| record["kind"] == "unknown_agent.omitted")
            .collect();
        assert_eq!(unknown.len(), 2);
        assert_eq!(unknown[0]["device_id"], "device-a");
        assert_eq!(unknown[1]["device_id"], "device-b");
        assert!(
            unknown
                .iter()
                .all(|record| record["agent_id"] == "future-agent")
        );
    }

    #[test]
    fn unknown_agent_diagnostics_have_a_reported_count_and_text_bound() {
        let mut runtime = super::super::tests::runtime();
        let report = KitReport {
            agents: (0..129)
                .map(|index| unknown(&format!("future-{index}")))
                .collect(),
            ..Default::default()
        };
        let (_, records) = crate::diagnostics::capture(|| {
            runtime.diagnose_unknown_kit_agents("device", &report);
            runtime.diagnose_unknown_kit_agents("device", &report);
        });
        assert_eq!(runtime.unknown_kit_agents.len(), 128);
        assert_eq!(
            records
                .iter()
                .filter(|record| record["kind"] == "unknown_agent.omitted")
                .count(),
            128
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record["kind"] == "unknown_agent.diagnostic_limit")
                .count(),
            1
        );
        let limit = records
            .iter()
            .find(|record| record["kind"] == "unknown_agent.diagnostic_limit")
            .unwrap();
        assert_eq!(
            limit["device_id"],
            serde_json::json!({"prefix": "device", "bytes": 6, "truncated": false})
        );
        assert_eq!(
            limit["agent_id"],
            serde_json::json!({"prefix": "future-128", "bytes": 10, "truncated": false})
        );
        let mut runtime = super::super::tests::runtime();
        let report = KitReport {
            agents: vec![unknown(&"x".repeat(513))],
            ..Default::default()
        };
        let (_, records) =
            crate::diagnostics::capture(|| runtime.diagnose_unknown_kit_agents("device", &report));
        assert!(runtime.unknown_kit_agents.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["kind"], "unknown_agent.diagnostic_limit");
        assert_eq!(records[0]["agent_id"]["bytes"], 513);
        assert_eq!(records[0]["agent_id"]["prefix"], "x".repeat(128));
        assert_eq!(records[0]["agent_id"]["truncated"], true);
    }

    #[test]
    fn unknown_identity_boundary_is_inclusive_and_failure_references_are_utf8_safe() {
        let mut runtime = super::super::tests::runtime();
        let report = KitReport {
            agents: vec![unknown(&"x".repeat(512))],
            ..Default::default()
        };
        let (_, records) =
            crate::diagnostics::capture(|| runtime.diagnose_unknown_kit_agents("device", &report));
        assert_eq!(runtime.unknown_kit_agents.len(), 1);
        assert_eq!(records[0]["kind"], "unknown_agent.omitted");
        assert_eq!(records[0]["agent_id"], "x".repeat(512));

        let mut runtime = super::super::tests::runtime();
        let device = "기".repeat(200);
        let report = KitReport {
            agents: vec![unknown(&"가".repeat(200))],
            ..Default::default()
        };
        let (_, records) = crate::diagnostics::capture(|| {
            runtime.diagnose_unknown_kit_agents(&device, &report);
            runtime.diagnose_unknown_kit_agents(&device, &report);
        });
        assert!(runtime.unknown_kit_agents.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0]["device_id"],
            serde_json::json!({"prefix": "기".repeat(42), "bytes": 600, "truncated": true})
        );
        assert_eq!(
            records[0]["agent_id"],
            serde_json::json!({"prefix": "가".repeat(42), "bytes": 600, "truncated": true})
        );
    }

    #[test]
    fn requests_for_one_machine_merge_into_the_widest() {
        let reinstall =
            |parts: &[ComponentId]| KitJob::Apply(Scope::reinstall(parts.iter().copied()));
        assert_eq!(
            KitJob::Status.merge(KitJob::Apply(Scope::automatic())),
            KitJob::Apply(Scope::automatic())
        );
        assert_eq!(
            KitJob::Apply(Scope::automatic()).merge(KitJob::Status),
            KitJob::Apply(Scope::automatic())
        );
        assert_eq!(
            reinstall(&[ComponentId::CoordinationRetirement])
                .merge(KitJob::Apply(Scope::automatic())),
            reinstall(&[ComponentId::CoordinationRetirement])
        );
        assert_eq!(
            reinstall(&[ComponentId::CoordinationRetirement]).merge(reinstall(&[
                ComponentId::Cli,
                ComponentId::CoordinationRetirement
            ])),
            reinstall(&[ComponentId::Cli, ComponentId::CoordinationRetirement])
        );
        assert_eq!(KitJob::Status.merge(KitJob::Status), KitJob::Status);
        // The operator's later switch wins for the agent it names.
        assert_eq!(
            KitJob::Apply(Scope::agents([], ["pi"]))
                .merge(KitJob::Apply(Scope::agents(["pi"], []))),
            KitJob::Apply(Scope::agents(["pi"], []))
        );
    }
}
