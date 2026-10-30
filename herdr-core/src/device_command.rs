//! `hide device list|add|remove|retry`: the SSH devices Settings › Devices
//! registers, for an agent the operator asked to add, connect or remove one
//! (PRD system-ssh-device-link D-11, B29-B32).
//!
//! Only a caller on the core's own machine may run them. `add` registers the
//! device without Hide's helper: installing Hide's kit writes the device
//! account's hook files, which the operator agrees to on the device's row,
//! so the answer says so instead of installing. Nothing here answers what
//! ssh asks, trusts a server key or allows the helper: those stay the
//! operator's, on the row, and an answer says what to ask them for.

use hide_node_link::device::{FailureAction, SignInWait, SshProblem};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum DeviceCommand {
    List,
    Add {
        ssh_alias: String,
        /// The name the device is shown under; the alias when absent.
        name: Option<String>,
        /// The device's Herdr socket when its server is not at the default
        /// path.
        herdr_socket_path: Option<String>,
    },
    Remove {
        device_id: String,
    },
    /// The row's Retry: an attempt that may ask the operator.
    Retry {
        device_id: String,
    },
}

/// The most bytes an alias, name, socket or id may take.
const VALUE_LIMIT: usize = 256;

fn valid_value(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= VALUE_LIMIT && !value.chars().any(char::is_control)
}

impl DeviceCommand {
    pub fn valid(&self) -> bool {
        match self {
            Self::List => true,
            Self::Add {
                ssh_alias,
                name,
                herdr_socket_path,
            } => {
                valid_value(ssh_alias)
                    && name.as_deref().is_none_or(valid_value)
                    && herdr_socket_path.as_deref().is_none_or(valid_value)
            }
            Self::Remove { device_id } | Self::Retry { device_id } => valid_value(device_id),
        }
    }
}

/// The id a device added under `alias` gets: the alias in lowercase letters,
/// digits, `_` and `-`, numbered past the ids already taken. Settings' Add
/// (`deviceIdFor` in the web shell) names a device the same way.
pub(crate) fn device_id_for<'a>(
    alias: &str,
    taken: impl Iterator<Item = &'a str> + Clone,
) -> String {
    let mut base = String::new();
    let mut in_run = false;
    for character in alias.trim().to_lowercase().chars() {
        let kept = character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '_'
            || character == '-';
        if kept {
            base.push(character);
        } else if !in_run {
            base.push('-');
        }
        in_run = !kept;
    }
    let base = match base.trim_matches('-') {
        "" => "device".to_owned(),
        trimmed => trimmed.to_owned(),
    };
    let free = |candidate: &str| {
        candidate != crate::node::LEGACY_LOCAL_DEVICE_ID && !taken.clone().any(|id| id == candidate)
    };
    if free(&base) {
        return base;
    }
    (2..)
        .map(|index| format!("{base}-{index}"))
        .find(|candidate| free(candidate))
        .expect("an unbounded range has a free id")
}

/// What the caller does about a refusal `code`.
pub fn next_action(code: &str) -> &'static str {
    match code {
        "machine_not_permitted" => {
            "Run hide device on the machine that runs Hide's core, not on a connected device"
        }
        "device_invalid" => {
            "Give one Host name from ~/.ssh/config, and with --socket an absolute path on the device"
        }
        "device_duplicate" => "Run hide device list: the device is already registered",
        "device_unknown" => "Run hide device list for the registered device ids",
        "device_local_remove_denied" => "The core's own machine cannot be removed",
        "device_dials_in" => {
            "The device connects to the core itself and reconnects when it can reach it; check its network"
        }
        "device_files_open" => {
            "Ask the operator to close the device's file tabs, or to remove it in Settings > Devices, which stores their drafts first"
        }
        _ => "Retry; the reason is in Hide's diagnostic log",
    }
}

/// `hide device retry`'s answer: whether this retry started an attempt, and
/// the device as `list` answers it after the retry.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct Retried {
    pub started: bool,
    pub device: serde_json::Map<String, serde_json::Value>,
}

/// The JSON Schema of [`Retried`], which `hide contract` publishes.
pub fn retried_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Retried)).expect("a JSON Schema serializes")
}

/// One device as `hide device` answers it (B29, B31): where its connection
/// is, what it waits on from the operator, why its last attempt failed and
/// the one thing to do next. It carries no question's words, no server key
/// and no key file: ssh's prompt can name the account, and none of it is the
/// agent's to answer. `remote_state` is the device's session status and
/// `problem` its last attempt's class, both as the row reads them.
pub(crate) fn device_answer(
    device: &crate::model::DeviceSnapshot,
    remote_state: Option<&str>,
    problem: Option<SshProblem>,
) -> serde_json::Value {
    let connection = connection(device, remote_state, problem);
    serde_json::json!({
        "id": device.id,
        "label": device.label,
        "ssh_alias": device.ssh_alias,
        "state": connection.state,
        "waits_on": connection.waits_on,
        "failure": connection.failure,
        "next_action": connection.next_action,
    })
}

/// Where a device's connection is, for `hide device` (B29, B30).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectionState {
    Connected,
    /// An attempt runs, or waits for a place among the attempts running.
    Connecting,
    /// Nothing moves until the operator acts on the device's row.
    OperatorTurn,
    /// The last attempt ended without a link and no attempt runs.
    Failed,
}

struct Connection {
    state: ConnectionState,
    waits_on: Option<serde_json::Value>,
    failure: Option<serde_json::Value>,
    next_action: Option<String>,
}

fn connection(
    device: &crate::model::DeviceSnapshot,
    remote_state: Option<&str>,
    problem: Option<SshProblem>,
) -> Connection {
    use serde_json::json;
    let name = device.label.as_deref().unwrap_or(&device.id);
    let id = &device.id;
    let row = format!("on {name}'s row in Settings > Devices");
    let turn = |waits_on: serde_json::Value, next_action: String| Connection {
        state: ConnectionState::OperatorTurn,
        waits_on: Some(waits_on),
        failure: None,
        next_action: Some(next_action),
    };
    let connecting = Connection {
        state: ConnectionState::Connecting,
        waits_on: None,
        failure: None,
        next_action: Some(format!(
            "Run hide device retry {id} --wait to wait for the attempt's outcome"
        )),
    };
    if device.state == "ready" {
        return Connection {
            state: ConnectionState::Connected,
            waits_on: None,
            failure: None,
            next_action: None,
        };
    }
    if let Some(sign_in) = &device.sign_in {
        return match &sign_in.wait {
            SignInWait::Secret { key_file, .. } => turn(
                json!({"kind": "sign_in", "input": "secret"}),
                if key_file.is_some() {
                    format!("Ask the operator to type the passphrase of the key ssh asks for {row}")
                } else {
                    format!("Ask the operator to type the password or code ssh asks for {row}")
                },
            ),
            SignInWait::Confirm { .. } => turn(
                json!({"kind": "sign_in", "input": "confirm"}),
                format!("Ask the operator to answer ssh's question {row}"),
            ),
            SignInWait::Touch { .. } => turn(
                json!({"kind": "sign_in", "input": "touch"}),
                format!("Ask the operator to touch their security key for {name}"),
            ),
            SignInWait::HostKey { .. } => turn(
                json!({"kind": "host_key"}),
                format!(
                    "Ask the operator to check the server's key fingerprint {row} and trust it only if it is that server's"
                ),
            ),
            SignInWait::Browser { url } => turn(
                json!({"kind": "browser", "url": url}),
                format!("Ask the operator to finish signing in to {name} in the browser at {url}"),
            ),
        };
    }
    if matches!(device.host.consent.as_str(), "none" | "outdated")
        || matches!(
            device.host.state.as_str(),
            "not_allowed" | "identity_changed"
        )
    {
        return turn(
            json!({"kind": "allow_and_install"}),
            format!(
                "Ask the operator to choose Allow and install {row}. Until then Hide installs nothing there, and the device's files, Git and terminal input wait for it"
            ),
        );
    }
    if device.host.state == "connecting" {
        return connecting;
    }
    if problem.is_none() && matches!(remote_state, Some("not_connected" | "connecting")) {
        return connecting;
    }
    let action = problem.map_or(FailureAction::Retry, SshProblem::action);
    let alias = device.ssh_alias.as_deref().unwrap_or(id);
    let next_action = match action {
        FailureAction::SignIn => format!(
            "Run hide device retry {id} --wait: it signs in again and may ask the operator {row}"
        ),
        FailureAction::CheckHostKey => format!(
            "Ask the operator to run ssh {alias} in a terminal and check why the server's key changed, then run hide device retry {id}"
        ),
        FailureAction::ReopenHide => "Ask the operator to quit Hide and open it again: an update replaced this Hide, and no retry of it can start ssh".to_owned(),
        FailureAction::Retry => format!(
            "Fix what detail names, or ask the operator to, then run hide device retry {id}"
        ),
    };
    Connection {
        state: ConnectionState::Failed,
        waits_on: None,
        failure: Some(json!({
            "action": action,
            "cause": problem.map(SshProblem::code),
            // ssh's last line, or what the link ended with, only where it is
            // what to fix: a refused sign-in's or a changed key's names the
            // account and server, and their action says all there is to do.
            "detail": match action {
                FailureAction::Retry => device.message.as_deref(),
                FailureAction::SignIn
                | FailureAction::CheckHostKey
                | FailureAction::ReopenHide => None,
            },
        })),
        next_action: Some(next_action),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_id_is_the_alias_in_id_letters_numbered_past_taken_ids() {
        let taken = ["studio", "studio-2"];
        assert_eq!(device_id_for("Studio", taken.iter().copied()), "studio-3");
        assert_eq!(
            device_id_for("my box.local", [].iter().copied()),
            "my-box-local"
        );
        assert_eq!(device_id_for("...", [].iter().copied()), "device");
        assert_eq!(device_id_for("local", [].iter().copied()), "local-2");
    }
}
