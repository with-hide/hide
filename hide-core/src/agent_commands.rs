//! Agent-owned command lists, shared by Desk and pane conversation inputs.
use crate::latest_file::{LatestFile, read_capped};
use hide_desk::AdapterId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct AgentCommands {
    pub(crate) lists: CommandLists,
    pub(crate) read_marks: ReadMarks,
    pub(crate) store: Option<AgentCommandsStore>,
}

/// Each agent's last command list, by agent.
pub(crate) type CommandLists =
    std::collections::BTreeMap<AdapterId, Vec<hide_desk::commands::Command>>;

/// When Hide last read an agent's list ahead of the operator
/// (`hide_desk::read_ahead`), and the digest of what that list depended on
/// then ([`hide_desk::read_ahead::mark`]): a list is read again when the
/// digest moves.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReadMark {
    pub mark: String,
    pub at_ms: u64,
}

impl ReadMark {
    /// The mark of a read made now of a list that depended on `facts`.
    pub(crate) fn of(facts: &str, at_ms: u64) -> Self {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(facts.as_bytes());
        Self {
            mark: digest[..16]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            at_ms,
        }
    }
}

/// Each agent's last read ahead, by agent.
pub(crate) type ReadMarks = std::collections::BTreeMap<AdapterId, ReadMark>;

/// What `agent-commands.json` keeps: each agent's last list, and when each
/// list read ahead was read.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SavedCommands {
    pub lists: CommandLists,
    pub read: ReadMarks,
}

const COMMANDS_SCHEMA_VERSION: u32 = 1;
/// Every agent at `MAX_COMMANDS` of the longest commands is under 1 MiB;
/// twice that leaves room for the framing.
const COMMANDS_READ_CAP: u64 = 2 * 1024 * 1024;

fn encode_commands(saved: &SavedCommands) -> Result<Vec<u8>, String> {
    let agents: serde_json::Map<String, serde_json::Value> = saved
        .lists
        .iter()
        .map(|(adapter, list)| {
            (
                adapter.as_str().to_owned(),
                hide_desk::commands::saved(list),
            )
        })
        .collect();
    let read: serde_json::Map<String, serde_json::Value> = saved
        .read
        .iter()
        .filter_map(|(adapter, read)| {
            Some((
                adapter.as_str().to_owned(),
                serde_json::to_value(read).ok()?,
            ))
        })
        .collect();
    // `read` is a key a build before it reads past.
    serde_json::to_vec(&serde_json::json!({
        "schema_version": COMMANDS_SCHEMA_VERSION,
        "agents": agents,
        "read": read,
    }))
    .map_err(|error| format!("could not be encoded: {error}"))
}

/// The lists in `bytes`, read leniently: an agent this build does not know
/// or a list that is no list is left out with a diagnostic, an entry that is
/// no command is skipped, and the rest is kept; a read mark that cannot be
/// read is dropped, so that agent is read again.
fn decode_commands(bytes: &[u8]) -> Result<SavedCommands, String> {
    let file: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| format!("could not be decoded: {error}"))?;
    let version = file
        .get("schema_version")
        .and_then(serde_json::Value::as_u64);
    if version != Some(u64::from(COMMANDS_SCHEMA_VERSION)) {
        return Err(format!(
            "has schema version {version:?}, expected {COMMANDS_SCHEMA_VERSION}"
        ));
    }
    let mut saved = SavedCommands::default();
    for (name, entries) in file
        .get("agents")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
    {
        let (Some(adapter), true) = (AdapterId::parse(name), entries.is_array()) else {
            crate::diagnostic!(serde_json::json!({
                "component": "agent_commands",
                "kind": "agent_commands.agent_skipped",
                "agent": name.chars().take(64).collect::<String>(),
            }));
            continue;
        };
        let read = hide_desk::commands::read_saved(adapter, entries);
        if !read.commands.is_empty() {
            saved.lists.insert(adapter, read.commands);
        }
    }
    for (name, read) in file
        .get("read")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
    {
        if let (Some(adapter), Ok(read)) = (
            AdapterId::parse(name),
            serde_json::from_value::<ReadMark>(read.clone()),
        ) && read.mark.len() <= 64
        {
            saved.read.insert(adapter, read);
        }
    }
    Ok(saved)
}

/// Each agent's last command list (`hide_kit::layout::agent_commands`), so a
/// restart lists them before a session opens; written off the lock.
pub(crate) struct AgentCommandsStore {
    path: PathBuf,
    file: LatestFile<SavedCommands>,
    legacy: Option<PathBuf>,
}

impl AgentCommandsStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            file: LatestFile::new(
                path.clone(),
                "hide-agent-commands-save",
                "agent_commands",
                "agent_commands",
                encode_commands,
            ),
            path,
            legacy: None,
        }
    }

    /// Reads the old Desk file only when the agent file does not exist.
    pub(crate) fn with_legacy(path: PathBuf, legacy: PathBuf) -> Self {
        Self {
            legacy: Some(legacy),
            ..Self::new(path)
        }
    }

    /// The lists the last run kept, read before the runtime lock is taken.
    /// A missing file is none; a file that cannot be read is none too, and
    /// the log says why.
    pub(crate) fn restore(&self) -> SavedCommands {
        let mut migrated = false;
        let read = read_capped(&self.path, COMMANDS_READ_CAP).and_then(|bytes| {
            match (bytes, &self.legacy) {
                (None, Some(legacy)) => {
                    let bytes = read_capped(legacy, COMMANDS_READ_CAP)?;
                    migrated = bytes.is_some();
                    Ok(bytes)
                }
                (bytes, _) => Ok(bytes),
            }
        });
        match read.and_then(|bytes| bytes.map(|bytes| decode_commands(&bytes)).transpose()) {
            Ok(lists) => {
                let lists = lists.unwrap_or_default();
                // The save owner performs the migration off the runtime lock.
                if migrated {
                    self.save(lists.clone());
                }
                lists
            }
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_commands",
                    "kind": "agent_commands.restore_failed",
                    "message": message,
                }));
                SavedCommands::default()
            }
        }
    }

    /// Queues `lists` to replace the file; only the newest of a burst is
    /// written.
    pub(crate) fn save(&self, lists: SavedCommands) {
        self.file.save(lists);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A saved command file is read leniently: an agent this build does
    /// not know, a list that is no list and an entry that is no command are
    /// left out, the rest kept; a broken file is no lists at all.
    #[test]
    fn a_saved_command_file_keeps_what_it_can_read() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("commands.json");
        let store = AgentCommandsStore::new(path.clone());
        std::fs::write(&path, serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "agents": {
                "claude": [{"name": "/compact", "description": "c", "input": {"hint": "[focus]"}}, 5, {"name": ""}],
                "nobody": [{"name": "/x", "description": "x"}],
                "pi": "not a list",
            },
        })).unwrap()).unwrap();
        let lists = store.restore().lists;
        assert_eq!(
            lists.keys().copied().collect::<Vec<_>>(),
            [AdapterId::CLAUDE]
        );
        assert_eq!(
            lists[&AdapterId::CLAUDE],
            [hide_desk::commands::Command {
                name: "/compact".into(),
                description: "c".into(),
                hint: Some("[focus]".into()),
            }]
        );
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(store.restore(), SavedCommands::default());
        std::fs::write(&path, br#"{"schema_version": 9, "agents": {}}"#).unwrap();
        assert_eq!(store.restore(), SavedCommands::default());
    }

    /// A list read ahead keeps when it was read and what it depended on, so a
    /// restart reads it again only when that moved; a file from before the
    /// mark, or a mark this build cannot read, reads the list again.
    #[test]
    fn a_read_mark_is_kept_beside_its_list_and_a_file_without_one_still_reads() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("commands.json");
        let saved = SavedCommands {
            lists: [(
                AdapterId::GROK,
                vec![hide_desk::commands::Command {
                    name: "/compact".into(),
                    description: "c".into(),
                    hint: None,
                }],
            )]
            .into(),
            read: [(AdapterId::GROK, ReadMark::of("cli /opt/grok 1 2\n", 7))].into(),
        };
        std::fs::write(&path, encode_commands(&saved).unwrap()).unwrap();
        assert_eq!(AgentCommandsStore::new(path.clone()).restore(), saved);
        assert_eq!(saved.read[&AdapterId::GROK].mark.len(), 32);
        assert_ne!(
            ReadMark::of("cli /opt/grok 1 3\n", 7).mark,
            saved.read[&AdapterId::GROK].mark
        );

        std::fs::write(
            &path,
            br#"{"schema_version": 1, "agents": {"grok": [{"name": "/x", "description": "x"}]},
            "read": {"grok": {"mark": 5}}}"#,
        )
        .unwrap();
        let read = AgentCommandsStore::new(path).restore();
        assert_eq!(read.lists.len(), 1);
        assert!(read.read.is_empty());
    }

    /// Upgrade reads the old lists exactly once, keeps the old bytes and prefers the new file thereafter.
    #[test]
    fn a_missing_agent_file_migrates_without_replacing_the_legacy_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("agent-commands.json");
        let legacy = folder.path().join("desk/commands.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        let bytes = br#"{"schema_version":1,"agents":{"claude":[{"name":"/compact","description":"Compact"}]},"read":{"claude":{"mark":"legacy-mark","at_ms":7}}}"#;
        std::fs::write(&legacy, bytes).unwrap();
        let expected;
        {
            let store = AgentCommandsStore::with_legacy(path.clone(), legacy.clone());
            expected = store.restore();
            assert_eq!(expected.lists[&AdapterId::CLAUDE][0].name, "/compact");
            assert_eq!(expected.read[&AdapterId::CLAUDE].mark, "legacy-mark");
        } // The existing LatestFile owner joins its save.
        assert_eq!(std::fs::read(&legacy).unwrap(), bytes);
        assert_eq!(AgentCommandsStore::new(path.clone()).restore(), expected);
        std::fs::write(&path, br#"{"schema_version":1,"agents":{}}"#).unwrap();
        assert!(
            AgentCommandsStore::with_legacy(path.clone(), legacy.clone())
                .restore()
                .lists
                .is_empty()
        );
        // A broken new file must not silently resurrect an obsolete old list.
        std::fs::write(&path, b"broken").unwrap();
        assert!(
            AgentCommandsStore::with_legacy(path.clone(), legacy)
                .restore()
                .lists
                .is_empty()
        );
        assert_eq!(std::fs::read(path).unwrap(), b"broken");
    }
}
