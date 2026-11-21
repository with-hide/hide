//! How a change of the ids a state folder names its machines by reaches
//! every key that names one.
//!
//! A folder names its owner's machine by leaving keys unqualified and every
//! other machine by an id: a dialed device by its registration id
//! (`remote:<id>:pane:w1:p1`), a node that links in by its node id. The
//! `KEYS` table says where each such key is and how its value changes
//! (`Moves`); a [`Change`] says what an id becomes. The one change made
//! today is the naming of devices by their machines' own ids (`devices`).

use crate::address::Kind;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::Moves;

/// How a change of a state folder spells the machines it gives another id,
/// as the one-time naming of devices by their machines' own ids does
/// (`super::devices`). The `KEYS` table says where each spelling is; this
/// says what it becomes.
pub(super) trait Change {
    /// The id a machine id becomes; `None` when it stays.
    fn device(&self, id: &str) -> Option<String>;
    /// The id an engine-scoped id of `kind` becomes; `None` when it stays.
    fn scoped(&self, kind: Kind, id: &str) -> Option<String>;
    /// The id a checkout id the table lacks becomes; `None` when it stays
    /// or is dropped (`lacks`).
    fn checkout(&self, _id: &str) -> Option<String> {
        None
    }
    /// Whether a registration or checkout id the table lacks belongs to a
    /// machine the change gives another id, and so names something no
    /// machine still has.
    fn lacks(&self, id: &str) -> bool;
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

/// What a mapping makes of one value: the same, another, or nothing (an
/// entry of a fold or recent list that names what neither machine has).
pub(super) enum Mapped {
    Same,
    To(String),
    Drop,
}

/// What `change` makes of `value` at a place whose row moves as `moves`.
pub(super) fn map_id(change: &dyn Change, ids: &IdTable, moves: Moves, value: &str) -> Mapped {
    let to = |mapped: Option<String>| mapped.map_or(Mapped::Same, Mapped::To);
    match moves {
        Moves::Device => to(change.device(value)),
        Moves::Pane => to(change.scoped(Kind::Pane, value)),
        Moves::Tab => to(change.scoped(Kind::Tab, value)),
        Moves::Registration => table(change, &ids.registrations, value),
        Moves::Checkout => match change.checkout(value) {
            Some(id) => Mapped::To(id),
            None => table(change, &ids.checkouts, value),
        },
        Moves::Fold => {
            // A device's own folds carry its id; a sleeping project's
            // fold carries the project's (PRD sidebar-titlebar-refresh D-19).
            if let Some(device) = value.strip_prefix("cleanup/") {
                to(change.device(device).map(|id| format!("cleanup/{id}")))
            } else if let Some(device) = value.strip_prefix("no_agents/") {
                to(change.device(device).map(|id| format!("no_agents/{id}")))
            } else if let Some(project) = value.strip_prefix("asleep/") {
                match table(change, &ids.registrations, project) {
                    Mapped::To(id) => Mapped::To(format!("asleep/{id}")),
                    other => other,
                }
            } else {
                table(change, &ids.registrations, value)
            }
        }
        Moves::Stays | Moves::OwnPath(_) | Moves::Labels | Moves::Whole => Mapped::Same,
    }
}

/// A registration or checkout id the change gives another id maps through
/// the table; one of another machine stays; one the table lacks of a
/// machine the change respells names something no machine still has.
fn table(change: &dyn Change, table: &BTreeMap<String, String>, value: &str) -> Mapped {
    if let Some(id) = table.get(value) {
        return Mapped::To(id.clone());
    }
    if change.lacks(value) {
        Mapped::Drop
    } else {
        Mapped::Same
    }
}

/// Maps every value or key `pattern` reaches in a store, as `map` says;
/// answers how many entries it dropped.
pub(super) fn apply_pattern(
    value: &mut Value,
    pattern: &str,
    moves: Moves,
    map: &dyn Fn(Moves, &str) -> Mapped,
) -> Result<usize, String> {
    if matches!(
        moves,
        Moves::Stays | Moves::OwnPath(_) | Moves::Labels | Moves::Whole
    ) {
        return Ok(0);
    }
    let parts: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    let mut pruned = 0;
    walk(value, &parts, &mut |slot| match slot {
        Slot::Value(value) => {
            let Some(text) = value.as_str() else {
                return Ok(false);
            };
            match map(moves, text) {
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
        Slot::Keys(object) => {
            let keys: Vec<String> = object.keys().cloned().collect();
            let mut renamed = Map::new();
            for key in keys {
                let entry = object.remove(&key).expect("listed key");
                let key = match map(moves, &key) {
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
            *object = renamed;
            Ok(false)
        }
    })?;
    Ok(pruned)
}

/// Moves each Project's choices whose machine `device` gives another id to
/// the key that id and the record's root make (`project_settings::key`),
/// naming the new id in the record; two records meeting at one key stop it.
pub(super) fn map_project_settings(
    value: &mut Value,
    device: &dyn Fn(&str) -> Option<String>,
) -> Result<(), String> {
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
        let renamed = record
            .get("device_id")
            .and_then(Value::as_str)
            .and_then(device);
        let new_key = match renamed {
            Some(device) => {
                let Some(root) = record.get("root").and_then(Value::as_str) else {
                    return Err(format!("/project_settings/{key} names no root"));
                };
                let new_key = crate::project_settings::key(&device, root);
                record["device_id"] = Value::String(device);
                new_key
            }
            None => key,
        };
        if moved.insert(new_key.clone(), record).is_some() {
            return Err(format!("two project settings would both be {new_key}"));
        }
    }
    value["project_settings"] = Value::Object(moved);
    Ok(())
}

/// The View layouts' machine ids and tabs (`workspace-views.json`): each
/// Agent layout's areas and canvases, each group's layout, each start's live
/// tab and pane, and each opener's machine, which a pattern cannot express.
pub(super) fn map_views(value: &mut Value, map: &dyn Fn(Moves, &str) -> Mapped) {
    for view in each(value, "workspaces") {
        let drafts: BTreeSet<String> = view
            .pointer("/agent_groups/drafts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        if let Some(layout) = view.get_mut("agent_layout") {
            map_layout_tabs(layout, &drafts, map);
        }
        if let Some(groups) = view.get_mut("agent_groups") {
            for group in each(groups, "items") {
                if let Some(layout) = group.get_mut("layout") {
                    map_layout_tabs(layout, &drafts, map);
                }
            }
        }
        if let Some(starts) = view.get_mut("agent_starts").and_then(Value::as_object_mut) {
            for start in starts.values_mut() {
                for (field, kind) in [("live_tab_id", Moves::Tab), ("pane_id", Moves::Pane)] {
                    if let Some(value) = start.get_mut(field)
                        && let Some(Mapped::To(id)) = value.as_str().map(|text| map(kind, text))
                    {
                        *value = Value::String(id);
                    }
                }
            }
        }
        map_trays(view, map);
        super::each_opener(view, &mut |device| match device
            .as_str()
            .map(|id| map(Moves::Device, id))
        {
            Some(Mapped::To(id)) => {
                *device = Value::String(id);
                true
            }
            _ => false,
        });
    }
}

/// Each card's project, checkout and session machine, on a tray or not
/// (PRD lane-desk-pair-view D-06, D-13); a tray's tab is a `KEYS` row.
fn map_trays(view: &mut Value, map: &dyn Fn(Moves, &str) -> Mapped) {
    fn set(value: Option<&mut Value>, kind: Moves, map: &dyn Fn(Moves, &str) -> Mapped) {
        if let Some(value) = value
            && let Some(Mapped::To(id)) = value.as_str().map(|text| map(kind, text))
        {
            *value = Value::String(id);
        }
    }
    fn cards(value: &mut Value, map: &dyn Fn(Moves, &str) -> Mapped) {
        match value {
            Value::Object(fields) => {
                if let Some(card) = fields.get_mut("card") {
                    set(card.get_mut("workspace_id"), Moves::Registration, map);
                    set(card.get_mut("checkout_id"), Moves::Checkout, map);
                    set(
                        card.get_mut("session")
                            .and_then(|session| session.get_mut("device_id")),
                        Moves::Device,
                        map,
                    );
                }
                for (key, entry) in fields.iter_mut() {
                    if key != "card" {
                        cards(entry, map);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| cards(item, map)),
            _ => {}
        }
    }
    for tray in each(view, "trays") {
        if let Some(layout) = tray.get_mut("layout") {
            cards(layout, map);
        }
    }
    if let Some(shelf) = view.get_mut("shelf") {
        cards(shelf, map);
    }
}

/// Every tab an Agent layout names: each area's front display and its
/// displays, and each canvas's tab; a draft group's tab is no engine tab.
fn map_layout_tabs(
    layout: &mut Value,
    drafts: &BTreeSet<String>,
    map: &dyn Fn(Moves, &str) -> Mapped,
) {
    fn node(value: &mut Value, drafts: &BTreeSet<String>, map: &dyn Fn(Moves, &str) -> Mapped) {
        if let Some(area) = value.get_mut("area") {
            if let Some(active) = area.get_mut("active") {
                tab(active, drafts, map);
            }
            for display in each(area, "displays") {
                if let Some(id) = display.get_mut("id") {
                    tab(id, drafts, map);
                }
            }
        }
        if let Some(split) = value.get_mut("split") {
            for side in ["first", "second"] {
                if let Some(child) = split.get_mut(side) {
                    node(child, drafts, map);
                }
            }
        }
    }
    fn tab(value: &mut Value, drafts: &BTreeSet<String>, map: &dyn Fn(Moves, &str) -> Mapped) {
        if value.as_str().is_some_and(|id| drafts.contains(id)) {
            return;
        }
        if let Some(Mapped::To(id)) = value.as_str().map(|text| map(Moves::Tab, text)) {
            *value = Value::String(id);
        }
    }
    if let Some(root) = layout.get_mut("root") {
        node(root, drafts, map);
    }
    if let Some(canvases) = layout.get_mut("canvases").and_then(Value::as_object_mut) {
        for canvas in canvases.values_mut() {
            tab(canvas, drafts, map);
        }
    }
}

/// The owner's settings that belong to its own machine and the map beside
/// each that keeps the same setting for every other machine, keyed by its
/// id (PRD core-host-node-move Q19): the Explorer's open folders, which the
/// web reads per machine, and each agent's 권한 (Settings › Agents), which
/// a start on that machine reads. Settings keyed by a project path are read the same
/// whichever machine holds the project, so they stay (`Moves::Stays`).
pub const OWN_PATHS: &[(&str, &str)] = &[
    ("expanded_paths", "device_expanded_paths"),
    ("agent_permissions", "device_agent_permissions"),
    ("add_project_folder", "device_add_project_folders"),
];

/// What a pattern reaches: a value, or the object whose keys it names.
pub(super) enum Slot<'a> {
    Value(&'a mut Value),
    Keys(&'a mut Map<String, Value>),
}

/// Calls `visit` for each slot `parts` reach (`*` is each array element or
/// object value, `{key}` the object's keys, a number the array's element at
/// that place); a value slot answering `true` is removed from the array or
/// object holding it.
pub(super) fn walk(
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
            // A number is the place in a tuple field (`remote_folder/0`).
            if let Value::Array(items) = value {
                let Some(entry) = key.parse::<usize>().ok().and_then(|at| items.get_mut(at)) else {
                    return Ok(false);
                };
                if walk(entry, rest, visit)? && rest.is_empty() {
                    return Err(format!("place {key} of a list cannot be cleared"));
                }
                return Ok(false);
            }
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

pub(super) fn each<'a>(value: &'a mut Value, key: &str) -> impl Iterator<Item = &'a mut Value> {
    value
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
}

/// The fields whose value is a machine id.
pub(super) const MACHINE_FIELDS: &[&str] = &[
    "device_id",
    "focused_device_id",
    "machine",
    "native_machine",
    "node_id",
    "target_id",
    "host_scope",
];

/// The first place where a key still names a machine by `old_id`: as
/// `remote:<old_id>:` anywhere, and, when `alias` (the id is not one the
/// machine keeps), as a machine field's value or an object key.
pub(super) fn still_naming_id(
    value: &Value,
    old_id: &str,
    alias: bool,
    at: &str,
) -> Option<String> {
    // A sleeping project's fold names the project after its own prefix.
    let carries = |text: &str| {
        crate::address::device_of(text.strip_prefix("asleep/").unwrap_or(text)) == Some(old_id)
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
                _ => still_naming_id(item, old_id, alias, &place),
            }
        }),
        Value::Object(map) => map.iter().find_map(|(key, entry)| {
            let place = format!("{at}/{key}");
            if carries(key) || is_alias(key) {
                return Some(place);
            }
            // A mission's words are the Conductor's text, like a body.
            if matches!(
                key.as_str(),
                "body" | "facts" | "title" | "line" | "decision" | "delay" | "report" | "criteria"
            ) {
                return None;
            }
            still_naming_id(entry, old_id, alias, &place)
        }),
        _ => None,
    }
}
