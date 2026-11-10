//! A Herdr saved session (`session.json` in Herdr's configuration folder)
//! read into the own engine's saved session (PRD
//! engine-bundle-4-herdr-removal D-3): the one place a Herdr session becomes
//! an engine session, whichever way it is moved (the update's move of
//! Hide's own Herdr, or the read-only import of an operator's Herdr).
//!
//! The file is only read, never written or moved. It is read up to
//! `hide_node_link::herdr_save::READ_CAP` bytes in every form Herdr v0.9.5 reads (`src/persist/
//! snapshot.rs` of the pinned tag: version 3 and the versions before it,
//! whose workspaces held one tab inline). Herdr's public ids are kept: a
//! workspace keeps its `w<n>`, and its tabs and panes the ids Herdr's restore
//! makes from the saved public numbers, so every Hide state keyed by a pane,
//! tab or workspace id needs no new key (D-3). A workspace whose save does
//! not hold together (an id held twice, a tree that does not lay out its
//! panes, a ratio outside 0.1 to 0.9, more panes or levels than a tab holds)
//! is left out with its reason and the rest are moved, unlike the engine's
//! own save, which is refused whole (D-6), since one damaged workspace of an
//! operator's Herdr should not keep the others back.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::ids::{self, Numbered};
use super::layout::{MAX_DEPTH, MAX_PANES};
use super::store::{
    SavedAgent, SavedDirection, SavedNode, SavedPane, SavedTab, SavedWorkspace, SavedWorktree,
};

/// The newest session format Herdr v0.9.5 writes and reads
/// (`hide_node_link::herdr_save::NEWEST_FORMAT`).
pub(crate) const NEWEST_FORMAT: u32 = hide_node_link::herdr_save::NEWEST_FORMAT;

/// The narrowest and widest split a tab keeps, as the own engine's layout
/// keeps them.
const RATIOS: std::ops::RangeInclusive<f32> = 0.1..=0.9;

/// What reading a Herdr session found.
#[derive(Debug)]
pub(crate) enum HerdrRead {
    /// No session was saved: nothing to move or import.
    Missing,
    Read(HerdrSession),
    /// A session is there and could not be read: why, in one line.
    Unreadable(String),
}

/// A Herdr session as read: each workspace in the order Herdr keeps them,
/// either read or refused with its reason.
#[derive(Debug)]
pub(crate) struct HerdrSession {
    pub(crate) workspaces: Vec<Result<HerdrWorkspace, Refused>>,
}

impl HerdrSession {
    /// The id of every workspace of the save that has one (all but the form
    /// before tabs), read or left out.
    pub(crate) fn ids(&self) -> impl Iterator<Item = &str> {
        self.workspaces.iter().filter_map(|read| match read {
            Ok(workspace) => workspace.id.as_deref(),
            Err(refused) => matches!(
                ids::workspace_of(&refused.workspace),
                Some((Numbered::Imported, _))
            )
            .then_some(refused.workspace.as_str()),
        })
    }
}

/// A workspace Herdr saved, in Herdr's form.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct HerdrWorkspace {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    custom_name: Option<String>,
    identity_cwd: String,
    #[serde(default)]
    worktree_space: Option<HerdrSpace>,
    #[serde(default)]
    public_pane_numbers: HashMap<u32, usize>,
    #[serde(default)]
    next_public_pane_number: usize,
    #[serde(default)]
    public_tab_numbers: Vec<usize>,
    #[serde(default)]
    next_public_tab_number: usize,
    tabs: Vec<HerdrTab>,
    #[serde(default)]
    active_tab: usize,
}

/// A workspace saved before Herdr kept tabs: its one tab inline.
#[derive(Deserialize)]
struct LegacyWorkspace {
    #[serde(default)]
    custom_name: Option<String>,
    layout: HerdrLayout,
    panes: HashMap<u32, HerdrPane>,
    zoomed: bool,
    #[serde(default)]
    focused: Option<u32>,
    #[serde(default)]
    root_pane: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
struct HerdrSpace {
    key: String,
    label: String,
    repo_root: String,
    checkout_path: String,
    is_linked_worktree: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct HerdrTab {
    #[serde(default)]
    custom_name: Option<String>,
    layout: HerdrLayout,
    panes: HashMap<u32, HerdrPane>,
    zoomed: bool,
    #[serde(default)]
    focused: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
enum HerdrLayout {
    Pane(u32),
    Split {
        direction: HerdrDirection,
        ratio: f32,
        first: Box<HerdrLayout>,
        second: Box<HerdrLayout>,
    },
}

/// Herdr's split direction, ratatui's: `Horizontal` lays its two sides out
/// side by side, `Vertical` one above the other.
#[derive(Clone, Copy, Debug, Deserialize)]
enum HerdrDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, Deserialize)]
struct HerdrPane {
    cwd: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    agent_name: Option<String>,
    #[serde(default)]
    managed_agent_kind: Option<String>,
    #[serde(default)]
    agent_session: Option<HerdrAgentSession>,
    #[serde(default)]
    agent_resume: Option<HerdrAgentResume>,
}

#[derive(Clone, Debug, Deserialize)]
struct HerdrAgentSession {
    agent: String,
    value: String,
}

#[derive(Clone, Debug, Deserialize)]
struct HerdrAgentResume {
    agent: String,
}

/// A workspace left out, with what names it to the operator and why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Refused {
    /// Its Herdr id, or its place in the session when it has none.
    pub(crate) workspace: String,
    /// Its name as Herdr showed it, when known.
    pub(crate) label: String,
    pub(crate) reason: String,
}

#[derive(Deserialize)]
struct Header {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    workspaces: Vec<Value>,
}

/// The Herdr session a machine read from its save
/// (`hide_host::herdr_save::fetch`, on that machine), only reading it.
pub(crate) fn of(fetched: &hide_node_link::herdr_save::Fetched) -> HerdrRead {
    match fetched {
        hide_node_link::herdr_save::Fetched::Missing => HerdrRead::Missing,
        hide_node_link::herdr_save::Fetched::Unreadable { reason } => {
            HerdrRead::Unreadable(reason.clone())
        }
        hide_node_link::herdr_save::Fetched::Text { text, .. } => match parse(text) {
            Ok(session) => HerdrRead::Read(session),
            Err(why) => HerdrRead::Unreadable(why),
        },
    }
}

/// The session `text` holds, each workspace read in Herdr's current form or
/// the inline one before tabs; a format newer than [`NEWEST_FORMAT`] or text
/// that is not a session is refused whole.
pub(crate) fn parse(text: &str) -> Result<HerdrSession, String> {
    let header: Header = serde_json::from_str(text)
        .map_err(|error| format!("it is not a Herdr session: {error}"))?;
    if header.version > NEWEST_FORMAT {
        return Err(format!(
            "it is format {}, newer than the format {NEWEST_FORMAT} this build reads",
            header.version
        ));
    }
    let workspaces = header
        .workspaces
        .into_iter()
        .enumerate()
        .map(|(index, raw)| workspace_of(index, raw))
        .collect();
    Ok(HerdrSession { workspaces })
}

fn workspace_of(index: usize, raw: Value) -> Result<HerdrWorkspace, Refused> {
    let named = Refused {
        workspace: raw
            .get("id")
            .and_then(Value::as_str)
            .map_or_else(|| format!("#{}", index + 1), str::to_owned),
        label: raw
            .get("custom_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        reason: String::new(),
    };
    let refused = |reason: String| Refused {
        reason,
        ..named.clone()
    };
    if raw.get("identity_cwd").is_some() {
        return serde_json::from_value(raw)
            .map_err(|error| refused(format!("its save is not a workspace: {error}")));
    }
    if raw.get("layout").is_some() {
        let legacy: LegacyWorkspace = serde_json::from_value(raw)
            .map_err(|error| refused(format!("its save is not a workspace: {error}")))?;
        let identity_cwd = legacy
            .root_pane
            .or_else(|| first_leaf(&legacy.layout))
            .and_then(|pane| legacy.panes.get(&pane))
            .or_else(|| legacy.panes.values().next())
            .map(|pane| pane.cwd.clone())
            .unwrap_or_default();
        return Ok(HerdrWorkspace {
            id: None,
            custom_name: legacy.custom_name,
            identity_cwd,
            worktree_space: None,
            public_pane_numbers: HashMap::new(),
            next_public_pane_number: 0,
            public_tab_numbers: Vec::new(),
            next_public_tab_number: 0,
            tabs: vec![HerdrTab {
                custom_name: None,
                layout: legacy.layout,
                panes: legacy.panes,
                zoomed: legacy.zoomed,
                focused: legacy.focused,
            }],
            active_tab: 0,
        });
    }
    Err(refused(
        "its save is neither a workspace of this format nor of the one before tabs".to_owned(),
    ))
}

fn first_leaf(layout: &HerdrLayout) -> Option<u32> {
    match layout {
        HerdrLayout::Pane(pane) => Some(*pane),
        HerdrLayout::Split { first, second, .. } => {
            first_leaf(first).or_else(|| first_leaf(second))
        }
    }
}

fn leaves(layout: &HerdrLayout, into: &mut Vec<u32>) {
    match layout {
        HerdrLayout::Pane(pane) => into.push(*pane),
        HerdrLayout::Split { first, second, .. } => {
            leaves(first, into);
            leaves(second, into);
        }
    }
}

/// A workspace moved into the engine's saved form.
#[derive(Debug)]
pub(crate) struct Moved {
    pub(super) saved: SavedWorkspace,
}

/// What a Herdr session becomes: the workspaces it moves, and those it
/// leaves out with why.
#[derive(Debug, Default)]
pub(crate) struct Converted {
    pub(crate) moved: Vec<Moved>,
    pub(crate) refused: Vec<Refused>,
}

impl Converted {
    /// How many tabs the moved workspaces hold.
    pub(crate) fn tabs(&self) -> usize {
        self.moved.iter().map(|moved| moved.saved.tabs.len()).sum()
    }

    /// How many of their panes ran an agent Herdr saved.
    pub(crate) fn agents(&self) -> usize {
        self.moved
            .iter()
            .flat_map(|moved| &moved.saved.tabs)
            .flat_map(|tab| &tab.panes)
            .filter(|pane| pane.agent.is_some())
            .count()
    }
}

/// Moves every workspace of `session` into the engine's saved form, except
/// one an earlier import moved (`moved_before`), which is Hide's from then on
/// and is passed over (D-9), and one the engine already holds an id of
/// (`held`), which is left out with why: only a session a development build
/// saved before this engine's own workspaces were numbered apart from
/// Herdr's can hold one (D-26). A workspace Herdr saved with no id (the form
/// before tabs) is numbered by `engine_id`, as Herdr numbers it again at each
/// restore, and is moved again by each import, as it has no id to keep.
pub(crate) fn convert(
    session: &HerdrSession,
    moved_before: &BTreeSet<String>,
    held: &BTreeSet<String>,
    mut engine_id: impl FnMut() -> String,
) -> Converted {
    let mut converted = Converted::default();
    // An id two workspaces of the session share names neither of them.
    let mut counted: BTreeMap<&str, usize> = BTreeMap::new();
    for workspace in session.workspaces.iter().flatten() {
        if let Some(id) = workspace.id.as_deref() {
            *counted.entry(id).or_default() += 1;
        }
    }
    for (index, read) in session.workspaces.iter().enumerate() {
        let workspace = match read {
            Ok(workspace) => workspace,
            Err(refused) => {
                converted.refused.push(refused.clone());
                continue;
            }
        };
        let label = workspace_label(workspace);
        let named = |reason: String| Refused {
            workspace: workspace
                .id
                .clone()
                .unwrap_or_else(|| format!("#{}", index + 1)),
            label: label.clone(),
            reason,
        };
        let id = match workspace.id.as_deref() {
            Some(id) if moved_before.contains(id) => continue,
            None => engine_id(),
            Some(id) if counted.get(id).copied().unwrap_or(0) > 1 => {
                converted.refused.push(named(format!(
                    "another workspace of the save has its id {id}"
                )));
                continue;
            }
            Some(id) if !matches!(ids::workspace_of(id), Some((Numbered::Imported, _))) => {
                converted
                    .refused
                    .push(named(format!("{id} is not a workspace id Herdr makes")));
                continue;
            }
            Some(id) if held.contains(id) => {
                converted.refused.push(named(format!(
                    "Hide's engine already has a workspace {id}, which a development build made"
                )));
                continue;
            }
            Some(id) => id.to_owned(),
        };
        match moved(workspace, &id, label.clone()) {
            Ok(moved) => converted.moved.push(moved),
            Err(reason) => converted.refused.push(named(reason)),
        }
    }
    converted
}

/// The name Herdr showed a workspace by: its own, else its folder's.
fn workspace_label(workspace: &HerdrWorkspace) -> String {
    workspace
        .custom_name
        .clone()
        .filter(|name| !name.is_empty())
        .or_else(|| {
            Path::new(&workspace.identity_cwd)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| workspace.identity_cwd.clone())
}

/// One workspace in the engine's saved form under `id`, its tabs and panes
/// under the ids Herdr's restore gives them (`src/persist/restore.rs`,
/// `restore_workspace`), or why it cannot be moved.
fn moved(workspace: &HerdrWorkspace, id: &str, label: String) -> Result<Moved, String> {
    if workspace.tabs.is_empty() {
        return Err("it holds no tab".to_owned());
    }
    // A pane Herdr saved no public number for takes the next one, in the
    // order Herdr's restore gives them: tab by tab, leaf by leaf.
    let mut next_pane = workspace
        .public_pane_numbers
        .values()
        .copied()
        .max()
        .map_or(1, |max| max + 1)
        .max(workspace.next_public_pane_number);
    let mut numbers = workspace.public_pane_numbers.clone();
    for tab in &workspace.tabs {
        let mut found = Vec::new();
        leaves(&tab.layout, &mut found);
        for raw in found {
            numbers.entry(raw).or_insert_with(|| {
                let number = next_pane;
                next_pane += 1;
                number
            });
        }
    }
    let mut seen = BTreeSet::new();
    let mut tabs = Vec::new();
    let mut tab_numbers = workspace.next_public_tab_number.saturating_sub(1);
    let mut pane_numbers = next_pane.saturating_sub(1);
    for (index, tab) in workspace.tabs.iter().enumerate() {
        let number = workspace
            .public_tab_numbers
            .get(index)
            .copied()
            .unwrap_or(index + 1);
        if number == 0 {
            return Err(format!("its tab {} has no number", index + 1));
        }
        tab_numbers = tab_numbers.max(number);
        let tab_id = ids::tab_id(id, number);
        if !seen.insert(tab_id.clone()) {
            return Err(format!("two of its tabs are {tab_id}"));
        }
        let mut laid = Vec::new();
        leaves(&tab.layout, &mut laid);
        let unique: BTreeSet<u32> = laid.iter().copied().collect();
        if unique.len() != laid.len()
            || unique != tab.panes.keys().copied().collect::<BTreeSet<_>>()
        {
            return Err(format!(
                "its tab {tab_id} does not lay out exactly the panes it holds"
            ));
        }
        if laid.len() > MAX_PANES {
            return Err(format!(
                "its tab {tab_id} holds more than {MAX_PANES} panes"
            ));
        }
        let pane_id = |raw: u32| -> Result<String, String> {
            let number = numbers
                .get(&raw)
                .copied()
                .filter(|number| *number > 0)
                .ok_or_else(|| format!("its pane {raw} has no number"))?;
            Ok(ids::pane_id(id, number))
        };
        let root =
            node(&tab.layout, &pane_id, 1).map_err(|why| format!("its tab {tab_id} {why}"))?;
        let mut panes = Vec::new();
        for raw in &laid {
            let pane = &tab.panes[raw];
            let pane_id = pane_id(*raw)?;
            pane_numbers = pane_numbers.max(numbers[raw]);
            if !seen.insert(pane_id.clone()) {
                return Err(format!("two of its panes are {pane_id}"));
            }
            panes.push(SavedPane {
                id: pane_id,
                cwd: wire_folder(&pane.cwd)?,
                label: pane.label.clone().filter(|label| !label.is_empty()),
                tokens: BTreeMap::new(),
                agent: agent_of(pane),
            });
        }
        let focused = tab
            .focused
            .filter(|focused| tab.panes.contains_key(focused))
            .or_else(|| laid.first().copied())
            .ok_or_else(|| format!("its tab {tab_id} holds no pane"))?;
        tabs.push(SavedTab {
            label: tab.custom_name.clone().unwrap_or_default(),
            focused: pane_id(focused)?,
            zoomed: tab.zoomed && laid.len() > 1,
            root,
            panes,
            id: tab_id,
        });
    }
    let active_tab = tabs
        .get(workspace.active_tab.min(tabs.len() - 1))
        .map(|tab| tab.id.clone())
        .unwrap_or_default();
    let worktree = workspace
        .worktree_space
        .as_ref()
        .map(|space| {
            Ok::<_, String>(SavedWorktree {
                repo_key: space.key.clone(),
                repo_name: space.label.clone(),
                repo_root: wire_folder(&space.repo_root)?,
                checkout_path: wire_folder(&space.checkout_path)?,
                linked: space.is_linked_worktree,
            })
        })
        .transpose()?;
    Ok(Moved {
        saved: SavedWorkspace {
            id: id.to_owned(),
            label,
            tokens: BTreeMap::new(),
            worktree,
            active_tab,
            tab_numbers,
            pane_numbers,
            tabs,
        },
    })
}

/// A tab's layout in the engine's saved form, or why it cannot be one: a
/// ratio outside what the engine's layout keeps, or deeper than a tab holds.
fn node(
    layout: &HerdrLayout,
    pane_id: &dyn Fn(u32) -> Result<String, String>,
    depth: usize,
) -> Result<SavedNode, String> {
    if depth > MAX_DEPTH {
        return Err(format!("is deeper than {MAX_DEPTH} levels"));
    }
    match layout {
        HerdrLayout::Pane(raw) => Ok(SavedNode::Pane { id: pane_id(*raw)? }),
        HerdrLayout::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            if !RATIOS.contains(ratio) {
                return Err(format!("splits at {ratio}, outside 0.1 to 0.9"));
            }
            Ok(SavedNode::Split {
                direction: match direction {
                    HerdrDirection::Horizontal => SavedDirection::Right,
                    HerdrDirection::Vertical => SavedDirection::Down,
                },
                ratio: *ratio,
                first: Box::new(node(first, pane_id, depth + 1)?),
                second: Box::new(node(second, pane_id, depth + 1)?),
            })
        }
    }
}

/// The agent a pane ran as Herdr saved it: the kind its session or resume
/// names (else the agent Herdr managed there), the name Herdr started it
/// under, and its session, an id or a session file's path, when known.
fn agent_of(pane: &HerdrPane) -> Option<SavedAgent> {
    let kind = pane
        .agent_session
        .as_ref()
        .map(|session| session.agent.clone())
        .or_else(|| {
            pane.agent_resume
                .as_ref()
                .map(|resume| resume.agent.clone())
        })
        .or_else(|| pane.managed_agent_kind.clone())
        .filter(|kind| !kind.trim().is_empty())?;
    Some(SavedAgent {
        kind,
        name: pane.agent_name.clone().filter(|name| !name.is_empty()),
        session: pane
            .agent_session
            .as_ref()
            .map(|session| session.value.clone())
            .filter(|session| !session.is_empty()),
    })
}

/// A folder Herdr saved, in the wire spelling the engine keeps folders in.
fn wire_folder(folder: &str) -> Result<String, String> {
    hide_platform::path::to_wire(Path::new(folder))
        .map_err(|error| format!("its folder {folder} has no whole spelling: {error}"))
}

/// An agent conversation an import did not resume, because the Herdr it
/// came from still runs and has it open (D-8): the import record keeps it
/// until it is resumed or its pane is closed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingResume {
    pub(crate) pane: String,
    pub(crate) kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session: Option<String>,
}

/// What an import added to the engine's saved session.
#[derive(Debug, Default)]
pub(crate) struct Imported {
    /// The workspaces moved, in the order Herdr kept them.
    pub(crate) workspaces: Vec<String>,
    pub(crate) tabs: usize,
    /// The agents that resume when the engine opens.
    pub(crate) agents: usize,
    pub(crate) refused: Vec<Refused>,
    /// The agents left waiting, since `resume` was false.
    pub(crate) pending: Vec<PendingResume>,
}

/// Adds the workspaces of `session` that no import moved before
/// (`moved_before`) to the engine's saved session in `engine_dir`, behind
/// the workspaces it holds, so the engine's next open restores them as it
/// restores its own (D-3): their panes open their folders and their agents
/// resume. With `resume` false (the Herdr they came from still runs and has
/// their conversations open, D-8) no agent is saved; each is handed back as
/// pending for the import record to keep. It runs while no engine is open on
/// `engine_dir`, which no other writer touches then; a saved session that
/// does not read is set aside as the engine's open sets it aside, and the
/// moved workspaces are saved alone.
pub(crate) fn import(
    engine_dir: &Path,
    session: &HerdrSession,
    moved_before: &BTreeSet<String>,
    resume: bool,
) -> Result<Imported, String> {
    let path = hide_kit::layout::engine_session_in(engine_dir);
    let backups = hide_kit::layout::engine_backups_in(engine_dir);
    let mut saved = super::restore::read(&path, &backups).to_saved();
    let held: BTreeSet<String> = saved
        .workspaces
        .iter()
        .map(|workspace| workspace.id.clone())
        .collect();
    let mut numbers = saved.workspace_numbers;
    let mut given = BTreeSet::new();
    let converted = convert(session, moved_before, &held, || {
        loop {
            numbers += 1;
            let id = ids::workspace_id(numbers);
            if !held.contains(&id) && given.insert(id.clone()) {
                break id;
            }
        }
    });
    saved.workspace_numbers = numbers;
    let mut imported = Imported {
        refused: converted.refused,
        ..Imported::default()
    };
    for mut moved in converted.moved {
        imported.workspaces.push(moved.saved.id.clone());
        imported.tabs += moved.saved.tabs.len();
        for pane in moved.saved.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
            if resume {
                imported.agents += usize::from(pane.agent.is_some());
            } else if let Some(agent) = pane.agent.take() {
                imported.pending.push(PendingResume {
                    pane: pane.id.clone(),
                    kind: agent.kind,
                    name: agent.name,
                    session: agent.session,
                });
            }
        }
        saved.workspaces.push(moved.saved);
    }
    if imported.workspaces.is_empty() {
        return Ok(imported);
    }
    // The engine reads the session back as it will at its open: one that
    // would not hold together is not written over the one it holds.
    let topology = super::topology::Topology::from_saved(saved)
        .map_err(|why| format!("the moved session does not hold together: {why}"))?;
    super::store::write(&path, &topology.to_saved())?;
    Ok(imported)
}

/// Gives each conversation of `pending` back to its pane in the engine's
/// saved session in `engine_dir`, where that pane is still saved with no
/// agent of its own: the engine's open resumes it there (D-8, B15). The
/// panes given back.
pub(crate) fn give_back(
    engine_dir: &Path,
    pending: &[PendingResume],
) -> Result<BTreeSet<String>, String> {
    let path = hide_kit::layout::engine_session_in(engine_dir);
    let backups = hide_kit::layout::engine_backups_in(engine_dir);
    let mut saved = super::restore::read(&path, &backups).to_saved();
    let mut given = BTreeSet::new();
    for pane in saved
        .workspaces
        .iter_mut()
        .flat_map(|workspace| &mut workspace.tabs)
        .flat_map(|tab| &mut tab.panes)
        .filter(|pane| pane.agent.is_none())
    {
        if let Some(waiting) = pending.iter().find(|waiting| waiting.pane == pane.id) {
            pane.agent = Some(super::store::SavedAgent {
                kind: waiting.kind.clone(),
                name: waiting.name.clone(),
                session: waiting.session.clone(),
            });
            given.insert(pane.id.clone());
        }
    }
    if given.is_empty() {
        return Ok(given);
    }
    let topology = super::topology::Topology::from_saved(saved).map_err(|why| {
        format!("the session with its conversations back does not hold together: {why}")
    })?;
    super::store::write(&path, &topology.to_saved())?;
    Ok(given)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(release: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/herdr-session")
                .join(format!("herdr-{release}.json")),
        )
        .unwrap()
    }

    /// Every workspace, tab and pane id a saved workspace holds, in order.
    fn ids_of(workspace: &SavedWorkspace) -> Vec<&str> {
        std::iter::once(workspace.id.as_str())
            .chain(workspace.tabs.iter().flat_map(|tab| {
                std::iter::once(tab.id.as_str())
                    .chain(tab.panes.iter().map(|pane| pane.id.as_str()))
            }))
            .collect()
    }

    fn no_engine_id() -> String {
        unreachable!("every workspace of a current save has its id")
    }

    // B33: each Herdr release Hide has shipped saved the same session in
    // the same form, and it reads back as Herdr's restore lays it out: the
    // same ids, folders, names, split and agent session.
    #[test]
    fn a_session_each_herdr_release_saved_moves_under_herdr_s_own_ids() {
        for release in ["0.9.5", "0.9.1", "0.8.2"] {
            let session = parse(&fixture(release)).unwrap();
            let converted = convert(&session, &BTreeSet::new(), &BTreeSet::new(), no_engine_id);
            assert!(
                converted.refused.is_empty(),
                "{release}: {:?}",
                converted.refused
            );
            let ids: Vec<Vec<&str>> = converted
                .moved
                .iter()
                .map(|moved| ids_of(&moved.saved))
                .collect();
            assert_eq!(
                ids,
                [
                    vec!["w1", "w1:t1", "w1:p1", "w1:p2", "w1:t2", "w1:p4"],
                    vec!["w2", "w2:t1", "w2:p1"],
                ],
                "{release}"
            );
            let first = &converted.moved[0].saved;
            assert_eq!(first.label, "proj");
            assert_eq!(first.active_tab, "w1:t1");
            // The next pane is past the one closed before the save (w1:p3).
            assert_eq!((first.tab_numbers, first.pane_numbers), (2, 4));
            let tab = &first.tabs[0];
            assert!(tab.zoomed);
            assert_eq!(tab.focused, "w1:p1");
            assert!(matches!(
                &tab.root,
                SavedNode::Split { direction: SavedDirection::Right, ratio, .. } if *ratio == 0.5
            ));
            assert!(tab.panes[0].cwd.ends_with("/repo"), "{}", tab.panes[0].cwd);
            let agent = tab.panes[0].agent.as_ref().unwrap();
            assert_eq!(
                (agent.kind.as_str(), agent.session.as_deref()),
                ("claude", Some("0b6a3f1e-claude-session"))
            );
            assert_eq!(tab.panes[1].label.as_deref(), Some("--label notes"));
            assert_eq!(first.tabs[1].label, "second");
        }
    }

    // D-3, B11: a workspace that does not hold together is left out with
    // its reason and the rest move.
    #[test]
    fn a_workspace_that_does_not_hold_together_is_left_out_and_the_rest_move() {
        let mut session: Value = serde_json::from_str(&fixture("0.9.5")).unwrap();
        let mut broken = session["workspaces"][1].clone();
        broken["id"] = "w7".into();
        broken["tabs"][0]["layout"] = serde_json::json!({"Split": {
            "direction": "Vertical", "ratio": 0.95,
            "first": {"Pane": 5}, "second": {"Pane": 6}}});
        broken["tabs"][0]["panes"]["6"] = serde_json::json!({"cwd": "/srv"});
        let mut twin = session["workspaces"][1].clone();
        twin["custom_name"] = "twin".into();
        let workspaces = session["workspaces"].as_array_mut().unwrap();
        workspaces.push(broken);
        workspaces.push(twin);
        workspaces.push(serde_json::json!({"neither": true}));
        let converted = convert(
            &parse(&session.to_string()).unwrap(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            no_engine_id,
        );
        let moved: Vec<&str> = converted
            .moved
            .iter()
            .map(|moved| moved.saved.id.as_str())
            .collect();
        assert_eq!(moved, ["w1"]);
        let refused: Vec<(&str, &str)> = converted
            .refused
            .iter()
            .map(|refused| (refused.workspace.as_str(), refused.reason.as_str()))
            .collect();
        assert_eq!(refused.len(), 4, "{refused:?}");
        assert!(refused.contains(&("w2", "another workspace of the save has its id w2")));
        assert!(
            refused
                .iter()
                .any(|(id, why)| *id == "w7" && why.contains("splits at 0.95")),
            "{refused:?}"
        );
        assert!(refused.iter().any(|(id, _)| *id == "#5"));
    }

    // D-26: a workspace the engine already holds (a development build's
    // save from before the engine numbered its own apart) is left out.
    #[test]
    fn a_workspace_the_engine_already_holds_is_left_out_with_why() {
        let converted = convert(
            &parse(&fixture("0.9.5")).unwrap(),
            &BTreeSet::new(),
            &BTreeSet::from(["w2".to_owned()]),
            no_engine_id,
        );
        assert_eq!(converted.moved.len(), 1);
        assert!(
            converted.refused[0]
                .reason
                .contains("already has a workspace w2")
        );
    }

    // The form before tabs: one tab inline, no id or public numbers, which
    // Herdr numbers again when it restores it.
    #[test]
    fn a_workspace_saved_before_tabs_moves_under_a_new_id() {
        let text = serde_json::json!({
            "version": 1,
            "workspaces": [{
                "custom_name": "old",
                "layout": {"Split": {"direction": "Vertical", "ratio": 0.4,
                    "first": {"Pane": 3}, "second": {"Pane": 9}}},
                "panes": {"3": {"cwd": "/srv/a"}, "9": {"cwd": "/srv/b", "managed_agent_kind": "codex"}},
                "zoomed": false,
                "focused": 9
            }],
            "active": 0,
            "selected": 0
        })
        .to_string();
        let converted = convert(
            &parse(&text).unwrap(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            || "h4".to_owned(),
        );
        let moved = &converted.moved[0].saved;
        assert_eq!(moved.id, "h4");
        assert_eq!(moved.tabs[0].id, "h4:t1");
        assert_eq!(moved.tabs[0].focused, "h4:p2");
        assert!(matches!(
            moved.tabs[0].root,
            SavedNode::Split {
                direction: SavedDirection::Down,
                ..
            }
        ));
        let agent = moved.tabs[0].panes[1].agent.as_ref().unwrap();
        assert_eq!(
            (agent.kind.as_str(), agent.session.as_deref()),
            ("codex", None)
        );
    }

    // D-3, D-9: an import adds Herdr's workspaces behind the engine's own,
    // the engine opens them as it saved them, and importing the same save
    // again moves nothing twice.
    #[test]
    fn an_import_adds_herdr_s_workspaces_beside_the_engine_s_and_converges() {
        let dir = tempfile::tempdir().unwrap();
        let mut own = super::super::topology::Topology::default();
        let first = own.reserve_workspace();
        own.add_workspace(
            &first,
            "own",
            super::super::topology::Pane::new("/srv/own", None),
            true,
        );
        super::super::store::write(
            &hide_kit::layout::engine_session_in(dir.path()),
            &own.to_saved(),
        )
        .unwrap();
        let session = parse(&fixture("0.9.5")).unwrap();
        let imported = import(dir.path(), &session, &BTreeSet::new(), true).unwrap();
        assert_eq!(imported.workspaces, ["w1", "w2"]);
        assert_eq!((imported.tabs, imported.agents), (3, 1));
        assert!(imported.pending.is_empty());
        let opened = super::super::restore::read(
            &hide_kit::layout::engine_session_in(dir.path()),
            &hide_kit::layout::engine_backups_in(dir.path()),
        )
        .to_saved();
        let ids: Vec<&str> = opened
            .workspaces
            .iter()
            .map(|workspace| workspace.id.as_str())
            .collect();
        assert_eq!(ids, ["h1", "w1", "w2"]);
        let agent = opened.workspaces[1].tabs[0].panes[0]
            .agent
            .as_ref()
            .unwrap();
        assert_eq!(agent.session.as_deref(), Some("0b6a3f1e-claude-session"));

        let again = import(
            dir.path(),
            &session,
            &imported.workspaces.iter().cloned().collect(),
            true,
        )
        .unwrap();
        assert!(
            again.workspaces.is_empty() && again.refused.is_empty(),
            "{again:?}"
        );
    }

    // D-8: while the Herdr an import reads still runs, no conversation it has
    // open is saved to resume; each waits for the import record.
    #[test]
    fn an_import_beside_a_running_herdr_saves_no_agent_and_hands_each_back() {
        let dir = tempfile::tempdir().unwrap();
        let session = parse(&fixture("0.9.5")).unwrap();
        let imported = import(dir.path(), &session, &BTreeSet::new(), false).unwrap();
        assert_eq!(imported.agents, 0);
        assert_eq!(
            imported.pending,
            [PendingResume {
                pane: "w1:p1".to_owned(),
                kind: "claude".to_owned(),
                name: None,
                session: Some("0b6a3f1e-claude-session".to_owned()),
            }]
        );
        let opened = super::super::restore::read(
            &hide_kit::layout::engine_session_in(dir.path()),
            &hide_kit::layout::engine_backups_in(dir.path()),
        )
        .to_saved();
        assert!(
            opened
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.tabs)
                .flat_map(|tab| &tab.panes)
                .all(|pane| pane.agent.is_none())
        );
    }

    // B15: once the Herdr stops, a waiting conversation goes back to its
    // pane, and only to a pane still saved with no agent of its own.
    #[test]
    fn a_waiting_conversation_goes_back_to_its_pane() {
        let dir = tempfile::tempdir().unwrap();
        let session = parse(&fixture("0.9.5")).unwrap();
        let imported = import(dir.path(), &session, &BTreeSet::new(), false).unwrap();
        let mut pending = imported.pending.clone();
        pending.push(PendingResume {
            pane: "w9:p1".to_owned(),
            kind: "codex".to_owned(),
            name: None,
            session: None,
        });
        assert_eq!(
            give_back(dir.path(), &pending).unwrap(),
            BTreeSet::from(["w1:p1".to_owned()])
        );
        let opened = super::super::restore::read(
            &hide_kit::layout::engine_session_in(dir.path()),
            &hide_kit::layout::engine_backups_in(dir.path()),
        )
        .to_saved();
        let agents: Vec<(&str, &str)> = opened
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.tabs)
            .flat_map(|tab| &tab.panes)
            .filter_map(|pane| Some((pane.id.as_str(), pane.agent.as_ref()?.kind.as_str())))
            .collect();
        assert_eq!(agents, [("w1:p1", "claude")]);
        // A second pass finds the pane taken and gives nothing.
        assert!(give_back(dir.path(), &pending).unwrap().is_empty());
    }

    // B11: a session this build cannot read says why; a missing one is none.
    #[test]
    fn a_newer_or_broken_session_is_unreadable_and_a_missing_one_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let read = |path: &Path| of(&hide_host::herdr_save::fetch(path));
        assert!(matches!(read(&path), HerdrRead::Missing));
        std::fs::write(&path, r#"{"version": 4, "workspaces": []}"#).unwrap();
        assert!(matches!(read(&path), HerdrRead::Unreadable(why) if why.contains("format 4")));
        std::fs::write(&path, "{").unwrap();
        assert!(matches!(read(&path), HerdrRead::Unreadable(_)));
        assert!(path.exists(), "the file is only read");
    }
}
