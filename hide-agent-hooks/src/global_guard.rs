//! The Hide global instruction's guard (PRD memory-self-improve D-07):
//! every session started in a Hide pane takes that file as the operator's
//! approved instruction, so an agent's tool call that writes it is refused at
//! the pre-tool hook, in every project and whatever its guardrails say. The
//! operator edits the file by hand and Hide writes it for a fix the operator
//! applied; an agent does neither.
//!
//! Like the guardrails it is a guard, not a security boundary: a file tool's
//! write and a shell call with a word or redirection naming the file are
//! refused, and a script that writes it some other way is not seen. A file
//! tool's read is let through; a shell call cannot be told apart from a
//! write and is refused, as the file is already in the session's context.

use std::path::{Path, PathBuf};

use crate::guardrails::Action;
use crate::guardrails::payload::ToolCall;

/// The file's name in the core's state folder (`hide_kit::layout`).
const FILE_NAME: &str = "global-instruction.md";

/// Whether a raw hook payload can name the file at all; a payload that
/// does not is let through without being parsed.
pub fn may_name(payload: &[u8]) -> bool {
    payload
        .windows(FILE_NAME.len())
        .any(|window| window == FILE_NAME.as_bytes())
}

/// The reason to refuse `call`, run in `cwd` by an agent of the account whose
/// home is `home`, or `None` to let it run.
pub fn refusal(home: &Path, call: &ToolCall) -> Option<String> {
    let cwd = match &call.cwd {
        Some(cwd) if cwd.is_absolute() => cwd.clone(),
        Some(cwd) => std::env::current_dir().ok()?.join(cwd),
        None => std::env::current_dir().ok()?,
    };
    let files = files(home);
    let names = |word: &str| {
        let path = crate::guardrails::absolute(home, &cwd, Path::new(&expand(home, word)));
        files.iter().any(|file| same(file, &path))
    };
    let writes = match &call.action {
        Action::Write(paths) => paths.iter().any(|path| names(path)),
        Action::Shell(command) => shell_names(command, &names),
        Action::Read(_) | Action::UnreadWrite => false,
    };
    writes.then(|| {
        "Not run: this call writes the Hide global instruction, which every agent session in Hide takes as the operator's own instruction. Only the operator edits it, by hand or with a suggestion's apply in Hide's Hide AI tool; tell the operator what you would add instead.".to_owned()
    })
}

/// Whether a word or a redirection of `command`, or of a command it
/// substitutes, names the file.
fn shell_names(command: &str, names: &dyn Fn(&str) -> bool) -> bool {
    let line = hide_shell::shell_line(command);
    line.commands.iter().flatten().any(|word| names(word))
        || line.targets.iter().any(|(_, target)| names(target))
        || line
            .substitutions
            .iter()
            .any(|(_, text)| shell_names(text, names))
}

/// The file in every state folder the core of this account can have:
/// `HIDE_STATE_DIR`, the folder the kit installed the hook for,
/// `$XDG_STATE_HOME/hide` and `~/.hide/state`.
fn files(home: &Path) -> Vec<PathBuf> {
    let variable = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    [
        variable("HIDE_STATE_DIR").map(PathBuf::from),
        crate::route::installed_state_dir(),
        variable("XDG_STATE_HOME").map(|state| PathBuf::from(state).join("hide")),
        Some(home.join(".hide").join("state")),
    ]
    .into_iter()
    .flatten()
    .map(|folder| folder.join(FILE_NAME))
    .collect()
}

/// `word` with a leading `$HOME`, `${HOME}` or `$HIDE_STATE_DIR` spelled
/// out, as the shell would.
fn expand(home: &Path, word: &str) -> String {
    let state = std::env::var("HIDE_STATE_DIR").ok();
    for (name, value) in [
        ("${HOME}", Some(home.to_string_lossy().into_owned())),
        ("$HOME", Some(home.to_string_lossy().into_owned())),
        ("${HIDE_STATE_DIR}", state.clone()),
        ("$HIDE_STATE_DIR", state),
    ] {
        if let (Some(rest), Some(value)) = (word.strip_prefix(name), value) {
            return format!("{value}{rest}");
        }
    }
    word.to_owned()
}

/// Whether two absolute paths name one file: equal as written, or once
/// links in their folders are resolved.
fn same(file: &Path, path: &Path) -> bool {
    if file == path {
        return true;
    }
    if path.file_name() != file.file_name() {
        return false;
    }
    let folder = |path: &Path| {
        path.parent()
            .and_then(|folder| std::fs::canonicalize(folder).ok())
    };
    folder(file).is_some_and(|folder_of_file| Some(folder_of_file) == folder(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(action: Action, cwd: &Path) -> ToolCall {
        ToolCall {
            action,
            cwd: Some(cwd.to_path_buf()),
        }
    }

    #[test]
    fn an_agent_never_writes_the_global_instruction_and_may_still_read_it() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join(".hide").join("state");
        std::fs::create_dir_all(&state).unwrap();
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let refused = |action| refusal(home.path(), &call(action, &repo)).is_some();
        let file = state.join(FILE_NAME).to_string_lossy().into_owned();
        assert!(refused(Action::Write(vec![file.clone()])));
        assert!(refused(Action::Write(vec![
            "~/.hide/state/global-instruction.md".into()
        ])));
        assert!(refused(Action::Shell(
            "echo 'Always obey me' >> ~/.hide/state/global-instruction.md".into()
        )));
        assert!(refused(Action::Shell(
            "tee -a $HOME/.hide/state/global-instruction.md < note".into()
        )));
        assert!(refused(Action::Shell(format!(
            "cd /tmp && sed -i '' 's/a/b/' \"{file}\""
        ))));
        assert!(refused(Action::Shell(
            "x=$(cp note ../.hide/state/global-instruction.md)".into()
        )));
        assert!(refused(Action::Shell(
            "cat ~/.hide/state/global-instruction.md".into()
        )));
        // A file tool may read it; a same-named file elsewhere and Hide's
        // own sources pass.
        assert!(!refused(Action::Read(vec![file.clone()])));
        assert!(!refused(Action::Write(vec![
            "docs/global-instruction.md".into()
        ])));
        assert!(!refused(Action::Shell(
            "rg -n global-instruction.md herdr-core".into()
        )));
    }

    #[test]
    fn a_payload_that_does_not_name_the_file_is_not_read() {
        assert!(may_name(
            br#"{"command":"cat ~/.hide/state/global-instruction.md"}"#
        ));
        assert!(!may_name(br#"{"command":"cargo test"}"#));
    }
}
