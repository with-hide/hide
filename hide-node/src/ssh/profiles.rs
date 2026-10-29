//! The saved-machine CLI is client configuration, not a connection probe.
//! One caller owns one bounded CLI child; config resolution never runs SSH.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_node_link::device::{HerdrProfile, HerdrProfileListing};
use hide_platform::process::{RUN_OUTPUT_CAP, restrict_to_login_environment, run_to_end};

const MAX_PROFILES: usize = 32;
const MAX_REGISTERED: usize = 64;
const MAX_FIELD_BYTES: usize = 2048;
const DISCOVERY_DEADLINE: Duration = Duration::from_secs(5);

pub(super) fn list(
    program: &Path,
    resolve: &super::hosts::Resolve,
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
    let targets: Vec<String> = listing
        .profiles
        .iter()
        .map(|p| p.target.as_str())
        .chain(registered.iter().take(MAX_REGISTERED).map(String::as_str))
        .collect::<BTreeSet<&str>>()
        .into_iter()
        .map(str::to_owned)
        .collect();
    listing.endpoints = endpoints(&targets, stop, |target| resolve(home, target, stop));
    Ok(listing)
}

/// Where each saved target leads, as `user@host:port`, by what `ssh -G`
/// makes of it: OpenSSH reads an alias, `user@host` and an `ssh://` address
/// itself, the account's configuration included.
fn endpoints(
    targets: &[String],
    stop: &AtomicBool,
    resolve: impl Fn(
        &str,
    ) -> Result<
        hide_node_link::device::SshAddress,
        hide_node_link::device::SshHostProblem,
    > + Sync,
) -> BTreeMap<String, Result<String, String>> {
    let unresolved = || {
        "The saved SSH target could not be resolved. Check SSH configuration and retry".to_owned()
    };
    let resolved = super::hosts::resolve_all(targets, stop, |target| {
        Target::parse(target).map_err(|_| hide_node_link::device::SshHostProblem::SshFailed)?;
        resolve(target)
    });
    targets
        .iter()
        .cloned()
        .zip(resolved)
        .map(|(target, address)| {
            let endpoint = address
                .map(|address| {
                    format!(
                        "{}@{}:{}",
                        address.user,
                        address.host.to_ascii_lowercase(),
                        address.port
                    )
                })
                .map_err(|_| unresolved());
            (target, endpoint)
        })
        .collect()
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

/// OpenSSH destination forms: an alias or host, `user@host`, or an
/// `ssh://` address. A literal hostname never requires a Host row, and
/// nothing that `ssh` would read as an option is one.
pub(super) struct Target;

impl Target {
    pub fn parse(target: &str) -> Result<(), String> {
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
        let address = match destination.rsplit_once('@') {
            Some((user, address)) if !user.is_empty() && !user.contains(['@', '/', ':']) => address,
            Some(_) => return Err(bad()),
            None => destination,
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A saved target is resolved by OpenSSH as it is written, and one that
    /// is no destination is never handed to `ssh`.
    #[test]
    fn saved_targets_resolve_as_ssh_reads_them() {
        let targets = ["ssh://grab@box:2222", "-oProxyCommand=x", "gone"].map(str::to_owned);
        let asked = std::sync::Mutex::new(Vec::new());
        let endpoints = endpoints(&targets, &AtomicBool::new(false), |target| {
            asked.lock().unwrap().push(target.to_owned());
            match target {
                "ssh://grab@box:2222" => Ok(hide_node_link::device::SshAddress {
                    user: "grab".to_owned(),
                    host: "BOX.example".to_owned(),
                    port: 2222,
                }),
                _ => Err(hide_node_link::device::SshHostProblem::SshFailed),
            }
        });
        assert_eq!(
            endpoints["ssh://grab@box:2222"].as_deref(),
            Ok("grab@box.example:2222")
        );
        assert!(endpoints["-oProxyCommand=x"].is_err());
        assert!(endpoints["gone"].is_err());
        assert!(
            !asked
                .lock()
                .unwrap()
                .iter()
                .any(|target| target.starts_with('-'))
        );
        assert!(Target::parse("ssh://u@host:0").is_err());
        assert!(validate_session("../default").is_err());
        assert!(validate_session("").is_err());
    }
}
