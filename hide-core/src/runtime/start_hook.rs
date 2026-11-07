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
//! A machine where the kit does not run (a daemon outside the app, a device
//! without consent, an unsupported platform) or whose kit cannot be judged
//! (its pass stopped, its helper predates agents) is not judged here, as
//! before; nor is a hook Hide does not put there by design (a system with no
//! such hook, another tool's file of the same name).

use std::collections::BTreeMap;

use hide_kit::{ComponentState, PartCause, Scope};

use super::Runtime;
use super::hosts::HostPhase;
use crate::model::KitSnapshot;

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
}

impl HookRefusal {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Off => "agent_hook_off",
            Self::NotSetUp => "agent_not_set_up",
            Self::Untrusted => "agent_hook_untrusted",
            Self::Missing => "agent_hook_missing",
            Self::Unconfirmed => "agent_hook_unconfirmed",
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
/// there; `None` is a machine the kit never ran on in this process. The one
/// place a kit report's piece chooses a start's refusal.
pub(crate) fn judge(kit: Option<&KitSnapshot>, kind: &str) -> StartHook {
    let Some(kit) = kit else {
        return StartHook::Ready;
    };
    if kit.unavailable.is_some() || kit.refused.is_some() {
        return StartHook::Ready;
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
    let Some(hook) = agent.hook.as_ref() else {
        return StartHook::Ready;
    };
    let cause = hook.why.as_ref().map(|why| why.cause);
    match hook.state {
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
        ComponentState::Failed if cause == Some(PartCause::CodexUntrusted) => {
            StartHook::Pass(HookRefusal::Untrusted)
        }
        ComponentState::Failed => StartHook::Pass(HookRefusal::Missing),
        // The agent's folder may be there by now (`AgentAbsent` is the hook
        // part's word for the same wait). Any other absence is one Hide puts
        // no hook into by design, or a program the start's own probe looks
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
        unavailable.unavailable = Some(crate::model::KitUnavailable::NoConsent);
        assert_eq!(judge(Some(&unavailable), "codex"), StartHook::Ready);
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
