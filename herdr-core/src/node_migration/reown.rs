//! The owner change a core move makes to a copy of the state folder (PRD
//! core-host-node-move D-03, B4, B6).
//!
//! A folder names its owner's machine by leaving keys unqualified and every
//! other machine by an id: a dialed device by the registration id the
//! operator gave it (`mini`, `remote:mini:pane:w1:p1`), a node that links in
//! by its node id (`remote:<node>:pane:w1:p1`). When the core moves, the
//! machine taking it becomes the owner, so its keys lose their qualifier,
//! and the machine giving it up gains one. `reown` makes that change on a
//! staging copy, through the same `KEYS` table and marker as the layer-1
//! conversion: each row says how its value moves (`Moves`), and a key that
//! still names the new owner by its old id afterwards stops the move with its
//! file and place (engineering principle 4). The source folder is never
//! written. A move back is the same change with the two machines swapped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{
    CORE_STATE, DELIVERY_LEDGER, FACTORY, KEYS, LABELS, LINKS, LOCAL_ISSUES, MARKER_FILE,
    MARKER_VERSION, Marker, Moves, PROJECT_MEMORY, Refusal, SESSION_SEARCH, WORKSPACE_VIEWS,
    read_json, read_marker, write,
};
use crate::node::NodeId;

/// The two machines of a move, by the names the folder uses for them before
/// and after it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnerChange {
    /// The node whose core wrote the folder; `node.json` must name it.
    pub old_owner: String,
    /// The id the result gives the old owner: its node id when it links in to
    /// the new core, the registration id it was dialed by when the new core
    /// dials it.
    pub old_owner_as: String,
    /// The node taking the core.
    pub new_owner: String,
    /// The id the folder gives the new owner before the move.
    pub new_owner_was: String,
    /// The registration the result keeps for the old owner.
    pub old_owner_registration: crate::model::DeviceRegistration,
    /// Each machine's Herdr socket, which names it in the ledger's host
    /// scopes while it owns the core. Both are required: a scope naming a
    /// machine whose socket is unknown would keep its old id unnoticed.
    pub old_owner_herdr_socket: String,
    pub new_owner_herdr_socket: String,
}

impl OwnerChange {
    fn device(&self, id: &str) -> Option<String> {
        if id == self.new_owner_was {
            Some(self.new_owner.clone())
        } else if id == self.old_owner {
            Some(self.old_owner_as.clone())
        } else {
            None
        }
    }

    /// A Herdr-scoped id (`pane`, `tab`): unqualified for the owner,
    /// `remote:<device>:<kind>:<raw>` for every other machine.
    fn scoped(&self, kind: &str, id: &str) -> Option<String> {
        let target = format!("remote:{}:{kind}:", self.new_owner_was);
        if let Some(raw) = id.strip_prefix(&target) {
            return Some(raw.to_owned());
        }
        if id.starts_with("remote:") || id.is_empty() {
            return None;
        }
        Some(format!("remote:{}:{kind}:{id}", self.old_owner_as))
    }
}

/// The registration and checkout ids of the two machines before and after
/// the move. The source core computes it from what it knows of each path
/// before it stops (`id_table`), because a project id is a digest of a root
/// only the machine holding it can resolve.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct IdTable {
    pub registrations: BTreeMap<String, String>,
    pub checkouts: BTreeMap<String, String>,
}

/// One registered project of either machine, as the source core knows it.
pub struct KnownProject<'a> {
    pub id: &'a str,
    pub device_id: &'a str,
    pub path: &'a str,
    /// The repository root the project id digests; the old owner's are
    /// resolved on the source machine, the new owner's are its registered
    /// path, which its node reported as the root.
    pub root: &'a Path,
    pub checkouts: Vec<(&'a str, &'a str)>,
}

/// The ids each known project and checkout of the two machines takes, made
/// with the functions the core makes them with.
pub fn id_table<'a>(
    change: &OwnerChange,
    projects: impl IntoIterator<Item = KnownProject<'a>>,
) -> IdTable {
    let mut table = IdTable::default();
    for project in projects {
        if project.device_id == change.old_owner {
            let id = crate::device_catalog::project_id(&change.old_owner_as, project.root);
            table.registrations.insert(project.id.to_owned(), id);
            for (checkout, path) in project.checkouts {
                table.checkouts.insert(
                    checkout.to_owned(),
                    crate::device_catalog::checkout_id(&change.old_owner_as, path),
                );
            }
        } else if project.device_id == change.new_owner_was {
            let id = crate::workspace::workspace_id_for_path(Path::new(project.path));
            for (checkout, path) in project.checkouts {
                table.checkouts.insert(
                    checkout.to_owned(),
                    crate::workspace::checkout_id_for_path(&id, Path::new(path)),
                );
            }
            table.registrations.insert(project.id.to_owned(), id);
        }
    }
    table
}

/// What an owner change did, for the move's diagnostic record.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct ReownOutcome {
    pub files: Vec<String>,
    /// Fold and recent entries naming a checkout or project neither machine
    /// still has, dropped because no id can be made for them.
    pub pruned: usize,
    /// Why the link record could not be opened, when it could not: it moves
    /// as it is and the new core's link worker rebuilds it, as after layer 1.
    pub links_unopened: Option<String>,
}

/// Changes the owner of the copy at `staging` from `change.old_owner` to
/// `change.new_owner`. A refusal names the file and leaves the copy for the
/// caller to remove; the copy is never used unless this returns.
pub fn reown(staging: &Path, change: &OwnerChange, ids: &IdTable) -> Result<ReownOutcome, Refusal> {
    let refuse = |file: &Path, reason: String| Refusal {
        file: file.to_path_buf(),
        reason,
    };
    let marker_path = staging.join(MARKER_FILE);
    match read_marker(&marker_path).map_err(|reason| refuse(&marker_path, reason))? {
        Some(owner) if owner == change.old_owner => {}
        Some(owner) => {
            return Err(refuse(
                &marker_path,
                format!(
                    "this folder belongs to node {owner}, not to {}, the machine the core \
                     moves from",
                    change.old_owner
                ),
            ));
        }
        None => {
            return Err(refuse(
                &marker_path,
                "this folder has no owner yet; the layer-1 conversion runs first".to_owned(),
            ));
        }
    }
    NodeId::parse(&change.new_owner).map_err(|error| refuse(staging, error))?;

    let mut outcome = ReownOutcome::default();
    // First, since the ledger names the cards of a Project folded into
    // another by the numbers the fold changed.
    let memory = staging.join(PROJECT_MEMORY);
    let mut renumbered = Vec::new();
    if memory.is_file() {
        let (changed, folded) =
            reown_memory(&memory, change).map_err(|reason| refuse(&memory, reason))?;
        if changed {
            outcome.files.push(PROJECT_MEMORY.to_owned());
        }
        renumbered = folded;
    }
    let mut mapper = Mapper {
        change,
        ids,
        renumbered: &renumbered,
        pruned: 0,
    };
    for file in [
        CORE_STATE,
        WORKSPACE_VIEWS,
        LABELS,
        DELIVERY_LEDGER,
        LOCAL_ISSUES,
    ] {
        let path = staging.join(file);
        let Some(mut value) = read_json(&path).map_err(|reason| refuse(&path, reason))? else {
            continue;
        };
        let before = value.clone();
        for (_, pattern, _, moves) in KEYS.iter().filter(|(listed, ..)| *listed == file) {
            mapper
                .apply(&mut value, pattern, *moves)
                .map_err(|reason| refuse(&path, format!("{pattern}: {reason}")))?;
        }
        mapper
            .whole_file(file, &mut value)
            .map_err(|reason| refuse(&path, reason))?;
        if let Some(place) = still_naming(&value, change, "") {
            return Err(refuse(
                &path,
                format!(
                    "{place} still names {} after the move; no row of the key table covers it",
                    change.new_owner_was
                ),
            ));
        }
        if value != before {
            let bytes =
                serde_json::to_vec(&value).map_err(|error| refuse(&path, error.to_string()))?;
            write(&path, &bytes).map_err(|reason| refuse(&path, reason))?;
            outcome.files.push(file.to_owned());
        }
    }

    let links = staging.join(LINKS);
    // Read before the link record's projects take their new keys.
    let retention_pairs = if links.is_file() {
        project_key_pairs(&links, change).map_err(|reason| refuse(&links, reason))?
    } else {
        Vec::new()
    };
    if links.is_file() {
        match reown_links(&links, change).map_err(|reason| refuse(&links, reason))? {
            Links::Changed => outcome.files.push(LINKS.to_owned()),
            Links::Unchanged => {}
            Links::Unopened(reason) => outcome.links_unopened = Some(reason),
        }
    }

    // The session search file is only ever read (D-20): the link record
    // names which ids its retention rows now belong to.
    if staging.join(SESSION_SEARCH).is_file()
        && !retention_pairs.is_empty()
        && outcome.links_unopened.is_none()
    {
        let (mut store, _) = crate::links::store::LinkStore::open(&links)
            .map_err(|reason| refuse(&links, reason))?;
        if store
            .alias_retention(&retention_pairs)
            .map_err(|reason| refuse(&links, reason))?
            > 0
            && !outcome.files.contains(&LINKS.to_owned())
        {
            outcome.files.push(LINKS.to_owned());
        }
    }

    let factory = staging.join(FACTORY);
    if factory.is_file()
        && reown_factory(staging, change).map_err(|reason| refuse(&factory, reason))?
    {
        outcome.files.push(FACTORY.to_owned());
    }

    let marker = serde_json::to_vec(&Marker {
        version: MARKER_VERSION,
        node: change.new_owner.clone(),
    })
    .map_err(|error| refuse(&marker_path, error.to_string()))?;
    write(&marker_path, &marker).map_err(|reason| refuse(&marker_path, reason))?;
    outcome.pruned = mapper.pruned;
    Ok(outcome)
}

struct Mapper<'a> {
    change: &'a OwnerChange,
    ids: &'a IdTable,
    /// The cards a fold of Project Memory gave other numbers (`reown_memory`).
    renumbered: &'a [Renumbered],
    pruned: usize,
}

/// The cards of a Project that a move folded into another copy of it
/// renumbered: the intent prefix its promotion starts carry, and each old
/// card number with its new one.
struct Renumbered {
    intent_prefix: String,
    numbers: BTreeMap<u64, u64>,
}

/// What a mapping makes of one value: the same, another, or nothing (an
/// entry of a fold or recent list that names what neither machine has).
enum Mapped {
    Same,
    To(String),
    Drop,
}

impl Mapper<'_> {
    fn map(&self, moves: Moves, value: &str) -> Mapped {
        let to = |mapped: Option<String>| mapped.map_or(Mapped::Same, Mapped::To);
        match moves {
            Moves::Device => to(self.change.device(value)),
            Moves::Pane => to(self.change.scoped("pane", value)),
            Moves::Tab => to(self.change.scoped("tab", value)),
            Moves::Registration => self.table(&self.ids.registrations, value),
            Moves::Checkout => self.table(&self.ids.checkouts, value),
            Moves::Fold => {
                // A device's own folds carry its id; a sleeping project's
                // fold carries the project's (PRD sidebar-titlebar-refresh D-19).
                if let Some(device) = value.strip_prefix("cleanup/") {
                    to(self.change.device(device).map(|id| format!("cleanup/{id}")))
                } else if let Some(device) = value.strip_prefix("no_agents/") {
                    to(self
                        .change
                        .device(device)
                        .map(|id| format!("no_agents/{id}")))
                } else if let Some(project) = value.strip_prefix("asleep/") {
                    match self.table(&self.ids.registrations, project) {
                        Mapped::To(id) => Mapped::To(format!("asleep/{id}")),
                        other => other,
                    }
                } else {
                    self.table(&self.ids.registrations, value)
                }
            }
            Moves::Stays | Moves::OwnPath(_) | Moves::Labels | Moves::Whole => Mapped::Same,
        }
    }

    /// A registration or checkout id of either machine maps through the
    /// table; one of another machine stays; one of the two machines the table
    /// lacks names something neither still has.
    fn table(&self, table: &BTreeMap<String, String>, value: &str) -> Mapped {
        if let Some(id) = table.get(value) {
            return Mapped::To(id.clone());
        }
        let theirs = format!("remote:{}:", self.change.new_owner_was);
        if value.starts_with("workspace:") || value.starts_with(&theirs) {
            Mapped::Drop
        } else {
            Mapped::Same
        }
    }

    fn apply(&mut self, value: &mut Value, pattern: &str, moves: Moves) -> Result<(), String> {
        if matches!(
            moves,
            Moves::Stays | Moves::OwnPath(_) | Moves::Labels | Moves::Whole
        ) {
            return Ok(());
        }
        let parts: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
        let mut pruned = 0;
        let result = walk(value, &parts, &mut |slot| match slot {
            Slot::Value(value) => {
                let Some(text) = value.as_str() else {
                    return Ok(false);
                };
                match self.map(moves, text) {
                    Mapped::Same => Ok(false),
                    Mapped::To(id) => {
                        *value = Value::String(id);
                        Ok(false)
                    }
                    Mapped::Drop => {
                        pruned += 1;
                        Ok(true)
                    }
                }
            }
            Slot::Keys(map) => {
                let keys: Vec<String> = map.keys().cloned().collect();
                let mut renamed = Map::new();
                for key in keys {
                    let entry = map.remove(&key).expect("listed key");
                    let key = match self.map(moves, &key) {
                        Mapped::Same => key,
                        Mapped::To(id) => id,
                        Mapped::Drop => {
                            pruned += 1;
                            continue;
                        }
                    };
                    if renamed.insert(key.clone(), entry).is_some() {
                        return Err(format!("two entries would both be {key}"));
                    }
                }
                *map = renamed;
                Ok(false)
            }
        });
        self.pruned += pruned;
        result.map(|_| ())
    }

    /// The rows a pattern cannot express: the owner's path-keyed settings
    /// and the `device_*` maps beside them, the Project settings, the device
    /// registrations, the labels, the ledger's host scopes and spawn machines, the local issues
    /// and the View layouts' tabs.
    fn whole_file(&mut self, file: &str, value: &mut Value) -> Result<(), String> {
        match file {
            CORE_STATE => {
                for (own, devices) in OWN_PATHS {
                    swap_own(value, own, devices, self.change)?;
                }
                // The old core held the new owner as a device, whose
                // selection is that machine's own Herdr focus and is never
                // saved, so the new core opens there.
                value["focus_from_herdr"] = Value::Bool(true);
                // A recent entry is its checkout: one whose checkout neither
                // machine still has goes, as a fold's does (its id was
                // counted as it was cleared).
                if let Some(recent) = value
                    .get_mut("recent_checkouts")
                    .and_then(Value::as_array_mut)
                {
                    recent.retain(|row| row.get("checkout_id").is_some_and(Value::is_string));
                }
                self.project_settings(value)?;
                self.device_registrations(value)
            }
            WORKSPACE_VIEWS => {
                for view in each(value, "workspaces") {
                    if let Some(layout) = view.get_mut("agent_layout") {
                        self.agent_layout(layout);
                    }
                    super::each_opener(view, &mut |device| match device
                        .as_str()
                        .map(|id| self.map(Moves::Device, id))
                    {
                        Some(Mapped::To(id)) => {
                            *device = Value::String(id);
                            true
                        }
                        _ => false,
                    });
                }
                Ok(())
            }
            LABELS => reown_labels(value, self.change),
            DELIVERY_LEDGER => {
                for agent in each(value, "agents") {
                    let scope = agent.get("host_scope").and_then(Value::as_str);
                    let mapped = match scope {
                        Some(scope) if scope == self.change.old_owner_herdr_socket => {
                            Some(Value::String(self.change.old_owner_as.clone()))
                        }
                        Some(scope) if scope == self.change.new_owner_was => {
                            Some(Value::String(self.change.new_owner_herdr_socket.clone()))
                        }
                        _ => None,
                    };
                    if let Some(mapped) = mapped {
                        agent["host_scope"] = mapped;
                    }
                }
                self.spawn_machines(value)?;
                self.promotion_targets(value);
                Ok(())
            }
            LOCAL_ISSUES => swap_own(value, "projects", "devices", self.change),
            _ => Ok(()),
        }
    }

    /// A Project's choices are kept at the digest of the id of the machine
    /// that holds it and the project's root, and name that id
    /// (`project_settings::key`). A machine the move gives another id takes
    /// its records with it: the record names the new id and moves to the
    /// digest of it, because a core reads a record only at the key its
    /// device and root make. The old owner's records are the core's own
    /// choices becoming a device's, the new owner's a device's becoming the
    /// core's own, and a third machine's stay as they are.
    fn project_settings(&mut self, value: &mut Value) -> Result<(), String> {
        let Some(records) = value
            .as_object_mut()
            .and_then(|object| object.get_mut("project_settings"))
        else {
            return Ok(());
        };
        let records = match records {
            Value::Object(records) => std::mem::take(records),
            Value::Null => return Ok(()),
            _ => return Err("project_settings is not a map".to_owned()),
        };
        let mut moved = Map::new();
        for (key, mut record) in records {
            let device = record.get("device_id").and_then(Value::as_str);
            let new_key = match device.map(|device| self.map(Moves::Device, device)) {
                Some(Mapped::To(device)) => {
                    let Some(root) = record.get("root").and_then(Value::as_str) else {
                        return Err(format!("/project_settings/{key} names no root"));
                    };
                    let new_key = crate::project_settings::key(&device, root);
                    record["device_id"] = Value::String(device);
                    new_key
                }
                _ => key,
            };
            if moved.insert(new_key.clone(), record).is_some() {
                return Err(format!("two project settings would both be {new_key}"));
            }
        }
        value["project_settings"] = Value::Object(moved);
        Ok(())
    }

    /// A promotion task names its card by number (`m3`), and a fold of
    /// Project Memory gave the cards of one copy new numbers, so the task is
    /// read through the new ones: the screen opens a task's card from this
    /// name. A start's intent carries the digest of its Project's id, which
    /// tells whose card a number is; the intent itself stays as it was,
    /// since it only has to be the same when asked again.
    fn promotion_targets(&self, value: &mut Value) {
        for spawn in each(value, "spawns") {
            let Some(intent) = spawn.get("intent").and_then(Value::as_str) else {
                continue;
            };
            let Some(renumbered) = self
                .renumbered
                .iter()
                .find(|renumbered| intent.starts_with(&renumbered.intent_prefix))
            else {
                continue;
            };
            let Some(task) = spawn.get_mut("task").filter(|task| {
                task.get("kind").and_then(Value::as_str)
                    == Some(crate::coordination::MEMORY_PROMOTE_TASK)
            }) else {
                continue;
            };
            let moved = task
                .get("target")
                .and_then(Value::as_str)
                .and_then(hide_memory::parse_card_name)
                .and_then(|number| renumbered.numbers.get(&number))
                .map(|number| hide_memory::card_name(*number));
            if let Some(name) = moved {
                task["target"] = Value::String(name);
            }
        }
    }

    /// A spawn's `machine` is the device its child lives on when that is not
    /// its caller's own, and none when it is (`coordination::spawn_machine`:
    /// the caller's machine stands in for it). So none says nothing about
    /// the core's machine: a delegation an agent on a device made names none
    /// and its child is on that device. The child's machine keeps its meaning
    /// by being read against the caller's machine after the move, which the
    /// agents' rows already carry: a machine the move renames is mapped, and
    /// one that is now the caller's own is written as none, since the ledger
    /// refuses a child whose machine is not the spawn's. A spawn whose caller
    /// the ledger lacks stops the move, since its meaning cannot be read.
    fn spawn_machines(&self, value: &mut Value) -> Result<(), String> {
        let machines: BTreeMap<String, String> = each(value, "agents")
            .filter_map(|agent| {
                Some((
                    agent.get("id")?.as_str()?.to_owned(),
                    agent.get("machine")?.as_str()?.to_owned(),
                ))
            })
            .collect();
        for (index, spawn) in each(value, "spawns").enumerate() {
            let Some(object) = spawn.as_object_mut() else {
                continue;
            };
            let Some(machine) = object.get("machine").and_then(Value::as_str) else {
                continue;
            };
            let caller = object
                .get("parent")
                .and_then(Value::as_str)
                .and_then(|parent| machines.get(parent))
                .ok_or_else(|| format!("/spawns/{index}/parent is not an agent of the ledger"))?;
            match self.change.device(machine) {
                Some(moved) if &moved == caller => {
                    object.remove("machine");
                }
                Some(moved) => {
                    object.insert("machine".to_owned(), Value::String(moved));
                }
                None if machine == caller => {
                    object.remove("machine");
                }
                None => {}
            }
        }
        Ok(())
    }

    /// The old owner gets the registration the change carries; the new
    /// owner's own registration goes, since a core does not register its own
    /// machine, and the name the operator gave it there stays as the core's
    /// `machine_label`. A node that dialed in was registered by its host
    /// name, no name of the operator's, so a move to it keeps none.
    fn device_registrations(&mut self, value: &mut Value) -> Result<(), String> {
        let Some(object) = value.as_object_mut() else {
            return Ok(());
        };
        let label = object
            .get("device_registrations")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter().find(|row| {
                    row.get("id").and_then(Value::as_str)
                        == Some(self.change.new_owner_was.as_str())
                })
            })
            .filter(|row| row.get("ssh_alias").is_some_and(Value::is_string))
            .and_then(|row| row.get("label"))
            .filter(|label| label.is_string())
            .cloned();
        match label {
            Some(label) => object.insert("machine_label".to_owned(), label),
            None => object.remove("machine_label"),
        };
        let registrations = object
            .entry("device_registrations")
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(rows) = registrations.as_array_mut() else {
            return Err("device_registrations is not a list".to_owned());
        };
        // The old owner takes the new owner's place in the list, so a move
        // and a move back leave the devices in the order they were. The
        // place is read once a row the old owner already had is gone, so it
        // counts only the rows that stay.
        let is = |row: &Value, id: &str| row.get("id").and_then(Value::as_str) == Some(id);
        rows.retain(|row| !is(row, &self.change.old_owner_as));
        let at = rows
            .iter()
            .position(|row| is(row, &self.change.new_owner_was));
        rows.retain(|row| !is(row, &self.change.new_owner_was));
        let registration = serde_json::to_value(&self.change.old_owner_registration)
            .map_err(|error| error.to_string())?;
        match at {
            Some(at) => rows.insert(at, registration),
            None => rows.push(registration),
        }
        Ok(())
    }

    /// Every tab an Agent layout names: each area's front display and its
    /// displays, and each canvas's tab.
    fn agent_layout(&mut self, layout: &mut Value) {
        fn node(mapper: &Mapper<'_>, value: &mut Value) {
            if let Some(area) = value.get_mut("area") {
                if let Some(active) = area.get_mut("active") {
                    tab(mapper, active);
                }
                for display in each(area, "displays") {
                    if let Some(id) = display.get_mut("id") {
                        tab(mapper, id);
                    }
                }
            }
            if let Some(split) = value.get_mut("split") {
                for side in ["first", "second"] {
                    if let Some(child) = split.get_mut(side) {
                        node(mapper, child);
                    }
                }
            }
        }
        fn tab(mapper: &Mapper<'_>, value: &mut Value) {
            if let Some(Mapped::To(id)) = value.as_str().map(|text| mapper.map(Moves::Tab, text)) {
                *value = Value::String(id);
            }
        }
        if let Some(root) = layout.get_mut("root") {
            node(self, root);
        }
        if let Some(canvases) = layout.get_mut("canvases").and_then(Value::as_object_mut) {
            for canvas in canvases.values_mut() {
                tab(self, canvas);
            }
        }
    }
}

/// The owner's settings that belong to its own machine and the map beside
/// each that keeps the same setting for every other machine, keyed by its
/// id (PRD core-host-node-move Q19): the Explorer's open folders, which the
/// web reads per machine. Settings keyed by a project path are read the same
/// whichever machine holds the project, so they stay (`Moves::Stays`).
pub const OWN_PATHS: &[(&str, &str)] = &[("expanded_paths", "device_expanded_paths")];

/// The old owner's `own` value moves under `devices[old_owner_as]`, and the
/// new owner's entry in `devices` becomes `own`; the rest of `devices`
/// changes only its keys. A new owner with no entry keeps `own` as an empty
/// value of its shape, because the reader requires some of these fields.
fn swap_own(
    value: &mut Value,
    own: &str,
    devices: &str,
    change: &OwnerChange,
) -> Result<(), String> {
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    let mine = object.remove(own);
    let emptied = mine.as_ref().map(|value| match value {
        Value::Array(_) => Value::Array(Vec::new()),
        Value::Object(_) => Value::Object(Map::new()),
        _ => Value::Null,
    });
    let mine = mine.filter(|value| !value.is_null());
    let mut map = match object.remove(devices) {
        Some(Value::Object(map)) => map,
        None | Some(Value::Null) => Map::new(),
        Some(_) => return Err(format!("{devices} is not a map")),
    };
    let theirs = map.remove(&change.new_owner_was);
    if map.contains_key(&change.old_owner_as) {
        return Err(format!(
            "{devices} already keeps {} as another machine",
            change.old_owner_as
        ));
    }
    if let Some(mine) = mine.filter(|value| !is_empty(value)) {
        map.insert(change.old_owner_as.clone(), mine);
    }
    if let Some(own_value) = theirs.or(emptied) {
        object.insert(own.to_owned(), own_value);
    }
    if !map.is_empty() {
        object.insert(devices.to_owned(), Value::Object(map));
    }
    Ok(())
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        Value::Null => true,
        _ => false,
    }
}

/// `labels.json` keeps only the owner's records on disk; a device's stay in
/// memory, because a record holds the conversation it was made from
/// (`labels/store.rs`), except for the `devices` marks (a session proof,
/// the hashes of analyzed turns and the label), which this move drops: they only spare a
/// request, and the new core makes the same turns' labels again. The old
/// owner's records therefore travel one hop as
/// `moved`, which the new core takes into memory for that machine and drops
/// from disk by the save its open makes at once; the new owner's, which the source core held
/// in memory and wrote to `moved` when it stopped, become its own.
fn reown_labels(value: &mut Value, change: &OwnerChange) -> Result<(), String> {
    let Some(object) = value.as_object_mut() else {
        return Err("the labels are not a map".to_owned());
    };
    object.remove("devices");
    // A section this build does not know may name a machine; it stops the
    // move rather than travel unconverted (Q18).
    if let Some(key) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "version" | "targets" | "moved"))
    {
        return Err(format!(
            "/{key} is a section of the labels this build does not move"
        ));
    }
    let mut targets = match object.remove("targets") {
        Some(Value::Object(map)) => map,
        None => Map::new(),
        Some(_) => return Err("targets is not a map".to_owned()),
    };
    let mut moved = match object.remove("moved") {
        Some(Value::Object(map)) => map,
        None => Map::new(),
        Some(_) => return Err("moved is not a map".to_owned()),
    };
    let mine = targets.remove(&change.old_owner);
    if !targets.is_empty() {
        return Err(format!(
            "targets keeps records of {}, which only the owner's may be",
            targets.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    let theirs = moved
        .remove(&crate::labels::device_target(&change.new_owner_was))
        .unwrap_or_default();
    let mut result = Map::new();
    if !is_empty(&theirs) {
        result.insert(change.new_owner.clone(), theirs);
    }
    object.insert("targets".to_owned(), Value::Object(result));
    // Only the old owner's travel on; any other machine's are read again
    // through its node, as after a restart.
    let mut next = Map::new();
    if let Some(mine) = mine.filter(|value| !is_empty(value)) {
        next.insert(crate::labels::device_target(&change.old_owner_as), mine);
    }
    if !next.is_empty() {
        object.insert("moved".to_owned(), Value::Object(next));
    }
    Ok(())
}

/// The link record's device column and its listing stamps: the new owner's
/// rows become the owner's, the old owner's keep or take its new id.
enum Links {
    Changed,
    Unchanged,
    Unopened(String),
}

/// Each project key of the two machines in the link record, with the key
/// it takes once its machine has its new id: the keys record retention is
/// kept under (`links::store::policies`). The record is only read here.
fn project_key_pairs(path: &Path, change: &OwnerChange) -> Result<Vec<(String, String)>, String> {
    let failed = |error: rusqlite::Error| error.to_string();
    let store = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(failed)?;
    let has_table: bool = store
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='projects')",
            [],
            |row| row.get(0),
        )
        .map_err(failed)?;
    if !has_table {
        return Ok(Vec::new());
    }
    let mut statement = store
        .prepare("SELECT key, device, root FROM projects WHERE device IN (?1, ?2)")
        .map_err(failed)?;
    let rows = statement
        .query_map([&change.old_owner, &change.new_owner_was], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(failed)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(failed)?;
    Ok(rows
        .into_iter()
        .filter_map(|(key, device, root)| {
            let to = change.device(&device)?;
            let new = hide_project::project_id(&to, Path::new(&root));
            (new != key).then_some((key, new))
        })
        .collect())
}

fn reown_links(path: &Path, change: &OwnerChange) -> Result<Links, String> {
    let mut store = match crate::links::store::LinkStore::open(path) {
        Ok((store, _)) => store,
        Err(error) => return Ok(Links::Unopened(error.to_string())),
    };
    let mut changed = false;
    let listed = |device: &str| format!("{}:{device}", crate::links::worker::LISTED_AT);
    let own_listed = store.meta(crate::links::worker::LISTED_AT)?;
    let their_listed = store.meta(&listed(&change.new_owner_was))?;
    if change.old_owner_as != change.old_owner && store.has_device(&change.old_owner)? {
        store.convert_device(&change.old_owner, &change.old_owner_as)?;
        changed = true;
    }
    if change.new_owner_was != change.new_owner && store.has_device(&change.new_owner_was)? {
        store.convert_device(&change.new_owner_was, &change.new_owner)?;
        changed = true;
    }
    if let Some(stamp) = own_listed {
        store.set_meta(&listed(&change.old_owner_as), &stamp)?;
        store.delete_meta(crate::links::worker::LISTED_AT)?;
        changed = true;
    }
    if let Some(stamp) = their_listed {
        store.set_meta(crate::links::worker::LISTED_AT, &stamp)?;
        store.delete_meta(&listed(&change.new_owner_was))?;
        changed = true;
    }
    Ok(if changed {
        Links::Changed
    } else {
        Links::Unchanged
    })
}

/// The Factories and their workers on either machine (Q17): a Factory and a
/// worker name their machine by node id, the owner's by none; a pane is
/// Herdr-scoped like every other.
fn reown_factory(staging: &Path, change: &OwnerChange) -> Result<bool, String> {
    let mut store = hide_factory::store::Store::open(
        &hide_kit::layout::factory_store(staging),
        &hide_kit::layout::factory_files(staging),
    )
    .map_err(|error| error.to_string())?;
    let machine = |node: Option<&str>| match node {
        None => Some(change.old_owner_as.clone()),
        Some(node) if node == change.new_owner_was => None,
        Some(node) => Some(node.to_owned()),
    };
    let pane = |pane: &str| {
        change
            .scoped("pane", pane)
            .unwrap_or_else(|| pane.to_owned())
    };
    store
        .reown(&machine, &pane)
        .map_err(|error| error.to_string())
}

/// Project Memory (PRD core-host-node-move): a Project is stored under the
/// id of its machine and the digest of that id and its root, and so are the
/// cards, receipts and cursors that follow it. A machine the move gives
/// another id takes its Projects with it, by the same re-key the layer-1
/// conversion makes (`MemoryStore::convert_device`), so a Project's Memory
/// is the new core's for the same Project and not an empty one beside it.
/// A store that does not open stops the move. A Project already stored under
/// the id a move would give is that Project as another machine named it when
/// its root is the same (a move made before Memory was re-keyed started a
/// fresh one under its own machine's id and carried it home), and is folded
/// into it (`MemoryStore::convert_device_merging`), the moving Project's
/// choices and rows winning, with one record of what was folded. A different
/// Project at that id stops the move.
fn reown_memory(path: &Path, change: &OwnerChange) -> Result<(bool, Vec<Renumbered>), String> {
    let store = crate::runtime::memory::open_memory_store(path)
        .map_err(|error| format!("Project Memory does not open: {error}"))?;
    let mut changed = false;
    let mut renumbered = Vec::new();
    for (from, to) in [
        (&change.new_owner_was, &change.new_owner),
        (&change.old_owner, &change.old_owner_as),
    ] {
        if from != to {
            let moved = store
                .convert_device_merging(from, to)
                .map_err(|error| format!("Project Memory of {from}: {error}"))?;
            for conversion in &moved {
                let Some(folded) = &conversion.merged else {
                    continue;
                };
                crate::diagnostic!(serde_json::json!({
                    "component": "core_move",
                    "kind": "memory.leftover_merged",
                    "from_id": conversion.from,
                    "into_id": conversion.to,
                    "rows": folded.rows,
                    "renumbered": folded.renumbered,
                    "batch_dropped": folded.batch_dropped,
                }));
                if folded.renumbered.iter().any(|(old, new)| old != new) {
                    renumbered.push(Renumbered {
                        intent_prefix: crate::runtime::memory_promotion::intent_prefix(
                            &conversion.to,
                        ),
                        numbers: folded.renumbered.clone(),
                    });
                }
            }
            changed |= !moved.is_empty();
        }
    }
    Ok((changed, renumbered))
}

/// What a pattern reaches: a value, or the object whose keys it names.
enum Slot<'a> {
    Value(&'a mut Value),
    Keys(&'a mut Map<String, Value>),
}

/// Calls `visit` for each slot `parts` reach (`*` is each array element or
/// object value, `{key}` the object's keys); a value slot answering `true`
/// is removed from the array or object holding it.
fn walk(
    value: &mut Value,
    parts: &[&str],
    visit: &mut dyn FnMut(Slot<'_>) -> Result<bool, String>,
) -> Result<bool, String> {
    let Some((part, rest)) = parts.split_first() else {
        return visit(Slot::Value(value));
    };
    match *part {
        "{key}" => {
            if let Value::Object(map) = value {
                if rest.is_empty() {
                    visit(Slot::Keys(map))?;
                } else {
                    for entry in map.values_mut() {
                        walk(entry, rest, visit)?;
                    }
                }
            }
            Ok(false)
        }
        "*" => {
            match value {
                Value::Array(items) => {
                    let mut kept = Vec::with_capacity(items.len());
                    for mut item in std::mem::take(items) {
                        if !walk(&mut item, rest, visit)? || !rest.is_empty() {
                            kept.push(item);
                        }
                    }
                    *items = kept;
                }
                Value::Object(map) => {
                    let keys: Vec<String> = map.keys().cloned().collect();
                    for key in keys {
                        let drop = walk(map.get_mut(&key).expect("listed key"), rest, visit)?;
                        if drop && rest.is_empty() {
                            map.remove(&key);
                        }
                    }
                }
                _ => {}
            }
            Ok(false)
        }
        key => {
            let Value::Object(map) = value else {
                return Ok(false);
            };
            let Some(entry) = map.get_mut(key) else {
                return Ok(false);
            };
            let drop = walk(entry, rest, visit)?;
            if drop && rest.is_empty() {
                // A single value naming nothing either machine has is cleared.
                map.insert(key.to_owned(), Value::Null);
            }
            Ok(false)
        }
    }
}

fn each<'a>(value: &'a mut Value, key: &str) -> impl Iterator<Item = &'a mut Value> {
    value
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
}

/// The fields whose value is a machine id.
const MACHINE_FIELDS: &[&str] = &[
    "device_id",
    "focused_device_id",
    "machine",
    "native_machine",
    "node_id",
    "target_id",
    "host_scope",
];

/// The first place, as a JSON pointer, where a key still names the new
/// owner by its old id: any value or object key carrying it as
/// `remote:<id>:`, and, when that id is an alias rather than its node id, a
/// machine field or an object key that is the alias itself (or its session
/// fold). Free text (a letter body, a label's conversation) is not a key and
/// is not read.
fn still_naming(value: &Value, change: &OwnerChange, at: &str) -> Option<String> {
    let old_id = change.new_owner_was.as_str();
    let qualified = format!("remote:{old_id}:");
    // A new owner known by its node id keeps that id as the owner's.
    let alias = old_id != change.new_owner;
    // A sleeping project's fold names the project after its own prefix.
    let carries = |text: &str| {
        text.strip_prefix("asleep/")
            .unwrap_or(text)
            .starts_with(&qualified)
    };
    let is_alias = |text: &str| {
        alias
            && (text == old_id
                || text == format!("cleanup/{old_id}")
                || text == format!("no_agents/{old_id}"))
    };
    match value {
        Value::String(text) if carries(text) => Some(at.to_owned()),
        Value::String(text)
            if is_alias(text)
                && at
                    .rsplit('/')
                    .next()
                    .is_some_and(|field| MACHINE_FIELDS.contains(&field)) =>
        {
            Some(at.to_owned())
        }
        Value::Array(items) => items.iter().enumerate().find_map(|(index, item)| {
            let place = format!("{at}/{index}");
            match item.as_str() {
                // A list of machine ids or session folds.
                Some(text) if is_alias(text) => Some(place),
                _ => still_naming(item, change, &place),
            }
        }),
        Value::Object(map) => map.iter().find_map(|(key, entry)| {
            let place = format!("{at}/{key}");
            if carries(key) || is_alias(key) {
                return Some(place);
            }
            if matches!(key.as_str(), "body" | "facts") {
                return None;
            }
            still_naming(entry, change, &place)
        }),
        _ => None,
    }
}

/// Where a staging copy lives beside the state folder it was made from.
pub fn staging_dir(state_dir: &Path, intent: &str) -> PathBuf {
    state_dir.join(super::MOVE_STAGING).join(intent)
}

#[cfg(test)]
mod tests {
    use super::super::tests::reach;
    use super::*;
    use serde_json::json;

    /// The MacBook, whose core wrote the folder, and the Mac mini, which the
    /// folder knows as the dialed device `mini`, and a third device that
    /// neither move touches.
    const M: &str = "8f1c2d3e-4a5b-4c6d-8e7f-90a1b2c3d4e5";
    const C: &str = "2b7e9f10-1111-4a2b-9c3d-5e6f7a8b9c0d";
    const ALIAS: &str = "mini";
    const X: &str = "studio";
    const M_ROOT: &str = "/Users/alice/alpha";
    const M_WORKTREE: &str = "/Users/alice/alpha-review";
    const C_ROOT: &str = "/Users/remote/beta";
    const X_ROOT: &str = "/srv/gamma";
    const M_SOCKET: &str = "/Users/alice/.config/herdr/herdr.sock";
    const C_SOCKET: &str = "/Users/remote/.config/herdr/herdr.sock";

    fn mini_registration() -> Value {
        json!({"id": ALIAS, "label": "Mac mini", "ssh_alias": "mini", "herdr_socket_path": C_SOCKET})
    }

    fn forward() -> OwnerChange {
        OwnerChange {
            old_owner: M.into(),
            old_owner_as: M.into(),
            new_owner: C.into(),
            new_owner_was: ALIAS.into(),
            old_owner_registration: serde_json::from_value(
                json!({"id": M, "label": "MacBook", "inbound": true}),
            )
            .unwrap(),
            old_owner_herdr_socket: M_SOCKET.into(),
            new_owner_herdr_socket: C_SOCKET.into(),
        }
    }

    fn back() -> OwnerChange {
        OwnerChange {
            old_owner: C.into(),
            old_owner_as: ALIAS.into(),
            new_owner: M.into(),
            new_owner_was: M.into(),
            old_owner_registration: serde_json::from_value(mini_registration()).unwrap(),
            old_owner_herdr_socket: C_SOCKET.into(),
            new_owner_herdr_socket: M_SOCKET.into(),
        }
    }

    fn own_workspace() -> String {
        crate::workspace::workspace_id_for_path(Path::new(M_ROOT))
    }
    fn own_checkout(path: &str) -> String {
        crate::workspace::checkout_id_for_path(&own_workspace(), Path::new(path))
    }
    fn device_project(device: &str, root: &str) -> String {
        crate::device_catalog::project_id(device, Path::new(root))
    }
    fn device_checkout(device: &str, path: &str) -> String {
        crate::device_catalog::checkout_id(device, path)
    }

    /// The id Project Memory and the settings keep a project under.
    fn device_digest(device: &str, root: &str) -> String {
        crate::project_settings::key(device, root)
    }

    /// Each Project of the Memory store with its machine, and the project
    /// id its receipt cursor names, in id order.
    fn memory_projects(state: &Path) -> Vec<(String, String, String)> {
        let connection = rusqlite::Connection::open_with_flags(
            state.join(PROJECT_MEMORY),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut statement = connection
            .prepare(
                "SELECT p.id, p.device_id, c.project_id FROM projects p \
                 JOIN receipt_cursors c ON c.project_id=p.id ORDER BY p.id",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// A Project's choices as the core saves them, with the protections a
    /// device's project can carry.
    fn project_settings(device: &str, root: &str) -> Value {
        json!({
            "version": 2, "device_id": device, "root": root,
            "repository": {"mode": "repository", "repository": "acme/p"},
            "issue_source": "github", "new_work": "worktree",
            "base_branch": {"mode": "branch", "branch": "trunk"},
            "guardrails": {"git_commands": true},
        })
    }

    /// The saved choices of the projects `(device, root)`, each at the key
    /// its device and root make.
    fn settings_of(projects: &[(&str, &str)]) -> Value {
        Value::Object(
            projects
                .iter()
                .map(|(device, root)| {
                    (
                        crate::project_settings::key(device, root),
                        project_settings(device, root),
                    )
                })
                .collect(),
        )
    }

    /// The table the MacBook's core makes before it stops.
    fn forward_ids() -> IdTable {
        let own = own_workspace();
        let mini = device_project(ALIAS, C_ROOT);
        let main = own_checkout(M_ROOT);
        let review = own_checkout(M_WORKTREE);
        let theirs = device_checkout(ALIAS, C_ROOT);
        id_table(
            &forward(),
            [
                KnownProject {
                    id: &own,
                    device_id: M,
                    path: M_ROOT,
                    root: Path::new(M_ROOT),
                    checkouts: vec![(&main, M_ROOT), (&review, M_WORKTREE)],
                },
                KnownProject {
                    id: &mini,
                    device_id: ALIAS,
                    path: C_ROOT,
                    root: Path::new(C_ROOT),
                    checkouts: vec![(&theirs, C_ROOT)],
                },
            ],
        )
    }

    /// The table the mini's core makes before it stops for a move back.
    fn back_ids() -> IdTable {
        let own = crate::workspace::workspace_id_for_path(Path::new(C_ROOT));
        let main = crate::workspace::checkout_id_for_path(&own, Path::new(C_ROOT));
        let mac = device_project(M, M_ROOT);
        let mac_main = device_checkout(M, M_ROOT);
        let mac_review = device_checkout(M, M_WORKTREE);
        id_table(
            &back(),
            [
                KnownProject {
                    id: &own,
                    device_id: C,
                    path: C_ROOT,
                    root: Path::new(C_ROOT),
                    checkouts: vec![(&main, C_ROOT)],
                },
                KnownProject {
                    id: &mac,
                    device_id: M,
                    path: M_ROOT,
                    root: Path::new(M_ROOT),
                    checkouts: vec![(&mac_main, M_ROOT), (&mac_review, M_WORKTREE)],
                },
            ],
        )
    }

    /// One value of each shape for a kind of id: the owner's, the dialed
    /// mini's and the third device's, and what each is after the move.
    fn shapes(kind: &str) -> Vec<(String, String)> {
        let pane = |raw: &str| raw.to_owned();
        match kind {
            "pane" | "tab" => vec![
                (
                    pane("w1:x1").replace('x', &kind[..1]),
                    format!("remote:{M}:{kind}:w1:{}1", &kind[..1]),
                ),
                (
                    format!("remote:{ALIAS}:{kind}:w2:{}1", &kind[..1]),
                    format!("w2:{}1", &kind[..1]),
                ),
                (
                    format!("remote:{X}:{kind}:w3:{}1", &kind[..1]),
                    format!("remote:{X}:{kind}:w3:{}1", &kind[..1]),
                ),
            ],
            "device" => vec![
                (M.into(), M.into()),
                (ALIAS.into(), C.into()),
                (X.into(), X.into()),
            ],
            "registration" => vec![
                (own_workspace(), device_project(M, M_ROOT)),
                (
                    device_project(ALIAS, C_ROOT),
                    crate::workspace::workspace_id_for_path(Path::new(C_ROOT)),
                ),
                (device_project(X, X_ROOT), device_project(X, X_ROOT)),
            ],
            "checkout" => vec![
                (own_checkout(M_WORKTREE), device_checkout(M, M_WORKTREE)),
                (
                    device_checkout(ALIAS, C_ROOT),
                    crate::workspace::checkout_id_for_path(
                        &crate::workspace::workspace_id_for_path(Path::new(C_ROOT)),
                        Path::new(C_ROOT),
                    ),
                ),
                (device_checkout(X, X_ROOT), device_checkout(X, X_ROOT)),
            ],
            "fold" => vec![
                (own_workspace(), device_project(M, M_ROOT)),
                (format!("cleanup/{ALIAS}"), format!("cleanup/{C}")),
                (format!("cleanup/{X}"), format!("cleanup/{X}")),
                (format!("no_agents/{ALIAS}"), format!("no_agents/{C}")),
                (format!("no_agents/{X}"), format!("no_agents/{X}")),
                (
                    format!("asleep/{}", own_workspace()),
                    format!("asleep/{}", device_project(M, M_ROOT)),
                ),
            ],
            other => panic!("no shapes for {other}"),
        }
    }

    fn before(kind: &str) -> Vec<String> {
        shapes(kind).into_iter().map(|(before, _)| before).collect()
    }

    fn keyed(kind: &str, value: Value) -> Value {
        Value::Object(
            before(kind)
                .into_iter()
                .map(|key| (key, value.clone()))
                .collect(),
        )
    }

    /// A folder as the MacBook's core leaves it at a move's stop: a value of
    /// every shape at every key the table names, and the labels the core
    /// held in memory for the mini written to `moved`.
    fn macbook_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        let write = |name: &str, value: Value| {
            std::fs::write(state.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
        };
        let panes = before("pane");
        let devices = before("device");
        let checkouts = before("checkout");
        let registrations = before("registration");
        let actor = |index: usize| json!({"pane_id": panes[index], "name": "a", "kind": "claude", "device_id": devices[index]});
        let archive = json!({"archive-1": {"project_root": M_ROOT, "members": (0..3).map(|index| json!({
            "machine": devices[index], "pane_id": panes[index], "checkout_path": M_ROOT, "cwd": M_ROOT,
            "resume": {"state": "done", "pane_id": panes[index]},
            "registration": {"id": format!("agent-{index}"), "actor": actor(index)},
        })).collect::<Vec<_>>()}});
        write(
            CORE_STATE,
            json!({
                "schema_version": 1,
                "workspace_registrations": [
                    {"id": registrations[0], "label": "alpha", "path": M_ROOT, "device_id": M, "primary_checkout_id": checkouts[0]},
                    {"id": registrations[1], "label": "beta", "path": C_ROOT, "device_id": ALIAS, "primary_checkout_id": checkouts[1]},
                    {"id": registrations[2], "label": "gamma", "path": X_ROOT, "device_id": X, "primary_checkout_id": checkouts[2]},
                ],
                "device_registrations": [mini_registration(), {"id": X, "label": "Studio", "ssh_alias": "studio"}],
                "focused_device_id": ALIAS,
                "expanded_inactive_project_device_ids": devices,
                "recent_checkouts": (0..3).map(|index| json!({
                    "device_id": devices[index], "checkout_id": checkouts[index],
                    "project_name": "p", "branch": "main", "device_name": "d",
                })).collect::<Vec<_>>(),
                "expanded_paths": [M_ROOT],
                "device_expanded_paths": {ALIAS: [C_ROOT], X: [X_ROOT]},
                "selected_path": M_ROOT,
                "operator_tab_names": keyed("tab", json!("review")),
                "project_settings": settings_of(&[(M, M_ROOT), (ALIAS, C_ROOT), (X, X_ROOT)]),
                "project_base_branches": {M_ROOT: "main", C_ROOT: "trunk", X_ROOT: "dev"},
                "project_issue_sources": {M_ROOT: "github", C_ROOT: "local", X_ROOT: "github"},
                "expanded_inactive_checkout_project_paths": [M_ROOT, C_ROOT, X_ROOT],
                "selected_pane_id": panes[1],
                "expanded_agent_pane_ids": panes,
                "sessions_expanded_agent_pane_ids": panes,
                "detail": {"items": (0..3).flat_map(|index| [
                    json!({"kind": "pull_request", "workspace_id": registrations[index], "number": 7}),
                    json!({"kind": "record", "session": {"device_id": devices[index], "agent": "claude", "session_id": "s"}}),
                ]).collect::<Vec<_>>(), "shown": true},
                "recent_pane_ids": panes,
                "pane_text_scales": keyed("pane", json!(1.2)),
                "pane_read_records": Value::Object(panes.iter().enumerate().map(|(index, pane)| (
                    pane.clone(),
                    json!({"demand": "none", "activity": "idle", "descendant_signals": [{"pane_id": panes[index]}]}),
                )).collect()),
                "pane_completions": keyed("pane", json!({"session": "s", "server": "m", "state_change_seq": 4})),
                "pane_launches": keyed("pane", json!({"kind": "claude", "choice": {"effort": "high"}, "session": "s"})),
                "request_verbs": keyed("pane", json!({"verb": "review"})),
                "pane_terminal_sizes": keyed("pane", json!([80, 24])),
                "resolved_sessions": keyed("pane", json!({"at_unix_ms": 1})),
                "session_resolution_inputs": keyed("pane", json!(1)),
                "agent_sleep": {
                    "stamps": keyed("pane", json!({})),
                    "records": Value::Object(panes.iter().enumerate().map(|(index, pane)| (
                        pane.clone(),
                        json!({"parent_pane_id": panes[index], "cwd": M_ROOT}),
                    )).collect()),
                    "dormant": {"sleep-1": {"node_id": M, "old_pane_id": panes[0], "cwd": M_ROOT}},
                },
                "session_archive": archive,
                "factory_secretary_pane": panes[0],
                "focused_checkout_id": checkouts[1],
                "collapsed_workspace_ids": registrations,
                "collapsed_checkout_ids": checkouts,
                "expanded_checkout_ids": checkouts,
                "session_collapsed_checkout_ids": checkouts,
                "session_open_folds": before("fold"),
            }),
        );
        let tabs = before("tab");
        let feedback_comments: Vec<Value> = [M, ALIAS, X]
            .iter()
            .map(|device| json!({"id": "c2", "recipient": {"device_id": device}, "shot": {"node": device, "name": "shot.png"}}))
            .collect();
        let feedback_waiting: Vec<Value> = [M, ALIAS, X]
            .iter()
            .map(|device| json!({"to": {"device_id": device}}))
            .collect();
        let layout = |tab: &str| {
            json!({
                "root": {"split": {"id": "s1", "axis": "horizontal", "ratio": 0.5,
                    "first": {"area": {"id": "a1", "active": tab, "displays": [{"id": tab}]}},
                    "second": {"area": {"id": "a2", "active": null, "displays": []}}}},
                "active_area": "a1", "next_id": 3, "canvases": {"a2": tab},
            })
        };
        write(
            WORKSPACE_VIEWS,
            json!({"schema_version": 3, "workspaces": [
                {"device_id": M, "path": M_ROOT, "agent_layout": layout(&tabs[0]),
                 "tab_views": {tabs[0].clone(): "d1"},
                 "view_layout": {"root": {"split": {"id": "s1", "first": {"area": {"id": "a1", "displays": [{"id": "d1", "opener": {"device_id": M, "terminal_id": "t1"}}]}},
                     "second": {"area": {"id": "a2", "displays": [{"id": "d2", "opener": {"device_id": ALIAS, "terminal_id": "t2"}}, {"id": "d3", "opener": {"device_id": X, "terminal_id": "t3"}}]}}}}},
                 "feedback": {"comments": feedback_comments, "waiting": feedback_waiting}},
                {"device_id": ALIAS, "path": C_ROOT, "agent_layout": layout(&tabs[1]),
                 "tab_views": {tabs[1].clone(): "d2"}, "view_layout": {}},
                {"device_id": X, "path": X_ROOT, "agent_layout": layout(&tabs[2]),
                 "tab_views": {tabs[2].clone(): "d3"}, "view_layout": {}},
            ]}),
        );
        write(
            LABELS,
            json!({
                "version": 1,
                "targets": {M: {"w1:p1": {"owner": "mac"}}},
                "moved": {"device:mini": {"w2:p1": {"owner": "mini"}}},
            }),
        );
        write(
            DELIVERY_LEDGER,
            json!({
                "version": 1, "next_id": 9,
                "letters": (0..3).map(|index| json!({
                    "sender": actor(index), "recipient": actor((index + 1) % 3),
                    "watch_warning": {"target": actor(index)}, "body": "from mini to studio",
                })).collect::<Vec<_>>(),
                "watches": (0..3).map(|index| json!({"parent": actor(index), "target": actor((index + 2) % 3)})).collect::<Vec<_>>(),
                "agents": [
                    {"id": "agent-1", "machine": M, "native_machine": M, "actor": actor(0), "pane": "w1:p1", "host_scope": M_SOCKET, "project": M_ROOT, "session": "s1", "instance": "t1"},
                    {"id": "agent-2", "machine": ALIAS, "native_machine": C, "actor": actor(1), "pane": "w2:p1", "host_scope": ALIAS, "project": C_ROOT, "session": "s2", "instance": "t2"},
                    {"id": "agent-3", "machine": X, "native_machine": "studio-machine", "actor": actor(2), "pane": "w3:p1", "host_scope": X, "project": X_ROOT, "session": "s3", "instance": "t3"},
                ],
                "spawns": [
                    {"parent": "agent-1", "repo": M_ROOT, "path": M_ROOT, "requested_path": M_ROOT, "pane": "w1:p2", "task": {"kind": "memory_promote", "target": "m1"}},
                    {"parent": "agent-1", "repo": C_ROOT, "path": C_ROOT, "requested_path": C_ROOT, "pane": "w2:p2", "machine": ALIAS},
                    {"parent": "agent-1", "repo": X_ROOT, "path": X_ROOT, "requested_path": X_ROOT, "pane": "w3:p2", "machine": X},
                ],
                "identity_waits": ([M, ALIAS, X].iter().enumerate().map(|(index, machine)| json!({
                    "id": format!("wait-{index}"), "machine": machine, "pane": format!("w{}:p3", index + 1),
                    "name": format!("fork-{index}"), "kind": "codex", "owed": {"for": "fork", "origin": "agent-1"},
                }))).collect::<Vec<_>>(),
                "node_clocks": {
                    ALIAS: {"connected_ms": 600_000, "since": null},
                    X: {"connected_ms": 60_000, "since": null},
                },
            }),
        );
        write(
            LOCAL_ISSUES,
            json!({"version": 1, "projects": {M_ROOT: [{"number": 1}]}, "devices": {ALIAS: {C_ROOT: [{"number": 2}]}}}),
        );
        write(MARKER_FILE, json!({"version": 1, "node": M}));
        let memory = hide_memory::MemoryStore::open(&state.join(PROJECT_MEMORY)).unwrap();
        for (device, root) in [(M, M_ROOT), (ALIAS, C_ROOT), (X, X_ROOT)] {
            memory
                .ensure_project(&device_digest(device, root), Path::new(root), device)
                .unwrap();
        }
        drop(memory);
        let connection = rusqlite::Connection::open(state.join(PROJECT_MEMORY)).unwrap();
        for (device, root) in [(M, M_ROOT), (ALIAS, C_ROOT), (X, X_ROOT)] {
            connection
                .execute(
                    "INSERT INTO receipt_cursors VALUES(?1,'claude','s1',0,x'',NULL,1,0)",
                    [device_digest(device, root)],
                )
                .unwrap();
        }
        drop(connection);
        let (links, _) = crate::links::store::LinkStore::open(&state.join(LINKS)).unwrap();
        links
            .set_meta(crate::links::worker::LISTED_AT, "100")
            .unwrap();
        links
            .set_meta(&format!("listed_at:{ALIAS}"), "200")
            .unwrap();
        drop(links);
        let connection = rusqlite::Connection::open(state.join(LINKS)).unwrap();
        for device in [M, ALIAS, X] {
            connection
                .execute(
                    "INSERT INTO sessions(device,agent,id) VALUES(?1,'claude','s')",
                    [device],
                )
                .unwrap();
        }
        dir
    }

    fn read(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn rows(state: &Path, file: &str) -> Vec<(&'static str, Moves)> {
        let _ = state;
        KEYS.iter()
            .filter(|(listed, ..)| *listed == file)
            .map(|(_, pattern, _, moves)| (*pattern, *moves))
            .collect()
    }

    fn kind(moves: Moves) -> Option<&'static str> {
        match moves {
            Moves::Device => Some("device"),
            Moves::Pane => Some("pane"),
            Moves::Tab => Some("tab"),
            Moves::Registration => Some("registration"),
            Moves::Checkout => Some("checkout"),
            Moves::Fold => Some("fold"),
            _ => None,
        }
    }

    /// A store compared as the core reads it: a null or empty value is the
    /// same as none.
    fn normalized(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .filter(|(_, entry)| !is_empty(entry))
                    .map(|(key, entry)| (key.clone(), normalized(entry)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(normalized).collect()),
            other => other.clone(),
        }
    }

    #[test]
    fn every_key_of_every_shape_is_spelled_as_the_new_core_writes_it() {
        let dir = macbook_folder();
        let state = dir.path();
        let original: BTreeMap<&str, Value> = [CORE_STATE, WORKSPACE_VIEWS, DELIVERY_LEDGER]
            .into_iter()
            .map(|file| (file, read(&state.join(file))))
            .collect();
        reown(state, &forward(), &forward_ids()).unwrap();

        for (file, value) in &original {
            let after = read(&state.join(file));
            for (pattern, moves) in rows(state, file) {
                let Some(kind) = kind(moves) else { continue };
                let reached = reach(value, pattern);
                assert!(
                    !reached.is_empty(),
                    "the fixture has no value at {file}{pattern}"
                );
                let mapping: BTreeMap<String, String> = shapes(kind).into_iter().collect();
                let mut expected: Vec<Value> = reached
                    .iter()
                    .map(|value| match value.as_str() {
                        Some(text) => json!(
                            mapping
                                .get(text)
                                .cloned()
                                .unwrap_or_else(|| text.to_owned())
                        ),
                        None => value.clone(),
                    })
                    .collect();
                let mut found = reach(&after, pattern);
                let key = |value: &Value| value.to_string();
                expected.sort_by_key(key);
                found.sort_by_key(key);
                assert_eq!(found, expected, "{file}{pattern}");
            }
        }

        let core = read(&state.join(CORE_STATE));
        assert_eq!(
            core["focus_from_herdr"],
            json!(true),
            "the new core opens on its own Herdr's focus"
        );
        assert_eq!(core["expanded_paths"], json!([C_ROOT]));
        assert_eq!(
            core["device_expanded_paths"],
            json!({M: [M_ROOT], X: [X_ROOT]})
        );
        // A Project's choices follow their machine to the id it has on the
        // new core, record and key together, so the new core reads each one
        // where it looks: the mini's become the core's own, the MacBook's
        // keep the node id it links in by, a third machine's stay.
        assert_eq!(
            core["project_settings"],
            settings_of(&[(M, M_ROOT), (C, C_ROOT), (X, X_ROOT)])
        );
        // Project Memory follows its machine the same way: the mini's
        // Projects, cards and cursors become the core's own, the MacBook's
        // keep the node id it links in by.
        let mut expected: Vec<(String, String, String)> = [(M, M_ROOT), (C, C_ROOT), (X, X_ROOT)]
            .iter()
            .map(|(device, root)| {
                let id = device_digest(device, root);
                (id.clone(), (*device).to_owned(), id)
            })
            .collect();
        expected.sort();
        assert_eq!(memory_projects(state), expected);
        // Settings keyed by a project path are read the same on either
        // core, whichever machine holds the project: they stay.
        assert_eq!(core["selected_path"], json!(M_ROOT));
        assert_eq!(
            core["project_base_branches"],
            json!({M_ROOT: "main", C_ROOT: "trunk", X_ROOT: "dev"})
        );
        assert_eq!(
            core["project_issue_sources"],
            json!({M_ROOT: "github", C_ROOT: "local", X_ROOT: "github"})
        );
        // As the core reads them: the stored form spells absent fields as
        // null where the runtime writes them.
        let registrations = |value: Value| -> Vec<crate::model::DeviceRegistration> {
            serde_json::from_value(value).unwrap()
        };
        assert_eq!(
            registrations(core["device_registrations"].clone()),
            registrations(
                json!([{"id": M, "label": "MacBook", "inbound": true}, {"id": X, "label": "Studio", "ssh_alias": "studio"}])
            )
        );
        let views = read(&state.join(WORKSPACE_VIEWS));
        // A View's opener on the mini names it by the new owner's node id,
        // wherever in a layout it sits (PRD view-opener-feedback B41).
        let mac_views = &views["workspaces"][0];
        let split = &mac_views["view_layout"]["root"]["split"];
        assert_eq!(
            split["first"]["area"]["displays"][0]["opener"]["device_id"],
            json!(M)
        );
        assert_eq!(
            split["second"]["area"]["displays"][0]["opener"]["device_id"],
            json!(C)
        );
        assert_eq!(
            split["second"]["area"]["displays"][1]["opener"]["device_id"],
            json!(X)
        );
        let mac = &views["workspaces"][0]["agent_layout"];
        let tab = format!("remote:{M}:tab:w1:t1");
        assert_eq!(mac["root"]["split"]["first"]["area"]["active"], json!(tab));
        assert_eq!(
            mac["root"]["split"]["first"]["area"]["displays"][0]["id"],
            json!(tab)
        );
        assert_eq!(mac["canvases"]["a2"], json!(tab));
        assert_eq!(
            views["workspaces"][1]["agent_layout"]["canvases"]["a2"],
            json!("w2:t1")
        );
        let ledger = read(&state.join(DELIVERY_LEDGER));
        assert_eq!(ledger["agents"][0]["host_scope"], json!(M));
        assert_eq!(ledger["agents"][1]["host_scope"], json!(C_SOCKET));
        assert_eq!(ledger["agents"][2]["host_scope"], json!(X));
        // A spawn's machine is read against its caller's, which is the
        // MacBook's agent on both cores: none stays none, the mini is now
        // the core's own machine and so named by its node id, and a third
        // device stays as it is.
        assert!(ledger["spawns"][0].get("machine").is_none());
        assert_eq!(ledger["spawns"][1]["machine"], json!(C));
        assert_eq!(ledger["spawns"][2]["machine"], json!(X));
        // A wait still names the machine whose writer judges it, and its pane
        // stays that machine's own.
        let waits = &ledger["identity_waits"];
        assert_eq!(waits[0]["machine"], json!(M));
        assert_eq!(waits[1]["machine"], json!(C));
        assert_eq!(waits[2]["machine"], json!(X));
        assert_eq!(waits[1]["pane"], json!("w2:p3"));
        assert_eq!(ledger["letters"][0]["body"], json!("from mini to studio"));
        let labels = read(&state.join(LABELS));
        assert_eq!(labels["targets"], json!({C: {"w2:p1": {"owner": "mini"}}}));
        assert_eq!(
            labels["moved"],
            json!({format!("device:{M}"): {"w1:p1": {"owner": "mac"}}})
        );
        let issues = read(&state.join(LOCAL_ISSUES));
        assert_eq!(issues["projects"], json!({C_ROOT: [{"number": 2}]}));
        assert_eq!(issues["devices"], json!({M: {M_ROOT: [{"number": 1}]}}));
        let links = crate::links::store::LinkStore::open(&state.join(LINKS))
            .unwrap()
            .0;
        assert_eq!(links.meta("listed_at").unwrap().as_deref(), Some("200"));
        assert_eq!(
            links.meta(&format!("listed_at:{M}")).unwrap().as_deref(),
            Some("100")
        );
        assert!(!links.has_device(ALIAS).unwrap());
        assert!(
            links.has_device(C).unwrap()
                && links.has_device(M).unwrap()
                && links.has_device(X).unwrap()
        );
        assert_eq!(read(&state.join(MARKER_FILE))["node"], json!(C));
    }

    /// A Factory row on `node` for `project`, with one Task whose worker
    /// runs in `pane` on that machine and which `producer` added.
    fn put_factory(
        store: &mut hide_factory::store::Store,
        project: &str,
        node: Option<&str>,
        pane: &str,
        producer: &str,
    ) {
        use hide_factory::model::{Card, Factory, SourceKind, Task, WorkerRef};
        store
            .put_factory(&Factory {
                id: project.into(),
                project: project.into(),
                node: node.map(str::to_owned),
                project_name: "p".into(),
                source: SourceKind::Local,
                repo: None,
                default_branch: "main".into(),
                config: Default::default(),
                closed: false,
                created_at: 1,
                next_task: 2,
                next_local_issue: 1,
                main: Default::default(),
                outside_read_at: None,
                outside_read_failures: 0,
                watch_day: 0,
                watch_sent_today: 0,
                watch_last_at: None,
                github_approval: None,
                paused: false,
                observer_day: 0,
                observer_calls: 0,
                observer_cap_notice_day: 0,
                activity: Vec::new(),
                github_block: None,
                holds: Vec::new(),
                commands: Vec::new(),
                next_command: 0,
            })
            .unwrap();
        let mut task = Task::draft(project, "t1", 1, Card::default(), 1);
        task.producer_pane = Some(producer.into());
        task.worker = Some(WorkerRef {
            factory: project.into(),
            node: node.map(str::to_owned),
            agent: Some("agent-1".into()),
            name: "w".into(),
            pane: Some(pane.into()),
            runtime: hide_factory::model::Runtime::CLAUDE,
            worktree: format!("{project}.worktrees/t1"),
            branch: "factory/t1".into(),
            started_at: 1,
            asleep: false,
            model: None,
            effort: None,
        });
        store.put_task(&task).unwrap();
    }

    /// Each Factory's machine and pane after the store was reowned, by project.
    fn factory_machines(state: &Path) -> Vec<(String, Option<String>, Option<String>, String)> {
        let store = hide_factory::store::Store::open(
            &hide_kit::layout::factory_store(state),
            &hide_kit::layout::factory_files(state),
        )
        .unwrap();
        let loaded = store.load().unwrap();
        loaded
            .factories
            .iter()
            .map(|factory| {
                let task = loaded
                    .tasks
                    .iter()
                    .find(|task| task.factory == factory.id)
                    .unwrap();
                let worker = task.worker.as_ref().unwrap();
                assert_eq!(worker.node, factory.node, "{}", factory.id);
                (
                    factory.id.clone(),
                    factory.node.clone(),
                    worker.pane.clone(),
                    task.producer_pane.clone().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn a_factory_and_its_worker_keep_the_machine_their_project_is_on_through_a_move_and_back() {
        let dir = macbook_folder();
        let state = dir.path();
        {
            let mut store = hide_factory::store::Store::open(
                &hide_kit::layout::factory_store(state),
                &hide_kit::layout::factory_files(state),
            )
            .unwrap();
            // The MacBook's own project, and one on a node that links in to
            // the MacBook's core, whose panes the folder qualifies.
            put_factory(&mut store, M_ROOT, None, "w1:p1", "w1:p2");
            put_factory(
                &mut store,
                X_ROOT,
                Some(X),
                &format!("remote:{X}:pane:w3:p1"),
                &format!("remote:{X}:pane:w3:p2"),
            );
        }
        let before = factory_machines(state);

        let outcome = reown(state, &forward(), &forward_ids()).unwrap();
        assert!(outcome.files.iter().any(|file| file == FACTORY));
        // On the mini's core the MacBook is a node like any other: its
        // Factory runs there, its worker's pane is under its node id.
        assert_eq!(
            factory_machines(state),
            vec![
                (
                    M_ROOT.to_owned(),
                    Some(M.to_owned()),
                    Some(format!("remote:{M}:pane:w1:p1")),
                    format!("remote:{M}:pane:w1:p2"),
                ),
                before[1].clone(),
            ]
        );

        reown(state, &back(), &back_ids()).unwrap();
        assert_eq!(factory_machines(state), before);
    }

    #[test]
    fn a_move_back_returns_every_store_to_what_it_was() {
        let dir = macbook_folder();
        let state = dir.path();
        let files = [
            CORE_STATE,
            WORKSPACE_VIEWS,
            LABELS,
            DELIVERY_LEDGER,
            LOCAL_ISSUES,
            MARKER_FILE,
        ];
        let original: Vec<Value> = files.iter().map(|file| read(&state.join(file))).collect();
        let memory = memory_projects(state);
        reown(state, &forward(), &forward_ids()).unwrap();
        // The mini's core takes the MacBook's records into memory and, when it
        // stops for the move back, writes them to `moved` again.
        let mut labels = read(&state.join(LABELS));
        let mac = labels["moved"]
            .as_object_mut()
            .unwrap()
            .remove(&format!("device:{M}"))
            .unwrap();
        labels["moved"] = json!({format!("device:{M}"): mac});
        std::fs::write(state.join(LABELS), serde_json::to_vec(&labels).unwrap()).unwrap();
        // Its first session takes the focus hint the rekey left, and its
        // next save drops the key; the move back leaves it again.
        let take_hint = || {
            let mut core = read(&state.join(CORE_STATE));
            let hint = core.as_object_mut().unwrap().remove("focus_from_herdr");
            std::fs::write(state.join(CORE_STATE), serde_json::to_vec(&core).unwrap()).unwrap();
            hint
        };
        assert_eq!(take_hint(), Some(json!(true)));

        reown(state, &back(), &back_ids()).unwrap();
        assert_eq!(take_hint(), Some(json!(true)));
        for (file, original) in files.iter().zip(&original) {
            assert_eq!(
                normalized(&read(&state.join(file))),
                normalized(original),
                "{file}"
            );
        }
        assert_eq!(memory_projects(state), memory, "Project Memory");
        let links = crate::links::store::LinkStore::open(&state.join(LINKS))
            .unwrap()
            .0;
        assert_eq!(links.meta("listed_at").unwrap().as_deref(), Some("100"));
        assert_eq!(
            links
                .meta(&format!("listed_at:{ALIAS}"))
                .unwrap()
                .as_deref(),
            Some("200")
        );
        assert!(!links.has_device(C).unwrap());
        assert!(links.has_device(ALIAS).unwrap() && links.has_device(M).unwrap());
    }

    /// The old session search file's record retention moves with the
    /// core (PRD overview-device-tasks D-20) without the file being
    /// written: each project's days follow its link-record key to the key
    /// its machine's new id gives it, and a project of a machine the move
    /// does not touch keeps its row.
    #[test]
    fn record_retention_follows_each_project_to_its_new_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(MARKER_FILE),
            serde_json::to_vec(&json!({"version": 1, "node": M})).unwrap(),
        )
        .unwrap();
        drop(crate::links::store::LinkStore::open(&dir.path().join(LINKS)).unwrap());
        let links = rusqlite::Connection::open(dir.path().join(LINKS)).unwrap();
        let search = rusqlite::Connection::open(dir.path().join(SESSION_SEARCH)).unwrap();
        search
            .execute_batch("CREATE TABLE policy(project TEXT PRIMARY KEY, days INTEGER NOT NULL);")
            .unwrap();
        for (device, root, days) in [(M, M_ROOT, 30), (ALIAS, C_ROOT, 7), (X, X_ROOT, 0)] {
            let key = hide_project::project_id(device, Path::new(root));
            links
                .execute(
                    "INSERT INTO projects(key, device, root, seen_at) VALUES(?1, ?2, ?3, 0)",
                    [&key, device, root],
                )
                .unwrap();
            search
                .execute(
                    "INSERT INTO policy VALUES(?1, ?2)",
                    rusqlite::params![key, days],
                )
                .unwrap();
        }
        drop((links, search));
        let before = std::fs::read(dir.path().join(SESSION_SEARCH)).unwrap();

        let outcome = reown(dir.path(), &forward(), &IdTable::default()).unwrap();
        assert!(!outcome.files.contains(&SESSION_SEARCH.to_owned()));
        assert_eq!(
            std::fs::read(dir.path().join(SESSION_SEARCH)).unwrap(),
            before,
            "the session search file is never written (D-20)"
        );
        let (store, _) = crate::links::store::LinkStore::open(&dir.path().join(LINKS)).unwrap();
        let policies = store.policies(&dir.path().join(SESSION_SEARCH)).unwrap();
        for (device, root, days) in [(M, M_ROOT, 30), (C, C_ROOT, 7), (X, X_ROOT, 0)] {
            assert_eq!(
                policies.get(&hide_project::project_id(device, Path::new(root))),
                Some(&days),
                "{device}"
            );
        }
    }

    #[test]
    fn the_core_state_a_move_writes_is_one_the_core_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CORE_STATE);
        crate::persistence::save(
            &path,
            &crate::model::UiStateSnapshot::default(),
            &Default::default(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join(MARKER_FILE),
            serde_json::to_vec(&json!({"version": 1, "node": M})).unwrap(),
        )
        .unwrap();
        reown(dir.path(), &forward(), &IdTable::default()).unwrap();
        let (_, _, disposition) = crate::persistence::load(&path);
        assert_eq!(disposition, crate::persistence::LoadDisposition::Loaded);
    }

    #[test]
    fn a_key_the_table_does_not_cover_stops_the_move_and_names_its_place() {
        let dir = macbook_folder();
        let state = dir.path();
        let mut core = read(&state.join(CORE_STATE));
        core["new_feature_panes"] = json!({"focus": format!("remote:{ALIAS}:pane:w2:p9")});
        std::fs::write(state.join(CORE_STATE), serde_json::to_vec(&core).unwrap()).unwrap();
        let refusal = reown(state, &forward(), &forward_ids()).unwrap_err();
        assert_eq!(refusal.file, state.join(CORE_STATE));
        assert!(
            refusal.reason.contains("/new_feature_panes/focus"),
            "{refusal}"
        );
        assert_eq!(
            read(&state.join(MARKER_FILE))["node"],
            json!(M),
            "the owner is unchanged"
        );
    }

    #[test]
    fn a_project_setting_that_cannot_be_given_its_new_key_stops_the_move() {
        let dir = macbook_folder();
        let state = dir.path();
        let mut core = read(&state.join(CORE_STATE));
        core["project_settings"][crate::project_settings::key(ALIAS, C_ROOT)]
            .as_object_mut()
            .unwrap()
            .remove("root");
        std::fs::write(state.join(CORE_STATE), serde_json::to_vec(&core).unwrap()).unwrap();
        let refusal = reown(state, &forward(), &forward_ids()).unwrap_err();
        assert_eq!(refusal.file, state.join(CORE_STATE));
        assert!(refusal.reason.contains("names no root"), "{refusal}");
    }

    /// Plants the leftover of a move made before Memory was re-keyed: the
    /// Project of `root` under `device`, with a receipt cursor the moving
    /// Project has no row for.
    fn plant_memory_leftover(state: &Path, device: &str, root: &str) {
        let store = hide_memory::MemoryStore::open(&state.join(PROJECT_MEMORY)).unwrap();
        store
            .ensure_project(&device_digest(device, root), Path::new(root), device)
            .unwrap();
        drop(store);
        rusqlite::Connection::open(state.join(PROJECT_MEMORY))
            .unwrap()
            .execute(
                "INSERT INTO receipt_cursors VALUES(?1,'claude','left-over',0,x'',NULL,1,0)",
                [device_digest(device, root)],
            )
            .unwrap();
    }

    fn memory_project_count(state: &Path) -> i64 {
        rusqlite::Connection::open_with_flags(
            state.join(PROJECT_MEMORY),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .unwrap()
    }

    fn memory_cursors(state: &Path, project: &str) -> Vec<String> {
        let connection = rusqlite::Connection::open_with_flags(
            state.join(PROJECT_MEMORY),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut statement = connection
            .prepare("SELECT session_id FROM receipt_cursors WHERE project_id=?1 ORDER BY 1")
            .unwrap();
        statement
            .query_map([project], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// A move made before Memory was re-keyed left a fresh Project of the
    /// same root under the other machine's own id, and the core carried it
    /// home: the next move, either way, folds it into the Project it
    /// names instead of stopping on it, and a retry changes nothing.
    #[test]
    fn a_memory_project_the_move_would_land_on_is_folded_forward_and_back() {
        let dir = macbook_folder();
        let state = dir.path();
        plant_memory_leftover(state, C, C_ROOT);

        reown(state, &forward(), &forward_ids()).unwrap();
        assert_eq!(memory_project_count(state), 3, "folded, not kept beside it");
        let mini = device_digest(C, C_ROOT);
        assert_eq!(
            memory_cursors(state, &mini),
            vec!["left-over".to_owned(), "s1".to_owned()]
        );

        // The move back lands on a leftover under the mini's registration.
        plant_memory_leftover(state, ALIAS, C_ROOT);
        reown(state, &back(), &back_ids()).unwrap();
        assert_eq!(memory_project_count(state), 3);
        assert_eq!(
            memory_cursors(state, &device_digest(ALIAS, C_ROOT)),
            vec!["left-over".to_owned(), "s1".to_owned()]
        );
        assert_eq!(read(&state.join(MARKER_FILE))["node"], json!(M));
    }

    /// A card of `root` under `device`, in a Project that is on.
    fn plant_memory_card(state: &Path, device: &str, root: &str, text: &str) {
        let mut store = hide_memory::MemoryStore::open(&state.join(PROJECT_MEMORY)).unwrap();
        let id = device_digest(device, root);
        store.set_enabled(&id, true, true).unwrap();
        store
            .apply_candidates(
                &hide_memory::AnalysisBatch {
                    id: format!("batch-{text}"),
                    project_id: id.clone(),
                    provider: "codex".into(),
                    analysis_provider: "codex".into(),
                    session_id: "session-1".into(),
                    content_hash: text.into(),
                    created_at_unix_ms: 100,
                },
                &[hide_memory::Candidate {
                    text: text.into(),
                    title: String::new(),
                    aliases: Vec::new(),
                    kind: hide_memory::CandidateKind::Rule,
                    confidence: 0.9,
                    salience: 0.8,
                    source_offsets: vec![7],
                    human_offsets: vec![7],
                    source_times: BTreeMap::from([(7, 90)]),
                    cited_paths: Vec::new(),
                    relation: hide_memory::CandidateRelation::New,
                }],
            )
            .unwrap();
    }

    /// A promotion task of a Project's card, as the ledger keeps it: the
    /// intent carries the digest of the Project's id.
    fn promotion_spawn(project_id: &str, target: &str) -> Value {
        json!({
            "parent": "agent-1", "repo": C_ROOT, "path": C_ROOT, "requested_path": C_ROOT,
            "intent": format!(
                "{}{target}-a1-x",
                crate::runtime::memory_promotion::intent_prefix(project_id)
            ),
            "task": {"kind": "memory_promote", "target": target},
        })
    }

    /// Both copies of a Project had an m1, so the leftover's became m2: the
    /// promotion task that named m1 for it follows its card, and the task of
    /// the moving Project's own m1 stays.
    #[test]
    fn a_promotion_task_follows_its_card_to_the_number_a_fold_gave_it() {
        let dir = macbook_folder();
        let state = dir.path();
        plant_memory_leftover(state, C, C_ROOT);
        plant_memory_card(state, ALIAS, C_ROOT, "the moving Project's card");
        plant_memory_card(state, C, C_ROOT, "the leftover's card");
        let leftover = device_digest(C, C_ROOT);
        let moving = device_digest(ALIAS, C_ROOT);
        let mut ledger = read(&state.join(DELIVERY_LEDGER));
        let spawns = ledger["spawns"].as_array_mut().unwrap();
        spawns.push(promotion_spawn(&leftover, "m1"));
        spawns.push(promotion_spawn(&moving, "m1"));
        spawns.push(promotion_spawn(&leftover, "m9"));
        std::fs::write(
            state.join(DELIVERY_LEDGER),
            serde_json::to_vec(&ledger).unwrap(),
        )
        .unwrap();

        reown(state, &forward(), &forward_ids()).unwrap();
        let ledger = read(&state.join(DELIVERY_LEDGER));
        let targets: Vec<&str> = ledger["spawns"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|spawn| spawn.pointer("/task/target")?.as_str())
            .collect();
        assert_eq!(
            targets[targets.len() - 3..],
            ["m2", "m1", "m9"],
            "the leftover's m1 is m2; the moving Project's m1 and a card the map lacks stay"
        );
    }

    #[test]
    fn a_memory_project_of_another_root_at_the_new_id_stops_the_move_and_names_the_store() {
        let dir = macbook_folder();
        let state = dir.path();
        // Another Project's row at the id the mini's would take.
        let store = hide_memory::MemoryStore::open(&state.join(PROJECT_MEMORY)).unwrap();
        store
            .ensure_project(
                &device_digest(C, C_ROOT),
                Path::new("/Users/remote/other"),
                C,
            )
            .unwrap();
        drop(store);
        let refusal = reown(state, &forward(), &forward_ids()).unwrap_err();
        assert_eq!(refusal.file, state.join(PROJECT_MEMORY));
        assert!(
            refusal.reason.contains("node_conversion_conflict"),
            "{refusal}"
        );
        assert_eq!(
            read(&state.join(MARKER_FILE))["node"],
            json!(M),
            "the owner is unchanged"
        );
    }

    #[test]
    fn a_labels_section_the_move_does_not_know_stops_it() {
        let dir = macbook_folder();
        let state = dir.path();
        let mut labels = read(&state.join(LABELS));
        labels["pending"] = json!({ M: {} });
        std::fs::write(state.join(LABELS), serde_json::to_vec(&labels).unwrap()).unwrap();
        let refusal = reown(state, &forward(), &forward_ids()).unwrap_err();
        assert_eq!(refusal.file, state.join(LABELS));
        assert!(refusal.reason.contains("/pending"), "{refusal}");
        assert_eq!(
            read(&state.join(MARKER_FILE))["node"],
            json!(M),
            "the owner is unchanged"
        );
    }

    /// The marks a device pane keeps (`labels/store.rs`) are the writer's own
    /// section: a move that refused them would fail on every machine that has
    /// labelled a device pane.
    #[test]
    fn the_device_marks_of_the_labels_do_not_stop_a_move_and_are_dropped() {
        let dir = macbook_folder();
        let state = dir.path();
        let mut labels = read(&state.join(LABELS));
        labels["devices"] = json!({
            format!("device:{ALIAS}"): {"w2:p1": {"owner": "v1:o", "analysis_turn_end": 9}}
        });
        std::fs::write(state.join(LABELS), serde_json::to_vec(&labels).unwrap()).unwrap();
        reown(state, &forward(), &forward_ids()).unwrap();
        assert!(read(&state.join(LABELS)).get("devices").is_none());
    }

    #[test]
    fn a_sleeping_or_no_agents_fold_still_naming_the_old_id_is_found() {
        let change = forward();
        let asleep = format!("asleep/remote:{ALIAS}:project:{C_ROOT}");
        let no_agents = format!("no_agents/{ALIAS}");
        for (state, place) in [
            (json!({ asleep.clone(): true }), format!("/{asleep}")),
            (json!({"folds": [asleep.clone()]}), "/folds/0".to_owned()),
            (json!({ no_agents.clone(): true }), format!("/{no_agents}")),
            (json!({"folds": [no_agents]}), "/folds/0".to_owned()),
        ] {
            assert_eq!(still_naming(&state, &change, ""), Some(place), "{state}");
        }
        let moved = format!("asleep/remote:{C}:project:{C_ROOT}");
        assert_eq!(
            still_naming(
                &json!({ moved: true, "no_agents/studio": true }),
                &change,
                ""
            ),
            None
        );
    }

    #[test]
    fn a_folder_another_node_owns_or_no_node_owns_yet_is_refused() {
        let dir = macbook_folder();
        let state = dir.path();
        std::fs::write(
            state.join(MARKER_FILE),
            br#"{"version":1,"node":"someone-else"}"#,
        )
        .unwrap();
        let core = std::fs::read(state.join(CORE_STATE)).unwrap();
        let refusal = reown(state, &forward(), &forward_ids()).unwrap_err();
        assert!(refusal.reason.contains("someone-else"), "{refusal}");
        assert_eq!(std::fs::read(state.join(CORE_STATE)).unwrap(), core);
        std::fs::remove_file(state.join(MARKER_FILE)).unwrap();
        let refusal = reown(state, &forward(), &forward_ids()).unwrap_err();
        assert!(refusal.reason.contains("no owner yet"), "{refusal}");
        assert_eq!(std::fs::read(state.join(CORE_STATE)).unwrap(), core);
    }

    #[test]
    fn fold_and_recent_entries_naming_what_neither_machine_has_are_dropped_and_counted() {
        let dir = macbook_folder();
        let state = dir.path();
        let mut core = read(&state.join(CORE_STATE));
        let gone = crate::workspace::checkout_id_for_path(
            &own_workspace(),
            Path::new("/Users/alice/gone"),
        );
        core["collapsed_checkout_ids"]
            .as_array_mut()
            .unwrap()
            .push(json!(gone));
        core["focused_checkout_id"] = json!(gone);
        let recent = core["recent_checkouts"].as_array().unwrap().len();
        core["recent_checkouts"]
            .as_array_mut()
            .unwrap()
            .push(json!({"device_id": M, "checkout_id": gone, "project_name": "gone", "branch": "main", "device_name": "MacBook"}));
        std::fs::write(state.join(CORE_STATE), serde_json::to_vec(&core).unwrap()).unwrap();
        let outcome = reown(state, &forward(), &forward_ids()).unwrap();
        assert_eq!(outcome.pruned, 3);
        let core = read(&state.join(CORE_STATE));
        assert!(!core["collapsed_checkout_ids"].to_string().contains(&gone));
        assert_eq!(core["focused_checkout_id"], Value::Null);
        assert_eq!(core["recent_checkouts"].as_array().unwrap().len(), recent);
    }

    #[test]
    fn the_labels_a_move_carries_reach_the_new_core_in_memory_and_never_its_disk() {
        let dir = macbook_folder();
        let state = dir.path();
        reown(state, &forward(), &forward_ids()).unwrap();
        let store = crate::labels::store::LabelStore::open(Some(state), None, C);
        assert_eq!(
            store.target(C).len(),
            1,
            "the mini's records are the core's own"
        );
        let mac = crate::labels::device_target(M);
        assert_eq!(
            store.target(&mac).len(),
            1,
            "the MacBook's records are in memory"
        );
        store.flush();
        drop(store);
        let on_disk = read(&state.join(LABELS));
        assert!(on_disk.get("moved").is_none(), "{on_disk}");
        assert_eq!(
            on_disk["targets"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            vec![C]
        );
    }

    /// The stores a move rewrites that the core loads whole and writes whole.
    /// `labels.json` is not one: its writer keeps the owner's records only,
    /// and a moved machine's are taken into memory
    /// (`the_labels_a_move_carries_reach_the_new_core_in_memory_and_never_its_disk`).
    const READ_AND_WRITTEN_WHOLE: [&str; 4] =
        [CORE_STATE, WORKSPACE_VIEWS, DELIVERY_LEDGER, LOCAL_ISSUES];

    /// `file` in `state` as the core's own reader loads it and its own writer
    /// writes it back: what the store holds after the new core's first save.
    fn through_its_reader(state: &Path, file: &str) -> Value {
        let path = state.join(file);
        let out = tempfile::tempdir().unwrap();
        let written = out.path().join(file);
        match file {
            CORE_STATE => {
                let (ui, sizes, disposition) = crate::persistence::load(&path);
                assert_eq!(
                    disposition,
                    crate::persistence::LoadDisposition::Loaded,
                    "{file}"
                );
                crate::persistence::save(&written, &ui, &sizes).unwrap();
            }
            WORKSPACE_VIEWS => {
                let (views, outcome) = crate::workspace_views::load(&path, 0);
                assert!(
                    matches!(
                        &outcome,
                        crate::workspace_views::LoadOutcome::Loaded { repairs, .. } if repairs.is_empty()
                    ),
                    "{file}: {outcome:?}"
                );
                crate::workspace_views::save(&written, &views).unwrap();
            }
            DELIVERY_LEDGER => {
                let ledger = crate::delivery::ledger::load(&path).unwrap();
                crate::delivery::ledger::save(&written, &ledger).unwrap();
            }
            LOCAL_ISSUES => {
                let store = crate::local_issues::load(&path).unwrap();
                crate::local_issues::save(&written, &store).unwrap();
            }
            other => panic!("no reader for {other}"),
        }
        read(&written)
    }

    /// Every value under `value` that holds something, by its place.
    fn leaves(value: &Value, at: &str, out: &mut BTreeMap<String, Value>) {
        match value {
            Value::Object(map) => {
                for (key, entry) in map {
                    leaves(entry, &format!("{at}/{key}"), out);
                }
            }
            Value::Array(items) => {
                for (index, entry) in items.iter().enumerate() {
                    leaves(entry, &format!("{at}/{index}"), out);
                }
            }
            Value::Null => {}
            leaf => {
                out.insert(at.to_owned(), leaf.clone());
            }
        }
    }

    /// The store's own empty file, with `fields` in place of its own.
    fn store_with(file: &str, path: &Path, fields: Value) {
        match file {
            CORE_STATE => crate::persistence::save(
                path,
                &crate::model::UiStateSnapshot::default(),
                &Default::default(),
            )
            .unwrap(),
            WORKSPACE_VIEWS => crate::workspace_views::save(
                path,
                &crate::workspace_views::WorkspaceViews::default(),
            )
            .unwrap(),
            DELIVERY_LEDGER => {
                crate::delivery::ledger::save(path, &crate::delivery::ledger::Ledger::default())
                    .unwrap()
            }
            LOCAL_ISSUES => {
                crate::local_issues::save(path, &crate::local_issues::LocalIssueStore::default())
                    .unwrap()
            }
            other => panic!("no writer for {other}"),
        }
        let mut value = read(path);
        for (key, field) in fields.as_object().unwrap() {
            value[key] = field.clone();
        }
        // Private, as every store's writer leaves its file.
        hide_platform::fs::atomic::write_file_durable(
            path,
            &serde_json::to_vec(&value).unwrap(),
            hide_platform::fs::atomic::DurableAccess::Private,
        )
        .unwrap();
    }

    fn local_issue(number: u32, title: &str) -> Value {
        json!({"number": number, "title": title, "body": "", "open": true,
               "created_at_unix_ms": 1, "updated_at_unix_ms": 1})
    }

    /// A folder the MacBook's core wrote through each store's own writer,
    /// holding something in every place a move's whole-file code changes:
    /// the owner's and the devices' expanded paths and registrations, the
    /// agents' host scopes, a spawn on each machine, View layouts naming
    /// tabs, and the owner's and the mini's Local issues.
    fn reader_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        let panes = before("pane");
        let checkouts = before("checkout");
        let registrations = before("registration");
        let tabs = before("tab");
        store_with(
            CORE_STATE,
            &state.join(CORE_STATE),
            json!({
                "workspace_registrations": [
                    {"id": registrations[0], "label": "alpha", "path": M_ROOT, "device_id": M, "primary_checkout_id": checkouts[0], "pinned": false},
                    {"id": registrations[1], "label": "beta", "path": C_ROOT, "device_id": ALIAS, "primary_checkout_id": checkouts[1], "pinned": false},
                ],
                "device_registrations": [mini_registration(), {"id": X, "label": "Studio", "ssh_alias": "studio"}],
                "focused_device_id": ALIAS,
                "expanded_paths": [M_ROOT],
                "device_expanded_paths": {ALIAS: [C_ROOT], X: [X_ROOT]},
                "selected_pane_id": panes[1],
            }),
        );
        let layout = |tab: &str| {
            json!({
                "root": {"area": {"id": "a1", "active": tab, "displays": [{"id": tab}]}},
                "active_area": "a1", "next_id": 2, "canvases": {"a1": tab},
            })
        };
        store_with(
            WORKSPACE_VIEWS,
            &state.join(WORKSPACE_VIEWS),
            json!({"workspaces": [
                {"device_id": M, "path": M_ROOT, "agent_layout": layout(&tabs[0])},
                {"device_id": ALIAS, "path": C_ROOT, "agent_layout": layout(&tabs[1])},
            ]}),
        );
        // The ledger's own rule: an agent's actor is on its machine and names
        // its session by digest.
        let actor = |index: usize, machine: &str| {
            json!({"pane_id": panes[index], "name": "a", "kind": "claude",
            "device_id": machine, "session": crate::wire::session_digest(&format!("s{index}"))})
        };
        // `native_machine` is the machine's own node id, as its agent hooks report it.
        let agent = |index: usize, machine: &str, native: &str, scope: &str, root: &str| {
            json!({"id": format!("agent-{}", index + 1), "name": "a", "machine": machine, "host_scope": scope,
                   "native_machine": native, "session": format!("s{index}"), "instance": format!("t{index}"),
                   "pane": "w1:p1", "parent": null, "project": root, "actor": actor(index, machine), "ended": false})
        };
        let spawn = |index: usize, machine: Option<&str>, root: &str| {
            let mut row = json!({"id": format!("spawn-{}", index + 3), "parent": "agent-1", "intent": "i", "name": "child",
                   "kind": "claude", "repo": root, "branch": "main", "requested_path": root, "args": [],
                   "path": root, "pane": "w1:p2", "child": null});
            if let Some(machine) = machine {
                row["machine"] = json!(machine);
            }
            row
        };
        store_with(
            DELIVERY_LEDGER,
            &state.join(DELIVERY_LEDGER),
            json!({
                // Each id's number is below `next_id` and taken once.
                "next_id": 5,
                "agents": [agent(0, M, M, M_SOCKET, M_ROOT), agent(1, ALIAS, C, ALIAS, C_ROOT)],
                "spawns": [spawn(0, None, M_ROOT), spawn(1, Some(ALIAS), C_ROOT)],
            }),
        );
        store_with(
            LOCAL_ISSUES,
            &state.join(LOCAL_ISSUES),
            json!({
                "projects": {M_ROOT: {"next_number": 3, "issues": [local_issue(1, "first"), local_issue(2, "second")]}},
                "devices": {ALIAS: {C_ROOT: {"next_number": 2, "issues": [local_issue(1, "on the mini")]}}},
            }),
        );
        write(
            &state.join(MARKER_FILE),
            &serde_json::to_vec(&json!({"version": 1, "node": M})).unwrap(),
        )
        .unwrap();
        dir
    }

    /// A key a move writes that its store's type does not model is gone at
    /// the new core's first save of that store, and a move back has nothing
    /// to give back (engineering principle 13: every store, not the one that
    /// lost its Local issues).
    #[test]
    fn every_value_a_move_writes_survives_the_new_cores_first_save_of_its_store() {
        let dir = reader_folder();
        let state = dir.path();
        // What each value of a store's file is when its reader saved it.
        let dropped = |file: &str, before: &Value, after: &Value| -> Vec<String> {
            let (mut written, mut kept) = (BTreeMap::new(), BTreeMap::new());
            leaves(&normalized(before), "", &mut written);
            leaves(&normalized(after), "", &mut kept);
            written
                .iter()
                .filter(|(at, value)| kept.get(*at) != Some(value))
                .map(|(at, value)| format!("{file}{at} = {value}"))
                .collect()
        };
        // The fixture holds only what its reader keeps, and the move reads
        // each file as that reader wrote it.
        for file in READ_AND_WRITTEN_WHOLE {
            let saved = through_its_reader(state, file);
            let lost = dropped(file, &read(&state.join(file)), &saved);
            assert!(
                lost.is_empty(),
                "the fixture holds what its reader drops: {lost:#?}"
            );
            hide_platform::fs::atomic::write_file_durable(
                &state.join(file),
                &serde_json::to_vec(&saved).unwrap(),
                hide_platform::fs::atomic::DurableAccess::Private,
            )
            .unwrap();
        }
        for (change, ids) in [(forward(), forward_ids()), (back(), back_ids())] {
            reown(state, &change, &ids).unwrap();
            for file in READ_AND_WRITTEN_WHOLE {
                let saved = through_its_reader(state, file);
                let lost = dropped(file, &read(&state.join(file)), &saved);
                assert!(
                    lost.is_empty(),
                    "toward {}, the reader drops {lost:#?}",
                    change.new_owner
                );
                hide_platform::fs::atomic::write_file_durable(
                    &state.join(file),
                    &serde_json::to_vec(&saved).unwrap(),
                    hide_platform::fs::atomic::DurableAccess::Private,
                )
                .unwrap();
            }
        }
    }

    /// The operator's MacBook Local issues ride the move, survive the new
    /// core's own Local issue saves, and are the MacBook's own again after
    /// the move back, with their numbers.
    #[test]
    fn local_issues_come_back_with_their_numbers_after_the_new_core_saves_its_own() {
        let dir = reader_folder();
        let state = dir.path();
        let path = state.join(LOCAL_ISSUES);
        let before = crate::local_issues::load(&path).unwrap();
        reown(state, &forward(), &forward_ids()).unwrap();
        let mut store = crate::local_issues::load(&path).unwrap();
        assert_eq!(
            store.create(None, C_ROOT, "made on the mini's core", "", 10),
            Ok(2)
        );
        crate::local_issues::save(&path, &store).unwrap();

        reown(state, &back(), &back_ids()).unwrap();
        let after = crate::local_issues::load(&path).unwrap();
        assert_eq!(after.projects, before.projects);
        assert_eq!(
            after.devices[ALIAS][C_ROOT]
                .issues
                .iter()
                .map(|issue| (issue.number, issue.title.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, "on the mini"), (2, "made on the mini's core")]
        );
    }

    /// A ledger the MacBook's core kept, in the shapes a core's spawns come
    /// in, written through the ledger's own writer: a delegation an agent on
    /// the mini made (the spawn names no machine, its child is on the mini),
    /// a handoff the MacBook's agent made to the mini, a delegation on the
    /// MacBook itself, and a delegation a mini agent made onto the MacBook.
    /// Letters and a watch cross the two machines.
    fn ledger_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DELIVERY_LEDGER);
        // (id, machine, pane, parent, origin): an agent on the owner has an
        // unqualified pane, one on the mini a qualified one.
        let rows = [
            ("agent-1", M, "w1:p1", None, None),
            ("agent-2", ALIAS, "w1:p2", None, None),
            ("agent-3", ALIAS, "w2:p2", Some("agent-2"), None),
            ("agent-4", ALIAS, "w3:p2", None, Some("agent-1")),
            ("agent-5", M, "w2:p1", Some("agent-1"), None),
            ("agent-6", M, "w3:p1", Some("agent-2"), None),
        ];
        let actor = |index: usize| {
            let (_, machine, pane, ..) = rows[index];
            let pane_id = if machine == M {
                pane.to_owned()
            } else {
                format!("remote:{machine}:pane:{pane}")
            };
            json!({"pane_id": pane_id, "name": format!("a{index}"), "kind": "claude",
                   "device_id": machine, "session": crate::wire::session_digest(&format!("s{index}"))})
        };
        let agents: Vec<Value> = rows
            .iter()
            .enumerate()
            .map(|(index, (id, machine, pane, parent, origin))| {
                let (native, scope, root) = if *machine == M {
                    (M, M_SOCKET, M_ROOT)
                } else {
                    (C, ALIAS, C_ROOT)
                };
                json!({"id": id, "name": format!("a{index}"), "machine": machine, "host_scope": scope,
                       "native_machine": native, "session": format!("s{index}"), "instance": format!("t{index}"),
                       "pane": pane, "parent": parent, "origin": origin, "project": root,
                       "actor": actor(index), "ended": false})
            })
            .collect();
        // (id, parent, mode, machine, child, pane)
        let spawn_rows = [
            ("spawn-7", "agent-2", "delegation", None, "agent-3", "w2:p2"),
            (
                "spawn-8",
                "agent-1",
                "handoff",
                Some(ALIAS),
                "agent-4",
                "w3:p2",
            ),
            ("spawn-9", "agent-1", "delegation", None, "agent-5", "w2:p1"),
            (
                "spawn-10",
                "agent-2",
                "delegation",
                Some(M),
                "agent-6",
                "w3:p1",
            ),
        ];
        let spawns: Vec<Value> = spawn_rows
            .iter()
            .map(|(id, parent, mode, machine, child, pane)| {
                let mut row = json!({"id": id, "parent": parent, "mode": mode, "intent": id, "name": "child",
                       "kind": "claude", "repo": C_ROOT, "branch": "main", "requested_path": null, "args": [],
                       "path": C_ROOT, "pane": pane, "child": child, "completed": true});
                if let Some(machine) = machine {
                    row["machine"] = json!(machine);
                }
                row
            })
            .collect();
        let letter = |id: &str, from: usize, to: usize| {
            json!({"id": id, "intent": id, "sender": actor(from), "recipient": actor(to),
                   "kind": "request", "body": "synthetic", "state": "delivered", "hook_confirmed": true,
                   "waiting_answer": false, "reply_to": null, "created_at_unix_ms": 1,
                   "finished_at_unix_ms": 2, "bell_errors": 0, "bell_sent": true})
        };
        store_with(
            DELIVERY_LEDGER,
            &path,
            json!({
                "next_id": 20,
                "letters": [letter("letter-11", 0, 1), letter("letter-12", 2, 4), letter("letter-13", 5, 3)],
                "watches": [{"id": "watch-14", "parent": actor(1), "target": actor(2),
                             "last_activity_at_unix_ms": 1, "first_warning_at_unix_ms": null, "warning_count": 0,
                             "activity_failures": 0, "last_status": "idle", "last_state_change_seq": null,
                             "status_changed_at_unix_ms": 1}],
                "agents": agents,
                "spawns": spawns,
            }),
        );
        write(
            &dir.path().join(MARKER_FILE),
            &serde_json::to_vec(&json!({"version": 1, "node": M})).unwrap(),
        )
        .unwrap();
        // The fixture is a ledger the MacBook's core loads.
        crate::delivery::ledger::load(&path).unwrap();
        dir
    }

    /// The machine each spawn says its child is on after a move, read the way
    /// the ledger reads it: its own, or its caller's.
    fn spawn_machines(ledger: &Value) -> Vec<(String, String)> {
        let machine_of = |agent: &str| {
            ledger["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == agent)
                .unwrap()["machine"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        ledger["spawns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|spawn| {
                (
                    spawn["id"].as_str().unwrap().to_owned(),
                    spawn["machine"].as_str().map_or_else(
                        || machine_of(spawn["parent"].as_str().unwrap()),
                        str::to_owned,
                    ),
                )
            })
            .collect()
    }

    /// The ledger a move writes is one the new core loads: every spawn's
    /// machine is where its child is, whoever made it, on the way and on the
    /// way back (the mini's core refused the copy of a real move because
    /// a delegation its own agent made named the MacBook).
    #[test]
    fn the_ledger_a_move_writes_loads_and_every_spawn_keeps_its_childs_machine() {
        let dir = ledger_folder();
        let state = dir.path();
        let path = state.join(DELIVERY_LEDGER);
        let original = crate::delivery::ledger::load(&path).unwrap();

        reown(state, &forward(), &IdTable::default()).unwrap();
        crate::delivery::ledger::load(&path).expect("the mini's core loads the moved ledger");
        let moved = read(&path);
        // Children that were on the mini are on the core's own machine now,
        // those on the MacBook are on a device under its node id.
        assert_eq!(
            spawn_machines(&moved),
            [
                ("spawn-7", C),
                ("spawn-8", C),
                ("spawn-9", M),
                ("spawn-10", M)
            ]
            .map(|(id, machine)| (id.to_owned(), machine.to_owned()))
        );
        // Written only where the child is not its caller's machine.
        assert!(moved["spawns"][0].get("machine").is_none());
        assert_eq!(moved["spawns"][1]["machine"], json!(C));
        assert!(moved["spawns"][2].get("machine").is_none());
        assert_eq!(moved["spawns"][3]["machine"], json!(M));
        // Letters and the watch name the machines of the agents they are
        // between.
        let devices = |rows: &str, side: &str| -> Vec<Value> {
            moved[rows]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row[side]["device_id"].clone())
                .collect()
        };
        assert_eq!(devices("letters", "sender"), [json!(M), json!(C), json!(M)]);
        assert_eq!(
            devices("letters", "recipient"),
            [json!(C), json!(M), json!(C)]
        );
        assert_eq!(devices("watches", "parent"), [json!(C)]);
        assert_eq!(devices("watches", "target"), [json!(C)]);

        reown(state, &back(), &IdTable::default()).unwrap();
        assert_eq!(crate::delivery::ledger::load(&path).unwrap(), original);
    }

    /// The machine taking the core keeps the name the operator registered it
    /// under while the old core dialed it: the new core reads it from its
    /// store and its own device row carries it, so a window on the machine
    /// the core left names it as it did before the move. A move back to a
    /// machine that dialed in, registered only by its host name, keeps none.
    #[test]
    fn the_machine_taking_the_core_keeps_the_name_it_was_registered_under() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CORE_STATE);
        let marker = |node: &str| {
            std::fs::write(
                dir.path().join(MARKER_FILE),
                serde_json::to_vec(&json!({"version": 1, "node": node})).unwrap(),
            )
            .unwrap();
        };
        let before = crate::model::UiStateSnapshot {
            device_registrations: serde_json::from_value(json!([
                mini_registration(),
                {"id": X, "label": "Studio", "ssh_alias": "studio"},
            ]))
            .unwrap(),
            ..Default::default()
        };
        crate::persistence::save(&path, &before, &Default::default()).unwrap();
        marker(M);

        reown(dir.path(), &forward(), &IdTable::default()).unwrap();
        let (moved, _, _) = crate::persistence::load(&path);
        assert_eq!(moved.machine_label.as_deref(), Some("Mac mini"));
        let node = NodeId::parse(C).unwrap();
        let rows = crate::workspace::devices(&node, Some("Mac-mini.local"), &moved);
        assert_eq!(
            (
                rows[0].id.as_str(),
                rows[0].label.as_deref(),
                rows[0].machine_name.as_deref()
            ),
            (C, Some("Mac mini"), Some("Mac-mini.local"))
        );

        crate::persistence::save(&path, &moved, &Default::default()).unwrap();
        marker(C);
        reown(dir.path(), &back(), &IdTable::default()).unwrap();
        let (back_home, _, _) = crate::persistence::load(&path);
        assert_eq!(back_home.machine_label, None);
        let node = NodeId::parse(M).unwrap();
        assert_eq!(
            crate::workspace::devices(&node, Some("MacBook-Pro.local"), &back_home)[0].label,
            None
        );
    }

    /// A folder that already registered the machine giving up the core (a
    /// row a hand edit or an older move left) still has the old owner take
    /// the new owner's place among the rows that stay.
    #[test]
    fn the_old_owner_takes_the_new_owners_place_when_the_list_already_named_it() {
        let ids = IdTable::default();
        let change = forward();
        let mut mapper = Mapper {
            change: &change,
            ids: &ids,
            renumbered: &[],
            pruned: 0,
        };
        let mut core = json!({"device_registrations": [
            {"id": M, "label": "MacBook (old)"},
            {"id": X, "label": "Studio"},
            mini_registration(),
            {"id": "loft", "label": "Loft"},
        ]});
        mapper.device_registrations(&mut core).unwrap();
        let ids: Vec<&str> = core["device_registrations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [X, M, "loft"]);
        assert_eq!(core["device_registrations"][1]["label"], json!("MacBook"));
    }
}
