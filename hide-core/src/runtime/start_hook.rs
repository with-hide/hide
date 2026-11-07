//! Whether the agent a spawn is about to start has Hide's hook on its
//! machine, judged before anything is made for it (with-hide/hide-private#661,
//! #662).
//!
//! A spawn registers its child only when the child's own hook reports its
//! session, so a child started without Hide's hook never registers, and a
//! Codex that has not trusted Hide's hooks stops at its review screen. A
//! machine's kit decides the hook at a pass (launch, connection, Reinstall),
//! and what it decided on changes between passes: the agent makes its own
//! folder at its first start or sign-in, and the operator approves the hooks
//! in Codex. So a spawn reads what the machine's last kit report says; where a
//! pass could change it, the spawn has one run on that machine's kit worker
//! and judges again from its answer; what no pass changes is refused with a
//! code whose next action says what the operator does.
//!
//! A machine where the kit does not run by design or by the operator's
//! choice (a daemon outside the app, a device without consent or with an
//! older one, an unsupported platform) is not judged here, as before; nor is
//! a helper that predates agents, or a hook Hide does not put there by design
//! (a system with no such hook, another tool's file of the same name). A kit
//! that should run and could not answer (its read failed, its pass stopped)
//! is asked for a pass like a missing hook, and refused when that pass
//! fails too.

use std::collections::BTreeMap;

use hide_kit::{ComponentState, PartCause, Scope};

use crate::model::KitUnavailable;

use super::Runtime;
use super::hosts::HostPhase;
use crate::model::{KitPieceSnapshot, KitSnapshot};

/// Why a spawn did not start its agent, as the one code each refusal
/// answers; `hided` words each code's next action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookRefusal {
    /// The agent is switched off for that machine in Settings › Agents.
    Off,
    /// The agent has not made its own folder there yet, so the kit has
    /// nothing to put its hook in.
    NotSetUp,
    /// Codex there has not trusted Hide's hooks and would open its review
    /// screen.
    Untrusted,
    /// Hide's hook is not in place there and a pass did not put it back.
    Missing,
    /// The pass the spawn asked for did not answer within its wait.
    Unconfirmed,
    /// The machine's kit could not be read, or its pass stopped before
    /// changing anything, so whether the hook is there is not known.
    Unchecked,
}

impl HookRefusal {
    const ALL: [Self; 6] = [
        Self::Off,
        Self::NotSetUp,
        Self::Untrusted,
        Self::Missing,
        Self::Unconfirmed,
        Self::Unchecked,
    ];

    /// Whether `code` is one of these refusals, as a surface that groups
    /// start failures (Desk, Memory) reads it.
    pub(crate) fn is_code(code: &str) -> bool {
        Self::ALL.iter().any(|refusal| refusal.code() == code)
    }

    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Off => "agent_hook_off",
            Self::NotSetUp => "agent_not_set_up",
            Self::Untrusted => "agent_hook_untrusted",
            Self::Missing => "agent_hook_missing",
            Self::Unconfirmed => "agent_hook_unconfirmed",
            Self::Unchecked => "agent_hook_unchecked",
        }
    }
}

/// What a start of one agent needs from its machine's kit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StartHook {
    /// Start: the hook is in place, the agent has none, or the kit is not
    /// judged on that machine.
    Ready,
    /// A pass may put the hook in place; the refusal stands when the pass's
    /// answer still says it is not.
    Pass(HookRefusal),
    /// No pass changes it.
    Refused(HookRefusal),
}

impl StartHook {
    /// The answer once the pass a start asked for has answered: what still
    /// waits for a pass is refused.
    pub(crate) fn after_pass(self) -> Result<(), HookRefusal> {
        match self {
            Self::Ready => Ok(()),
            Self::Pass(refusal) | Self::Refused(refusal) => Err(refusal),
        }
    }
}

/// What `kit`, one machine's last kit report, says about starting `kind`
/// there; `None` is a machine the kit never ran on in this process. The
/// agent's hook is judged by [`piece_start`].
pub(crate) fn judge(kit: Option<&KitSnapshot>, kind: &str) -> StartHook {
    let Some(kit) = kit else {
        return StartHook::Ready;
    };
    match kit.unavailable {
        Some(
            KitUnavailable::Standalone
            | KitUnavailable::NoConsent
            | KitUnavailable::ConsentOutdated
            | KitUnavailable::UnsupportedPlatform,
        ) => return StartHook::Ready,
        // No worker runs this machine's passes until Hide starts again.
        Some(KitUnavailable::InstallerUnavailable) => {
            return StartHook::Refused(HookRefusal::Unchecked);
        }
        Some(KitUnavailable::Unreadable) => return StartHook::Pass(HookRefusal::Unchecked),
        None => {}
    }
    if kit.refused.is_some() {
        return StartHook::Pass(HookRefusal::Unchecked);
    }
    let Some(id) = hide_agent_adapter::adapter(kind).map(|row| row.id) else {
        return StartHook::Ready;
    };
    let Some(agent) = kit.agents.iter().find(|agent| agent.id == id) else {
        // The machine's first pass is still running: its answer is the one
        // to judge by. A report without agents is a helper that predates
        // them.
        return if kit.agents.is_empty() && kit.busy {
            StartHook::Pass(HookRefusal::Unconfirmed)
        } else {
            StartHook::Ready
        };
    };
    if !agent.enabled {
        return StartHook::Refused(HookRefusal::Off);
    }
    agent.hook.as_ref().map_or(StartHook::Ready, piece_start)
}

/// What one piece of an agent on a machine, its hook or its skill, needs
/// before the agent starts there: in place, a pass of the kit, or something
/// only the operator does. The one place that says which pieces a pass can
/// put in: the spawn's judgment, the read that finds an agent's folder made
/// and Check again on a Codex that had not trusted Hide's hook all ask it.
pub(crate) fn piece_start(piece: &KitPieceSnapshot) -> StartHook {
    let cause = piece.why.as_ref().map(|why| why.cause);
    match piece.state {
        ComponentState::Installed => StartHook::Ready,
        ComponentState::Off => StartHook::Refused(HookRefusal::Off),
        // The operator took it out: only Reinstall brings it back (D-26).
        ComponentState::Removed => StartHook::Refused(HookRefusal::Missing),
        // The operator's own edit is theirs until Reinstall and may still
        // report; any other older entry is replaced by a pass.
        ComponentState::Outdated if cause == Some(PartCause::Edited) => StartHook::Ready,
        ComponentState::Outdated | ComponentState::NotInstalled => {
            StartHook::Pass(HookRefusal::Missing)
        }
        // Only a pass asks Codex to trust it again, and the operator may
        // have approved it in Codex since.
        ComponentState::Failed if cause == Some(PartCause::CodexUntrusted) => {
            StartHook::Pass(HookRefusal::Untrusted)
        }
        ComponentState::Failed => StartHook::Pass(HookRefusal::Missing),
        // The agent's folder may be there by now (`AgentAbsent` is the hook
        // part's word for the same wait). Any other absence is one Hide puts
        // no piece into by design, or a program the start's own probe looks
        // for (`agent_not_installed`).
        ComponentState::Absent => match cause {
            Some(PartCause::AgentAbsent | PartCause::NotCreatedYet) => {
                StartHook::Pass(HookRefusal::NotSetUp)
            }
            _ => StartHook::Ready,
        },
    }
}

/// The passes starts asked of one machine's kit, so a start can tell the
/// answer of the pass it asked for from one that ran before it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct StartPasses {
    /// How many passes starts asked for.
    asked: u64,
    /// `asked` when the pass now running was taken; that pass answers every
    /// request up to it.
    running: Option<u64>,
    /// The highest request a finished pass answered.
    answered: u64,
}

pub(super) type StartPassLedger = BTreeMap<String, StartPasses>;

impl Runtime {
    /// What a start of `kind` on `machine` needs from that machine's kit now.
    pub(crate) fn start_hook(&self, machine: &str, kind: &str) -> StartHook {
        if machine != self.node.as_str() && self.device_kit_declined(machine).is_some() {
            return StartHook::Ready;
        }
        judge(self.kit_states.get(machine), kind)
    }

    /// The answer for a start of `kind` on `machine` once the pass it asked
    /// for has answered. A pass whose call failed leaves the machine's last
    /// report in place with the failure beside it, so a hook still not in
    /// place is then refused as unchecked rather than for what that older
    /// report said.
    pub(crate) fn start_hook_after_pass(
        &self,
        machine: &str,
        kind: &str,
    ) -> Result<(), HookRefusal> {
        let hook = self.start_hook(machine, kind);
        let failed = self
            .kit_states
            .get(machine)
            .is_some_and(|kit| kit.check_failed.is_some());
        match hook {
            StartHook::Ready => Ok(()),
            _ if failed => Err(HookRefusal::Unchecked),
            hook => hook.after_pass(),
        }
    }

    /// Queues one automatic pass on `machine`'s kit for a start and answers
    /// the request it waits on (`start_pass_answered`). It is the pass a
    /// launch or a connection runs, so it changes only what those would.
    /// Refused `machine_unavailable` for a device whose helper is not
    /// connected, whose work waits for the next connection.
    pub(crate) fn ask_start_pass(&mut self, machine: &str) -> Result<u64, &'static str> {
        if machine != self.node.as_str()
            && !matches!(
                self.device_hosts.get(machine).map(|host| &host.phase),
                Some(HostPhase::Ready { .. })
            )
        {
            return Err("machine_unavailable");
        }
        self.queue_kit_scope(machine, Scope::automatic());
        let passes = self.start_passes.entry(machine.to_owned()).or_default();
        passes.asked += 1;
        let asked = passes.asked;
        crate::diagnostic!(serde_json::json!({
            "component": "kit", "kind": "start_pass.asked", "device_id": machine,
            "request": asked,
        }));
        if let Some(context) = self.worker_context.as_ref() {
            context.notifier.notify();
        }
        Ok(asked)
    }

    /// Whether a pass that started after request `asked` has answered, with
    /// a report or a failure.
    pub(crate) fn start_pass_answered(&self, machine: &str, asked: u64) -> bool {
        self.start_passes
            .get(machine)
            .is_some_and(|passes| passes.answered >= asked)
    }

    /// Whether `machine` can still answer: this machine always can, a device
    /// while its helper is connected or connecting.
    pub(crate) fn kit_reachable(&self, machine: &str) -> bool {
        machine == self.node.as_str()
            || matches!(
                self.device_hosts.get(machine).map(|host| &host.phase),
                Some(HostPhase::Ready { .. } | HostPhase::Connecting)
            )
    }

    /// `machine`'s kit worker took a pass: it answers every request asked
    /// so far.
    pub(super) fn start_pass_taken(&mut self, machine: &str) {
        if let Some(passes) = self.start_passes.get_mut(machine) {
            passes.running = Some(passes.asked);
        }
    }

    /// `machine`'s kit answered, with a report or a failure: the pass that
    /// was running has answered the requests it was taken for.
    pub(super) fn start_pass_finished(&mut self, machine: &str) {
        if let Some(passes) = self.start_passes.get_mut(machine)
            && let Some(answered) = passes.running.take()
        {
            passes.answered = passes.answered.max(answered);
        }
    }

    /// A device's kit work is dropped (its consent went, it is removed):
    /// nothing it was asked answers now, and a start waiting on it sees the
    /// device unreachable.
    pub(super) fn forget_start_passes(&mut self, machine: &str) {
        self.start_passes.remove(machine);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KitAgentSnapshot, KitPieceSnapshot};
    use hide_kit::{Availability, PartWhy};

    fn kit(state: ComponentState, cause: Option<PartCause>, enabled: bool) -> KitSnapshot {
        KitSnapshot {
            agents: vec![KitAgentSnapshot {
                id: "codex".to_owned(),
                label: "Codex".to_owned(),
                availability: Availability::Available,
                enabled,
                chosen: true,
                skill: KitPieceSnapshot {
                    state: ComponentState::Installed,
                    why: None,
                    location: None,
                },
                hook: Some(KitPieceSnapshot {
                    state,
                    why: cause.map(|cause| PartWhy { cause, value: None }),
                    location: None,
                }),
                partial: false,
                features: Vec::new(),
                sessions: None,
                update: None,
                doc_url: String::new(),
            }],
            ..KitSnapshot::default()
        }
    }

    fn judged(state: ComponentState, cause: Option<PartCause>) -> StartHook {
        judge(Some(&kit(state, cause, true)), "codex")
    }

    #[test]
    fn an_installed_hook_starts_and_a_machine_whose_kit_does_not_run_is_not_judged() {
        assert_eq!(judged(ComponentState::Installed, None), StartHook::Ready);
        assert_eq!(judge(None, "codex"), StartHook::Ready);
        let mut unavailable = kit(ComponentState::NotInstalled, None, true);
        // The kit does not run there by design or by the operator's choice.
        for cause in [
            KitUnavailable::Standalone,
            KitUnavailable::NoConsent,
            KitUnavailable::ConsentOutdated,
            KitUnavailable::UnsupportedPlatform,
        ] {
            unavailable.unavailable = Some(cause);
            assert_eq!(
                judge(Some(&unavailable), "codex"),
                StartHook::Ready,
                "{cause:?}"
            );
        }
    }

    /// A kit that should run and gave no answer is asked for a pass, and
    /// what it does not answer is refused as unchecked, never started.
    #[test]
    fn a_kit_that_could_not_answer_waits_for_a_pass_and_is_never_read_as_ready() {
        let unchecked = StartHook::Pass(HookRefusal::Unchecked);
        let unreadable = KitSnapshot::unavailable(KitUnavailable::Unreadable);
        assert_eq!(judge(Some(&unreadable), "claude"), unchecked);
        // A kit whose worker never started answers no pass.
        let no_worker = KitSnapshot::unavailable(KitUnavailable::InstallerUnavailable);
        assert_eq!(
            judge(Some(&no_worker), "claude"),
            StartHook::Refused(HookRefusal::Unchecked)
        );
        let refused = KitSnapshot {
            refused: Some(crate::model::KitRefusalSnapshot {
                cause: hide_kit::RefusalCause::Busy,
                path: None,
                variable: None,
                stop: None,
            }),
            ..KitSnapshot::default()
        };
        assert_eq!(judge(Some(&refused), "claude"), unchecked);
        assert_eq!(unchecked.after_pass(), Err(HookRefusal::Unchecked));
    }

    #[test]
    fn a_hook_a_pass_can_put_in_waits_for_one_and_names_the_refusal_if_it_does_not() {
        let cases = [
            (
                ComponentState::Absent,
                Some(PartCause::AgentAbsent),
                HookRefusal::NotSetUp,
            ),
            (
                ComponentState::Absent,
                Some(PartCause::NotCreatedYet),
                HookRefusal::NotSetUp,
            ),
            (
                ComponentState::Failed,
                Some(PartCause::CodexUntrusted),
                HookRefusal::Untrusted,
            ),
            (ComponentState::NotInstalled, None, HookRefusal::Missing),
            (
                ComponentState::Outdated,
                Some(PartCause::HelperGone),
                HookRefusal::Missing,
            ),
            (
                ComponentState::Failed,
                Some(PartCause::Unreadable),
                HookRefusal::Missing,
            ),
        ];
        for (state, cause, refusal) in cases {
            let answer = judged(state, cause);
            assert_eq!(answer, StartHook::Pass(refusal), "{state:?} {cause:?}");
            assert_eq!(answer.after_pass(), Err(refusal));
        }
    }

    #[test]
    fn what_no_pass_changes_is_refused_at_once_and_the_operators_edit_starts() {
        assert_eq!(
            judge(Some(&kit(ComponentState::Off, None, false)), "codex"),
            StartHook::Refused(HookRefusal::Off)
        );
        assert_eq!(
            judged(ComponentState::Removed, Some(PartCause::TakenOut)),
            StartHook::Refused(HookRefusal::Missing)
        );
        assert_eq!(
            judged(ComponentState::Outdated, Some(PartCause::Edited)),
            StartHook::Ready
        );
        assert_eq!(
            judged(ComponentState::Absent, Some(PartCause::Unsupported)),
            StartHook::Ready
        );
    }

    #[test]
    fn a_machine_whose_first_pass_runs_waits_for_it() {
        let running = KitSnapshot {
            busy: true,
            ..KitSnapshot::default()
        };
        assert_eq!(
            judge(Some(&running), "claude"),
            StartHook::Pass(HookRefusal::Unconfirmed)
        );
        assert_eq!(
            judge(Some(&KitSnapshot::default()), "claude"),
            StartHook::Ready
        );
    }
}
