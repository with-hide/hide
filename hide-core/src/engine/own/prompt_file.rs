//! A start's first input in a file of the pane's machine (docs/delivery.md,
//! The input ladder): the engine has the pane's node keep it privately
//! (`Call::PromptFile`) and types a line that runs the `hide` beside that
//! node, `agent exec --prompt-file`, which reads the file, removes it and
//! hands the text to the program as its last argument ([`read_by_hide`]).
//! No shell reads the text, so no shell's syntax or locale can change it.
//! The start owns the file: its executor removes it once read, the engine
//! removes it when nothing was typed, and the node drops a file whose start
//! has ended (`PROMPT_FILE_IN_USE`).

use std::time::Duration;

use hide_agent_hooks::hook_request::{AGENT_EXEC_ATTEMPT, AGENT_EXEC_PROMPT_FILE};
use hide_node_link::protocol::{Call, PromptFileKept};
use sha2::{Digest, Sha256};

use super::topology::refused;
use super::{Answer, OwnEngine};
use crate::node_access::{call_as, link_failure_code};

/// How long the node has to write or remove a prompt file.
const PROMPT_FILE_WAIT: Duration = Duration::from_secs(10);

/// A prompt file the pane's node keeps for one start.
pub(super) struct Staged {
    start: String,
    pub(super) kept: PromptFileKept,
}

impl OwnEngine<'_> {
    /// Has the pane's node keep `prompt`, the first input of the start named
    /// `name` in `pane`, for the start line to read.
    pub(super) fn stage_prompt(&self, pane: &str, name: &str, prompt: &str) -> Answer<Staged> {
        let start = start_of(pane, name);
        let kept = call_as::<PromptFileKept>(
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
        Ok(Staged { start, kept })
    }

    /// Removes the prompt file of a start that typed nothing; a failure is
    /// logged, and the node's cap takes the file in time.
    pub(super) fn drop_prompt(&self, pane: &str, staged: Staged) {
        let removed = self
            .node()
            .map_err(|error| error.to_string())
            .and_then(|node| {
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

/// `words`, a start line ([`crate::launch::StartLine`]), as the `hide` of
/// `kept` runs it with the first input in `kept`'s file: a profile's line,
/// which runs `hide agent exec` already, names the file after its attempt;
/// any other runs the program through it.
pub(super) fn read_by_hide(
    mut words: Vec<String>,
    attempt: bool,
    kept: &PromptFileKept,
) -> Vec<String> {
    let file = [AGENT_EXEC_PROMPT_FILE.to_owned(), kept.path.clone()];
    // `<hide> agent exec --attempt <id> -- <program> ...`
    if attempt && words.get(3).map(String::as_str) == Some(AGENT_EXEC_ATTEMPT) {
        words.splice(5..5, file);
        return words;
    }
    let mut line = vec![kept.exec.clone(), "agent".to_owned(), "exec".to_owned()];
    line.extend(file);
    line.push("--".to_owned());
    line.extend(words);
    line
}

/// The file name of the start named `name` in `pane`: the same start writes
/// the same file again, and nothing of either reaches the name.
fn start_of(pane: &str, name: &str) -> String {
    let digest = Sha256::digest(format!("{pane}\n{name}").as_bytes());
    let named: String = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("start-{named}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &[&str]) -> Vec<String> {
        line.iter().map(|word| (*word).to_owned()).collect()
    }

    /// The start line runs the node's `hide` with the file, the attempt of a
    /// profile's start staying the first words after `agent exec`.
    #[test]
    fn a_start_with_a_first_input_runs_through_the_nodes_hide_with_its_file() {
        let kept = PromptFileKept {
            path: "/s/prompt-files/start-1".to_owned(),
            exec: "/k/hide".to_owned(),
        };
        assert_eq!(
            read_by_hide(words(&["claude", "--", "x"]), false, &kept),
            words(&[
                "/k/hide",
                "agent",
                "exec",
                "--prompt-file",
                "/s/prompt-files/start-1",
                "--",
                "claude",
                "--",
                "x",
            ])
        );
        assert_eq!(
            read_by_hide(
                words(&[
                    "/k/hide",
                    "agent",
                    "exec",
                    "--attempt",
                    "a1",
                    "--",
                    "claude",
                    "--"
                ]),
                true,
                &kept,
            ),
            words(&[
                "/k/hide",
                "agent",
                "exec",
                "--attempt",
                "a1",
                "--prompt-file",
                "/s/prompt-files/start-1",
                "--",
                "claude",
                "--",
            ])
        );
    }
}
