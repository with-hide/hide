//! Which agent versions' composers read as their declaration says, kept
//! across a restart.
//!
//! The doorbell reads a composer only for a version it has checked: the
//! first time Hide sees a version, at a moment the composer is known empty
//! (the turn a prompt hook started has ended and no key came since), the
//! screen is read once and judged by the agent's declaration
//! (`hide_agent_adapter::composer`). A pass lets letters held on a possible
//! draft be released when the screen says the composer is empty; a fail
//! means the agent's screen changed under the declaration, and that version
//! keeps holding as it did before (`docs/delivery.md`, Reading the composer).

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::latest_file::{LatestFile, read_capped};

const SCHEMA_VERSION: u32 = 1;
/// Versions remembered at once; the oldest check goes first. An agent
/// updates every few days, so this is months of every agent's versions.
pub(crate) const CHECK_LIMIT: usize = 128;
const READ_CAP: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Check {
    /// The adapter id (`hide_agent_adapter::AgentAdapter::id`).
    pub agent: String,
    pub version: String,
    /// Whether the screen read empty when the composer was known to be.
    pub passed: bool,
    pub checked_at_unix_ms: u64,
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

/// The checks with `check` in place of any earlier one of its agent and
/// version, the oldest dropped past `CHECK_LIMIT`.
pub(crate) fn record(checks: &mut Vec<Check>, check: Check) {
    checks.retain(|old| old.agent != check.agent || old.version != check.version);
    checks.push(check);
    if checks.len() > CHECK_LIMIT {
        checks.sort_by_key(|check| std::cmp::Reverse(check.checked_at_unix_ms));
        checks.truncate(CHECK_LIMIT);
    }
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
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(store.restore().is_empty());
        }
    }

    #[test]
    fn a_new_check_replaces_its_version_and_the_oldest_leaves_past_the_cap() {
        let mut checks = Vec::new();
        for at in 0..CHECK_LIMIT as u64 {
            record(&mut checks, check(&format!("1.0.{at}"), true, at));
        }
        record(&mut checks, check("1.0.5", false, 1_000));
        assert_eq!(checks.len(), CHECK_LIMIT);
        assert!(checks.contains(&check("1.0.5", false, 1_000)));
        record(&mut checks, check("2.0.0", true, 2_000));
        assert_eq!(checks.len(), CHECK_LIMIT);
        assert!(!checks.iter().any(|check| check.version == "1.0.0"));
    }
}
