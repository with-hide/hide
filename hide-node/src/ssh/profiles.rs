//! The saved-machine CLI is client configuration, not a connection probe.
//! One caller owns one bounded CLI child; config resolution never runs SSH.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_node_link::device::{HerdrProfile, HerdrProfileListing};
use hide_platform::process::{RUN_OUTPUT_CAP, restrict_to_login_environment, run_to_end};

const MAX_PROFILES: usize = 32;
const MAX_REGISTERED: usize = 64;
const MAX_FIELD_BYTES: usize = 2048;
const DISCOVERY_DEADLINE: Duration = Duration::from_secs(5);
const MAX_CONFIG_FILES: usize = 16;
const MAX_CONFIG_BYTES: usize = 256 * 1024;

pub(super) fn list(
    program: &Path,
    home: &Path,
    registered: &[String],
    stop: &AtomicBool,
) -> Result<HerdrProfileListing, String> {
    if registered.len() > MAX_REGISTERED {
        return Err("There are too many devices to compare saved machine routes safely".to_owned());
    }
    let mut command = hide_platform::programs::command(program);
    restrict_to_login_environment(&mut command);
    command
        .env(hide_platform::host::HOME_VARIABLE, home)
        .args(["machine", "list", "--json"]);
    if std::env::var_os(hide_platform::host::HOME_VARIABLE).as_deref() == Some(home.as_os_str()) {
        if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
            command.env("XDG_STATE_HOME", state);
        }
    } else {
        // A fixture or an explicitly different account must never inherit
        // the operator's platform state directory (including LOCALAPPDATA).
        command.env("XDG_STATE_HOME", home.join(".local/state"));
    }
    let finished = run_to_end(&mut command, DISCOVERY_DEADLINE, stop)
        .map_err(|_| "Could not read saved Herdr machines. Check Herdr and retry".to_owned())?;
    if !finished.succeeded() {
        return Err("Herdr could not list its saved machines. Check Herdr and retry".to_owned());
    }
    let mut listing = parse(&finished.stdout)?;
    let config = ssh_config(&home.join(".ssh/config"));
    let targets: BTreeSet<&str> = listing
        .profiles
        .iter()
        .map(|p| p.target.as_str())
        .chain(registered.iter().take(MAX_REGISTERED).map(String::as_str))
        .collect();
    listing.endpoints = targets
        .into_iter()
        .map(|target| {
            let endpoint = config
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|contents| {
                    super::SshAlias::from_target_contents(
                        target,
                        contents,
                        home.join(".ssh/known_hosts"),
                    )
                    .map_err(|_| "The saved SSH target could not be resolved".to_owned())
                })
                .map(|alias| {
                    format!(
                        "{}@{}:{}",
                        alias.user,
                        alias.hostname.to_ascii_lowercase(),
                        alias.port
                    )
                })
                .map_err(|_| {
                    "The saved SSH target could not be resolved. Check SSH configuration and retry"
                        .to_owned()
                });
            (target.to_owned(), endpoint)
        })
        .collect();
    Ok(listing)
}

fn parse(output: &str) -> Result<HerdrProfileListing, String> {
    if output.len() >= RUN_OUTPUT_CAP {
        return Err("Herdr's saved machine list is too large. Reduce it and retry".to_owned());
    }
    let mut profiles: Vec<HerdrProfile> = serde_json::from_str(output).map_err(|_| {
        "Herdr returned an unreadable saved machine list. Check Herdr and retry".to_owned()
    })?;
    let mut ids = BTreeSet::new();
    for profile in &profiles {
        if [
            &profile.id,
            &profile.label,
            &profile.target,
            &profile.session,
        ]
        .iter()
        .any(|v| v.is_empty() || v.len() > MAX_FIELD_BYTES || v.chars().any(char::is_control))
            || !ids.insert(profile.id.clone())
        {
            return Err(
                "Herdr returned an invalid saved machine profile. Check Herdr and retry".to_owned(),
            );
        }
    }
    let truncated = profiles.len() > MAX_PROFILES;
    profiles.truncate(MAX_PROFILES);
    Ok(HerdrProfileListing {
        profiles,
        endpoints: BTreeMap::new(),
        truncated,
    })
}

pub(super) fn validate_session(session: &str) -> Result<(), String> {
    if !hide_node_link::device::valid_herdr_session(session) {
        return Err("The saved Herdr session name is invalid. Fix the profile in Herdr".to_owned());
    }
    Ok(())
}

/// OpenSSH destination forms. A literal hostname never requires a Host row.
pub(super) struct Target {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
}

impl Target {
    pub fn parse(target: &str) -> Result<Self, String> {
        let bad = || "The saved SSH target is not a supported SSH destination".to_owned();
        if target.is_empty()
            || target.len() > MAX_FIELD_BYTES
            || target.starts_with('-')
            || target.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(bad());
        }
        let uri = target.starts_with("ssh://");
        let destination = target.strip_prefix("ssh://").unwrap_or(target);
        let (user, address) = match destination.rsplit_once('@') {
            Some((user, address)) if !user.is_empty() && !user.contains(['@', '/', ':']) => {
                (Some(user.to_owned()), address)
            }
            Some(_) => return Err(bad()),
            None => (None, destination),
        };
        let (host, port) = if let Some(bracketed) = address.strip_prefix('[') {
            let (host, rest) = bracketed.split_once(']').ok_or_else(bad)?;
            let port = if rest.is_empty() {
                None
            } else {
                Some(
                    rest.strip_prefix(':')
                        .ok_or_else(bad)?
                        .parse::<u16>()
                        .map_err(|_| bad())?,
                )
            };
            (host, port)
        } else if uri {
            match address.rsplit_once(':') {
                Some((host, port)) => (host, Some(port.parse::<u16>().map_err(|_| bad())?)),
                None => (address, None),
            }
        } else {
            (address, None)
        };
        if host.is_empty()
            || host.starts_with('-')
            || host.contains(['/', '@', '[', ']'])
            || port == Some(0)
        {
            return Err(bad());
        }
        Ok(Self {
            host: host.to_owned(),
            user,
            port,
        })
    }
}

/// Flatten Include in-place with the same count/byte/depth caps as Add device.
/// An invalid or unreadable config is a failure, not a different endpoint.
pub(super) fn ssh_config(path: &Path) -> Result<String, String> {
    fn read(
        path: &Path,
        home: &Path,
        count: &mut usize,
        depth: usize,
        output: &mut String,
    ) -> Result<(), String> {
        if *count >= MAX_CONFIG_FILES || depth > 4 {
            return Err("SSH configuration exceeds the include limit".to_owned());
        }
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("SSH configuration could not be read: {error}")),
        };
        let mut contents = String::new();
        file.take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_string(&mut contents)
            .map_err(|error| error.to_string())?;
        if contents.len() > MAX_CONFIG_BYTES {
            return Err("SSH configuration exceeds the file size limit".to_owned());
        }
        *count += 1;
        for line in contents.lines() {
            let Some((key, arguments)) = super::hosts::split_line(line) else {
                output.push('\n');
                continue;
            };
            if key.eq_ignore_ascii_case("include") {
                for argument in arguments {
                    let reader = super::hosts::Reader {
                        home,
                        files: 0,
                        found: Default::default(),
                    };
                    for included in reader.expand_include(&argument) {
                        read(&included, home, count, depth + 1, output)?;
                    }
                }
            } else if key.eq_ignore_ascii_case("match") {
                return Err(
                    "SSH Match rules cannot be resolved safely for a saved target".to_owned(),
                );
            } else {
                output.push_str(line);
                output.push('\n');
            }
        }
        Ok(())
    }
    let home = path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "SSH configuration has no account home".to_owned())?;
    let mut output = String::new();
    read(path, home, &mut 0, 0, &mut output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn saved_configuration_preserves_target_session_and_disabled_profiles() {
        let listing = parse(r#"[{"id":"a","label":"Shared","target":"grab@box","session":"review","enabled":false},{"id":"b","label":"Shared","target":"box","session":"default","enabled":true}]"#).unwrap();
        assert_eq!(listing.profiles[0].target, "grab@box");
        assert_eq!(listing.profiles[0].session, "review");
        assert!(!listing.profiles[0].enabled);
        assert_eq!(listing.profiles.len(), 2);
        assert!(parse("{}").is_err());
        assert!(parse(r#"[{"id":"a","label":"A","target":"box","enabled":true}]"#).is_err());
    }

    #[test]
    fn arbitrary_targets_resolve_without_a_config_and_explicit_values_win() {
        let known = PathBuf::from("/private/known_hosts");
        let alias = super::super::SshAlias::from_target_contents(
            "ssh://grab@box:2222",
            "Host box\n User other\n Port 22\n HostName BOX.example\n",
            known.clone(),
        )
        .unwrap();
        assert_eq!(
            (alias.user.as_str(), alias.hostname.as_str(), alias.port),
            ("grab", "BOX.example", 2222)
        );
        let plain =
            super::super::SshAlias::from_target_contents("grab@10.0.0.2", "", known).unwrap();
        assert_eq!(
            (plain.user.as_str(), plain.hostname.as_str(), plain.port),
            ("grab", "10.0.0.2", 22)
        );
        assert!(Target::parse("-oProxyCommand=x").is_err());
        assert!(Target::parse("ssh://u@host:0").is_err());
    }

    #[test]
    fn named_session_status_never_uses_an_inherited_or_default_route() {
        let command = super::super::RemoteReadCommand::HerdrSessionStatus {
            session: "review".to_owned(),
            helper_root: None,
        }
        .command_line()
        .unwrap();
        assert!(command.contains("herdr --session 'review' status server --json"));
        assert!(!command.contains("server start"));
        assert!(validate_session("../default").is_err());
        assert!(validate_session("").is_err());
    }
}
