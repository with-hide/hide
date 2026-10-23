//! `hide device list|add|remove`: the SSH devices Settings › Devices
//! registers, for an agent the operator asked to add or remove one.
//!
//! Only a caller on the core's own machine may run them. `add` registers the
//! device without Hide's helper: installing Hide's kit writes the device
//! account's hook files, which the operator agrees to on the device's row,
//! so the answer says so instead of installing.

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
            Self::Remove { device_id } => valid_value(device_id),
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
        "device_files_open" => {
            "Ask the operator to close the device's file tabs, or to remove it in Settings > Devices, which stores their drafts first"
        }
        _ => "Retry; the reason is in Hide's diagnostic log",
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
