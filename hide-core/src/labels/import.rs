//! The one-time move from the retired `agent-context-labels` plugin (PRD
//! labels-in-hided D-11).
//!
//! On the first run with no `labels.json`, this machine's plugin state is
//! read once so a pane whose current session the plugin had already labeled
//! keeps its label and spends no request (B4). Only entries the plugin had
//! proven an owner for are taken; an ownerless entry is never adopted. The
//! label is still shown only once the pane's current reference proves that
//! owner, exactly as for the core's own records. Device state is not read
//! (PRD non-goal): device panes start from their provider name.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::json;

use super::analysis::LabelEnd;
use super::store::PaneRecord;

/// Where the plugin kept its display state under a home.
pub(crate) fn plugin_state_path(home: &Path) -> std::path::PathBuf {
    home.join(".local/state/hide.agent-context-labels/display-state.json")
}

#[derive(Debug, Default, Deserialize)]
struct DisplayStates {
    #[serde(default)]
    panes: BTreeMap<String, PluginPane>,
}

#[derive(Debug, Default, Deserialize)]
struct PluginPane {
    #[serde(default)]
    session_owner: Option<String>,
    #[serde(default)]
    state_change_seq: u64,
    #[serde(default)]
    changed_unix_ms: u64,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    progress: String,
    #[serde(default)]
    expected_reply: String,
    #[serde(default)]
    task_input_cursor: Option<u64>,
    #[serde(default)]
    semantic_attention: Option<String>,
    #[serde(default)]
    analysis_turn_start: Option<u64>,
    #[serde(default)]
    analysis_turn_end: Option<u64>,
}

/// The plugin's proven panes as core records; empty when there is no
/// plugin state or it cannot be read (the reason goes to the log).
pub(crate) fn plugin_state(home: &Path) -> BTreeMap<String, PaneRecord> {
    let path = plugin_state_path(home);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "import.unreadable",
                "message": error.to_string(),
            }));
            return BTreeMap::new();
        }
    };
    let states: DisplayStates = match serde_json::from_slice(&bytes) {
        Ok(states) => states,
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "import.malformed",
                "message": error.to_string(),
            }));
            return BTreeMap::new();
        }
    };
    states
        .panes
        .into_iter()
        .filter_map(|(pane_id, pane)| {
            let owner = pane
                .session_owner
                .filter(|owner| !owner.trim().is_empty())?;
            let question = pane.semantic_attention.as_deref() == Some("question")
                && !pane.expected_reply.trim().is_empty();
            Some((
                pane_id,
                PaneRecord {
                    owner: Some(owner),
                    goal: pane.task,
                    line: if question {
                        pane.expected_reply
                    } else {
                        pane.progress
                    },
                    end: question.then_some(LabelEnd::Question),
                    task_input_cursor: pane.task_input_cursor,
                    analysis_turn_start: pane.analysis_turn_start,
                    analysis_turn_end: pane.analysis_turn_end,
                    state_change_seq: pane.state_change_seq,
                    changed_unix_ms: pane.changed_unix_ms,
                    ..PaneRecord::default()
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_owned_plugin_entries_are_imported_and_a_store_is_written_once() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join(".local/state/hide.agent-context-labels");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("display-state.json"),
            json!({"panes": {
                "w1:p1": {"session_owner": "v1:owned", "state_change_seq": 4, "changed_unix_ms": 1700000000000_u64,
                          "task": "기존 라벨 가져오기", "progress": "진행", "expected_reply": "답하세요",
                          "semantic_attention": "question", "analysis_turn_start": 7, "analysis_turn_end": 7,
                          "unseen": true, "interrupted": false},
                "w1:p2": {"state_change_seq": 1, "changed_unix_ms": 1, "task": "주인 없는 라벨"}
            }})
            .to_string(),
        )
        .unwrap();
        let imported = plugin_state(home.path());
        assert_eq!(imported.len(), 1);
        let record = &imported["w1:p1"];
        assert_eq!(record.end, Some(LabelEnd::Question));
        assert_eq!(record.line, "답하세요");
        assert_eq!(record.goal.as_deref(), Some("기존 라벨 가져오기"));
        assert_eq!(record.analysis_turn_end, Some(7));

        let state_dir = tempfile::tempdir().unwrap();
        let store = super::super::store::LabelStore::open(
            Some(state_dir.path()),
            Some(home.path()),
            crate::node::TEST_NODE,
        );
        assert_eq!(store.target(super::super::store::LOCAL_TARGET), imported);
        // A store that exists is never re-imported over.
        std::fs::remove_dir_all(&state).unwrap();
        let reopened = super::super::store::LabelStore::open(
            Some(state_dir.path()),
            Some(home.path()),
            crate::node::TEST_NODE,
        );
        assert_eq!(reopened.target(super::super::store::LOCAL_TARGET), imported);
    }
}
