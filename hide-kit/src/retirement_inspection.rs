//! Read-only predicates the coordination retirement inspects with, on this
//! machine and on a device.
//! They check path authority and occupancy; no legacy contents are imported or rewritten.

use serde_json::Value;

pub const MAX_LEDGER_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_STATE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 1024;

/// Authority is a selected HOME or a checkout already registered on that
/// machine. Wire paths compare by whole names without filesystem access.
pub fn inspection_root<'a>(
    path: &str,
    roots: impl IntoIterator<Item = &'a str>,
) -> Result<&'a str, String> {
    roots
        .into_iter()
        .find(|root| hide_platform::path::wire_relative(root, path).is_ok())
        .ok_or_else(|| "a sasu run state has no trusted HOME or registered checkout root; register its owning checkout, then retry retirement".into())
}

/// A legacy folder may be renamed only below HOME, separate from the
/// current kit and state. A selected root itself is never a removal target.
pub fn legacy_location(home: &str, path: &str, state: &str) -> Result<(), String> {
    let relative = hide_platform::path::wire_relative(home, path).map_err(|_| {
        "the legacy coordination home has no trusted HOME anchor; inspect its location before retrying retirement".to_owned()
    })?;
    let kit = hide_platform::path::RelPath::parse(".hide/kit").unwrap();
    let overlaps = |other: &hide_platform::path::RelPath| {
        relative.starts_with(other) || other.starts_with(&relative)
    };
    if relative.is_root()
        || overlaps(&kit)
        || hide_platform::path::wire_relative(home, state).is_ok_and(|state| overlaps(&state))
    {
        return Err("the legacy coordination home overlaps active Hide state; inspect the override before retrying retirement".into());
    }
    Ok(())
}

pub fn legacy_ledger(ledger: &Value) -> Result<(), String> {
    if ledger["schema"] != "hcoord.ledger.v1" {
        return Err("legacy coordination ledger has an unknown schema; inspect it before retrying retirement".into());
    }
    for (table, active, closed) in [
        ("requests", "open", &["answered", "canceled"][..]),
        ("watches", "active", &["stopped"][..]),
    ] {
        let entries = ledger[table].as_object().ok_or_else(|| {
            format!("legacy {table} cannot be inspected; retirement has not begun")
        })?;
        for value in entries.values() {
            let status = value["status"]
                .as_str()
                .ok_or_else(|| format!("legacy {table} has no status; retirement has not begun"))?;
            if status == active {
                return Err(format!(
                    "legacy coordination has {active} {table}; finish or close them, then retry retirement"
                ));
            }
            if !closed.contains(&status) {
                return Err(format!(
                    "legacy {table} has an unknown status; inspect it before retrying retirement"
                ));
            }
        }
    }
    Ok(())
}

pub fn delivery_ledger(ledger: &Value) -> Result<(), String> {
    if ledger["version"] != 1 {
        return Err(
            "Hide delivery ledger has an unknown version; inspect it before retirement".into(),
        );
    }
    let letters = ledger["letters"]
        .as_array()
        .ok_or("Hide requests cannot be inspected; retirement has not begun")?;
    let watches = ledger["watches"]
        .as_array()
        .ok_or("Hide watches cannot be inspected; retirement has not begun")?;
    for letter in letters {
        let hook_confirmed = match letter.get("hook_confirmed") {
            None | Some(Value::Null) => None,
            Some(Value::Bool(confirmed)) => Some(*confirmed),
            _ => {
                return Err(
                    "Hide request status cannot be inspected; retirement has not begun".into(),
                );
            }
        };
        if !matches!(
            letter["state"].as_str(),
            Some(
                "pending" | "delivered" | "acknowledged" | "cancelled" | "expired" | "undelivered"
            )
        ) || !letter["waiting_answer"].is_boolean()
            || (hook_confirmed == Some(true)
                && matches!(
                    letter["state"].as_str(),
                    Some("pending" | "undelivered" | "expired")
                ))
            || (hook_confirmed == Some(false) && letter["state"] == "delivered")
            || (letter["waiting_answer"] == true
                && matches!(
                    letter["state"].as_str(),
                    Some("cancelled" | "undelivered" | "expired")
                ))
        {
            return Err("Hide request status cannot be inspected; retirement has not begun".into());
        }
    }
    // A legacy Ack has no intake proof, but stays closed operationally.
    // Only an explicit unconfirmed Ack joins Pending in awaiting intake.
    if letters.iter().any(|value| {
        value["state"] == "pending"
            || value["waiting_answer"] == true
            || (value["state"] == "acknowledged" && value["hook_confirmed"] == false)
    }) {
        return Err("Hide has open requests; finish or close them, then retry retirement".into());
    }
    if !watches.is_empty() {
        return Err("Hide has active watches; stop them, then retry retirement".into());
    }
    Ok(())
}

/// State paths from the authoritative latest supervisor revision.
pub fn supervisor_states(index: &Value) -> Result<Vec<&str>, String> {
    match index["schema"].as_str() {
        Some("sasu.supervisor.index.v1") => legacy_supervisor_states(index),
        Some("sasu.supervisor.index.v2.hide") => registered_supervisor_states(index),
        _ => Err("sasu registry has an unknown schema; inspect it before retirement".into()),
    }
}

fn legacy_supervisor_states(index: &Value) -> Result<Vec<&str>, String> {
    if !index["tickExecutor"].is_null() {
        return Err(
            "sasu supervisor still has an active tick; stop it and retry retirement".into(),
        );
    }
    let entries = index["entries"]
        .as_array()
        .ok_or("sasu indexed runs cannot be inspected")?;
    if !entries.is_empty() {
        return Err(
            "sasu supervisor still owns indexed runs; retire them and retry retirement".into(),
        );
    }
    let coordinated = if index.get("coordinated").is_none() {
        &[][..]
    } else {
        index["coordinated"]
            .as_array()
            .ok_or("sasu coordinated runs cannot be inspected")?
            .as_slice()
    };
    if coordinated.len() > MAX_ENTRIES {
        return Err("sasu runs exceed the preflight entry bound".into());
    }
    coordinated
        .iter()
        .map(|entry| {
            entry["runInstanceId"]
                .as_str()
                .ok_or("sasu run has no identity; inspect its registry before retirement")?;
            entry["statePath"].as_str().ok_or_else(|| {
                "sasu run has no state path; inspect its registry before retirement".into()
            })
        })
        .collect()
}

fn registered_supervisor_states(index: &Value) -> Result<Vec<&str>, String> {
    let entries = index["entries"]
        .as_array()
        .ok_or("sasu indexed runs cannot be inspected")?;
    if entries.len() > MAX_ENTRIES {
        return Err("sasu runs exceed the preflight entry bound".into());
    }
    // Write identities only fence the registry's optimistic updates. They
    // are not runs and are never imported or interpreted as occupancy.
    let writes = index["appliedWrites"]
        .as_array()
        .ok_or("sasu registry write history cannot be inspected")?;
    if writes.len() > MAX_ENTRIES
        || writes.iter().any(|value| {
            value
                .as_str()
                .is_none_or(|write| write.is_empty() || write.len() > 128)
        })
    {
        return Err("sasu registry write history cannot be inspected".into());
    }
    entries
        .iter()
        .map(|entry| {
            let required = |field: &str| {
                entry[field]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        format!("sasu run has no {field}; inspect its registry before retirement")
                    })
            };
            let path = required("statePath")?;
            let identity = required("runInstanceId")?;
            required("registrationId")?;
            required("addedAt")?;
            if path.len() > 4096 || identity.len() > 256 {
                return Err("sasu run identity exceeds the preflight size bound".into());
            }
            if !entry.get("recipientAuthorityKey").is_some_and(|value| {
                value.is_null()
                    || value.as_str().is_some_and(|key| {
                        key.len() == 64
                            && key
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    })
            }) {
                return Err("sasu run recipient authority cannot be inspected".into());
            }
            Ok(path)
        })
        .collect()
}

pub fn indexed_run(state: &Value) -> Result<(), String> {
    match state["status"].as_str() {
        Some("retired") => Ok(()),
        Some("active") => Err("a sasu run is active; retire it and retry retirement".into()),
        _ => Err("sasu run status cannot be inspected; retirement has not begun".into()),
    }
}

/// Other tools keep artifacts under agents/runs too. Only Sasu's source marker
/// makes an unknown status its unreadable run; an active status always blocks.
pub fn checkout_run(state: &Value) -> Result<(), String> {
    if state["status"] == "active"
        || state["schema"]
            .as_str()
            .is_some_and(|schema| schema.starts_with("sasu.implement.state.v"))
    {
        indexed_run(state)?;
    }
    Ok(())
}

/// A bounded receipt can bypass occupancy only after retirement completed.
pub fn completed(progress: &Value, homes: &[String]) -> Result<bool, String> {
    let complete = progress["complete"]
        .as_bool()
        .ok_or("retirement progress is unreadable; inspect it before retrying")?;
    progress["step"]
        .as_str()
        .ok_or("retirement progress is unreadable; inspect it before retrying")?;
    if !progress["failure"].is_null() && !progress["failure"].is_string() {
        return Err("retirement progress is unreadable; inspect it before retrying".into());
    }
    let pairs = progress["homes"]
        .as_array()
        .ok_or("retirement progress is unreadable; inspect it before retrying")?;
    for pair in pairs {
        let pair = pair
            .as_array()
            .filter(|pair| pair.len() == 2)
            .ok_or("retirement progress names an unexpected folder; inspect it before retrying")?;
        let source = pair[0]
            .as_str()
            .ok_or("retirement progress is unreadable")?;
        let destination = pair[1]
            .as_str()
            .ok_or("retirement progress is unreadable")?;
        let Some((parent, name)) = source.rsplit_once('/') else {
            return Err(
                "retirement progress names an unexpected folder; inspect it before retrying".into(),
            );
        };
        if !homes.iter().any(|home| home == source)
            || !destination.starts_with(&format!("{parent}/{name}.retired-"))
            || destination[parent.len() + 1..].contains('/')
        {
            return Err(
                "retirement progress names an unexpected folder; inspect it before retrying".into(),
            );
        }
    }
    Ok(complete)
}
