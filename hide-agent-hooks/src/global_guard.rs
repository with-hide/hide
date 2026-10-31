//! The Hide global instruction's guard (PRD memory-self-improve D-07):
//! every session started in a Hide pane takes that file as the operator's
//! approved instruction, so an agent's tool call that writes it is refused at
//! the pre-tool hook, in every project and whatever its guardrails say. The
//! operator edits the file by hand and Hide writes it for a fix the operator
//! applied; an agent does neither.
//!
//! Like the guardrails it is a guard, not a security boundary: a file tool's
//! write naming the file or the file a link there points to, and a shell call
//! that names the file beside a state folder, or that runs in one, are
//! refused, and a script that spells either some other way is not seen. A
//! file tool's read is let through; a shell call cannot be told apart from a
//! write and is refused, as the file is already in the session's context.
//! Names are compared without case, as macOS and Windows find files.

use std::path::{Path, PathBuf};

use crate::guardrails::Action;
use crate::guardrails::payload::ToolCall;

/// The file's name in the core's state folder (`hide_kit::layout`).
const FILE_NAME: &str = "global-instruction.md";

const REFUSAL: &str = "Not run: this call writes the Hide global instruction, which every agent session in Hide takes as the operator's own instruction. Only the operator edits it, by hand or with a suggestion's apply in Hide's Hide AI tool; tell the operator what you would add instead.";

/// Whether a raw hook payload can name the file, or the file a link there
/// points to, at all; a payload that names neither is let through without
/// being parsed.
pub fn may_name(home: &Path, payload: &[u8]) -> bool {
    let names = files(home);
    let mut names = names
        .iter()
        .filter_map(|file| file.file_name())
        .map(|name| name.as_encoded_bytes());
    names.any(|name| {
        !name.is_empty()
            && payload
                .windows(name.len())
                .any(|window| window.eq_ignore_ascii_case(name))
    })
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
        Action::Shell(command) => {
            shell_names(command, &names) || beside_state(command, &cwd, &files)
        }
        Action::Read(_) | Action::UnreadWrite => false,
    };
    writes.then(|| REFUSAL.to_owned())
}

/// The reason to refuse a call whose payload names the file but was cut
/// before Hide could read what it does: it is refused rather than guessed.
pub fn unread_refusal() -> String {
    format!("{REFUSAL} The call was too large to check.")
}

/// One refusal, as one structured line in the guard's log: the runtime and
/// the pane, never the command or its paths. The refusal stands when the line
/// cannot be written.
pub fn record_refusal(home: &Path, runtime: &str, pane: Option<&str>) {
    let line = serde_json::json!({
        "component": "global_instruction",
        "kind": "guard.refused",
        "runtime": runtime,
        "pane": pane,
        "at_unix_ms": crate::spawn_guard::now_ms(),
    })
    .to_string();
    let _ = crate::spawn_guard::append_line(&crate::spawn_guard::log_path(home), &line);
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

/// Whether `command` names the file by its bare name where the name can
/// only be read against a state folder: the command also names one, or runs
/// in one. This catches `cd <state> && ... >> global-instruction.md`, a
/// `bash -c` script and an `eval`, which no single word resolves.
fn beside_state(command: &str, cwd: &Path, files: &[PathBuf]) -> bool {
    let command = command.to_lowercase();
    if !command.contains(FILE_NAME) {
        return false;
    }
    let folders = files.iter().filter_map(|file| file.parent());
    let in_state = folders
        .clone()
        .any(|folder| same(&folder.join(FILE_NAME), &cwd.join(FILE_NAME)));
    in_state
        || [
            ".hide/state",
            ".hide\\state",
            "hide_state_dir",
            "xdg_state_home",
        ]
        .iter()
        .any(|mention| command.contains(mention))
        || folders.into_iter().any(|folder| {
            let folder = folder.to_string_lossy().to_lowercase();
            !folder.is_empty() && command.contains(&folder)
        })
}

/// Where the core of this account can keep its state folder, in the order
/// the guard and the scripts' prefilter list them.
#[derive(Clone, Copy, Debug)]
enum StateFolder {
    /// The folder the kit installed the hook or script for (`crate::route`).
    Installed,
    /// A variable naming a folder, and the parts below it.
    Variable(&'static str, &'static [&'static str]),
    /// Parts below the account's home.
    Home(&'static [&'static str]),
}

/// Every state folder the core of this account can have: the folder the kit
/// installed the hook for, `HIDE_STATE_DIR`, `$XDG_STATE_HOME/hide` and
/// `~/.hide/state`. The helper's guard ([`files`]) and the OpenCode, Pi and
/// omp prefilter ([`prefilter_constants`]) both read this list.
const STATE_FOLDERS: [StateFolder; 4] = [
    StateFolder::Installed,
    StateFolder::Variable("HIDE_STATE_DIR", &[]),
    StateFolder::Variable("XDG_STATE_HOME", &["hide"]),
    StateFolder::Home(&[".hide", "state"]),
];

/// The file in every state folder of [`STATE_FOLDERS`], and the file a link
/// at any of them points to, which is the one the core reads and writes.
fn files(home: &Path) -> Vec<PathBuf> {
    let below = |folder: PathBuf, parts: &[&str]| {
        parts.iter().fold(folder, |folder, part| folder.join(part))
    };
    let named = STATE_FOLDERS
        .iter()
        .filter_map(|folder| match folder {
            StateFolder::Installed => crate::route::installed_state_dir(),
            StateFolder::Variable(name, parts) => hide_platform::host::variable(name)
                .filter(|value| !value.is_empty())
                .map(|value| below(PathBuf::from(value), parts)),
            StateFolder::Home(parts) => Some(below(home.to_path_buf(), parts)),
        })
        .map(|folder| folder.join(FILE_NAME))
        .collect::<Vec<_>>();
    let targets = named
        .iter()
        .filter_map(|file| Some(file.parent()?.join(std::fs::read_link(file).ok()?)))
        .collect::<Vec<_>>();
    named.into_iter().chain(targets).collect()
}

/// The prefilter Hide's OpenCode plugin and Pi and omp extension run before
/// they ask the helper (`crate::plugin`): the placeholders each of their
/// templates carries and the JavaScript each becomes. It is made from this
/// guard's own file name and [`STATE_FOLDERS`], and `namesGlobal` tests the
/// text a call would reach the helper with as [`may_name`] tests a hook's
/// payload, so every call the guard refuses is sent to the helper
/// (`tests/fixtures/global-guard-corpus.json`).
pub(crate) fn prefilter_constants() -> [(&'static str, String); 3] {
    let quote = |text: &str| serde_json::to_string(text).expect("strings serialize");
    let joined = |parts: &[&str]| {
        parts
            .iter()
            .map(|part| quote(part))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let folders = STATE_FOLDERS
        .iter()
        .map(|folder| match folder {
            StateFolder::Installed => format!(
                "ROUTE[0] === {} ? Buffer.from(ROUTE[1], \"base64url\").toString() : \"\"",
                quote(crate::route::ARGUMENT)
            ),
            StateFolder::Variable(name, []) => format!("process.env.{name} ?? \"\""),
            StateFolder::Variable(name, parts) => format!(
                "process.env.{name} ? path.join(process.env.{name}, {}) : \"\"",
                joined(parts)
            ),
            StateFolder::Home(parts) => format!("path.join(homedir(), {})", joined(parts)),
        })
        .map(|line| format!("  {line},\n"))
        .collect::<String>();
    [
        ("__HIDE_GLOBAL_INSTRUCTION__", quote(FILE_NAME)),
        (
            "__HIDE_STATE_DIRS__",
            format!("[\n{folders}].filter(Boolean)"),
        ),
        (
            "__HIDE_NAMES_GLOBAL__\n",
            include_str!("global_guard/names_global.js").to_owned(),
        ),
    ]
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
/// links in their folders are resolved, without case.
fn same(file: &Path, path: &Path) -> bool {
    let lower = |path: &Path| path.to_string_lossy().to_lowercase();
    if lower(file) == lower(path) {
        return true;
    }
    let name = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
    };
    if name(file) != name(path) {
        return false;
    }
    let folder = |path: &Path| {
        path.parent()
            .and_then(|folder| std::fs::canonicalize(folder).ok())
            .map(|folder| lower(&folder))
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

    /// The corpus every runtime's prefilter is also run on
    /// (`tests/fixtures/global-guard-corpus.json`): the helper refuses
    /// exactly the calls it marks refused, may still read the file, and its
    /// payload test passes on every call it marks named, a refused one always
    /// among them.
    #[test]
    fn the_guard_refuses_and_reads_the_corpus_as_it_says() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/global-guard-corpus.json"))
                .unwrap();
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join(".hide").join("state");
        std::fs::create_dir_all(&state).unwrap();
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let cases = corpus["cases"].as_array().unwrap();
        assert!(cases.len() >= 10);
        for case in cases {
            let text = case["text"]
                .as_str()
                .unwrap()
                .replace("{state}", &state.to_string_lossy());
            let cwd = match case["cwd"].as_str().unwrap() {
                "state" => &state,
                _ => &repo,
            };
            let (action, input) = match case["tool"].as_str().unwrap() {
                "shell" => (
                    Action::Shell(text.clone()),
                    serde_json::json!({ "command": text }),
                ),
                "write" => (
                    Action::Write(vec![text.clone()]),
                    serde_json::json!({ "file_path": text }),
                ),
                "read" => (
                    Action::Read(vec![text.clone()]),
                    serde_json::json!({ "file_path": text }),
                ),
                other => panic!("unknown corpus tool {other}"),
            };
            let refused = case["refused"].as_bool().unwrap();
            let named = case["named"].as_bool().unwrap();
            assert!(named || !refused, "a refused call is named: {text}");
            assert_eq!(
                refusal(home.path(), &call(action, cwd)).is_some(),
                refused,
                "refusal of {text}"
            );
            let payload = serde_json::json!({ "tool_input": input }).to_string();
            assert_eq!(
                may_name(home.path(), payload.as_bytes()),
                named,
                "payload test of {text}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_file_a_link_there_points_to_is_guarded_as_the_file() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join(".hide").join("state");
        std::fs::create_dir_all(&state).unwrap();
        let dotfiles = home.path().join("dotfiles");
        std::fs::create_dir_all(&dotfiles).unwrap();
        let target = dotfiles.join("agents.md");
        std::fs::write(&target, "x\n").unwrap();
        std::os::unix::fs::symlink(&target, state.join(FILE_NAME)).unwrap();
        let write = Action::Write(vec![target.to_string_lossy().into_owned()]);
        assert!(refusal(home.path(), &call(write, home.path())).is_some());
        // A payload naming only the target is read, not passed unparsed.
        assert!(may_name(
            home.path(),
            br#"{"file_path":"/x/dotfiles/Agents.md"}"#
        ));
    }
}
