//! The agent adapters: which coding agents Hide knows how to reach, and what
//! it puts where each one reads it (issue #517).
//!
//! The leaf `hide-agent-adapter` crate declares each agent's programs, skill
//! location, hook dialect and documentation. This module projects those
//! declarations into kit pieces and owns their installation policy. The content an
//! agent reads is the same for all of them and is read from the `hide` binary
//! at run time (`hide guide`), so a new agent needs a shared declaration
//! and a fixture, never another copy of this content (engineering rule 2, 7).
//!
//! Three things a row can turn on for an agent the operator switched on:
//!
//! - Hide's skill stubs ([`SKILLS`]: `hide`, `hide-conductor` and `hide-plan`;
//!   [`RETIRED_SKILLS`] are taken out), in the folder the agent reads
//!   ([`SkillDir`]); the folder is shared by every agent that reads it, so
//!   the stubs stay while any of them is on;
//! - a hook: Claude Code and Codex keep their kit parts
//!   ([`HookSupport::Part`]); Grok, whose documented command hooks
//!   can refuse a shell call and report subagents, get Hide's hook file of
//!   the basic tier ([`HookSupport::Guidance`]); and OpenCode, Pi and omp get
//!   a script file of Hide's in their own folder ([`HookSupport::Plugin`]);
//! - nothing else. What Hide never writes is listed in `docs/agent-hooks.md`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use hide_agent_hooks::guidance::GuidanceAgent;
use hide_platform::programs::Program;
use serde::{Deserialize, Serialize};

use crate::{ComponentId, ComponentState, KitTarget};

pub use hide_agent_adapter::{Feature, HerdrIntegration};

/// A folder an agent reads skills from, under the account's home.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SkillDir {
    /// `~/.agents/skills`, which most agents read.
    Shared,
    /// `~/.claude/skills`: Claude Code does not read the shared folder.
    Claude,
    /// `~/.gemini/config/skills`: the one global folder whose skills agy's
    /// model is given (agy 1.4.0), beside Hide's hook block in that folder.
    /// `~/.gemini/antigravity-cli/skills` is the operator's and never written.
    Antigravity,
}

impl SkillDir {
    pub const ALL: [SkillDir; 3] = [Self::Shared, Self::Claude, Self::Antigravity];

    /// The stable name the record carries.
    pub fn code(self) -> &'static str {
        match self {
            Self::Shared => "agents",
            Self::Claude => "claude",
            Self::Antigravity => "antigravity",
        }
    }

    /// The agent's own folder the skill folder sits in (none for the shared
    /// one), and the skill folder's name inside it.
    fn relative(self) -> (&'static [&'static str], &'static str) {
        match self {
            Self::Shared => (&[".agents"], "skills"),
            Self::Claude => (&[".claude"], "skills"),
            Self::Antigravity => (&[".gemini", "config"], "skills"),
        }
    }

    fn folder(self, home: &Path) -> PathBuf {
        self.relative()
            .0
            .iter()
            .fold(home.to_owned(), |path, part| path.join(part))
    }

    /// `~/<folder>/skills`.
    pub fn path(self, home: &Path) -> PathBuf {
        self.folder(home).join(self.relative().1)
    }

    /// Whether the folder can be written without creating the agent's own
    /// folder: the shared folder is Hide's to create, an agent's own is not,
    /// because its presence is how the hook code tells that the agent's
    /// settings are there (`hide_agent_hooks`' runtime-absent answer), so a
    /// pass that made it would let the next pass write a hook this one did
    /// not, and a second apply would not be a no-op.
    pub(crate) fn writable(self, home: &Path) -> bool {
        self == Self::Shared || self.folder(home).is_dir()
    }
}

/// What Hide writes into an agent's hook configuration, if anything.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookSupport {
    /// Claude Code and Codex: the six-event hook that is also one kit part.
    Part(ComponentId),
    /// Hide's hook for an agent of the basic tier with documented command
    /// hooks: the spawn guard, the subagent count and the session
    /// guidance (`hide_agent_hooks::guidance`).
    Guidance(GuidanceAgent),
    /// A script file of Hide's own in the agent's plugin or extension folder
    /// (`hide_agent_hooks::plugin`): OpenCode's plugin, Pi's and omp's
    /// extension, each judged like the skill stub.
    Plugin(hide_agent_adapter::PluginDialect),
    /// No hook: the agent gets the skill only. `docs/agent-hooks.md` carries
    /// the reason and the page that supports it.
    None,
}

/// One agent Hide knows.
#[derive(Clone, Copy, Debug)]
pub struct AgentAdapter {
    /// The stable id the record, the wire and the Settings rows carry.
    pub id: &'static str,
    /// The name the operator reads (a product name, never translated).
    pub label: &'static str,
    /// The program names the vendor's install documentation gives the CLI;
    /// the agent is installed on a machine when one of them is found
    /// ([`Detection`]). A folder the agent creates does not count: a CLI that
    /// was removed leaves its folder behind.
    pub executables: &'static [&'static str],
    /// The folder the agent reads skills from.
    pub skill_dir: SkillDir,
    pub hook: HookSupport,
    /// Herdr's integration for the agent. Every supported agent has one: an
    /// agent the pinned Herdr ships no integration for is not supported
    /// (`docs/agent-hooks.md`, Which agents Hide supports).
    pub herdr: HerdrIntegration,
    /// Whether the agent is on without the operator choosing, as Claude Code
    /// and Codex have been since their hooks became part of the kit.
    pub default_on: bool,
    declaration: &'static hide_agent_adapter::AgentAdapter,
    /// The official page the row's answers come from.
    pub doc_url: &'static str,
}

// The kit keeps its policy types and derives every row from the leaf.
const fn project(declaration: &'static hide_agent_adapter::AgentAdapter) -> AgentAdapter {
    use hide_agent_adapter::{GuidanceDialect, HookDialect, HookInstall, SkillLocation};
    AgentAdapter {
        id: declaration.id,
        label: declaration.label,
        executables: declaration.executables,
        skill_dir: match declaration.skill_location {
            SkillLocation::Shared => SkillDir::Shared,
            SkillLocation::Claude => SkillDir::Claude,
            SkillLocation::Antigravity => SkillDir::Antigravity,
        },
        hook: match declaration.hook {
            HookInstall::Runtime(HookDialect::ClaudeCode) => {
                HookSupport::Part(ComponentId::ClaudeCodeHook)
            }
            HookInstall::Runtime(HookDialect::Codex) => HookSupport::Part(ComponentId::CodexHook),
            // OpenCode's, Pi's and omp's dialects are spoken by a script of
            // Hide's, and Grok's and Cursor's by the guidance file, never by a
            // settings file; the table is a constant, so a declaration saying
            // otherwise fails the build.
            HookInstall::Runtime(HookDialect::OpenCode | HookDialect::Pi | HookDialect::Omp) => {
                panic!("this agent's hook is a script file of Hide's, not a settings file")
            }
            HookInstall::Runtime(
                HookDialect::Grok | HookDialect::Cursor | HookDialect::Antigravity,
            ) => {
                panic!("Grok's, Cursor's and agy's hooks are installed as a guidance file")
            }
            HookInstall::Guidance(GuidanceDialect::Cursor) => {
                HookSupport::Guidance(GuidanceAgent::Cursor)
            }
            HookInstall::Guidance(GuidanceDialect::Grok) => {
                HookSupport::Guidance(GuidanceAgent::Grok)
            }
            HookInstall::Guidance(GuidanceDialect::Antigravity) => {
                HookSupport::Guidance(GuidanceAgent::Antigravity)
            }
            HookInstall::Plugin(dialect) => HookSupport::Plugin(dialect),
            HookInstall::None => HookSupport::None,
        },
        herdr: declaration.herdr,
        default_on: declaration.default_on,
        doc_url: declaration.doc_url,
        declaration,
    }
}

const fn project_all() -> [AgentAdapter; hide_agent_adapter::ADAPTERS.len()] {
    let mut rows = [project(&hide_agent_adapter::ADAPTERS[0]); hide_agent_adapter::ADAPTERS.len()];
    let mut index = 0;
    while index < rows.len() {
        rows[index] = project(&hide_agent_adapter::ADAPTERS[index]);
        index += 1;
    }
    rows
}

/// The leaf declaration projected into the kit's installation policy.
pub const ADAPTERS: &[AgentAdapter] = &project_all();

/// One skill stub Hide puts in every skill folder. A stub points at the
/// binary and carries no usage, so it stays right as the help changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Skill {
    /// The skill's name, and the folder it lives in.
    pub name: &'static str,
    description: &'static str,
    body: &'static str,
    /// Raise it when the stub's text changes, so an older stub is replaced.
    pub version: u32,
}

/// The one stub: the guide to Hide, which `hide guide` prints and which
/// routes browser and mission work to their own guides.
pub const GUIDE_SKILL: Skill = Skill {
    name: "hide",
    description: "Only for a session in a pane Hide manages, never an ordinary terminal. Work inside Hide, the operator's workspace for coding agents: send and answer letters to other agents, start agents and follow them, run a mission of agents as its Conductor, read and drive a browser display, show the operator a file or a diff, and read or add Project Memory. Use when a task involves another agent, a mission, a page in Hide's browser, the operator's screen in Hide, or Project Memory, or when you need to know what Hide offers here.",
    // The intent table is the session-start note's own (PRD
    // agent-beside-and-session-guide D-14); version 1 had none.
    body: concat!(
        "First check that this session runs in a pane Hide manages: Hide's session guidance names this checkout, or `hide workspace info` answers `\"ok\":true`. If neither, stop here; this skill does not apply. Inside Hide, run `hide guide` and follow what it prints. It explains what Hide keeps, how to send and answer letters, how to start and follow agents here or on a connected device, how to show the operator a file or a diff and how to use Project Memory, and it sends a browser display to `hide browser help` and a mission of agents to `hide mission help`.\n\n",
        hide_agent_hooks::intent_table!()
    ),
    version: 2,
};

/// The Conductor's stub: a mission of agents, which `hide mission help`
/// owns. Version 1 had a plain description that Pi's strict front matter
/// dropped (`Skill::text`); it is an older version of this stub, replaced by
/// the marker rule like any other. Version 3 names the decision's choices
/// and the node results (PRD mission-plan-visibility B34).
pub const CONDUCTOR_SKILL: Skill = Skill {
    name: "hide-conductor",
    description: "Conduct a mission of agents in Hide: plan it with the operator, start the workers, replan, report. Use when the operator asks for a mission or for several PRs carried as a group.",
    body: "First check that this session runs in a pane Hide manages: Hide's session guidance names this checkout, or `hide workspace info` answers `\"ok\":true`. If neither, stop here; this skill does not apply. Inside Hide, run `hide mission help` and follow what it prints: it is the Conductor's contract, with the commands, how to cut the mission into nodes and brief each worker, the cap and the candidates, replanning on the operator's words, asking the operator a decision with choices they can answer from the map, whose answer arrives in your conversation as their own message, recording a direct change's commit and a verify node's verdict, merging, the report and taking a mission over.",
    version: 3,
};

/// An agent's own plan (PRD hide-plan D-16), which `hide plan help` owns.
pub const PLAN_SKILL: Skill = Skill {
    name: "hide-plan",
    description: "Keep your own plan of the task in Hide: its steps, what each waits on, which one you are on, and the operator's review when they ask for one. Use when a task in a Hide pane takes more than a few steps, or the operator asks you to plan first or have the plan reviewed.",
    body: "First check that this session runs in a pane Hide manages: Hide's session guidance names this checkout, or `hide workspace info` answers `\"ok\":true`. If neither, stop here; this skill does not apply. Inside Hide, run `hide plan help` and follow what it prints: it is the plan's contract, with the commands, the steps' shape, when to put the plan up for review with `--ask`, how to approve or drop it on the operator's words, and how to keep its steps current.",
    version: 1,
};

/// Every stub Hide writes; each skill folder holds all of them. A new stub
/// is one more entry here and nothing else: the observe, write and remove
/// passes read the array.
pub const SKILLS: [Skill; 3] = [GUIDE_SKILL, CONDUCTOR_SKILL, PLAN_SKILL];

/// A stub an earlier build wrote and this one takes out, by its exact text:
/// a file that differs in any byte was edited by the operator and stays.
#[derive(Clone, Copy, Debug)]
pub struct RetiredSkill {
    pub name: &'static str,
    /// The whole file as that build wrote it.
    pub text: &'static str,
}

/// `hide-browser` version 1, whose guide `hide guide` now routes to.
pub const RETIRED_SKILLS: [RetiredSkill; 1] = [RetiredSkill {
    name: "hide-browser",
    text: concat!(
        "---\nname: hide-browser\n",
        "description: Read and drive a browser display inside Hide. Use when a task involves a web page, a local app in a browser tab, or checking what a page shows.\n",
        "---\n\n<!-- hide-skill@1: written by Hide; remove it from Settings, Agents -->\n\n",
        "Run `hide browser help` and follow what it prints. It explains how to open a page, read it by refs, check a change with `--diff`, and what to do when a command fails.\n",
    ),
}];

/// The marker line of a stub, which proves Hide wrote it and says which
/// text it was. A file without it is the operator's and is never touched.
const SKILL_MARKER_NAME: &str = "hide-skill";

pub fn adapter(id: &str) -> Option<&'static AgentAdapter> {
    let declaration = hide_agent_adapter::adapter(id)?;
    ADAPTERS.iter().find(|row| row.id == declaration.id)
}

/// The id of the agent whose hook is the kit part `part`, when there is one.
pub fn agent_of_part(part: ComponentId) -> Option<&'static str> {
    adapter_of_part(part).map(|adapter| adapter.id)
}

/// The agent whose hook is the kit part `part`, when there is one.
pub(crate) fn adapter_of_part(part: ComponentId) -> Option<&'static AgentAdapter> {
    ADAPTERS
        .iter()
        .find(|adapter| adapter.hook == HookSupport::Part(part))
}

impl AgentAdapter {
    /// Whether the hook entries Herdr's integration writes for this agent are
    /// covered by Hide's trust for it: only Codex asks the operator to review
    /// a hook before it runs, and Hide records that trust for it
    /// (`hide_agent_hooks::codex_trust`, PRD codex-herdr-hook-trust).
    pub(crate) fn trusts_herdr_hook(&self) -> bool {
        self.hook == HookSupport::Part(ComponentId::CodexHook)
    }

    /// Capabilities come from the shared declaration, independently.
    pub fn supports(&self, feature: Feature) -> bool {
        self.declaration.supports(feature)
    }

    /// The existing wire field carries the Basic collaboration grade.
    pub fn partial(&self) -> bool {
        self.declaration.basic()
    }
}

/// What one pass found out about the agents on a machine: each adapter's
/// program, looked for once on one search. Every decision of the pass reads
/// this instead of searching again.
pub(crate) struct Detection {
    rows: Vec<(&'static str, Option<PathBuf>)>,
}

impl Detection {
    pub(crate) fn probe(target: &KitTarget) -> Self {
        let path = search_path(target);
        let rows = ADAPTERS
            .iter()
            .map(|adapter| {
                let program = adapter
                    .executables
                    .iter()
                    .find_map(|name| hide_platform::host::find_program(&path, name));
                (adapter.id, program)
            })
            .collect();
        Self { rows }
    }

    /// Whether the agent is installed here: one of its programs is found.
    pub(crate) fn installed(&self, adapter: &AgentAdapter) -> bool {
        self.executable(adapter).is_some()
    }

    pub(crate) fn executable(&self, adapter: &AgentAdapter) -> Option<&Path> {
        self.rows
            .iter()
            .find(|(id, _)| *id == adapter.id)
            .and_then(|(_, program)| program.as_deref())
    }
}

/// Where the agents' programs are looked for: the folders the account's
/// login shell puts on its `PATH`, then the daemon's own `PATH` and the
/// usual install folders (`hide_platform::programs`). Every program the kit
/// checks or runs for an agent (whether it is installed, whether its version
/// takes Hide's hook, the Codex it asks about trust and its daemon) is found
/// here and run with it, so they are the one program the operator's shell
/// runs. A join fails only for a folder whose name holds the separator,
/// which no search can hold, so then nothing is found.
pub(crate) fn search_path(target: &KitTarget) -> OsString {
    search(&target.home, target.login_shell.as_deref(), &target.stop)
}

/// The program `name` on [`search`], for a target being built.
pub(crate) fn find(
    home: &Path,
    login_shell: Option<&Path>,
    stop: &AtomicBool,
    name: &str,
) -> Option<Program> {
    Program::find(&search(home, login_shell, stop), name)
}

fn search(home: &Path, login_shell: Option<&Path>, stop: &AtomicBool) -> OsString {
    // A test machine's own agents must not decide what a fixture home finds,
    // so under test the search is the fixture's `~/.local/bin` and what the
    // fixture's stand-in shell answers, never this process's `PATH`.
    #[cfg(test)]
    {
        let shell = hide_platform::programs::login_shell_path(home, login_shell, stop);
        let folders = shell
            .iter()
            .flat_map(std::env::split_paths)
            .chain([home.join(".local/bin")]);
        std::env::join_paths(folders).unwrap_or_default()
    }
    #[cfg(not(test))]
    {
        hide_platform::programs::search_path(home, login_shell, stop).unwrap_or_default()
    }
}

/// The account's login shell: `$SHELL` on macOS and Linux, which launchd
/// gives an app and sshd gives an exec channel. Windows has none to ask; a
/// process there is given the account's own `Path`. A Unix process started
/// without `SHELL` asks none, and the log says so.
pub(crate) fn login_shell() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    hide_platform::host::default_shell()
        .inspect_err(|error| eprintln!("kit.login_shell_missing kind={:?}", error.kind()))
        .ok()
}

// --- The skill stub ------------------------------------------------------------

impl Skill {
    /// The text of the stub. The description is a quoted YAML string: a
    /// plain one ends at its first `: `, and a strict reader such as Pi's
    /// then drops the whole skill (version 1's `hide-conductor`).
    pub(crate) fn text(self) -> String {
        // A JSON string is a YAML double-quoted scalar.
        let description = serde_json::Value::from(self.description).to_string();
        format!(
            "---\nname: {}\ndescription: {description}\n---\n\n<!-- {SKILL_MARKER_NAME}@{}: written by Hide; remove it from Settings, Agents -->\n\n{}\n",
            self.name, self.version, self.body
        )
    }

    pub(crate) fn file_in(self, root: &Path) -> PathBuf {
        root.join(self.name).join("SKILL.md")
    }

    /// What the skills folder `root` holds of this stub.
    fn observe_in(self, root: &Path) -> SkillObserved {
        let path = self.file_in(root);
        match std::fs::read_to_string(&path) {
            Ok(text) => match skill_marker_version(&text) {
                Some(version) if version == self.version && text == self.text() => {
                    SkillObserved::Current
                }
                Some(version) if version < self.version => SkillObserved::Stale,
                Some(_) => SkillObserved::Edited,
                None => SkillObserved::Foreign(vec![self.name]),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => SkillObserved::Missing,
            Err(error) => {
                SkillObserved::Unreadable(format!("{} could not be read: {error}", path.display()))
            }
        }
    }
}

/// The version in the stub's marker line. The marker counts only where Hide
/// puts it, as the first line after the front matter and in Hide's own form:
/// a file that merely mentions `hide-skill@` somewhere is the operator's.
fn skill_marker_version(text: &str) -> Option<u32> {
    let body = text.strip_prefix("---\n")?;
    let (_, after) = body.split_once("\n---\n")?;
    let line = after.lines().find(|line| !line.trim().is_empty())?.trim();
    let rest = line
        .strip_prefix("<!-- ")?
        .strip_prefix(SKILL_MARKER_NAME)?
        .strip_prefix('@')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    rest[digits.len()..]
        .starts_with(':')
        .then_some(())
        .filter(|()| line.ends_with("-->"))?;
    digits.parse().ok()
}

/// What one skill folder holds, before the kit decides anything.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SkillObserved {
    Current,
    /// Hide's stub of an older version: a pass replaces it.
    Stale,
    /// Hide's marker at this version (or a newer one) over text that is not
    /// the stub's: the operator edited it. A pass leaves it as it is and only
    /// Reinstall puts Hide's text back.
    Edited,
    Missing,
    /// Files there that Hide did not write, by the stub names they hold;
    /// they are left as they are.
    Foreign(Vec<&'static str>),
    Unreadable(String),
}

pub(crate) fn observe_skill(dir: SkillDir, home: &Path) -> SkillObserved {
    observe_skill_in(&dir.path(home))
}

/// What the skills folder `root` holds of Hide's stubs, as one piece: a
/// folder that lacks a stub another of Hide's is beside (one written before
/// that stub existed) is stale, and one where the operator's file holds a
/// stub's name is foreign once Hide's others are current. A retired stub
/// Hide wrote and nobody edited makes the folder stale too, so the next pass
/// takes it out and, in a folder an earlier build filled with only those,
/// writes the current stub, which a recorded absence alone would not. An
/// edited one is the operator's and decides nothing: a recorded folder that
/// holds only edited ones reads as the operator's removal, which Reinstall
/// undoes, so a `hide` they deleted is not written back on every pass.
pub(crate) fn observe_skill_in(root: &Path) -> SkillObserved {
    let observed = combine(SKILLS.map(|skill| skill.observe_in(root)));
    if matches!(observed, SkillObserved::Unreadable(_)) {
        return observed;
    }
    let mut retired = Vec::new();
    for skill in RETIRED_SKILLS {
        match skill.observe_in(root) {
            Ok(found) => retired.push(found),
            Err(reason) => return SkillObserved::Unreadable(reason),
        }
    }
    if retired.contains(&RetiredObserved::Unedited) {
        SkillObserved::Stale
    } else {
        observed
    }
}

/// What a skills folder holds of one retired stub.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetiredObserved {
    /// Nothing there, or a file without Hide's marker: the operator's.
    NotHides,
    Unedited,
    /// Hide's marker over other text: the operator's now, and it stays.
    Edited,
}

impl RetiredSkill {
    fn file_in(self, root: &Path) -> PathBuf {
        root.join(self.name).join("SKILL.md")
    }

    fn observe_in(self, root: &Path) -> Result<RetiredObserved, String> {
        let path = self.file_in(root);
        match std::fs::read_to_string(&path) {
            Ok(text) if text == self.text => Ok(RetiredObserved::Unedited),
            Ok(text) if skill_marker_version(&text).is_some() => Ok(RetiredObserved::Edited),
            Ok(_) => Ok(RetiredObserved::NotHides),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(RetiredObserved::NotHides)
            }
            Err(error) => Err(format!("{} could not be read: {error}", path.display())),
        }
    }
}

/// Takes out every retired stub in `root` that Hide wrote and nobody edited,
/// and its folder when nothing else is in it.
fn retire_in(root: &Path) -> Result<bool, String> {
    let mut removed = false;
    for skill in RETIRED_SKILLS {
        if skill.observe_in(root)? == RetiredObserved::Unedited {
            let path = skill.file_in(root);
            std::fs::remove_file(&path)
                .map_err(|error| format!("{} could not be removed: {error}", path.display()))?;
            let _ = std::fs::remove_dir(root.join(skill.name));
            removed = true;
        }
    }
    Ok(removed)
}

fn combine(each: [SkillObserved; SKILLS.len()]) -> SkillObserved {
    if let Some(SkillObserved::Unreadable(reason)) = each
        .iter()
        .find(|observed| matches!(observed, SkillObserved::Unreadable(_)))
    {
        return SkillObserved::Unreadable(reason.clone());
    }
    let foreign: Vec<&'static str> = each
        .iter()
        .flat_map(|observed| match observed {
            SkillObserved::Foreign(names) => names.clone(),
            _ => Vec::new(),
        })
        .collect();
    let own: Vec<&SkillObserved> = each
        .iter()
        .filter(|observed| !matches!(observed, SkillObserved::Foreign(_)))
        .collect();
    let any = |wanted: &[SkillObserved]| own.iter().any(|observed| wanted.contains(observed));
    if own.is_empty() {
        SkillObserved::Foreign(foreign)
    } else if own
        .iter()
        .all(|observed| **observed == SkillObserved::Missing)
    {
        SkillObserved::Missing
    } else if any(&[SkillObserved::Stale, SkillObserved::Missing]) {
        SkillObserved::Stale
    } else if any(&[SkillObserved::Edited]) {
        SkillObserved::Edited
    } else if own.len() < each.len() {
        // Hide's other stubs are in place; the operator's file keeps the
        // name, which is what the report says.
        SkillObserved::Foreign(foreign)
    } else {
        SkillObserved::Current
    }
}

/// Writes every stub that is missing or older, and takes out the retired
/// ones Hide wrote; an edited stub only when the operator asked for it again
/// (`reinstall`), and an operator's file never.
pub(crate) fn install_skill(dir: SkillDir, home: &Path, reinstall: bool) -> Result<(), String> {
    let root = dir.path(home);
    for skill in SKILLS {
        let write = match skill.observe_in(&root) {
            SkillObserved::Missing | SkillObserved::Stale => true,
            SkillObserved::Edited => reinstall,
            SkillObserved::Current | SkillObserved::Foreign(_) => false,
            SkillObserved::Unreadable(reason) => return Err(reason),
        };
        if write {
            crate::write_atomically(
                &skill.file_in(&root),
                skill.text().as_bytes(),
                hide_platform::fs::Access::KeepOrPrivate,
            )?;
        }
    }
    retire_in(&root)?;
    Ok(())
}

/// Takes the stub, and any retired one, out when it is Hide's, and the folder
/// with it when nothing else is in it.
pub(crate) fn remove_skill(dir: SkillDir, home: &Path) -> Result<bool, String> {
    remove_skill_in(&dir.path(home))
}

/// [`remove_skill`] for any skills folder, which the retirement of an agent
/// Hide no longer knows reaches by its path.
pub(crate) fn remove_skill_in(root: &Path) -> Result<bool, String> {
    let mut removed = false;
    for skill in SKILLS {
        match skill.observe_in(root) {
            // An edited stub carries the operator's changes, so it stays.
            SkillObserved::Missing | SkillObserved::Foreign(_) | SkillObserved::Edited => {}
            SkillObserved::Unreadable(reason) => return Err(reason),
            SkillObserved::Current | SkillObserved::Stale => {
                let path = skill.file_in(root);
                std::fs::remove_file(&path)
                    .map_err(|error| format!("{} could not be removed: {error}", path.display()))?;
                // Only an empty folder goes; anything else in it is the
                // operator's.
                let _ = std::fs::remove_dir(root.join(skill.name));
                removed = true;
            }
        }
    }
    Ok(retire_in(root)? || removed)
}

/// The skills folder the stubs are in.
pub(crate) fn skill_location(dir: SkillDir, home: &Path) -> String {
    dir.path(home).display().to_string()
}

/// One piece of an agent (its skill, its hook) as the report states it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PieceReport {
    pub state: ComponentState,
    /// Why, in one sentence for the log and the `hide` CLI; `why` is what
    /// the app words.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<crate::PartWhy>,
    #[serde(default)]
    pub location: Option<String>,
}

/// Why an agent has no switch on this machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// Installed, and at least one of its pieces (the skill, the hook) works
    /// on this system: the switch works.
    Available,
    /// None of its programs is found on this machine.
    NotInstalled,
    /// Installed, but neither the skill folder nor the hook is documented for
    /// this system, so there is nothing for the switch to do. This build never
    /// reports it, since every supported agent reads its skill folder on every
    /// system (`docs/agent-hooks.md`, Support table); a connected device still
    /// running an older build does, so it stays as the transition path and the
    /// shells keep wording it.
    UnsupportedSystem,
}

/// What the switch can do for an agent: every supported agent's skill can be
/// put on every system, so it needs only the agent installed.
pub(crate) fn availability(installed: bool) -> Availability {
    if installed {
        Availability::Available
    } else {
        Availability::NotInstalled
    }
}

/// One agent as Settings shows it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    pub id: String,
    pub label: String,
    pub availability: Availability,
    /// Whether the agent is on: switched on by the operator, or on by
    /// default and not switched off.
    pub enabled: bool,
    /// The machine's record holds the operator's own choice for this agent,
    /// on or off. An agent that is on only by default has none, so a shell
    /// can tell one the operator switched on, whose program then went away
    /// and which keeps its switch, from a default that never had a program.
    /// Absent in a report from a build that predates it, which reads as no
    /// choice.
    #[serde(default)]
    pub chosen: bool,
    pub skill: PieceReport,
    /// `None` for an agent with no hook (skill only).
    pub hook: Option<PieceReport>,
    /// Herdr's integration for the agent. This build fills it for every
    /// agent; `None` only in a report from a device helper whose build
    /// predates the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herdr: Option<PieceReport>,
    pub doc_url: String,
}

impl AgentReport {
    /// Whether Reinstall would change something for this agent: only an
    /// agent that is on has pieces to repair.
    pub fn needs_attention(&self) -> bool {
        self.enabled
            && (self.skill.state.needs_attention()
                || self
                    .hook
                    .as_ref()
                    .is_some_and(|hook| hook.state.needs_attention())
                || self
                    .herdr
                    .as_ref()
                    .is_some_and(|herdr| herdr.state.needs_attention()))
    }
}
