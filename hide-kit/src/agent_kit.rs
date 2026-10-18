//! What the kit does for each agent the operator switched on: the skill stub
//! and, for an agent with a guidance hook, that hook (issue #517), and for
//! OpenCode, Pi and omp, the script file of Hide's in their own folder.
//!
//! It follows the rules every other part follows (`crate` docs): what
//! another tool wrote is never touched, a piece that failed does not stop
//! the others and says why, and a piece the kit installed and the operator
//! then took away stays away until Reinstall. The record's `agents` map is
//! the operator's choice; its `installed` set holds `hook:<agent>`,
//! `skill:<folder>` and `herdr:<agent>` (Herdr's own integration, installed
//! through the machine's Herdr CLI) for what the kit has put down.
//!
//! Claude Code and Codex keep their hook as a kit part ([`HookSupport::Part`])
//! because Memory, pane diagnosis and the device rows read it there. Their
//! switch gates that part here ([`part_gate`]); everything else about the
//! part is the code that already ran.

use std::path::Path;

use hide_agent_adapter::PluginDialect;
use hide_agent_hooks::codex_trust::{HookEntry, learn_herdr_entries};
use hide_agent_hooks::guidance::GuidanceAgent;
use hide_agent_hooks::plugin::{self, PluginObserved};
use hide_agent_hooks::route::HookTarget;
use hide_agent_hooks::{HookStatus, InstallFailure};

use crate::agents::{
    ADAPTERS, AgentAdapter, AgentReport, Detection, HookSupport, PieceReport, SkillDir,
    SkillObserved, adapter_of_part, availability, install_skill, observe_skill, remove_skill,
    skill_location,
};
use crate::herdr_integration::{self, Integration, Statuses};
use crate::record::Record;
use crate::{
    ComponentId, ComponentState, KitTarget, Observed, RemoveOutcome, Retirement, Scope,
    retired_agents,
};

/// A kit part's id, state, reason and location, as the agent whose hook it
/// is reports it.
pub(crate) type PartView = (ComponentId, ComponentState, Option<String>, Option<String>);

fn hook_code(adapter: &AgentAdapter) -> String {
    format!("hook:{}", adapter.id)
}

fn skill_code(dir: SkillDir) -> String {
    format!("skill:{}", dir.code())
}

fn herdr_code(adapter: &AgentAdapter) -> String {
    format!("herdr:{}", adapter.id)
}

/// Whether the agent is on: the operator's choice made in `scope` now, else
/// the one on record, else what its adapter does without a choice.
pub(crate) fn enabled(record: &Record, scope: &Scope, adapter: &AgentAdapter) -> bool {
    if scope.agent_off.contains(adapter.id) {
        return false;
    }
    if scope.agent_on.contains(adapter.id) {
        return true;
    }
    record
        .agent_choice(adapter.id)
        .unwrap_or(adapter.default_on)
}

/// A machine the kit has never been applied to waits for the operator's first
/// choice before any agent gets anything: the agents that are on by default
/// are recorded off, the record is marked as awaiting the choice, and an
/// operator's switch on in the same pass still wins. A machine with a record
/// keeps what it had, so an upgrade changes nothing. The mark lives in the
/// record, so it outlasts a quit between the hold and the answer.
/// True when this pass changed the record.
pub(crate) fn hold_for_onboarding(record: &mut Record) -> bool {
    if !record.is_fresh() {
        return false;
    }
    for adapter in ADAPTERS.iter().filter(|adapter| adapter.default_on) {
        record.set_agent_choice(adapter.id, false);
    }
    record.await_choice()
}

/// A scope that names an agent is the operator's explicit choice, which
/// answers the first-run question (the choices themselves are recorded by
/// [`apply`]).
fn answers_choice(scope: &Scope) -> bool {
    !scope.agent_on.is_empty() || !scope.agent_off.is_empty()
}

/// What the agent switch says about the kit part that is its hook.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PartGate {
    /// The agent is on, so the part may be installed.
    pub(crate) enabled: bool,
    /// The operator is switching the agent off in this scope.
    pub(crate) turning_off: bool,
    /// The operator is switching the agent on in this scope.
    pub(crate) turning_on: bool,
}

/// The gate of a part an agent switch governs, `None` for any other part.
/// The hook parts follow their agent's switch in every way.
pub(crate) fn part_gate(record: &Record, scope: &Scope, part: ComponentId) -> Option<PartGate> {
    let adapter = adapter_of_part(part)?;
    Some(PartGate {
        enabled: enabled(record, scope, adapter),
        turning_off: scope.agent_off.contains(adapter.id),
        turning_on: scope.agent_on.contains(adapter.id),
    })
}

/// Whether a part that is gone is the operator's switch rather than a
/// removal by hand: only when the agent that governs it is off.
pub(crate) fn part_is_off(record: &Record, part: ComponentId) -> bool {
    part_gate(record, &Scope::default(), part).is_some_and(|gate| !gate.enabled)
}

// --- A guidance hook -----------------------------------------------------------

fn helper(target: &KitTarget) -> std::path::PathBuf {
    target.kit_dir.join(hide_agent_hooks::HELPER_BINARY_NAME)
}

fn observe_guidance(target: &KitTarget, adapter: &AgentAdapter, agent: GuidanceAgent) -> Observed {
    use hide_agent_hooks::guidance;
    if let Err(why) = agent.supported_here() {
        return Observed::Unsupported(format!(
            "{}'s hook is not written here: {why}",
            adapter.label
        ));
    }
    let observed = match guidance::status(agent, &target.home) {
        // Installed, but the agent has not made its settings folder yet; the
        // hook goes in on the pass after it has.
        HookStatus::RuntimeAbsent => {
            return Observed::Absent(format!(
                "{} has not created {} yet; the hook is put in once it has",
                adapter.label,
                agent.home_directory(&target.home).display()
            ));
        }
        // Reading a file never says Off: the kit's record does (`HookStatus::Off`).
        HookStatus::NotInstalled | HookStatus::Off => Observed::Missing,
        HookStatus::Outdated { version } => Observed::Stale(format!(
            "version {version} of the hook is there; this build writes version {}",
            agent.version()
        )),
        HookStatus::Failed {
            reason: InstallFailure::HelperMissing { helper, .. },
        } => Observed::Stale(format!("the hook points at {helper}, which is gone")),
        HookStatus::Failed { reason } => return Observed::Blocked(reason.message()),
        HookStatus::Installed { .. } => {
            let wanted = helper(target);
            let state = crate::layout::state_dir_from_process(&target.home);
            match guidance::installed_helper_path(agent, &target.home) {
                Some(found) if Path::new(&found) != wanted => Observed::Stale(format!(
                    "the hook runs {found}, not this build's {}",
                    wanted.display()
                )),
                // Hide AI's agy folder goes in with agy's hook, so a pass
                // after it went missing or lost agy's trust writes it again.
                Some(_)
                    if agent == GuidanceAgent::Antigravity
                        && hide_agent_hooks::antigravity_trust::read_only_due(&target.home) =>
                {
                    Observed::Stale("Hide AI's tool-free folder for agy is missing or untrusted".to_owned())
                }
                Some(_) if guidance::matches_install(agent, &target.home, HookTarget::new(&wanted, &state)) => {
                    Observed::Current
                }
                Some(_) if guidance::matches_helper(agent, &target.home, &wanted) => {
                    Observed::Stale("the hook's captured state route changed".to_owned())
                }
                // Hide's marker over an entry Hide did not write: the operator
                // edited it, and it is theirs until Reinstall.
                Some(_) => Observed::Edited(
                    "Hide's hook entries were edited after Hide wrote them; Reinstall puts Hide's back"
                        .to_owned(),
                ),
                None => Observed::Stale("the hook's command is not one Hide wrote".to_owned()),
            }
        }
    };
    if matches!(observed, Observed::Current) {
        return observed;
    }
    if !helper(target).is_file() {
        return Observed::Blocked(format!(
            "this build has no hook helper at {}",
            helper(target).display()
        ));
    }
    observed
}

fn install_guidance(target: &KitTarget, agent: GuidanceAgent) -> Result<(), String> {
    let helper = helper(target);
    let state = crate::layout::state_dir_from_process(&target.home);
    hide_agent_hooks::guidance::install(agent, &target.home, HookTarget::new(&helper, &state))
        .map(|_| ())
        .map_err(|failure| failure.message())
}

fn remove_guidance(target: &KitTarget, agent: GuidanceAgent) -> RemoveOutcome {
    match hide_agent_hooks::guidance::remove(agent, &target.home) {
        Ok(outcome) if outcome.removed_entries > 0 => RemoveOutcome::Removed,
        Ok(_) => RemoveOutcome::Absent,
        Err(failure) => RemoveOutcome::Failed {
            reason: failure.message(),
        },
    }
}

// --- OpenCode's plugin, Pi's and omp's extension ---------------------------------

fn remove_plugin(target: &KitTarget, dialect: PluginDialect) -> RemoveOutcome {
    match plugin::of(dialect).remove(&target.home) {
        Ok(true) => RemoveOutcome::Removed,
        Ok(false) => RemoveOutcome::Absent,
        Err(reason) => RemoveOutcome::Failed { reason },
    }
}

/// The plugin or extension piece of an agent that is on and installed here.
fn plugin_state(
    target: &KitTarget,
    adapter: &AgentAdapter,
    dialect: PluginDialect,
    recorded: bool,
) -> (ComponentState, Option<String>) {
    let file = plugin::of(dialect);
    let noun = file.noun();
    let helper = helper(target);
    let state = crate::layout::state_dir_from_process(&target.home);
    let observed = file.observe(&target.home, HookTarget::new(&helper, &state));
    if !matches!(
        observed,
        PluginObserved::Current | PluginObserved::ConfigAbsent | PluginObserved::Foreign(_)
    ) && !helper.is_file()
    {
        return (
            ComponentState::Failed,
            Some(format!(
                "this build has no hook helper at {}",
                helper.display()
            )),
        );
    }
    match observed {
        PluginObserved::Current => (ComponentState::Installed, None),
        PluginObserved::Stale(reason) => (ComponentState::Outdated, Some(reason)),
        PluginObserved::Edited => (
            ComponentState::Outdated,
            Some(format!(
                "the {noun} was edited after Hide wrote it; Reinstall puts Hide's back"
            )),
        ),
        PluginObserved::Missing if recorded => (
            ComponentState::Removed,
            Some("taken out after Hide installed it; Reinstall puts it back".to_owned()),
        ),
        PluginObserved::Missing => (ComponentState::NotInstalled, None),
        PluginObserved::ConfigAbsent => (
            ComponentState::Absent,
            Some(format!(
                "{} has not created {} yet; the {noun} is put in once it has",
                adapter.label,
                file.config_directory(&target.home).display()
            )),
        ),
        PluginObserved::Foreign(name) => (
            ComponentState::Absent,
            Some(format!(
                "a {noun} named {name} that Hide did not write is already there; Hide left it alone"
            )),
        ),
        PluginObserved::Unreadable(reason) => (ComponentState::Failed, Some(reason)),
    }
}

fn plugin_piece(
    target: &KitTarget,
    adapter: &AgentAdapter,
    dialect: PluginDialect,
    record: &Record,
    on: bool,
    detection: &Detection,
    failures: Option<&AgentFailures>,
) -> PieceReport {
    let file = plugin::of(dialect);
    let location = Some(file.path(&target.home).display().to_string());
    let failure = failures.and_then(|failures| failures.hook.get(adapter.id).cloned());
    let (state, reason) = if !on {
        // Hide's unedited file still there after the switch-off: the removal
        // failed or never ran, and the row must not read Off over it. An edited
        // one is the operator's and stays by design.
        let left = failure.or_else(|| {
            (record.agent_choice(adapter.id) == Some(false)
                && matches!(
                    file.observe(
                        &target.home,
                        HookTarget::new(
                            &helper(target),
                            &crate::layout::state_dir_from_process(&target.home)
                        )
                    ),
                    PluginObserved::Current | PluginObserved::Stale(_)
                ))
            .then(|| {
                format!(
                    "Hide's {} is still in place; switch the agent on and off again to remove it",
                    file.noun()
                )
            })
        });
        match left {
            Some(reason) => (ComponentState::Failed, Some(reason)),
            None => (ComponentState::Off, None),
        }
    } else if !detection.installed(adapter) {
        return PieceReport {
            state: ComponentState::Absent,
            reason: Some(not_found(adapter)),
            location: None,
        };
    } else if let Some(reason) = failure {
        (ComponentState::Failed, Some(reason))
    } else {
        plugin_state(
            target,
            adapter,
            dialect,
            record.contains_piece(&hook_code(adapter)),
        )
    };
    PieceReport {
        state,
        reason,
        location,
    }
}

// --- One pass ------------------------------------------------------------------

/// Why an agent that is on has nothing put in place: none of its programs is
/// found here.
fn not_found(adapter: &AgentAdapter) -> String {
    let programs: Vec<String> = adapter
        .executables
        .iter()
        .map(|name| format!("`{name}`"))
        .collect();
    format!("{} is not found on this machine", programs.join(" or "))
}

fn dir_state(
    observed: &SkillObserved,
    recorded: bool,
    failure: Option<String>,
) -> (ComponentState, Option<String>) {
    if let Some(reason) = failure {
        return (ComponentState::Failed, Some(reason));
    }
    match observed {
        SkillObserved::Current => (ComponentState::Installed, None),
        SkillObserved::Stale => (
            ComponentState::Outdated,
            Some("an older version of the skill is there".to_owned()),
        ),
        SkillObserved::Edited => (
            ComponentState::Outdated,
            Some(
                "the skill was edited after Hide wrote it; Reinstall puts Hide's text back"
                    .to_owned(),
            ),
        ),
        SkillObserved::Missing if recorded => (
            ComponentState::Removed,
            Some("taken out after Hide installed it; Reinstall puts it back".to_owned()),
        ),
        SkillObserved::Missing => (ComponentState::NotInstalled, None),
        SkillObserved::Foreign => (
            ComponentState::Absent,
            Some(format!(
                "a skill named {} that Hide did not write is already there; Hide left it alone",
                crate::agents::SKILL_NAME
            )),
        ),
        SkillObserved::Unreadable(reason) => (ComponentState::Failed, Some(reason.clone())),
    }
}

/// Judges every agent without changing anything.
pub(crate) fn status(target: &KitTarget, record: &Record, parts: &[PartView]) -> Vec<AgentReport> {
    let scope = Scope::default();
    let detection = Detection::probe(target);
    let dirs = observe_dirs(target, record, &scope, &detection);
    let herdr = Statuses::probe(target);
    ADAPTERS
        .iter()
        .map(|adapter| {
            let on = enabled(record, &scope, adapter);
            report_agent(
                target, adapter, record, on, &dirs, &detection, &herdr, None, parts,
            )
        })
        .collect()
}

/// The skill folders' states, with a folder wanted only while an agent that
/// reads it is on and installed here.
fn observe_dirs(
    target: &KitTarget,
    record: &Record,
    scope: &Scope,
    detection: &Detection,
) -> Vec<(SkillDir, SkillObserved, bool)> {
    SkillDir::ALL
        .into_iter()
        .map(|dir| {
            let wanted = ADAPTERS.iter().any(|adapter| {
                adapter.skill_dir == dir
                    && enabled(record, scope, adapter)
                    && detection.installed(adapter)
                    && dir.writable(&target.home)
            });
            (dir, observe_skill(dir, &target.home), wanted)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn report_agent(
    target: &KitTarget,
    adapter: &AgentAdapter,
    record: &Record,
    on: bool,
    dirs: &[(SkillDir, SkillObserved, bool)],
    detection: &Detection,
    herdr: &HerdrView,
    failures: Option<&AgentFailures>,
    parts: &[PartView],
) -> AgentReport {
    let installed = detection.installed(adapter);
    let availability = availability(installed);
    let skill = if !on {
        let location = skill_location(adapter.skill_dir, &target.home);
        // A switch-off whose removal failed, or that left Hide's stub behind,
        // is not Off: the stub is still read by the agent.
        let left = failures
            .and_then(|failures| failures.skill.get(&adapter.skill_dir).cloned())
            .or_else(|| left_behind(record, adapter, dirs, &location));
        match left {
            Some(reason) => PieceReport {
                state: ComponentState::Failed,
                reason: Some(reason),
                location: Some(location),
            },
            None => PieceReport {
                state: ComponentState::Off,
                reason: None,
                location: Some(location),
            },
        }
    } else if !installed {
        // On, and its program is gone or was never here: a pass leaves what
        // Hide put down in place, and the switch still takes it out.
        PieceReport {
            state: ComponentState::Absent,
            reason: Some(not_found(adapter)),
            location: None,
        }
    } else {
        let observed = dirs
            .iter()
            .find(|(dir, _, _)| *dir == adapter.skill_dir)
            .map(|(_, observed, _)| observed.clone())
            .unwrap_or(SkillObserved::Missing);
        let recorded = record.contains_piece(&skill_code(adapter.skill_dir));
        let failure = failures.and_then(|failures| failures.skill.get(&adapter.skill_dir).cloned());
        let (state, reason) = if observed == SkillObserved::Missing
            && failure.is_none()
            && !adapter.skill_dir.writable(&target.home)
        {
            // Hide does not create an agent's own folder, so a Reinstall
            // cannot help until the agent has made it.
            (
                ComponentState::Absent,
                Some(format!(
                    "{} has not created its own folder yet; the skill is put in once it has",
                    adapter.label
                )),
            )
        } else {
            dir_state(&observed, recorded, failure)
        };
        PieceReport {
            state,
            reason,
            location: Some(skill_location(adapter.skill_dir, &target.home)),
        }
    };
    let hook = match adapter.hook {
        HookSupport::None => None,
        HookSupport::Part(part) => {
            let found = parts.iter().find(|(id, ..)| *id == part);
            Some(match found {
                Some((_, state, reason, location)) => PieceReport {
                    state: *state,
                    reason: reason.clone(),
                    location: location.clone(),
                },
                None => PieceReport {
                    state: ComponentState::Absent,
                    reason: None,
                    location: None,
                },
            })
        }
        HookSupport::Guidance(agent) => Some(guidance_piece(
            target, adapter, agent, record, on, detection, failures,
        )),
        HookSupport::Plugin(dialect) => Some(plugin_piece(
            target, adapter, dialect, record, on, detection, failures,
        )),
    };
    let herdr = Some(herdr_piece(
        target, adapter, record, on, detection, herdr, failures,
    ));
    AgentReport {
        id: adapter.id.to_owned(),
        label: adapter.label.to_owned(),
        availability,
        enabled: on,
        chosen: record.agent_choice(adapter.id).is_some(),
        skill,
        hook,
        herdr,
        doc_url: adapter.doc_url.to_owned(),
    }
}

/// Herdr's integrations as one pass read them: `None` for a machine with no
/// Herdr CLI Hide can find, else the listing or why it could not be read.
type HerdrView = Option<Result<Statuses, String>>;

/// One agent's Herdr integration as the row states it.
fn herdr_piece(
    target: &KitTarget,
    adapter: &AgentAdapter,
    record: &Record,
    on: bool,
    detection: &Detection,
    herdr: &HerdrView,
    failures: Option<&AgentFailures>,
) -> PieceReport {
    let integration = adapter.herdr;
    let folder = integration
        .folder
        .iter()
        .fold(target.home.clone(), |path, part| path.join(part));
    let location = Some(folder.display().to_string());
    let code = herdr_code(adapter);
    let recorded = record.contains_piece(&code);
    let listed = match herdr {
        Some(Ok(statuses)) => statuses.of(integration.target),
        _ => None,
    };
    let piece = |state, reason: Option<String>| PieceReport {
        state,
        reason,
        location: location.clone(),
    };
    let failure = failures.and_then(|failures| failures.herdr.get(adapter.id).cloned());
    if !on {
        // Hide's own integration is still there after the switch-off: the
        // removal failed or never ran, and the row must not read Off over it.
        let left = failure.or_else(|| {
            (record.agent_choice(adapter.id) == Some(false)
                && recorded
                && matches!(listed, Some(Integration::Current | Integration::Outdated)))
            .then(|| {
                "Hide's Herdr integration is still in place; switch the agent on and off again to remove it".to_owned()
            })
        });
        return match left {
            Some(reason) => piece(ComponentState::Failed, Some(reason)),
            None => piece(ComponentState::Off, None),
        };
    }
    if !detection.installed(adapter) {
        return PieceReport {
            state: ComponentState::Absent,
            reason: Some(not_found(adapter)),
            location: None,
        };
    }
    if let Some(reason) = failure {
        return piece(ComponentState::Failed, Some(reason));
    }
    match herdr {
        None => piece(
            ComponentState::Absent,
            Some("Herdr is not found on this machine".to_owned()),
        ),
        Some(Err(reason)) => piece(ComponentState::Failed, Some(reason.clone())),
        Some(Ok(_)) if !folder.is_dir() => piece(
            ComponentState::Absent,
            Some(format!(
                "{} has not created its own folder yet; Herdr's integration is put in once it has",
                adapter.label
            )),
        ),
        Some(Ok(_)) => match listed {
            None => piece(
                ComponentState::Absent,
                Some(format!(
                    "this Herdr has no integration for {}; update Herdr to get it",
                    adapter.label
                )),
            ),
            Some(Integration::Current) => piece(ComponentState::Installed, None),
            // An integration Hide did not install is the operator's: it is
            // judged present and never replaced.
            Some(Integration::Outdated) if !recorded => piece(ComponentState::Installed, None),
            Some(Integration::Outdated) => piece(
                ComponentState::Outdated,
                Some("an older version of Herdr's integration is there".to_owned()),
            ),
            Some(Integration::Missing) if recorded => piece(
                ComponentState::Removed,
                Some("taken out after Hide installed it; Reinstall puts it back".to_owned()),
            ),
            Some(Integration::Missing) => piece(ComponentState::NotInstalled, None),
        },
    }
}

/// Hide's skill stub is still in the folder of an agent the operator switched
/// off, with no agent that is on and installed reading it: the removal failed
/// or never ran, and the row must not read Off over it.
fn left_behind(
    record: &Record,
    adapter: &AgentAdapter,
    dirs: &[(SkillDir, SkillObserved, bool)],
    location: &str,
) -> Option<String> {
    if record.agent_choice(adapter.id) != Some(false)
        || !record.contains_piece(&skill_code(adapter.skill_dir))
    {
        return None;
    }
    let (_, observed, wanted) = dirs.iter().find(|(dir, _, _)| *dir == adapter.skill_dir)?;
    (!wanted && matches!(observed, SkillObserved::Current | SkillObserved::Stale)).then(|| {
        format!(
            "Hide's skill is still in {location}; switch the agent on and off again to remove it"
        )
    })
}

fn guidance_piece(
    target: &KitTarget,
    adapter: &AgentAdapter,
    agent: GuidanceAgent,
    record: &Record,
    on: bool,
    detection: &Detection,
    failures: Option<&AgentFailures>,
) -> PieceReport {
    let location = Some(agent.config_path(&target.home).display().to_string());
    if !on {
        let left = failures
            .and_then(|failures| failures.hook.get(adapter.id).cloned())
            .or_else(|| {
                matches!(
                    hide_agent_hooks::guidance::status(agent, &target.home),
                    HookStatus::Installed { .. } | HookStatus::Outdated { .. }
                )
                .then(|| {
                    "Hide's entry is still in the file; switch the agent on and off again to remove it"
                        .to_owned()
                })
            });
        return match left {
            Some(reason) => PieceReport {
                state: ComponentState::Failed,
                reason: Some(reason),
                location,
            },
            None => PieceReport {
                state: ComponentState::Off,
                reason: None,
                location,
            },
        };
    }
    if !detection.installed(adapter) {
        return PieceReport {
            state: ComponentState::Absent,
            reason: Some(not_found(adapter)),
            location: None,
        };
    }
    let recorded = record.contains_piece(&hook_code(adapter));
    let (state, reason) = match failures.and_then(|failures| failures.hook.get(adapter.id)) {
        Some(reason) => (ComponentState::Failed, Some(reason.clone())),
        None => match observe_guidance(target, adapter, agent) {
            Observed::Current => (ComponentState::Installed, None),
            Observed::Stale(reason) | Observed::Edited(reason) => {
                (ComponentState::Outdated, Some(reason))
            }
            Observed::Missing if recorded => (
                ComponentState::Removed,
                Some("taken out after Hide installed it; Reinstall puts it back".to_owned()),
            ),
            Observed::Missing => (ComponentState::NotInstalled, None),
            Observed::Blocked(reason) => (ComponentState::Failed, Some(reason)),
            Observed::Absent(reason) | Observed::Unsupported(reason) => {
                (ComponentState::Absent, Some(reason))
            }
        },
    };
    PieceReport {
        state,
        reason,
        location,
    }
}

/// Why a piece that was tried failed, by agent and folder.
#[derive(Default)]
struct AgentFailures {
    hook: std::collections::BTreeMap<&'static str, String>,
    skill: std::collections::BTreeMap<SkillDir, String>,
    herdr: std::collections::BTreeMap<&'static str, String>,
}

/// What a pass did to the agents' own pieces, before anyone reports it: the
/// pass sets Codex's trust between this and [`reports`], because Herdr's
/// integration, which the trust covers, is only in place once this has run.
pub(crate) struct Applied {
    failures: AgentFailures,
    herdr: HerdrView,
    detection: Detection,
    pub(crate) changed: bool,
    pub(crate) retirement: Retirement,
}

impl Applied {
    /// Whether Herdr reports its integration `name` current right now, which
    /// is the only state in which what the kit recorded of it is trusted.
    pub(crate) fn herdr_current(&self, name: &str) -> bool {
        matches!(&self.herdr, Some(Ok(statuses)) if statuses.of(name) == Some(Integration::Current))
    }
}

/// Applies the operator's per-agent choices in `scope` and what the pass
/// owes every agent that is on.
pub(crate) fn apply(target: &KitTarget, scope: &Scope, record: &mut Record) -> Applied {
    let mut changed = false;
    let mut failures = AgentFailures::default();
    // Looked for once: every decision below reads this, not the PATH.
    let detection = Detection::probe(target);

    // The agents Hide stopped supporting: what the record says an earlier
    // build put down for them goes first, so the shared skill folder below is
    // judged for the agents that are left.
    // An agent the operator switched on keeps its pieces whether or not its
    // program is found (D-20, D-26), so the stub stays for it too; one that is
    // only on by default needs its program found to count.
    let live_reads_shared = ADAPTERS.iter().any(|adapter| {
        adapter.skill_dir == SkillDir::Shared
            && enabled(record, scope, adapter)
            && (detection.installed(adapter) || record.agent_choice(adapter.id) == Some(true))
    });
    let retirement = retired_agents::retire(target, record, live_reads_shared);
    changed |= retirement.changed;

    // What the operator chose now is the record's, whatever else this pass
    // can or cannot do for the agent. A switch on for an agent that is not
    // installed here has nothing to switch.
    if answers_choice(scope) {
        changed |= record.answer_choice();
    }
    for adapter in ADAPTERS {
        if scope.agent_on.contains(adapter.id) && detection.installed(adapter) {
            changed |= record.set_agent_choice(adapter.id, true);
        } else if scope.agent_off.contains(adapter.id) {
            changed |= record.set_agent_choice(adapter.id, false);
        }
    }

    // Guidance hooks.
    for adapter in ADAPTERS {
        let HookSupport::Guidance(agent) = adapter.hook else {
            continue;
        };
        let on = enabled(record, scope, adapter);
        let code = hook_code(adapter);
        if !on {
            if scope.agent_off.contains(adapter.id) {
                match hide_agent_hooks::guidance::remove(agent, &target.home) {
                    Ok(_) => changed |= record.forget_piece(&code),
                    Err(failure) => {
                        failures.hook.insert(adapter.id, failure.message());
                    }
                }
            }
            continue;
        }
        // An agent whose program is gone keeps the hook Hide wrote: only the
        // operator's switch takes it out (D-20, D-26).
        if !detection.installed(adapter) {
            continue;
        }
        let observed = observe_guidance(target, adapter, agent);
        let recorded = record.contains_piece(&code);
        let restore = scope.agent_on.contains(adapter.id);
        let install_now = match &observed {
            Observed::Stale(_) => true,
            // The operator's edit is theirs until Reinstall.
            Observed::Edited(_) => restore,
            Observed::Missing => restore || !recorded,
            _ => false,
        };
        if install_now && let Err(reason) = install_guidance(target, agent) {
            failures.hook.insert(adapter.id, reason);
            continue;
        }
        let after = if install_now {
            observe_guidance(target, adapter, agent)
        } else {
            observed
        };
        if matches!(after, Observed::Current) {
            changed |= record.insert_piece(&code);
        }
    }

    // OpenCode's plugin, Pi's and omp's extension: a file of Hide's own,
    // kept like the skill stub.
    for adapter in ADAPTERS {
        let HookSupport::Plugin(dialect) = adapter.hook else {
            continue;
        };
        let file = plugin::of(dialect);
        let code = hook_code(adapter);
        if !enabled(record, scope, adapter) {
            if scope.agent_off.contains(adapter.id) {
                match file.remove(&target.home) {
                    Ok(_) => changed |= record.forget_piece(&code),
                    Err(reason) => {
                        failures.hook.insert(adapter.id, reason);
                    }
                }
            }
            continue;
        }
        // An agent whose program is gone keeps the file Hide wrote: only the
        // operator's switch takes it out (D-20, D-26).
        if !detection.installed(adapter) {
            continue;
        }
        let helper = helper(target);
        let state = crate::layout::state_dir_from_process(&target.home);
        let hook_target = HookTarget::new(&helper, &state);
        let observed = file.observe(&target.home, hook_target);
        let recorded = record.contains_piece(&code);
        let restore = scope.agent_on.contains(adapter.id);
        let install_now = helper.is_file()
            && match &observed {
                PluginObserved::Stale(_) => true,
                // The operator's edit is theirs until Reinstall.
                PluginObserved::Edited => restore,
                PluginObserved::Missing => restore || !recorded,
                _ => false,
            };
        if install_now && let Err(reason) = file.install(&target.home, hook_target) {
            failures.hook.insert(adapter.id, reason);
            continue;
        }
        let current = if install_now {
            file.observe(&target.home, hook_target) == PluginObserved::Current
        } else {
            observed == PluginObserved::Current
        };
        if current {
            changed |= record.insert_piece(&code);
        }
    }

    // Skill folders, shared by every agent that reads one.
    for dir in SkillDir::ALL {
        let readers = || {
            ADAPTERS
                .iter()
                .filter(move |adapter| adapter.skill_dir == dir)
        };
        let wanted = readers().any(|adapter| {
            enabled(record, scope, adapter)
                && detection.installed(adapter)
                && dir.writable(&target.home)
        });
        let switched_off = readers().any(|adapter| scope.agent_off.contains(adapter.id));
        let switched_on = readers()
            .any(|adapter| scope.agent_on.contains(adapter.id) && detection.installed(adapter));
        let code = skill_code(dir);
        if wanted {
            let observed = observe_skill(dir, &target.home);
            let recorded = record.contains_piece(&code);
            let install_now = match &observed {
                SkillObserved::Stale => true,
                // The operator's edit is theirs until Reinstall.
                SkillObserved::Edited => switched_on,
                SkillObserved::Missing => switched_on || !recorded,
                _ => false,
            };
            if install_now && let Err(reason) = install_skill(dir, &target.home) {
                failures.skill.insert(dir, reason);
                continue;
            }
            let after = if install_now {
                observe_skill(dir, &target.home)
            } else {
                observed
            };
            if matches!(after, SkillObserved::Current) {
                changed |= record.insert_piece(&code);
            }
        } else if switched_off {
            match remove_skill(dir, &target.home) {
                Ok(_) => changed |= record.forget_piece(&code),
                Err(reason) => {
                    failures.skill.insert(dir, reason);
                }
            }
        }
    }

    let herdr = apply_herdr(
        target,
        scope,
        record,
        &detection,
        &mut failures,
        &mut changed,
    );

    Applied {
        failures,
        herdr,
        detection,
        changed,
        retirement: retirement.report,
    }
}

/// Every agent after a pass, as [`apply`] left it. `parts` are the kit parts'
/// reports, for the agents whose hook is one.
pub(crate) fn reports(
    target: &KitTarget,
    scope: &Scope,
    record: &Record,
    applied: &Applied,
    parts: &[PartView],
) -> Vec<AgentReport> {
    let dirs = observe_dirs(target, record, scope, &applied.detection);
    ADAPTERS
        .iter()
        .map(|adapter| {
            let on = enabled(record, scope, adapter);
            report_agent(
                target,
                adapter,
                record,
                on,
                &dirs,
                &applied.detection,
                &applied.herdr,
                Some(&applied.failures),
                parts,
            )
        })
        .collect()
}

/// Herdr's integrations: installed for an agent that is on and found, taken
/// out for an agent switched off now, and only ever what the record says Hide
/// installed. Answers the machine's integrations as they are afterwards.
fn apply_herdr(
    target: &KitTarget,
    scope: &Scope,
    record: &mut Record,
    detection: &Detection,
    failures: &mut AgentFailures,
    changed: &mut bool,
) -> HerdrView {
    let before = Statuses::probe(target);
    let mut installed_now: Vec<(&AgentAdapter, Option<HookFiles>)> = Vec::new();
    let mut touched = false;
    for adapter in ADAPTERS {
        let integration = adapter.herdr;
        let code = herdr_code(adapter);
        let recorded = record.contains_piece(&code);
        if !enabled(record, scope, adapter) {
            if !(scope.agent_off.contains(adapter.id) && recorded) {
                continue;
            }
            match &before {
                Some(Ok(statuses))
                    if statuses.of(integration.target) == Some(Integration::Missing) =>
                {
                    *changed |= record.forget_piece(&code);
                }
                Some(Ok(_)) => match herdr_integration::uninstall(target, integration.target) {
                    Ok(()) => {
                        *changed |= record.forget_piece(&code);
                        touched = true;
                    }
                    Err(reason) => {
                        failures.herdr.insert(adapter.id, reason);
                    }
                },
                Some(Err(reason)) => {
                    failures.herdr.insert(adapter.id, reason.clone());
                }
                None => {
                    failures.herdr.insert(
                        adapter.id,
                        "Herdr is not found on this machine, so Hide's integration stays"
                            .to_owned(),
                    );
                }
            }
            continue;
        }
        let folder = integration
            .folder
            .iter()
            .fold(target.home.clone(), |path, part| path.join(part));
        if !detection.installed(adapter) || !folder.is_dir() {
            continue;
        }
        let Some(Ok(statuses)) = &before else {
            continue;
        };
        let restore = scope.agent_on.contains(adapter.id);
        let install_now = match statuses.of(integration.target) {
            Some(Integration::Missing) => restore || !recorded,
            // Only what Hide installed is Hide's to bring up to date.
            Some(Integration::Outdated) => recorded,
            Some(Integration::Current) | None => false,
        };
        if !install_now {
            continue;
        }
        // What Herdr writes is learned from the file it writes in (PRD
        // codex-herdr-hook-trust D-02): its entries before the call and right
        // after it, so the window another writer could land an entry in is
        // the one subprocess.
        let file_before = adapter.trusts_herdr_hook().then(|| read_hook_file(target));
        match herdr_integration::install(target, integration.target) {
            Ok(()) => {
                let files = file_before.map(|before| HookFiles {
                    before,
                    after: read_hook_file(target),
                });
                installed_now.push((adapter, files));
                touched = true;
            }
            Err(reason) => {
                failures.herdr.insert(adapter.id, reason);
            }
        }
    }
    if !touched {
        return before;
    }
    // What Herdr reports now decides what is recorded: an install that did
    // not take is not Hide's.
    let after = Statuses::probe(target);
    if let Some(Ok(statuses)) = &after {
        for (adapter, files) in installed_now {
            let integration = adapter.herdr;
            if statuses.of(integration.target) == Some(Integration::Current) {
                *changed |= record.insert_piece(&herdr_code(adapter));
                if let Some(files) = files {
                    *changed |= learn_herdr_hooks(record, adapter, files);
                }
            }
        }
    }
    after
}

/// A Codex hook file's entries at one moment, or why they could not be read.
type HookFile = Result<std::collections::BTreeSet<HookEntry>, String>;

fn read_hook_file(target: &KitTarget) -> HookFile {
    hide_agent_hooks::codex_trust::hook_entries(&target.home)
}

/// The hook file's entries around one integration install.
struct HookFiles {
    before: HookFile,
    after: HookFile,
}

/// Keeps in the record the entries Herdr's integration for `adapter` wrote
/// into the agent's hook file during the install just made, so Codex's trust
/// can be for exactly those bytes. A file that could not be read leaves the
/// record as it was. An install that changed the file in a way an integration
/// install does not (an entry of someone else's removed or edited, or more
/// entries added than Herdr's integration adds) teaches nothing, so the review
/// screen is left to the operator for what it wrote; either cause goes to the
/// log. True when the record changed.
fn learn_herdr_hooks(record: &mut Record, adapter: &AgentAdapter, files: HookFiles) -> bool {
    let event = |kind: &str, detail: &str| {
        eprintln!(
            "{}",
            serde_json::json!({
                "component": "kit",
                "kind": kind,
                "agent": adapter.id,
                "detail": detail,
            })
        );
    };
    let (before, after) = match (files.before, files.after) {
        (Ok(before), Ok(after)) => (before, after),
        (Err(detail), _) | (_, Err(detail)) => {
            event("herdr_hook_unread", &detail);
            return false;
        }
    };
    match learn_herdr_entries(&before, &after, record.herdr_hook_entries(adapter.id)) {
        Ok(entries) => record.set_herdr_hook_entries(adapter.id, entries),
        // What was recorded stays: it was seen when Herdr wrote it, and only
        // a byte-equal entry in the file is ever matched against it.
        Err(refused) => {
            event("herdr_hook_not_learned", &refused.to_string());
            false
        }
    }
}

/// Takes every Hide piece off a machine: each guidance hook's marked
/// entries, each unedited script file of Hide's and each skill stub that is
/// Hide's.
pub(crate) fn remove(target: &KitTarget) -> Vec<(String, RemoveOutcome)> {
    let mut outcomes = Vec::new();
    for adapter in ADAPTERS {
        match adapter.hook {
            HookSupport::Guidance(agent) => {
                outcomes.push((hook_code(adapter), remove_guidance(target, agent)));
            }
            HookSupport::Plugin(dialect) => {
                outcomes.push((hook_code(adapter), remove_plugin(target, dialect)));
            }
            HookSupport::Part(_) | HookSupport::None => {}
        }
    }
    for dir in SkillDir::ALL {
        let outcome = match remove_skill(dir, &target.home) {
            Ok(true) => RemoveOutcome::Removed,
            Ok(false) => RemoveOutcome::Absent,
            Err(reason) => RemoveOutcome::Failed { reason },
        };
        outcomes.push((skill_code(dir), outcome));
    }
    outcomes.extend(retired_agents::remove(target));
    outcomes
}

/// Takes the Herdr integrations Hide installed off a machine that is being
/// removed from Hide, by the record's ownership; an integration the operator
/// installed stays.
pub(crate) fn remove_herdr(target: &KitTarget, record: &Record) -> Vec<(String, RemoveOutcome)> {
    let owned: Vec<&AgentAdapter> = ADAPTERS
        .iter()
        .filter(|adapter| record.contains_piece(&herdr_code(adapter)))
        .collect();
    if owned.is_empty() {
        return Vec::new();
    }
    let statuses = Statuses::probe(target);
    owned
        .into_iter()
        .map(|adapter| {
            let integration = adapter.herdr;
            let outcome = match &statuses {
                None => RemoveOutcome::Failed {
                    reason: "Herdr is not found on this machine".to_owned(),
                },
                Some(Err(reason)) => RemoveOutcome::Failed {
                    reason: reason.clone(),
                },
                Some(Ok(listed)) if listed.of(integration.target) == Some(Integration::Missing) => {
                    RemoveOutcome::Absent
                }
                Some(Ok(_)) => match herdr_integration::uninstall(target, integration.target) {
                    Ok(()) => RemoveOutcome::Removed,
                    Err(reason) => RemoveOutcome::Failed { reason },
                },
            };
            (herdr_code(adapter), outcome)
        })
        .collect()
}

/// The kit parts' reports, in the shape [`apply`] and [`status`] take.
pub(crate) fn part_views(components: &[crate::ComponentReport]) -> Vec<PartView> {
    components
        .iter()
        .map(|part| {
            (
                part.id,
                part.state,
                part.reason.clone(),
                part.location.clone(),
            )
        })
        .collect()
}
