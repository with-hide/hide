//! Herdr's own integration for an agent, installed and removed through the
//! machine's Herdr CLI (PRD settings-cleanup D-13).
//!
//! Herdr learns an agent's session id, and for some agents its lifecycle,
//! from a hook script or plugin that `herdr integration install <agent>`
//! puts in the agent's configuration. Lineage, the mailbox identity, labels
//! and sleep all need that session id, so the kit installs it when the
//! operator switches the agent on, on this Mac with the bundled Herdr and on a
//! device with its own.
//!
//! Hide only ever takes out what it put there: the kit's record names the
//! integrations it installed (`herdr:<agent>`), and an integration that was
//! already in place when Hide first looked is the operator's and stays, on and
//! off. Every call is one owned child with a deadline and only the
//! account's login variables and `HOME` (`crate::process::run`), so a daemon
//! started from a Herdr pane never aims it at that pane's server. Only the
//! kit pass calls it, never anything holding the runtime lock.
//!
//! The CLI reports every target in one `status` listing, one
//! `<target>: <state> ... (<path>)` line each, with the state `not installed`,
//! `current (vN)` or `outdated (vA < vB)`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::KitTarget;

/// How long one Herdr call may take; each answers in well under a second.
const DEADLINE: Duration = Duration::from_secs(15);

/// What Herdr reports of one integration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Integration {
    Current,
    /// An older version of Herdr's script or plugin is there.
    Outdated,
    Missing,
}

/// The integrations of one machine, read with one `herdr integration status`.
pub(crate) struct Statuses(BTreeMap<String, Integration>);

impl Statuses {
    /// Reads the machine's integrations; the error is the reason the row
    /// shows, or `None` for a machine with no Herdr CLI Hide can find.
    pub(crate) fn probe(target: &KitTarget) -> Option<Result<Self, String>> {
        let herdr = target.herdr_bin.as_deref()?;
        Some(
            run(herdr, target, &["integration", "status"])
                .and_then(|stdout| parse(&stdout))
                .map(Self),
        )
    }

    /// `None` for a target this Herdr does not list.
    pub(crate) fn of(&self, name: &str) -> Option<Integration> {
        self.0.get(name).copied()
    }
}

fn parse(stdout: &str) -> Result<BTreeMap<String, Integration>, String> {
    let mut found = BTreeMap::new();
    for line in stdout.lines() {
        // `letta (experimental): not installed (...)`: the name is the first
        // word, the state follows the colon.
        let Some((name, state)) = line.split_once(':') else {
            continue;
        };
        let Some(name) = name.split_whitespace().next() else {
            continue;
        };
        let state = state.trim_start();
        let integration = if state.starts_with("not installed") {
            Integration::Missing
        } else if state.starts_with("current") {
            Integration::Current
        } else if state.starts_with("outdated") {
            Integration::Outdated
        } else {
            continue;
        };
        found.insert(name.to_owned(), integration);
    }
    if found.is_empty() {
        return Err("`herdr integration status` listed no integration Hide can read".to_owned());
    }
    Ok(found)
}

pub(crate) fn install(target: &KitTarget, name: &str) -> Result<(), String> {
    change(target, "install", name)
}

pub(crate) fn uninstall(target: &KitTarget, name: &str) -> Result<(), String> {
    change(target, "uninstall", name)
}

fn change(target: &KitTarget, verb: &str, name: &str) -> Result<(), String> {
    let herdr = target
        .herdr_bin
        .as_deref()
        .ok_or_else(|| "Herdr is not found on this machine".to_owned())?;
    run(herdr, target, &["integration", verb, name]).map(drop)
}

/// Runs the Herdr CLI and answers its stdout, or why it failed in a
/// sentence: its own last line of error when it printed one.
fn run(herdr: &Path, target: &KitTarget, args: &[&str]) -> Result<String, String> {
    let finished = crate::process::run(herdr, args, &[], &target.home, DEADLINE, &target.stop)?;
    if finished.succeeded() {
        return Ok(finished.stdout);
    }
    let line = finished.last_error_line();
    let what = args.join(" ");
    Err(if line.is_empty() {
        format!("`herdr {what}` failed")
    } else {
        format!("`herdr {what}` failed: {line}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listing_is_read_by_target_name_and_state() {
        let listing = "\
pi: current (v9) (/h/.pi/agent/extensions/herdr-agent-state.ts)
claude: outdated (v3 < v10) (/h/.claude/hooks/herdr-agent-state.sh)
codex: not installed (/h/.codex/herdr-agent-state.sh)
letta (experimental): not installed (/h/.letta/hooks/herdr-agent-session.sh)
";
        let found = parse(listing).unwrap();
        assert_eq!(found["pi"], Integration::Current);
        assert_eq!(found["claude"], Integration::Outdated);
        assert_eq!(found["codex"], Integration::Missing);
        assert_eq!(found["letta"], Integration::Missing);
        assert!(parse("herdr 0.9.1\n").is_err());
    }
}
