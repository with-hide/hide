//! A start's first input in a file of the pane's machine (docs/delivery.md,
//! The input ladder): the engine has the pane's node keep it privately
//! (`Call::PromptFile`) and types a line that reads the file and removes it
//! (`hide_shell::Syntax::file_word`), so the text never crosses the pane's
//! shell as typed text; a shell whose locale cannot read it would change it.
//! The start owns the file: its line removes it once read, the engine removes
//! it when nothing was typed, and the node keeps at most
//! `PROMPT_FILES_CAP` of a line that never ran.

use std::time::Duration;

use hide_node_link::protocol::Call;
use sha2::{Digest, Sha256};

use super::topology::refused;
use super::{Answer, OwnEngine};
use crate::node_access::{call_as, link_failure_code};

/// How long the node has to write or remove a prompt file.
const PROMPT_FILE_WAIT: Duration = Duration::from_secs(10);

/// A prompt file the pane's node keeps for one start.
pub(super) struct Staged {
    start: String,
    pub(super) path: String,
}

impl OwnEngine<'_> {
    /// Has the pane's node keep `prompt`, the first input of the start named
    /// `name` in `pane`, for the start line to read.
    pub(super) fn stage_prompt(&self, pane: &str, name: &str, prompt: &str) -> Answer<Staged> {
        let start = start_of(pane, name);
        let path = call_as::<String>(
            self.node()?.as_ref(),
            Call::PromptFile {
                start: start.clone(),
                text: prompt.to_owned(),
            },
            PROMPT_FILE_WAIT,
        )
        .map_err(|error| {
            refused(
                &link_failure_code(&error),
                format!("keeping the first input of the start in pane {pane}: {error}"),
            )
        })?;
        Ok(Staged { start, path })
    }

    /// Removes the prompt file of a start that typed nothing; a failure is
    /// logged, and the node's cap takes the file in time.
    pub(super) fn drop_prompt(&self, pane: &str, staged: Staged) {
        let removed = self.node().map_err(|error| error.to_string()).and_then(|node| {
            call_as::<serde_json::Value>(
                node.as_ref(),
                Call::PromptFileRemove {
                    start: staged.start.clone(),
                },
                PROMPT_FILE_WAIT,
            )
            .map_err(|error| error.to_string())
        });
        if let Err(cause) = removed {
            crate::diagnostic!(serde_json::json!({
                "component": "own_engine", "kind": "start.prompt_file_kept",
                "pane": pane, "start": staged.start, "cause": cause,
            }));
        }
    }
}

/// The file name of the start named `name` in `pane`: the same start writes
/// the same file again, and nothing of either reaches the name.
fn start_of(pane: &str, name: &str) -> String {
    let digest = Sha256::digest(format!("{pane}\n{name}").as_bytes());
    format!("start-{}", hex::encode(&digest[..12]))
}
