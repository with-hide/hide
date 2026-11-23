//! The change requests a tool call named: the ones a session acted on, as
//! opposed to an address that only passed through a tool's output.
//!
//! A tool's output prints whatever it lists (a browser's open tabs, `gh pr
//! list`, a log), so an address in it says only that the session saw the
//! change request; that is evidence of creation when it is printed as its
//! host makes the change request, and of nothing else. A call names a change
//! request when it acts on that one: a host's command given its number or
//! address (`gh pr view 12`, `glab mr view !12`), an API path under it, a
//! command that opens or fetches its address, a tool whose argument is the
//! address, or a host's tool given its repository and number. What each
//! host's commands and addresses look like is that host's
//! (`crate::hosts`); this module walks the call and asks them.
//!
//! The shell text is read by `hide-shell`, as the hooks read a call before it
//! runs and in the syntax of the shell that ran it (`crate::call_shell`, which
//! the caller passes): heredoc bodies, quotes and comments are text, so an
//! address in a commit message or a letter's body names nothing, and a
//! PowerShell `cd .\app` moves to `app`. A command hidden a layer
//! deeper (`bash -c`, a script, an alias) is not looked for.
//!
//! A call that makes a change request ([`creates_pull_request`]) is read the
//! same way, and the address its output gives back ([`created_addresses`])
//! is the one it made, whatever the clocks say.

use crate::hosts;
use hide_shell::Syntax;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// A change request a tool call named.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NamedPr {
    /// The repository's stored name when the call said which repository (an
    /// address, `--repo`, an API path, a host tool's arguments): `owner/name`
    /// on github.com, `gitlab.com/group/sub/project`. `None` when it named
    /// only a number, which is a change request of the repository the call
    /// ran in.
    pub repository: Option<String>,
    pub number: u64,
    /// For a number alone, the folder the call ran in when the record says
    /// it was not the session's own (a `cd` before the command, the tool's
    /// working directory); `None` is the session's folder.
    pub folder: Option<String>,
    /// The key of the host whose command or address named it; `None` is
    /// GitHub's, as every record and older node before hosts were named.
    /// A number alone belongs only to a project of this host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

/// How deep `$(...)` substitutions are read.
const SUBSTITUTION_DEPTH: usize = 2;

/// The keys a shell tool carries its command under: Claude Code's, Codex's,
/// Grok's, Pi's, omp's and OpenCode's `command` (a string, or Codex's argv),
/// Codex's `cmd`, and agy's `CommandLine`.
const COMMAND_KEYS: [&str; 3] = ["command", "cmd", "CommandLine"];

/// The keys a shell tool names its working folder under: Codex's `workdir`,
/// agy's `Cwd`, Cursor's and Codex's local shell `working_directory`.
const FOLDER_KEYS: [&str; 5] = ["workdir", "cwd", "Cwd", "working_directory", "directory"];

/// The change requests one tool call's input names. `folder` is where the
/// record says the session was when it made the call, when it says so, and
/// `syntax` the shell's that ran it.
pub(crate) fn tool_call(input: &Value, folder: Option<&str>, syntax: Syntax) -> Vec<NamedPr> {
    let mut found = Found::default();
    let Some(object) = input.as_object() else {
        return found.named;
    };
    let folder = FOLDER_KEYS
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .filter(|folder| !folder.trim().is_empty())
        .or(folder);
    let place = folder.map_or(Place::Session, |folder| Place::At(PathBuf::from(folder)));
    commands(object, &place, syntax, &mut found);
    // A tool whose argument is the address itself: a fetch, a browser's
    // navigation.
    for (key, value) in object {
        if COMMAND_KEYS.contains(&key.as_str()) {
            continue;
        }
        if let Some((host, repository, number)) =
            value.as_str().and_then(hosts::change_request_address)
        {
            found.push(host, Some(repository), number, None);
        }
    }
    for host in hosts::STANDARD {
        host.tool_arguments(object, &mut found);
    }
    found.named
}

/// Every shell command the call carries, under each of [`COMMAND_KEYS`].
fn commands(
    object: &serde_json::Map<String, Value>,
    place: &Place,
    syntax: Syntax,
    found: &mut Found,
) {
    for key in COMMAND_KEYS {
        match object.get(key) {
            Some(Value::String(text)) => shell(text, place.clone(), 0, syntax, found),
            Some(Value::Array(argv)) => {
                let words: Option<Vec<String>> = argv
                    .iter()
                    .map(|word| word.as_str().map(str::to_owned))
                    .collect();
                match words.as_deref() {
                    // `["bash", "-lc", "<script>"]` or `["powershell.exe",
                    // "-Command", "<script>"]`: the script is the line, in
                    // the syntax of the shell the argv names.
                    Some([shell_program, flag, script])
                        if flag.starts_with('-')
                            && flag.to_ascii_lowercase().contains('c')
                            && let Some(script_syntax) = shell_of(shell_program) =>
                    {
                        shell(script, place.clone(), 0, script_syntax, found);
                    }
                    Some(words) => {
                        let mut place = place.clone();
                        command(words, &mut place, syntax, found);
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }
}

/// Whether a tool call makes a change request: a host's create command
/// (`gh pr create`, `glab mr create`), an API POST to where its host makes
/// one, a `git push` with an option its host makes one from, or a host's
/// tool named for it. `name` is the tool's own name when the record keeps
/// one, and `syntax` the shell's that ran it. A shell call with an `||`
/// makes no claim: the command after it runs because one failed, and what it
/// prints is no making.
pub(crate) fn creates_pull_request(name: Option<&str>, input: &Value, syntax: Syntax) -> bool {
    if let Some(name) = name.map(str::to_ascii_lowercase)
        && hosts::STANDARD.iter().any(|host| host.creating_tool(&name))
    {
        return true;
    }
    let Some(object) = input.as_object() else {
        return false;
    };
    let mut found = Found::default();
    commands(object, &Place::Session, syntax, &mut found);
    found.creates && !found.fallback
}

/// Whether a tool call runs `git commit` (PRD project-panel-results D-20):
/// a shell command whose git subcommand, past git's own options, is
/// `commit`, read in `syntax`. Whether the commit took is the checkout's to
/// say.
pub(crate) fn runs_git_commit(input: &Value, syntax: Syntax) -> bool {
    let Some(object) = input.as_object() else {
        return false;
    };
    let mut found = Found::default();
    commands(object, &Place::Session, syntax, &mut found);
    found.commits
}

/// The change requests a creating call's output gives back: an address
/// alone on its line, as `gh pr create` and `glab mr create` print the one
/// they made and a push that made one prints it after git's `remote:`, or
/// the address field of the host's answer to the create request. An address
/// inside other text, a description the answer echoes included, is not the
/// one made.
pub(crate) fn created_addresses(outputs: &[&str]) -> Vec<(String, u64)> {
    let mut made: Vec<(String, u64)> = Vec::new();
    let mut keep = |address: (String, u64)| {
        if made.len() < crate::MAX_SIGHTINGS_PER_OUTPUT && !made.contains(&address) {
            made.push(address);
        }
    };
    for output in outputs {
        for line in output.lines() {
            let line = line.trim();
            let line = line.strip_prefix("remote:").unwrap_or(line);
            if let Some((_, repository, number)) = hosts::change_request_address(line) {
                keep((repository, number));
            }
        }
        if let Ok(answer) = serde_json::from_str::<Value>(output.trim()) {
            for host in hosts::STANDARD {
                if let Some((repository, number)) = answer
                    .get(host.made_field())
                    .and_then(Value::as_str)
                    .and_then(|address| host.address(address.trim()))
                {
                    keep((repository, number));
                }
            }
        }
    }
    made
}

#[derive(Default)]
pub(crate) struct Found {
    named: Vec<NamedPr>,
    /// A command among them makes a change request.
    pub(crate) creates: bool,
    /// A line among them holds an `||`.
    fallback: bool,
    /// A command among them is `git commit`.
    commits: bool,
}

impl Found {
    /// A change request `host` named: by its repository's stored name, or
    /// by its number alone in `folder`.
    pub(crate) fn push(
        &mut self,
        host: &'static str,
        repository: Option<String>,
        number: u64,
        folder: Option<String>,
    ) {
        let folder = folder.filter(|_| repository.is_none());
        let named = NamedPr {
            repository,
            number,
            folder,
            host: hosts::recorded_key(host),
        };
        if !self.named.contains(&named) {
            self.named.push(named);
        }
    }
}

/// Where a command runs, as far as the call says.
#[derive(Clone)]
pub(crate) enum Place {
    /// The session's own folder.
    Session,
    At(PathBuf),
    /// A `cd` this reading cannot follow (home, `-`, a variable): a number
    /// named here belongs to no known repository.
    Unknown,
}

fn shell(text: &str, mut place: Place, depth: usize, syntax: Syntax, found: &mut Found) {
    let line = hide_shell::shell_line(text, syntax);
    found.fallback |= line.fallback;
    for words in &line.commands {
        command(words, &mut place, syntax, found);
    }
    if depth < SUBSTITUTION_DEPTH {
        for (_, text) in &line.substitutions {
            shell(text, place.clone(), depth + 1, syntax, found);
        }
    }
}

fn command(words: &[String], place: &mut Place, syntax: Syntax, found: &mut Found) {
    let mut assignments: Vec<&str> = Vec::new();
    let mut at = 0;
    while let Some(word) = words.get(at) {
        if hide_shell::is_assignment(word) {
            assignments.push(word);
        } else if !hide_shell::is_prefix(word, true) {
            break;
        }
        at += 1;
    }
    let Some((first, args)) = words[at..].split_first() else {
        return;
    };
    let program = syntax.program(first);
    if syntax.changes_folder(&program).is_some() {
        // A bare one goes home or swaps folders, neither of which this
        // reading follows.
        *place = match args.iter().find(|arg| !arg.starts_with('-') || *arg == "-") {
            Some(target) => moved(place, &syntax.path_word(target)),
            None => Place::Unknown,
        };
        return;
    }
    if let Some(host) = hosts::of_program(&program) {
        host.command(args, &assignments, place, found);
        return;
    }
    match program.as_str() {
        "git" => git(args, found),
        "open" | "xdg-open" | "start" | "curl" | "wget" => addresses(args, found),
        "hide"
            if args.first().is_some_and(|word| word == "browser")
                && args.get(1).is_some_and(|word| word == "open") =>
        {
            addresses(&args[2..], found);
        }
        _ => {}
    }
}

/// `git commit`, and a `git push` whose push option makes a change request
/// on the remote's host (`-o merge_request.create`).
fn git(args: &[String], found: &mut Found) {
    let Some((subcommand, rest)) = git_subcommand(args) else {
        return;
    };
    match subcommand {
        "commit" => found.commits = true,
        "push" => {
            let mut words = rest.iter();
            while let Some(word) = words.next() {
                let option = match word.as_str() {
                    "-o" | "--push-option" => words.next().map(String::as_str),
                    other => other
                        .strip_prefix("--push-option=")
                        .or_else(|| other.strip_prefix("-o").filter(|value| !value.is_empty())),
                };
                if option.is_some_and(|option| {
                    hosts::STANDARD
                        .iter()
                        .any(|host| host.creating_push_option(option))
                }) {
                    found.creates = true;
                }
            }
        }
        _ => {}
    }
}

/// The subcommand of a `git` call and the words after it, past git's own
/// options (`-C <path>`, `-c <name=value>` and the `--flag[=value]` forms).
fn git_subcommand(args: &[String]) -> Option<(&str, &[String])> {
    let mut at = 0;
    while let Some(word) = args.get(at) {
        match word.as_str() {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => at += 2,
            option if option.starts_with('-') => at += 1,
            subcommand => return Some((subcommand, &args[at + 1..])),
        }
    }
    None
}

/// Every argument that is a change request's address.
fn addresses(args: &[String], found: &mut Found) {
    for arg in args {
        if let Some((host, repository, number)) = hosts::change_request_address(arg) {
            found.push(host, Some(repository), number, None);
        }
    }
}

/// A number `host` named with its repository, or with where the call ran
/// when it named no repository.
pub(crate) fn name(
    host: &'static str,
    repository: Option<String>,
    number: u64,
    place: &Place,
    found: &mut Found,
) {
    match (repository, place) {
        (Some(repository), _) => found.push(host, Some(repository), number, None),
        (None, Place::Session) => found.push(host, None, number, None),
        (None, Place::At(folder)) => {
            found.push(
                host,
                None,
                number,
                Some(folder.to_string_lossy().into_owned()),
            );
        }
        (None, Place::Unknown) => {}
    }
}

/// The first word that is not a flag or a value-taking flag's value.
pub(crate) fn first_positional<'a>(args: &'a [String], value_flags: &[&str]) -> Option<&'a str> {
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            return words.next().map(String::as_str);
        }
        if word.starts_with('-') && word.len() > 1 {
            if value_flags.contains(&word.as_str()) {
                words.next();
            }
            continue;
        }
        return Some(word);
    }
    None
}

/// The value of a flag given as `<short> v`, `<long> v`, `<long>=v` or
/// `<short>v`.
pub(crate) fn flag_value<'a>(args: &'a [String], short: &str, long: &str) -> Option<&'a str> {
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let value = if word == short || word == long {
            words.next().map(String::as_str)
        } else {
            word.strip_prefix(long)
                .and_then(|rest| rest.strip_prefix('='))
                .or_else(|| word.strip_prefix(short).filter(|rest| !rest.is_empty()))
        };
        if value.is_some() {
            return value;
        }
    }
    None
}

/// Whether an API call sends a POST: `-X POST` or `--method POST`, or with
/// no method named, one of `field_flags`, which `gh api` and `glab api`
/// send as one.
pub(crate) fn api_posts(args: &[String], field_flags: &[&str]) -> bool {
    let mut method = None;
    let mut fields = false;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "-X" | "--method" => method = words.next().map(|value| value.to_ascii_uppercase()),
            flag if field_flags.contains(&flag) => fields = true,
            other => {
                if let Some(value) = other
                    .strip_prefix("--method=")
                    .or_else(|| other.strip_prefix("-X").filter(|value| !value.is_empty()))
                {
                    method = Some(value.to_ascii_uppercase());
                }
            }
        }
    }
    method.map_or(fields, |method| method == "POST")
}

pub(crate) fn pr_number(text: &str) -> Option<u64> {
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse::<u64>().ok())
        .flatten()
        .filter(|number| *number > 0)
}

/// The syntax of the shell an argv's first word names, when it is a shell
/// that runs a script it is handed.
fn shell_of(word: &str) -> Option<Syntax> {
    match Syntax::PowerShell.program(word).as_str() {
        "bash" | "sh" | "zsh" | "dash" | "ksh" => Some(Syntax::Posix),
        "pwsh" | "powershell" => Some(Syntax::PowerShell),
        _ => None,
    }
}

/// Where a `cd` to `target` leads from `place`.
fn moved(place: &Place, target: &str) -> Place {
    if target == "-" || target.starts_with('~') || target.contains('$') || target.contains('`') {
        return Place::Unknown;
    }
    let target = Path::new(target);
    if target.is_absolute() {
        return Place::At(normal(target));
    }
    match place {
        Place::At(folder) => Place::At(normal(&folder.join(target))),
        // The session's folder is the reader's to say, not this line's.
        Place::Session | Place::Unknown => Place::Unknown,
    }
}

/// `.` and `..` resolved by the path's own words, as a shell's `cd` does.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The cases below are POSIX shell calls; the PowerShell ones name their
    // syntax.
    fn tool_call(input: &Value, folder: Option<&str>) -> Vec<NamedPr> {
        super::tool_call(input, folder, Syntax::Posix)
    }

    fn creates_pull_request(name: Option<&str>, input: &Value) -> bool {
        super::creates_pull_request(name, input, Syntax::Posix)
    }

    fn runs_git_commit(input: &Value) -> bool {
        super::runs_git_commit(input, Syntax::Posix)
    }

    /// A PowerShell call is read as PowerShell reads it: `\` separates
    /// names, a command's name has no case, `Set-Location` and its aliases
    /// move the folder, and an argv that runs PowerShell hands it a
    /// PowerShell script.
    #[test]
    fn a_powershell_call_is_read_in_its_own_syntax() {
        let powershell = |input: Value| super::tool_call(&input, Some("/work"), Syntax::PowerShell);
        let in_app = vec![NamedPr {
            repository: None,
            number: 5,
            folder: Some("/work/app".into()),
            host: None,
        }];
        for command in [
            r"cd .\app; gh pr view 5",
            r"Set-Location -Path .\app; GH.EXE pr view 5",
            r"sl app
gh pr view 5",
            r"Push-Location .\app; gh pr view 5",
        ] {
            assert_eq!(
                powershell(json!({ "command": command })),
                in_app,
                "{command}"
            );
        }
        assert_eq!(
            tool_call(
                &json!({"command": ["powershell.exe", "-Command", r"cd .\app; gh pr view 5"]}),
                Some("/work")
            ),
            in_app
        );
        assert!(super::creates_pull_request(
            None,
            &json!({"command": "GH.exe pr create --fill"}),
            Syntax::PowerShell
        ));
        assert!(super::runs_git_commit(
            &json!({"command": r"& 'C:\Program Files\Git\cmd\git.exe' commit -m x"}),
            Syntax::PowerShell
        ));
    }

    fn shell_call(command: &str) -> Vec<NamedPr> {
        tool_call(&json!({ "command": command }), Some("/work/app"))
    }

    fn here(number: u64) -> NamedPr {
        NamedPr {
            repository: None,
            number,
            folder: Some("/work/app".into()),
            host: None,
        }
    }

    fn at(repository: &str, number: u64) -> NamedPr {
        NamedPr {
            repository: Some(repository.into()),
            number,
            folder: None,
            host: None,
        }
    }

    #[test]
    fn a_gh_command_that_acts_on_one_pull_request_names_it() {
        assert_eq!(shell_call("gh pr view 1628"), vec![here(1628)]);
        assert_eq!(shell_call("gh pr diff 1628 --name-only"), vec![here(1628)]);
        assert_eq!(
            shell_call("gh pr review --approve 1640 -b 'looks good'"),
            vec![here(1640)]
        );
        assert_eq!(
            shell_call("gh pr view --json title,body 12 -q .title"),
            vec![here(12)]
        );
        assert_eq!(
            shell_call("gh pr comment 7 --body-file - <<'EOF'\nsee #9\nEOF"),
            vec![here(7)]
        );
        assert_eq!(
            shell_call("gh pr checkout https://github.com/Acme/App/pull/31"),
            vec![at("Acme/App", 31)]
        );
        assert_eq!(
            shell_call("gh pr merge 5 -R acme/app --squash"),
            vec![at("acme/app", 5)]
        );
        assert_eq!(
            shell_call("GH_REPO=acme/app gh pr view 5"),
            vec![at("acme/app", 5)]
        );
        assert_eq!(
            shell_call("/opt/homebrew/bin/gh pr view '#6'"),
            vec![here(6)]
        );
    }

    /// Every written form of a repository override that named a repository
    /// before an unreadable one named nothing still names it.
    #[test]
    fn a_gh_repository_override_that_named_a_repository_still_does() {
        for written in [
            "acme/app",
            "Acme/App",
            "acme/app/",
            "acme/app.git",
            "github.com/acme/app",
            "https://github.com/acme/app",
            "http://github.com/acme/app",
            "https://github.com/acme/app/",
            "https://github.com/acme/app.git",
            "my.org/app",
        ] {
            let expected = written
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches("github.com/")
                .trim_end_matches('/')
                .trim_end_matches(".git")
                .to_owned();
            for command in [
                format!("gh pr view 5 -R {written}"),
                format!("gh pr view 5 --repo={written}"),
                format!("GH_REPO={written} gh pr view 5"),
                format!("gh browse 5 -R {written}"),
            ] {
                assert_eq!(shell_call(&command), vec![at(&expected, 5)], "{command}");
            }
        }
    }

    /// An override in force that names no repository this reading can place
    /// names nothing, rather than the folder's pull request of that number:
    /// an address with a user part, a server another host names, a path
    /// that is no repository. `-R` wins over `GH_REPO`, and an Enterprise
    /// server's `host/owner/name` names that server's repository.
    #[test]
    fn a_gh_repository_override_that_names_no_repository_names_nothing() {
        for command in [
            "gh pr view 12 -R https://other.example/acme/app/extra",
            "gh pr view 12 -R https://user:secret@github.com/acme/app",
            "gh pr view 12 -R git@github.com:acme/app.git",
            "gh pr view 12 -R gitlab.com/acme/app",
            "gh pr view 12 -R acme",
            "gh pr view 12 -R localhost:8080/acme/app",
            "GH_REPO=gitlab.com/acme/app gh pr view 12",
            "GH_REPO=https://user:secret@github.com/acme/app gh pr view 12",
            "GH_REPO=acme gh api repos/{owner}/{repo}/pulls/12",
            "GH_REPO=acme/app gh pr view 12 -R gitlab.com/acme/app",
        ] {
            assert_eq!(shell_call(command), vec![], "{command}");
        }
        assert_eq!(
            shell_call("GH_REPO=gitlab.com/acme/app gh pr view 12 -R acme/app"),
            vec![at("acme/app", 12)]
        );
        assert_eq!(
            shell_call("GH_REPO=acme/app gh api repos/{owner}/{repo}/pulls/12"),
            vec![at("acme/app", 12)]
        );
        assert_eq!(shell_call("GH_REPO= gh pr view 12"), vec![here(12)]);
        for written in ["ghe.example/acme/app", "https://GHE.example/acme/app.git"] {
            assert_eq!(
                shell_call(&format!("gh pr view 12 -R {written}")),
                vec![at("ghe.example/acme/app", 12)],
                "{written}"
            );
        }
    }

    #[test]
    fn a_gh_api_path_under_one_pull_request_names_it() {
        assert_eq!(
            shell_call("gh api repos/acme/app/pulls/44/comments --paginate"),
            vec![at("acme/app", 44)]
        );
        assert_eq!(
            shell_call("gh api -X POST /repos/{owner}/{repo}/issues/45/comments -f body=hi"),
            vec![here(45)]
        );
        assert_eq!(shell_call("gh api repos/acme/app/pulls?state=open"), vec![]);
        assert_eq!(shell_call("gh api graphql -f query=x"), vec![]);
    }

    #[test]
    fn a_listing_a_creation_and_text_that_mentions_an_address_name_nothing() {
        for command in [
            "gh pr list --state open",
            "gh pr status",
            "gh pr create --title fix --body 'Stacked on https://github.com/acme/app/pull/12'",
            "gh pr view",
            "git commit -m 'https://github.com/acme/app/pull/12'",
            "echo https://github.com/acme/app/pull/12",
            "hide request send w1:p2 --intent x --body https://github.com/acme/app/pull/12",
            "cat <<EOF\ngh pr view 12\nEOF",
            "# gh pr view 12",
            "python3 scripts/premerge-check.py 12",
            "aside repl \"openTab('https://github.com/acme/app/pull/12')\"",
        ] {
            assert_eq!(shell_call(command), vec![], "{command}");
        }
    }

    #[test]
    fn a_command_that_opens_or_fetches_an_address_names_it() {
        assert_eq!(
            shell_call("open https://github.com/acme/app/pull/3/files"),
            vec![at("acme/app", 3)]
        );
        assert_eq!(
            shell_call("hide browser open https://github.com/acme/app/pull/4 --wait"),
            vec![at("acme/app", 4)]
        );
        assert_eq!(
            tool_call(
                &json!({"url": "https://github.com/acme/app/pull/8", "prompt": "summarize"}),
                None
            ),
            vec![at("acme/app", 8)]
        );
        assert_eq!(
            tool_call(
                &json!({"url": "https://github.com/acme/app/pulls", "prompt": "see https://github.com/acme/app/pull/8"}),
                None
            ),
            vec![]
        );
        assert_eq!(
            tool_call(
                &json!({"owner": "acme", "repo": "app", "pull_number": 9}),
                None
            ),
            vec![at("acme/app", 9)]
        );
    }

    #[test]
    fn a_number_alone_belongs_to_the_folder_the_call_ran_in() {
        assert_eq!(
            shell_call("cd ../app-review && gh pr view 5"),
            vec![NamedPr {
                repository: None,
                number: 5,
                folder: Some("/work/app-review".into()),
                host: None,
            }]
        );
        assert_eq!(
            tool_call(
                &json!({"cmd": "gh pr diff 5", "workdir": "/work/other"}),
                Some("/work/app")
            ),
            vec![NamedPr {
                repository: None,
                number: 5,
                folder: Some("/work/other".into()),
                host: None,
            }]
        );
        assert_eq!(
            tool_call(&json!({"command": "gh pr view 5"}), None),
            vec![NamedPr {
                repository: None,
                number: 5,
                folder: None,
                host: None,
            }]
        );
        // A folder this reading cannot follow names no repository.
        for command in [
            "cd ~/projects/other && gh pr view 5",
            "cd \"$REPO\" && gh pr view 5",
            "cd - && gh pr view 5",
        ] {
            assert_eq!(shell_call(command), vec![], "{command}");
        }
        assert_eq!(
            tool_call(&json!({"command": "cd sub && gh pr view 5"}), None),
            vec![]
        );
        // A repository named outright does not depend on the folder.
        assert_eq!(
            shell_call("cd ~/x && gh pr view 5 --repo acme/app"),
            vec![at("acme/app", 5)]
        );
    }

    /// The review of #1859: the four pull requests mission-13's record lost
    /// were each made by a Bash call that wrote the body in a heredoc and
    /// ran `gh pr create`; a listing or a view makes nothing.
    #[test]
    fn a_creating_call_is_told_from_one_that_lists_or_views() {
        for command in [
            "cat > /tmp/body.md <<'EOF'\nStacked on https://github.com/acme/app/pull/12\nEOF\ngh pr create --repo acme/app --base main --head fix/x --title 'fix: x' --body-file /tmp/body.md",
            "cd /work/app && GH_REPO=acme/app gh pr create --fill",
            "gh pr new -t x -b y",
            "gh api -X POST repos/acme/app/pulls -f title=x -f head=a -f base=main",
            "gh api repos/{owner}/{repo}/pulls -f title=x",
            "gh api --method=post /repos/acme/app/pulls --input body.json",
        ] {
            assert!(
                creates_pull_request(None, &json!({ "command": command })),
                "{command}"
            );
        }
        assert!(creates_pull_request(
            None,
            &json!({"command": ["bash", "-lc", "gh pr create --fill"]})
        ));
        // The shape each of mission-13's #1810 to #1813 was made with.
        assert!(creates_pull_request(
            None,
            &json!({"command": "cd /work/app && B=/tmp/s/pr-body.md; cat > $B <<'EOF'\n## Summary\nSee https://github.com/acme/app/pull/12\nEOF\ngh pr create --repo acme/app --base main --head fix/x --title \"fix: x\" --body-file $B 2>&1 | tail -1"})
        ));
        // The review of #1859: what runs after `||` runs because the create
        // failed, so the call claims nothing.
        for command in [
            "gh pr create --fill || gh pr view 42 --json url --jq .url",
            "gh pr create --fill 2>/dev/null || gh pr list --head fix/x",
            "x=$(gh pr create --fill || gh pr view 42)",
        ] {
            assert!(
                !creates_pull_request(None, &json!({ "command": command })),
                "{command}"
            );
        }
        assert!(creates_pull_request(
            Some("mcp__github__create_pull_request"),
            &json!({"owner": "acme", "repo": "app", "title": "x"})
        ));
        for command in [
            "gh pr list --state open",
            "gh pr view 12",
            "gh api repos/acme/app/pulls",
            "gh api -X GET repos/acme/app/pulls -f state=open",
            "gh api -X POST repos/acme/app/pulls/12/comments -f body=x",
            "echo gh pr create",
            "cat <<EOF\ngh pr create\nEOF",
            "# gh pr create",
        ] {
            assert!(
                !creates_pull_request(None, &json!({ "command": command })),
                "{command}"
            );
        }
        assert!(!creates_pull_request(
            Some("mcp__github__get_pull_request"),
            &json!({"owner": "acme", "repo": "app", "pull_number": 3})
        ));
    }

    #[test]
    fn a_creating_calls_output_gives_back_the_address_it_made_alone() {
        let printed = "Warning: 1 uncommitted change\n\nCreating pull request for fix/x into main in acme/app\n\nhttps://github.com/acme/app/pull/1812\n";
        assert_eq!(
            created_addresses(&[printed]),
            [("acme/app".to_owned(), 1812)]
        );
        let answer = r#"{"html_url":"https://github.com/acme/app/pull/77","body":"Stacked on https://github.com/acme/app/pull/12","url":"https://api.github.com/repos/acme/app/pulls/77"}"#;
        assert_eq!(created_addresses(&[answer]), [("acme/app".to_owned(), 77)]);
        assert_eq!(
            created_addresses(&[
                "pull request create failed: see https://github.com/acme/app/pull/5 first"
            ]),
            []
        );
    }

    #[test]
    fn a_substitution_and_an_argv_call_are_read_as_their_commands() {
        assert_eq!(
            shell_call("body=$(gh pr view 21 --json body -q .body) && echo \"$body\""),
            vec![here(21)]
        );
        assert_eq!(
            tool_call(
                &json!({"command": ["bash", "-lc", "gh pr view 22"], "workdir": "/work/app"}),
                None
            ),
            vec![here(22)]
        );
        assert_eq!(
            tool_call(
                &json!({"command": ["gh", "pr", "view", "23"], "workdir": "/work/app"}),
                None
            ),
            vec![here(23)]
        );
    }

    /// D-20: a call that runs `git commit`, past git's own options and in a
    /// list, is a commit; another git command or a commit only named is not.
    #[test]
    fn a_git_commit_call_is_read_as_a_commit() {
        for command in [
            "git commit -m 'fix: x'",
            "git -C /work/app -c user.name=x commit --amend --no-edit",
            "cd /work/app && git add -A && git commit -m x && git push",
        ] {
            assert!(runs_git_commit(&json!({ "command": command })), "{command}");
        }
        assert!(runs_git_commit(
            &json!({ "command": ["git", "commit", "-m", "x"] })
        ));
        for command in [
            "git status",
            "git log --grep commit",
            "echo git commit",
            "gh pr create --fill",
        ] {
            assert!(
                !runs_git_commit(&json!({ "command": command })),
                "{command}"
            );
        }
    }

    fn on_gitlab(repository: Option<&str>, number: u64) -> NamedPr {
        NamedPr {
            repository: repository.map(str::to_owned),
            number,
            folder: repository.is_none().then(|| "/work/app".to_owned()),
            host: Some("gitlab".into()),
        }
    }

    /// `glab mr note` takes a note's id before the merge request on the
    /// commands that act on one note; an address with a user part names
    /// nothing, and a group with a dotted name is read as `glab` reads it.
    #[test]
    fn a_glab_call_names_the_merge_request_and_not_a_note_or_a_credential() {
        let project = Some("gitlab.com/acme/app");
        for (command, expected) in [
            ("glab mr note list 7", vec![on_gitlab(None, 7)]),
            ("glab mr note resolve 3f2a 7", vec![on_gitlab(None, 7)]),
            ("glab mr note draft list", vec![]),
            ("glab mr show 7 -R acme/app", vec![on_gitlab(project, 7)]),
            // Any user part, a Git login's included, is never read.
            ("glab mr view 7 -R git@gitlab.com:acme/app.git", vec![]),
            (
                "glab mr view 7 -R ssh://git@gitlab.com/acme/app.git",
                vec![],
            ),
            (
                "glab mr view 7 -R ssh://gitlab.com/acme/app.git",
                vec![on_gitlab(project, 7)],
            ),
            (
                "glab mr view 7 -R https://oauth2:token@gitlab.com/acme/app",
                vec![],
            ),
            (
                "glab mr view 7 -R my.group/app",
                vec![on_gitlab(Some("gitlab.com/my.group/app"), 7)],
            ),
            ("glab mr view 7 -R 12345", vec![]),
            (
                "GITLAB_HOST=https://gitlab.com glab mr view 7",
                vec![on_gitlab(None, 7)],
            ),
        ] {
            assert_eq!(shell_call(command), expected, "{command}");
        }
    }

    /// A short flag takes a value on one `glab mr` verb and is a switch on
    /// another: a number given as `update`'s description or `approve`'s SHA
    /// is not the merge request, and the merge request after `merge -d` or
    /// `view -s` is.
    #[test]
    fn a_glab_flags_value_is_read_by_its_verb_and_never_taken_for_the_merge_request() {
        for (command, expected) in [
            ("glab mr update -d 77 12", vec![on_gitlab(None, 12)]),
            (
                "glab mr update --description 77 12",
                vec![on_gitlab(None, 12)],
            ),
            ("glab mr update -t 99 -m 3 12 -y", vec![on_gitlab(None, 12)]),
            ("glab mr update -d 77", vec![]),
            ("glab mr merge -d 12", vec![on_gitlab(None, 12)]),
            ("glab mr merge -s -d -m 5 12", vec![on_gitlab(None, 12)]),
            ("glab mr approve -s 1234 12", vec![on_gitlab(None, 12)]),
            ("glab mr view -s 12", vec![on_gitlab(None, 12)]),
            ("glab mr view -p 2 12", vec![on_gitlab(None, 12)]),
            ("glab mr checkout -b 5 12", vec![on_gitlab(None, 12)]),
            (
                "glab mr note create --line 42 12 -m x",
                vec![on_gitlab(None, 12)],
            ),
        ] {
            assert_eq!(shell_call(command), expected, "{command}");
        }
    }

    #[test]
    fn a_glab_create_and_a_merge_request_post_make_one_and_a_listing_does_not() {
        for command in [
            "glab mr create --fill --yes",
            "glab mr new -f",
            "glab mr for 3",
            "glab api projects/:id/merge_requests -f source_branch=x -f target_branch=main -f title=x",
            "glab api -X POST projects/acme%2Fapp/merge_requests --input body.json",
            "git -C /work/app push -o merge_request.create origin x",
            "git push -omr.create origin x",
        ] {
            assert!(
                creates_pull_request(None, &json!({ "command": command })),
                "{command}"
            );
        }
        for command in [
            "glab mr list",
            "glab api projects/:id/merge_requests",
            "glab api -X POST projects/:id/merge_requests/3/notes -f body=x",
            "git push -o ci.skip origin x",
            "git push origin x || glab mr view 3",
            "git push -o merge_request.create origin x || glab mr view 3",
        ] {
            assert!(
                !creates_pull_request(None, &json!({ "command": command })),
                "{command}"
            );
        }
    }

    #[test]
    fn a_creating_calls_output_gives_back_a_merge_request_alone_or_after_remote() {
        let glab = "\nCreating merge request for x into main in acme/app\n\nhttps://gitlab.com/acme/sub/app/-/merge_requests/19\n";
        assert_eq!(
            created_addresses(&[glab]),
            [("gitlab.com/acme/sub/app".to_owned(), 19)]
        );
        let answer = r#"{"iid":20,"web_url":"https://gitlab.com/acme/app/-/merge_requests/20","description":"see https://gitlab.com/acme/app/-/merge_requests/3"}"#;
        assert_eq!(
            created_addresses(&[answer]),
            [("gitlab.com/acme/app".to_owned(), 20)]
        );
        let plain = "remote: To create a merge request for x, visit:\nremote:   https://gitlab.com/acme/app/-/merge_requests/new?merge_request%5Bsource_branch%5D=x\n";
        assert_eq!(created_addresses(&[plain]), []);
    }
}
