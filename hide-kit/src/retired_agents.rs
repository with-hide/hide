//! The fourteen agents Hide stopped supporting (PRD settings-cleanup D-06,
//! and Gemini CLI once Hide's support list became the agents the pinned
//! Herdr ships an integration for): the one-time pass that takes out what an
//! earlier build put on a machine for them.
//!
//! An earlier kit wrote a skill stub and, for seven of them, a SessionStart
//! guidance hook, and recorded each in `~/.hide/kit/installed.json`. This pass
//! reads that record for ownership: only a piece the record names is looked
//! at, and then only what carries Hide's own marker is taken (the stub's
//! marker line, the hook entry's `hide-guidance` source), so a file the
//! operator wrote or edited stays. Each piece leaves the record once it is out,
//! and an agent with nothing left in the record is not looked at again, so a
//! later pass asks nothing and rescans nothing; a piece that could not be
//! taken out stays in the record and the next pass tries again.
//!
//! A transition path: it goes with the release after the one that ships it,
//! together with the layouts of the seven hooks in `hide_agent_hooks::guidance`.
//! Gemini CLI joined later than the other thirteen, so the release that ships
//! its retirement is the one the whole path waits for.

use std::path::PathBuf;

use hide_agent_hooks::guidance::GuidanceAgent;

use crate::Retirement;
use crate::agents::{SkillDir, remove_skill, remove_skill_in};
use crate::record::Record;
use crate::{KitTarget, RemoveOutcome};

/// Where a retired agent read skills from.
#[derive(Clone, Copy)]
enum Skills {
    /// The shared folder, which live agents read as well.
    Shared,
    /// A folder only that agent read, `~/<folder>/skills`.
    Own(&'static str),
}

struct Retired {
    id: &'static str,
    label: &'static str,
    skills: Skills,
    hook: Option<GuidanceAgent>,
}

const fn retired(
    id: &'static str,
    label: &'static str,
    skills: Skills,
    hook: Option<GuidanceAgent>,
) -> Retired {
    Retired {
        id,
        label,
        skills,
        hook,
    }
}

const RETIRED: [Retired; 14] = [
    retired(
        "copilot-cli",
        "GitHub Copilot CLI",
        Skills::Shared,
        Some(GuidanceAgent::Copilot),
    ),
    retired("amp", "Amp", Skills::Shared, None),
    retired(
        "factory-droid",
        "Factory Droid",
        Skills::Shared,
        Some(GuidanceAgent::Droid),
    ),
    retired(
        "kiro",
        "Kiro",
        Skills::Own(".kiro"),
        Some(GuidanceAgent::Kiro),
    ),
    retired(
        "qwen-code",
        "Qwen Code",
        Skills::Own(".qwen"),
        Some(GuidanceAgent::Qwen),
    ),
    retired("goose", "Goose", Skills::Shared, None),
    retired("cline", "Cline", Skills::Own(".cline"), None),
    retired("kilo-code", "Kilo Code", Skills::Shared, None),
    retired("crush", "Crush", Skills::Shared, None),
    retired("junie", "Junie", Skills::Shared, Some(GuidanceAgent::Junie)),
    retired(
        "augment",
        "Augment",
        Skills::Shared,
        Some(GuidanceAgent::Augment),
    ),
    retired("kimi-code", "Kimi Code", Skills::Shared, None),
    retired("mistral-vibe", "Mistral Vibe", Skills::Shared, None),
    retired(
        "gemini-cli",
        "Gemini CLI",
        Skills::Shared,
        Some(GuidanceAgent::Gemini),
    ),
];

fn own_skill_code(folder: &str) -> String {
    format!("skill:{}", folder.trim_start_matches('.'))
}

fn own_skill_root(target: &KitTarget, folder: &str) -> PathBuf {
    target.home.join(folder).join("skills")
}

/// What one pass did.
#[derive(Default)]
pub(crate) struct Outcome {
    pub(crate) report: Retirement,
    /// The record changed and must be saved.
    pub(crate) changed: bool,
}

/// Takes out what the record says Hide put down for each retired agent that
/// is not marked retired yet. `live_reads_shared` says that an agent that is
/// on and installed still reads the shared skills folder, whose stub then
/// stays. A pass whose record could not be read stops before this
/// (`record::load`): without it the kit cannot tell its own files from the
/// operator's (engineering rule 4).
pub(crate) fn retire(target: &KitTarget, record: &mut Record, live_reads_shared: bool) -> Outcome {
    let mut outcome = Outcome::default();
    for agent in &RETIRED {
        let hook_code = format!("hook:{}", agent.id);
        let own_skill = match agent.skills {
            Skills::Own(folder) => Some(own_skill_code(folder)),
            Skills::Shared => None,
        };
        // Nothing in the record names this agent: nothing of it is Hide's.
        let owned = record.agent_choice(agent.id).is_some()
            || (agent.hook.is_some() && record.contains_piece(&hook_code))
            || own_skill
                .as_deref()
                .is_some_and(|code| record.contains_piece(code));
        if !owned {
            continue;
        }
        if let Some(hook) = agent.hook
            && record.contains_piece(&hook_code)
        {
            match hide_agent_hooks::guidance::remove(hook, &target.home) {
                Ok(removed) => {
                    if removed.removed_entries > 0 {
                        outcome.report.removed.push(format!("{} hook", agent.label));
                    }
                    outcome.changed |= record.forget_piece(&hook_code);
                }
                Err(failure) => {
                    outcome.report.failures.push(format!(
                        "{} hook: {}",
                        agent.label,
                        failure.message()
                    ));
                    continue;
                }
            }
        }
        if let Skills::Own(folder) = agent.skills
            && let Some(code) = &own_skill
            && record.contains_piece(code)
        {
            match remove_skill_in(&own_skill_root(target, folder)) {
                Ok(removed) => {
                    if removed {
                        outcome
                            .report
                            .removed
                            .push(format!("{} skill", agent.label));
                    }
                    outcome.changed |= record.forget_piece(code);
                }
                Err(reason) => {
                    outcome
                        .report
                        .failures
                        .push(format!("{} skill: {reason}", agent.label));
                    continue;
                }
            }
        }
        outcome.changed |= record.forget_agent_choice(agent.id);
    }
    // The shared stub was Hide's for the agents that read it. It stays while
    // an agent that is still supported reads it; otherwise nothing Hide
    // switched on needs it. This is judged from what the record still names,
    // not from which retired agent was on in this pass: a stub that could not
    // be removed keeps its record entry, so every later pass tries again even
    // though the retired agents that owned it have left the record.
    if !live_reads_shared && record.contains_piece(&format!("skill:{}", SkillDir::Shared.code())) {
        match remove_skill(SkillDir::Shared, &target.home) {
            Ok(removed) => {
                if removed {
                    outcome.report.removed.push("shared skill".to_owned());
                }
                outcome.changed |=
                    record.forget_piece(&format!("skill:{}", SkillDir::Shared.code()));
            }
            Err(reason) => outcome
                .report
                .failures
                .push(format!("shared skill: {reason}")),
        }
    }
    outcome
}

/// What taking Hide off a machine removes for the retired agents: their
/// marked hook entries and their own skill folders.
pub(crate) fn remove(target: &KitTarget) -> Vec<(String, RemoveOutcome)> {
    let mut outcomes = Vec::new();
    for agent in &RETIRED {
        if let Some(hook) = agent.hook {
            let outcome = match hide_agent_hooks::guidance::remove(hook, &target.home) {
                Ok(done) if done.removed_entries > 0 => RemoveOutcome::Removed,
                Ok(_) => RemoveOutcome::Absent,
                Err(failure) => RemoveOutcome::Failed {
                    reason: failure.message(),
                },
            };
            outcomes.push((format!("hook:{}", agent.id), outcome));
        }
        if let Skills::Own(folder) = agent.skills {
            let outcome = match remove_skill_in(&own_skill_root(target, folder)) {
                Ok(true) => RemoveOutcome::Removed,
                Ok(false) => RemoveOutcome::Absent,
                Err(reason) => RemoveOutcome::Failed { reason },
            };
            outcomes.push((own_skill_code(folder), outcome));
        }
    }
    outcomes
}
