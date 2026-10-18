//! Where this machine's core runs (PRD core-host-node-remote-core D-02,
//! D-03): `<state>/core-placement.json`. Absent, this machine is its own
//! core, as every machine was before this record existed. Present, it names
//! the core's machine and this daemon runs in the node role: it starts no
//! core and dials that machine instead.
//!
//! Only the move (layer 5), the node's disconnect and reconnect (B16) and
//! test fixtures write the record. It is the
//! account's own private file; one that is anything else is refused rather
//! than read, and so is one that names this machine as its own core.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hide_platform::fs::private;
use serde::{Deserialize, Serialize};

/// The record's largest size.
const RECORD_CAP: u64 = 4096;
/// The longest alias, program or state folder a record may name.
const MAX_VALUE: usize = 1024;

/// The core's machine, as this machine reaches it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// The SSH alias in this account's `~/.ssh/config`.
    pub alias: String,
    /// The core machine's node id, which its attach role must name.
    pub node: String,
    /// The `hided` the core machine runs as its attach role.
    pub program: String,
    /// The core's state folder on its machine, when it is not that
    /// account's default.
    #[serde(default)]
    pub state_dir: Option<String>,
    /// The move that placed the core there, until it committed: the node's
    /// links name it, and the core's first link with it commits the move
    /// (PRD core-host-node-move amendment 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_intent: Option<String>,
    /// The operator ended the link from this machine's window (PRD
    /// core-host-node-move B16): the node dials nothing until they
    /// reconnect, across restarts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disconnected: bool,
}

/// Every change this process makes to a record already there: the node's
/// update of its core rewrites the program, the move's commit clears its
/// intent, and the operator's disconnect sets its flag, each reading the
/// record as the others left it.
static CHANGES: Mutex<()> = Mutex::new(());

/// Changes the record `state_dir` holds with `change`, after any change
/// this process made before it; `false` when there is no record.
pub fn update(
    state_dir: &Path,
    own_node: &str,
    change: impl FnOnce(&mut Placement),
) -> Result<bool, String> {
    let _changing = CHANGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(mut placement) = read(state_dir, own_node)? else {
        return Ok(false);
    };
    let before = placement.clone();
    change(&mut placement);
    if placement != before {
        write(state_dir, &placement)?;
    }
    Ok(true)
}

/// Records `placement` for `state_dir`, replacing any record there.
pub fn write(state_dir: &Path, placement: &Placement) -> Result<(), String> {
    let path = record_path(state_dir);
    let bytes = serde_json::to_vec_pretty(placement).map_err(|error| error.to_string())?;
    hide_platform::fs::atomic::write_file_durable(
        &path,
        &bytes,
        hide_platform::fs::atomic::DurableAccess::Private,
    )
    .map(|_| ())
    .map_err(|error| format!("{}: {error}", path.display()))
}

/// Removes the record: this machine is its own core again.
pub fn remove(state_dir: &Path) -> Result<(), String> {
    let path = record_path(state_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

pub fn record_path(state_dir: &Path) -> PathBuf {
    hide_kit::layout::core_placement(state_dir)
}

/// The placement `state_dir` records; `None` when this machine is its own
/// core.
pub fn read(state_dir: &Path, own_node: &str) -> Result<Option<Placement>, String> {
    let path = record_path(state_dir);
    let file = match private::open_own_file(&path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "the core placement record could not be opened: {error}"
            ));
        }
    };
    if !private::is_private(&path).unwrap_or(false) {
        return Err("the core placement record is readable by other accounts".to_owned());
    }
    let mut bytes = Vec::new();
    file.take(RECORD_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("the core placement record could not be read: {error}"))?;
    if bytes.len() as u64 > RECORD_CAP {
        return Err("the core placement record is too large".to_owned());
    }
    let placement: Placement = serde_json::from_slice(&bytes).map_err(|error| {
        format!("the core placement record is not one this build reads: {error}")
    })?;
    herdr_core::node::NodeId::parse(&placement.node)
        .map_err(|error| format!("the core placement record names no node: {error}"))?;
    if placement.node == own_node {
        return Err("the core placement record names this machine".to_owned());
    }
    if placement.alias.trim().is_empty() || placement.alias.starts_with('-') {
        return Err("the core placement record names no SSH alias".to_owned());
    }
    // Each value is written into a command line on the core's machine.
    let plain = |value: &str| value.len() <= MAX_VALUE && !value.chars().any(char::is_control);
    if ![
        placement.alias.as_str(),
        placement.program.as_str(),
        placement.state_dir.as_deref().unwrap_or_default(),
    ]
    .into_iter()
    .all(plain)
    {
        return Err(
            "the core placement record holds a control character or an over-long value".to_owned(),
        );
    }
    // Paths on the core's machine, in the spelling between machines.
    if !hide_platform::path::is_wire_absolute(&placement.program) {
        return Err("the core placement record's program is not an absolute path".to_owned());
    }
    if placement
        .state_dir
        .as_deref()
        .is_some_and(|dir| !hide_platform::path::is_wire_absolute(dir))
    {
        return Err("the core placement record's state folder is not an absolute path".to_owned());
    }
    Ok(Some(placement))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = "own-node";

    fn write(dir: &Path, text: &str) {
        let path = record_path(dir);
        std::fs::write(&path, text).unwrap();
        private::restrict_to_owner(&path).unwrap();
    }

    #[test]
    fn an_absent_record_leaves_this_machine_its_own_core() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path(), OWN), Ok(None));
    }

    #[test]
    fn a_record_names_the_core_machine() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{"alias":"mini","node":"core-node","program":"/opt/hided","state_dir":"/tmp/c"}"#,
        );
        assert_eq!(
            read(dir.path(), OWN),
            Ok(Some(Placement {
                alias: "mini".to_owned(),
                node: "core-node".to_owned(),
                program: "/opt/hided".to_owned(),
                state_dir: Some("/tmp/c".to_owned()),
                move_intent: None,
                disconnected: false,
            }))
        );
    }

    #[test]
    fn a_record_naming_this_machine_or_shared_with_others_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{"alias":"mini","node":"own-node","program":"/opt/hided"}"#,
        );
        assert!(read(dir.path(), OWN).is_err());
        write(
            dir.path(),
            r#"{"alias":"mini","node":"core-node","program":"/opt/hided"}"#,
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                record_path(dir.path()),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            assert!(read(dir.path(), OWN).is_err());
        }
    }
}
