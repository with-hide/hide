//! The pull requests a tool call named: the ones a session acted on, as
//! opposed to an address that only passed through a tool's output.
//!
//! A tool's output prints whatever it lists (a browser's open tabs, `gh pr
//! list`, a log), so an address in it says only that the session saw the
//! pull request; that is evidence of creation when it is printed as GitHub
//! makes the pull request, and of nothing else. A call names a pull request
//! when it acts on that one: a `gh pr` command given its number or address,
//! a `gh api` path under `repos/<owner>/<name>/pulls/<number>`, a command
//! that opens or fetches its address, a tool whose argument is the address,
//! or a GitHub tool given its owner, name and number.
//!
//! The shell text is read by `hide-shell`, as the hooks read a call before it
//! runs: heredoc bodies, quotes and comments are text, so an address in a
//! commit message or a letter's body names nothing. A command hidden a layer
//! deeper (`bash -c`, a script, an alias) is not looked for.

use crate::{pull_request_addresses, sightings::is_name};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// A pull request a tool call named.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NamedPr {
    /// `owner/name` when the call said which repository: an address,
    /// `--repo`, a `repos/<owner>/<name>` path, a GitHub tool's arguments.
    /// `None` when it named only a number, which is a pull request of the
    /// repository the call ran in.
    pub repository: Option<String>,
    pub number: u64,
    /// For a number alone, the folder the call ran in when the record says
    /// it was not the session's own (a `cd` before the command, the tool's
    /// working directory); `None` is the session's folder.
    pub folder: Option<String>,
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

/// The keys a GitHub tool takes a pull request's number under.
const NUMBER_KEYS: [&str; 4] = ["pull_number", "pullNumber", "pr_number", "prNumber"];

/// The pull requests one tool call's input names. `folder` is where the
/// record says the session was when it made the call, when it says so.
pub(crate) fn tool_call(input: &Value, folder: Option<&str>) -> Vec<NamedPr> {
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
    for key in COMMAND_KEYS {
        match object.get(key) {
            Some(Value::String(text)) => shell(text, place.clone(), 0, &mut found),
            Some(Value::Array(argv)) => {
                let words: Option<Vec<String>> = argv
                    .iter()
                    .map(|word| word.as_str().map(str::to_owned))
                    .collect();
                match words.as_deref() {
                    // `["bash", "-lc", "<script>"]`: the script is the line.
                    Some([shell_program, flag, script])
                        if is_shell(shell_program)
                            && flag.starts_with('-')
                            && flag.contains('c') =>
                    {
                        shell(script, place.clone(), 0, &mut found);
                    }
                    Some(words) => {
                        let mut place = place.clone();
                        command(words, &mut place, &mut found);
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }
    // A tool whose argument is the address itself: a fetch, a browser's
    // navigation.
    for (key, value) in object {
        if COMMAND_KEYS.contains(&key.as_str()) {
            continue;
        }
        if let Some((repository, number)) = value.as_str().and_then(whole_address) {
            found.push(Some(repository), number, None);
        }
    }
    let owner = object.get("owner").and_then(Value::as_str);
    let name = object.get("repo").and_then(Value::as_str);
    let number = NUMBER_KEYS
        .iter()
        .find_map(|key| object.get(*key))
        .and_then(|value| match value {
            Value::Number(number) => number.as_u64(),
            Value::String(text) => pr_number(text),
            _ => None,
        });
    if let (Some(owner), Some(name), Some(number)) = (owner, name, number)
        && is_name(owner)
        && is_name(name)
        && number > 0
    {
        found.push(Some(format!("{owner}/{name}")), number, None);
    }
    found.named
}

#[derive(Default)]
struct Found {
    named: Vec<NamedPr>,
}

impl Found {
    fn push(&mut self, repository: Option<String>, number: u64, folder: Option<String>) {
        let folder = folder.filter(|_| repository.is_none());
        let named = NamedPr {
            repository,
            number,
            folder,
        };
        if !self.named.contains(&named) {
            self.named.push(named);
        }
    }
}

/// Where a command runs, as far as the call says.
#[derive(Clone)]
enum Place {
    /// The session's own folder.
    Session,
    At(PathBuf),
    /// A `cd` this reading cannot follow (home, `-`, a variable): a number
    /// named here belongs to no known repository.
    Unknown,
}

fn shell(text: &str, mut place: Place, depth: usize, found: &mut Found) {
    let line = hide_shell::shell_line(text);
    for words in &line.commands {
        command(words, &mut place, found);
    }
    if depth < SUBSTITUTION_DEPTH {
        for (_, text) in &line.substitutions {
            shell(text, place.clone(), depth + 1, found);
        }
    }
}

fn command(words: &[String], place: &mut Place, found: &mut Found) {
    let mut repository_variable = None;
    let mut at = 0;
    while let Some(word) = words.get(at) {
        if hide_shell::is_assignment(word) {
            if let Some(value) = word.strip_prefix("GH_REPO=") {
                repository_variable = repository_of(value);
            }
        } else if !hide_shell::is_prefix(word, true) {
            break;
        }
        at += 1;
    }
    let Some((first, args)) = words[at..].split_first() else {
        return;
    };
    match program(first) {
        "cd" | "pushd" => {
            *place = match args.iter().find(|arg| !arg.starts_with('-') || *arg == "-") {
                Some(target) => moved(place, target),
                None => Place::Unknown,
            };
        }
        "gh" => gh(args, repository_variable, place, found),
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

/// Every argument that is a pull request's address.
fn addresses(args: &[String], found: &mut Found) {
    for arg in args {
        if let Some((repository, number)) = whole_address(arg) {
            found.push(Some(repository), number, None);
        }
    }
}

/// `gh pr <verb> <target>`, `gh api <path>` and `gh browse <number>`.
fn gh(args: &[String], variable: Option<String>, place: &Place, found: &mut Found) {
    let Some((subcommand, rest)) = args.split_first() else {
        return;
    };
    let (target, repository) = match subcommand.as_str() {
        "pr" => {
            let Some((verb, rest)) = rest.split_first() else {
                return;
            };
            if !PR_VERBS.contains(&verb.as_str()) {
                return;
            }
            (
                first_positional(rest, PR_VALUE_FLAGS),
                flag_repository(rest),
            )
        }
        "browse" => (
            first_positional(rest, BROWSE_VALUE_FLAGS),
            flag_repository(rest),
        ),
        "api" => {
            if let Some(endpoint) = first_positional(rest, API_VALUE_FLAGS)
                && let Some((repository, number)) = api_path(endpoint)
            {
                name(repository, number, place, found);
            }
            return;
        }
        _ => return,
    };
    let Some(target) = target else {
        return;
    };
    if let Some((repository, number)) = whole_address(target) {
        found.push(Some(repository), number, None);
    } else if let Some(number) = pr_number(target.strip_prefix('#').unwrap_or(target)) {
        name(repository.or(variable), number, place, found);
    }
}

/// A number with its repository, or with where the call ran when it named
/// no repository.
fn name(repository: Option<String>, number: u64, place: &Place, found: &mut Found) {
    match (repository, place) {
        (Some(repository), _) => found.push(Some(repository), number, None),
        (None, Place::Session) => found.push(None, number, None),
        (None, Place::At(folder)) => {
            found.push(None, number, Some(folder.to_string_lossy().into_owned()));
        }
        (None, Place::Unknown) => {}
    }
}

/// The `gh pr` commands that act on one pull request; `list`, `status` and
/// `create` name none.
const PR_VERBS: [&str; 16] = [
    "view",
    "diff",
    "checkout",
    "co",
    "review",
    "comment",
    "merge",
    "edit",
    "close",
    "reopen",
    "ready",
    "checks",
    "update-branch",
    "lock",
    "unlock",
    "revert",
];

/// `gh pr` flags whose value is the next word, which a number there is not
/// the target: the repository and the free text a command takes.
const PR_VALUE_FLAGS: &[&str] = &[
    "-R",
    "--repo",
    "-b",
    "--body",
    "-F",
    "--body-file",
    "-t",
    "--title",
    "--subject",
    "-q",
    "--jq",
    "--json",
    "-T",
    "--template",
    "-B",
    "--base",
    "-H",
    "--head",
    "--branch",
    "--color",
    "--match-head-commit",
    "--author-email",
    "--add-label",
    "--remove-label",
    "--add-reviewer",
    "--remove-reviewer",
    "--add-assignee",
    "--remove-assignee",
    "--add-project",
    "--remove-project",
    "--milestone",
    "--reason",
    "--interval",
];

const BROWSE_VALUE_FLAGS: &[&str] = &["-R", "--repo", "-b", "--branch", "-c", "--commit"];

const API_VALUE_FLAGS: &[&str] = &[
    "-X",
    "--method",
    "-f",
    "--raw-field",
    "-F",
    "--field",
    "-H",
    "--header",
    "-q",
    "--jq",
    "-t",
    "--template",
    "--input",
    "--hostname",
    "--cache",
    "-p",
    "--preview",
];

/// The first word that is not a flag or a value-taking flag's value.
fn first_positional<'a>(args: &'a [String], value_flags: &[&str]) -> Option<&'a str> {
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

/// `-R owner/name`, `--repo owner/name`, `--repo=owner/name`, `-Rowner/name`.
fn flag_repository(args: &[String]) -> Option<String> {
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let value = match word.as_str() {
            "-R" | "--repo" => words.next().map(String::as_str),
            other => other
                .strip_prefix("--repo=")
                .or_else(|| other.strip_prefix("-R").filter(|rest| !rest.is_empty())),
        };
        if let Some(value) = value {
            return repository_of(value);
        }
    }
    None
}

/// `owner/name`, `github.com/owner/name` or its address; another host is
/// not GitHub's repository, and names nothing here.
fn repository_of(value: &str) -> Option<String> {
    let value = value
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let value = value.strip_prefix("github.com/").unwrap_or(value);
    let mut parts = value.trim_end_matches('/').split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    let name = name.strip_suffix(".git").unwrap_or(name);
    (is_name(owner) && is_name(name)).then(|| format!("{owner}/{name}"))
}

/// `repos/<owner>/<name>/pulls/<number>` (or `issues/<number>`, which GitHub
/// answers for a pull request too), with `{owner}`/`{repo}` or `:owner`/
/// `:repo` standing for the repository the call runs in.
fn api_path(endpoint: &str) -> Option<(Option<String>, u64)> {
    let path = endpoint
        .strip_prefix("https://api.github.com/")
        .unwrap_or(endpoint)
        .trim_start_matches('/');
    let path = path.split(['?', '#']).next()?;
    let mut parts = path.split('/');
    if parts.next()? != "repos" {
        return None;
    }
    let (owner, name) = (parts.next()?, parts.next()?);
    if !matches!(parts.next()?, "pulls" | "issues") {
        return None;
    }
    let number = pr_number(parts.next()?)?;
    let placeholder = |part: &str| part.starts_with('{') || part.starts_with(':');
    if placeholder(owner) || placeholder(name) {
        return Some((None, number));
    }
    (is_name(owner) && is_name(name)).then(|| (Some(format!("{owner}/{name}")), number))
}

/// A word that is a pull request's address and nothing else (a trailing
/// `/files`, `#discussion` or query is part of the address).
fn whole_address(word: &str) -> Option<(String, u64)> {
    let word = word.trim();
    if word.chars().any(char::is_whitespace) {
        return None;
    }
    let host = word
        .strip_prefix("https://")
        .or_else(|| word.strip_prefix("http://"))
        .unwrap_or(word);
    let host = host.strip_prefix("www.").unwrap_or(host);
    if !host.starts_with("github.com/") {
        return None;
    }
    pull_request_addresses(word).into_iter().next()
}

fn pr_number(text: &str) -> Option<u64> {
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse::<u64>().ok())
        .flatten()
        .filter(|number| *number > 0)
}

/// A command word's program name: its last path part, without Windows'
/// `.exe`.
fn program(word: &str) -> &str {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    name.strip_suffix(".exe").unwrap_or(name)
}

fn is_shell(word: &str) -> bool {
    matches!(program(word), "bash" | "sh" | "zsh" | "dash" | "ksh")
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

    fn shell_call(command: &str) -> Vec<NamedPr> {
        tool_call(&json!({ "command": command }), Some("/work/app"))
    }

    fn here(number: u64) -> NamedPr {
        NamedPr {
            repository: None,
            number,
            folder: Some("/work/app".into()),
        }
    }

    fn at(repository: &str, number: u64) -> NamedPr {
        NamedPr {
            repository: Some(repository.into()),
            number,
            folder: None,
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
            }]
        );
        assert_eq!(
            tool_call(&json!({"command": "gh pr view 5"}), None),
            vec![NamedPr {
                repository: None,
                number: 5,
                folder: None,
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
}
