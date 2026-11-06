//! Which agent versions' composers read as their declaration says, kept
//! across a restart.
//!
//! The doorbell reads a composer only for a version that passed its check:
//! at a moment the composer is known empty (the turn a prompt hook started
//! has ended and no key came since), the screen is read and judged by the
//! agent's declaration (`hide_agent_adapter::composer`). One screen read
//! empty passes the version, and letters held on a possible draft may then
//! be released when the screen says the composer is empty. Any other screen
//! is a miss, not a verdict on the version: one pane may show a dialog, a
//! named session's rule or anything else the declaration does not place, so
//! the version holds as it did before and is checked again at a later
//! known-empty moment, each miss doubling the wait (`docs/delivery.md`,
//! Catching an agent's screen change).

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::latest_file::{LatestFile, read_capped};

// A file without `misses` is one an earlier build wrote, whose failed check
// was final; it reads as unconfirmed with no misses, checked at once.
const SCHEMA_VERSION: u32 = 1;
/// Versions remembered at once; the oldest check goes first. An agent
/// updates every few days, so this is months of every agent's versions.
pub(crate) const CHECK_LIMIT: usize = 128;
const READ_CAP: u64 = 256 * 1024;
/// The wait after a version's first miss before it is checked again; each
/// later miss doubles it, up to `RETRY_MAX_MS`.
pub(crate) const RETRY_FIRST_MS: u64 = 60_000;
/// The longest wait between checks of a version that keeps missing: one
/// screen read an hour is all a version whose screen did change costs.
pub(crate) const RETRY_MAX_MS: u64 = 60 * 60 * 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Check {
    /// The adapter id (`hide_agent_adapter::AgentAdapter::id`).
    pub agent: String,
    pub version: String,
    /// Whether a screen read empty when the composer was known to be. Once
    /// passed, the version is not checked again.
    pub passed: bool,
    /// The last check.
    pub checked_at_unix_ms: u64,
    /// Checks whose screen did not read empty at a known-empty moment.
    #[serde(default)]
    pub misses: u32,
    /// The step that stopped the last miss (`composer::Miss::code`), never
    /// the screen's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_miss: Option<String>,
}

impl Check {
    /// Whether the version is to be checked at a known-empty moment `now`:
    /// it has not passed, and the wait its misses earned is over.
    pub(crate) fn due(&self, now: u64) -> bool {
        !self.passed
            && now
                >= self
                    .checked_at_unix_ms
                    .saturating_add(retry_after(self.misses))
    }
}

/// How long a version waits after its last check: nothing before its first
/// miss, then `RETRY_FIRST_MS` doubled for each later miss, at most
/// `RETRY_MAX_MS`.
pub(crate) fn retry_after(misses: u32) -> u64 {
    match misses {
        0 => 0,
        misses => RETRY_FIRST_MS
            .saturating_mul(1 << (misses - 1).min(16))
            .min(RETRY_MAX_MS),
    }
}

#[derive(Deserialize, Serialize)]
struct Stored {
    schema_version: u32,
    checks: Vec<Check>,
}

fn encode(checks: &Arc<[Check]>) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&Stored {
        schema_version: SCHEMA_VERSION,
        checks: checks.to_vec(),
    })
    .map_err(|error| format!("could not be encoded: {error}"))
}

fn decode(bytes: &[u8]) -> Result<Vec<Check>, String> {
    let stored: Stored =
        serde_json::from_slice(bytes).map_err(|error| format!("could not be decoded: {error}"))?;
    if stored.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "has schema version {}, expected {SCHEMA_VERSION}",
            stored.schema_version
        ));
    }
    if stored.checks.len() > CHECK_LIMIT {
        return Err(format!(
            "holds {} checks, past {CHECK_LIMIT}",
            stored.checks.len()
        ));
    }
    if !stored.checks.iter().all(|check| {
        hide_agent_adapter::adapter(&check.agent).is_some_and(|row| row.id == check.agent)
            && valid_version(&check.version)
            && check
                .last_miss
                .as_deref()
                .is_none_or(|code| hide_agent_adapter::composer::Miss::from_code(code).is_some())
    }) {
        return Err("holds a check that names no agent version".to_owned());
    }
    Ok(stored.checks)
}

/// A version as `--version` prints it after parsing: a short run of digits
/// and dots.
pub(crate) fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 32
        && version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
}

/// Records a check of `agent`'s `version` at `at`: passed, or missed at the
/// step `miss` names, counted on the misses before it. The oldest check is
/// dropped past `CHECK_LIMIT`. Returns the version's record.
pub(crate) fn record(
    checks: &mut Vec<Check>,
    agent: &str,
    version: &str,
    miss: Option<&'static str>,
    at: u64,
) -> Check {
    let earlier = checks
        .iter()
        .position(|check| check.agent == agent && check.version == version)
        .map(|index| checks.remove(index));
    let misses = earlier.as_ref().map_or(0, |check| check.misses);
    let check = Check {
        agent: agent.to_owned(),
        version: version.to_owned(),
        passed: miss.is_none(),
        checked_at_unix_ms: at,
        misses: misses.saturating_add(u32::from(miss.is_some())),
        last_miss: miss
            .map(str::to_owned)
            .or_else(|| earlier.and_then(|check| check.last_miss)),
    };
    checks.push(check.clone());
    if checks.len() > CHECK_LIMIT {
        checks.sort_by_key(|check| std::cmp::Reverse(check.checked_at_unix_ms));
        checks.truncate(CHECK_LIMIT);
    }
    check
}

pub(crate) struct ComposerChecks {
    path: PathBuf,
    file: LatestFile<Arc<[Check]>>,
}

impl ComposerChecks {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            file: LatestFile::new(
                path.clone(),
                "hide-composer-checks-save",
                "delivery",
                "composer_checks",
                encode,
            ),
            path,
        }
    }

    /// The versions the last runs checked. A missing file is the first run;
    /// any other reason a file is not used is stated, then the file is
    /// ignored, every version is checked again, and the next save replaces it.
    pub(crate) fn restore(&self) -> Vec<Check> {
        let discard = |reason: String| {
            crate::diagnostic!(serde_json::json!({
                "component": "delivery",
                "kind": "composer_checks.discarded",
                "path": self.path.to_string_lossy(),
                "message": reason,
            }));
            Vec::new()
        };
        match read_capped(&self.path, READ_CAP) {
            Ok(None) => Vec::new(),
            Ok(Some(bytes)) => decode(&bytes).unwrap_or_else(discard),
            Err(reason) => discard(reason),
        }
    }

    /// Queues `checks` to replace the file; only the newest of a burst is
    /// written.
    pub(crate) fn save(&self, checks: Vec<Check>) {
        self.file.save(checks.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(version: &str, passed: bool, at: u64) -> Check {
        Check {
            agent: "claude-code".into(),
            version: version.into(),
            passed,
            checked_at_unix_ms: at,
            misses: u32::from(!passed),
            last_miss: (!passed).then(|| "no_top_rule".into()),
        }
    }

    #[test]
    fn saved_checks_come_back() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("composer-checks.json");
        {
            let store = ComposerChecks::new(path.clone());
            store.save(vec![check("2.1.306", true, 10)]);
            store.save(vec![
                check("2.1.306", true, 10),
                check("2.1.307", false, 20),
            ]);
        }
        let restored = ComposerChecks::new(path).restore();
        assert_eq!(
            restored,
            vec![check("2.1.306", true, 10), check("2.1.307", false, 20)]
        );
    }

    #[test]
    fn a_missing_damaged_other_version_or_foreign_file_restores_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("composer-checks.json");
        let store = ComposerChecks::new(path.clone());
        assert!(store.restore().is_empty());
        for bytes in [
            &b"not json"[..],
            br#"{"schema_version":2,"checks":[]}"#,
            br#"{"schema_version":1,"checks":[{"agent":"claude","version":"1.0.0","passed":true,"checked_at_unix_ms":1}]}"#,
            br#"{"schema_version":1,"checks":[{"agent":"codex","version":"1.0.0; rm","passed":true,"checked_at_unix_ms":1}]}"#,
            br#"{"schema_version":1,"checks":[{"agent":"codex","version":"1.0.0","passed":false,"checked_at_unix_ms":1,"misses":1,"last_miss":"screen text"}]}"#,
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(store.restore().is_empty());
        }
    }

    #[test]
    fn a_new_check_replaces_its_version_and_the_oldest_leaves_past_the_cap() {
        let mut checks = Vec::new();
        for at in 0..CHECK_LIMIT as u64 {
            record(&mut checks, "claude-code", &format!("1.0.{at}"), None, at);
        }
        record(
            &mut checks,
            "claude-code",
            "1.0.5",
            Some("no_top_rule"),
            1_000,
        );
        assert_eq!(checks.len(), CHECK_LIMIT);
        assert!(checks.contains(&check("1.0.5", false, 1_000)));
        record(&mut checks, "claude-code", "2.0.0", None, 2_000);
        assert_eq!(checks.len(), CHECK_LIMIT);
        assert!(!checks.iter().any(|check| check.version == "1.0.0"));
    }

    /// One odd screen does not decide a version: each miss is counted and
    /// waits longer before the next check, and a screen read empty passes
    /// the version for good.
    #[test]
    fn misses_wait_longer_each_time_until_a_screen_reads_empty() {
        let mut checks = Vec::new();
        let first = record(
            &mut checks,
            "claude-code",
            "2.1.314",
            Some("no_top_rule"),
            0,
        );
        assert_eq!((first.passed, first.misses), (false, 1));
        assert!(!first.due(RETRY_FIRST_MS - 1));
        assert!(first.due(RETRY_FIRST_MS));
        let second = record(
            &mut checks,
            "claude-code",
            "2.1.314",
            Some("placeholder_style"),
            RETRY_FIRST_MS,
        );
        assert_eq!(second.misses, 2);
        assert_eq!(second.last_miss.as_deref(), Some("placeholder_style"));
        assert!(!second.due(3 * RETRY_FIRST_MS - 1));
        assert!(second.due(3 * RETRY_FIRST_MS));
        assert_eq!(retry_after(40), RETRY_MAX_MS);
        let passed = record(
            &mut checks,
            "claude-code",
            "2.1.314",
            None,
            10 * RETRY_MAX_MS,
        );
        assert!(passed.passed && !passed.due(u64::MAX));
        assert_eq!(passed.misses, 2, "the misses stay for diagnosis");
        assert_eq!(checks.len(), 1);
    }

    /// An earlier build's failed check was final; this build reads it as a
    /// version not yet confirmed and checks it at the next moment.
    #[test]
    fn an_earlier_builds_failed_check_is_checked_again() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("composer-checks.json");
        std::fs::write(
            &path,
            br#"{"schema_version":1,"checks":[{"agent":"claude-code","version":"2.1.314","passed":false,"checked_at_unix_ms":5}]}"#,
        )
        .unwrap();
        let restored = ComposerChecks::new(path).restore();
        assert_eq!(restored.len(), 1);
        assert_eq!(
            (restored[0].misses, restored[0].last_miss.clone()),
            (0, None)
        );
        assert!(restored[0].due(5));
    }
}
