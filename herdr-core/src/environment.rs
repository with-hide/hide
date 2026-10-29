use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::model::EnvironmentStatusSnapshot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvironmentVariableSpec {
    pub key: &'static str,
    pub required: bool,
    pub format: &'static str,
    pub absent_behavior: &'static str,
}

/// `HOME`, or `USERPROFILE` on Windows.
pub const HOME_KEY: &str = hide_platform::host::HOME_VARIABLE;
pub const SSH_AUTH_SOCK_KEY: &str = "SSH_AUTH_SOCK";
pub const HERDR_SOCKET_PATH_KEY: &str = "HERDR_SOCKET_PATH";
pub const CODEX_HOME_KEY: &str = "CODEX_HOME";

pub const REGISTRY: [EnvironmentVariableSpec; 4] = [
    EnvironmentVariableSpec {
        key: HOME_KEY,
        required: false,
        format: "absolute home-directory path",
        absent_behavior: "Provider usage and home-relative integrations are unavailable",
    },
    EnvironmentVariableSpec {
        key: SSH_AUTH_SOCK_KEY,
        required: false,
        format: "absolute Unix-domain socket path",
        absent_behavior: "Devices still sign in with the none method, their key files (IdentityFile, or OpenSSH's default ~/.ssh keys) and an ssh config IdentityAgent; a device whose only usable key needs this agent says so on its row",
    },
    EnvironmentVariableSpec {
        key: HERDR_SOCKET_PATH_KEY,
        required: false,
        format: "absolute Unix-domain socket path",
        absent_behavior: "Use the default local Herdr socket path",
    },
    EnvironmentVariableSpec {
        key: CODEX_HOME_KEY,
        required: false,
        format: "absolute Codex home-directory path",
        absent_behavior: "Use $HOME/.codex for Codex usage credentials and sessions",
    },
];

#[derive(Clone, Debug)]
pub struct EnvironmentReport {
    pub statuses: Vec<EnvironmentStatusSnapshot>,
    pub home_path: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
}

impl EnvironmentReport {
    /// The report for an embedder that named its own home: everything the
    /// process `HOME` would have decided follows `home` instead, and the
    /// Codex home is the one under it, never the process's `CODEX_HOME`.
    pub fn with_home(mut self, home: PathBuf) -> Self {
        // The process's own home keeps every decision the environment made,
        // `CODEX_HOME` included; another home takes nothing from it.
        if self.home_path.as_ref() == Some(&home) {
            return self;
        }
        self.codex_home = Some(home.join(".codex"));
        self.home_path = Some(home);
        self
    }
}

pub fn read_and_validate() -> EnvironmentReport {
    validate_with(|key| std::env::var_os(key))
}

fn validate_with(mut read: impl FnMut(&str) -> Option<OsString>) -> EnvironmentReport {
    let home = read(HOME_KEY);
    let mut statuses = Vec::with_capacity(REGISTRY.len());
    let mut home_path = None;
    let mut codex_home = None;

    for spec in REGISTRY {
        let value = read(spec.key);
        let (state, message) = match spec.key {
            HOME_KEY => match home.as_ref() {
                None => (
                    "absent",
                    "Home directory is unavailable; provider usage and home-relative integrations are disabled",
                ),
                Some(value) if value.is_empty() || !Path::new(value).is_absolute() => (
                    "invalid",
                    "Home directory configuration is invalid; provider usage and home-relative integrations are disabled",
                ),
                Some(value) => {
                    home_path = Some(PathBuf::from(value));
                    ("available", "Home directory configuration is available")
                }
            },
            // Each device authenticates as its own ssh config says, so a
            // missing agent only stops the devices that would use it, and
            // those report it on their own rows.
            SSH_AUTH_SOCK_KEY => match value {
                None => (
                    "absent",
                    "SSH agent socket is unavailable; devices sign in with their IdentityFile or default ~/.ssh keys, or an ssh config IdentityAgent",
                ),
                Some(value) if value.is_empty() || !Path::new(&value).is_absolute() => (
                    "invalid",
                    "SSH agent socket configuration is invalid; devices sign in with their IdentityFile or default ~/.ssh keys, or an ssh config IdentityAgent",
                ),
                Some(_) => ("available", "SSH agent socket configuration is available"),
            },
            HERDR_SOCKET_PATH_KEY => match value {
                None => (
                    "default",
                    "Herdr socket override is absent; the default local socket is used",
                ),
                Some(value) if value.is_empty() || !Path::new(&value).is_absolute() => (
                    "invalid",
                    "Herdr socket override is invalid; the configured default remains in use",
                ),
                Some(_) => ("available", "Herdr socket override is available"),
            },
            CODEX_HOME_KEY => match value {
                None => {
                    codex_home = home_path.as_ref().map(|home: &PathBuf| home.join(".codex"));
                    (
                        "default",
                        "Codex home override is absent; $HOME/.codex is used",
                    )
                }
                Some(value) if value.is_empty() || !Path::new(&value).is_absolute() => (
                    "invalid",
                    "Codex home override is invalid; Codex usage credentials are unavailable",
                ),
                Some(value) => {
                    codex_home = Some(PathBuf::from(value));
                    ("available", "Codex home override is available")
                }
            },
            _ => unreachable!("every environment key is declared in REGISTRY"),
        };
        statuses.push(EnvironmentStatusSnapshot {
            key: spec.key.to_owned(),
            required: spec.required,
            format: spec.format.to_owned(),
            state: state.to_owned(),
            absent_behavior: spec.absent_behavior.to_owned(),
            message: message.to_owned(),
        });
    }

    EnvironmentReport {
        statuses,
        home_path,
        codex_home,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn registry_is_enumerable_and_does_not_expose_values() {
        assert_eq!(REGISTRY.len(), 4);
        assert_eq!(REGISTRY[0].key, hide_platform::host::HOME_VARIABLE);
        let secret_like_value = OsString::from("/private/tmp/private-agent.sock");
        let report = validate_with(|key| match key {
            SSH_AUTH_SOCK_KEY | HERDR_SOCKET_PATH_KEY => Some(secret_like_value.clone()),
            _ => None,
        });
        let encoded = serde_json::to_string(&report.statuses).unwrap();
        assert!(!encoded.contains("private-agent.sock"));
    }

    #[test]
    fn an_absent_agent_socket_is_reported_without_disabling_devices() {
        let report = validate_with(|_| None);
        assert_eq!(report.statuses[1].state, "absent");
        assert!(!report.statuses[1].required);
        assert!(report.statuses[1].message.contains("IdentityFile"));
        assert_eq!(report.statuses[2].state, "default");
        assert!(report.home_path.is_none());
        assert!(report.codex_home.is_none());
    }

    #[test]
    fn every_registered_key_has_a_distinct_contract() {
        let contracts = REGISTRY
            .iter()
            .map(|spec| (spec.key, (spec.format, spec.absent_behavior)))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(contracts.len(), REGISTRY.len());
        assert!(contracts.values().all(|(format, behavior)| {
            !format.trim().is_empty() && !behavior.trim().is_empty()
        }));
    }

    #[test]
    fn another_home_takes_nothing_of_the_process_codex_home() {
        let report = validate_with(|key| match key {
            HOME_KEY => Some(OsString::from("/Users/example")),
            CODEX_HOME_KEY => Some(OsString::from("/Users/example/custom-codex")),
            _ => None,
        });
        let same = report.clone().with_home(PathBuf::from("/Users/example"));
        assert_eq!(
            same.codex_home.as_deref(),
            Some(Path::new("/Users/example/custom-codex"))
        );
        let private = report.with_home(PathBuf::from("/private/tmp/test-home"));
        assert_eq!(
            private.home_path.as_deref(),
            Some(Path::new("/private/tmp/test-home"))
        );
        assert_eq!(
            private.codex_home.as_deref(),
            Some(Path::new("/private/tmp/test-home/.codex"))
        );
    }

    #[test]
    fn socket_override_is_reported_alongside_the_ssh_agent_socket() {
        let report = validate_with(|key| match key {
            SSH_AUTH_SOCK_KEY => Some(OsString::from("/private/tmp/agent.sock")),
            HERDR_SOCKET_PATH_KEY => Some(OsString::from("/private/tmp/herdr.sock")),
            _ => None,
        });

        assert_eq!(report.statuses[1].state, "available");
        assert_eq!(report.statuses[2].state, "available");
    }

    #[test]
    fn invalid_provider_overrides_do_not_fall_back_to_home() {
        let report = validate_with(|key| match key {
            HOME_KEY => Some(OsString::from("/private/tmp/hide-home")),
            CODEX_HOME_KEY => Some(OsString::from("relative-codex")),
            _ => None,
        });

        assert_eq!(
            report.home_path,
            Some(PathBuf::from("/private/tmp/hide-home"))
        );
        assert!(report.codex_home.is_none());
        assert_eq!(report.statuses[3].state, "invalid");
    }
}
