//! gitlab.com's part of a session's reading: `glab mr <verb> <target>`,
//! `glab api projects/<project>/merge_requests/<iid>`, a `git push` option
//! that makes a merge request, and
//! `gitlab.com/<group>[/<subgroup>...]/<project>/-/merge_requests/<iid>`.
//!
//! An issue and a merge request share no number space on GitLab, so a
//! number is read only where the command or the address says it is a merge
//! request: `12` or `!12` after a `glab mr` verb, never `#12`. A project
//! named by its numeric id, a server other than gitlab.com, or an address
//! with a user part (which can carry a credential) names nothing.

use super::{SessionHost, assigned, carries_user, is_name};
use crate::named_prs::{Found, Place, first_positional, flag_value, name, pr_number};

pub(super) struct Gitlab;

pub(super) static GITLAB: Gitlab = Gitlab;

const KEY: &str = "gitlab";

const SERVER: &str = "gitlab.com";

/// What sits between a project's path and a merge request's number in its
/// address.
const MERGE_REQUESTS: &str = "/-/merge_requests/";

/// The most groups and subgroups gitlab.com nests a project under (20),
/// plus the project.
const MAX_DEPTH: usize = 21;

impl SessionHost for Gitlab {
    fn key(&self) -> &'static str {
        KEY
    }

    fn program(&self) -> &'static str {
        "glab"
    }

    fn names_server(&self, server: &str) -> bool {
        server == SERVER
    }

    fn command(&self, args: &[String], assignments: &[&str], place: &Place, found: &mut Found) {
        // `glab` reads the server from these before the checkout's remote; a
        // call sent to another server names none of gitlab.com's.
        if ["GITLAB_HOST", "GL_HOST"]
            .iter()
            .filter_map(|variable| assigned(assignments, variable))
            .any(|server| !is_server(server))
        {
            return;
        }
        let Some((subcommand, rest)) = args.split_first() else {
            return;
        };
        match subcommand.as_str() {
            "mr" => mr(rest, place, found),
            "api" => api(rest, place, found),
            _ => {}
        }
    }

    fn address(&self, word: &str) -> Option<(String, u64)> {
        let rest = word
            .strip_prefix("https://")
            .or_else(|| word.strip_prefix("http://"))
            .unwrap_or(word);
        change_request_after(rest.strip_prefix(SERVER)?.strip_prefix('/')?)
    }

    /// The first distinct ones, as GitHub's scan keeps.
    fn addresses_in(&self, text: &str) -> Vec<(usize, String, u64)> {
        let marker = "gitlab.com/";
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(at) = text[from..].find(marker) {
            let start = from + at;
            from = start + marker.len();
            // `notgitlab.com/` is another server.
            let joined = text[..start]
                .bytes()
                .next_back()
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'));
            if joined || carries_user(text, start) {
                continue;
            }
            if let Some((repository, number)) = change_request_after(&text[from..])
                && !crate::sightings::keep_distinct(&mut found, (start, repository, number))
            {
                break;
            }
        }
        found
    }

    /// No GitLab tool's arguments are known yet: an address among them is
    /// read as any tool's.
    fn tool_arguments(
        &self,
        _object: &serde_json::Map<String, serde_json::Value>,
        _found: &mut Found,
    ) {
    }

    /// `web_url`, as the merge requests API answers a create.
    fn made_field(&self) -> &'static str {
        "web_url"
    }

    /// No GitLab tool's name is known yet: a merge request made through one
    /// is found by its address as any printed one is.
    fn creating_tool(&self, _name: &str) -> bool {
        false
    }

    /// `merge_request.create` and its short form `mr.create`, both checked
    /// on the test project: the push answers with the merge request's
    /// address on a `remote:` line.
    fn creating_push_option(&self, option: &str) -> bool {
        matches!(option, "merge_request.create" | "mr.create")
    }
}

/// The repository's stored name and the number of the merge request whose
/// address continues `<path>/-/merge_requests/<iid>` after the server.
fn change_request_after(after_server: &str) -> Option<(String, u64)> {
    let at = after_server.find(MERGE_REQUESTS)?;
    let path = project_path(&after_server[..at])?;
    let digits: String = after_server[at + MERGE_REQUESTS.len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let number = digits.parse::<u64>().ok().filter(|number| *number > 0)?;
    Some((format!("{SERVER}/{path}"), number))
}

/// A project's path: a group, any subgroups and the project, each a name.
fn project_path(path: &str) -> Option<&str> {
    let parts: Vec<&str> = path.split('/').collect();
    (parts.len() >= 2 && parts.len() <= MAX_DEPTH && parts.iter().all(|part| is_name(part)))
        .then_some(path)
}

/// Whether a server word (`gitlab.com`, `https://gitlab.com/`) is
/// gitlab.com.
fn is_server(value: &str) -> bool {
    let value = value
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let value = value.strip_suffix(":443").unwrap_or(value);
    value.eq_ignore_ascii_case(SERVER)
}

/// The `glab mr` commands that act on one merge request, with the aliases
/// `glab` takes for them; `list` and `create` name none.
const MR_VERBS: [&str; 19] = [
    "approve",
    "approvers",
    "checkout",
    "close",
    "delete",
    "diff",
    "issues",
    "merge",
    "accept",
    "note",
    "rebase",
    "reopen",
    "revoke",
    "subscribe",
    "todo",
    "unsubscribe",
    "update",
    "view",
    "show",
];

/// The `glab mr <verb>` flags whose value is the next word, so a number
/// there is not the target, as `glab mr <verb> --help` lists them: a short
/// flag means one thing on one verb and another on the next (`-d` is
/// `update`'s description and `merge`'s switch to remove the source branch,
/// `-s` `approve`'s SHA and `view`'s switch), so each verb has its own. A
/// flag a verb does not list is read as a switch.
fn value_flags(verb: &str) -> &'static [&'static str] {
    match verb {
        "update" => &[
            "-R",
            "--repo",
            "-a",
            "--assignee",
            "--attach",
            "-d",
            "--description",
            "--description-file",
            "-l",
            "--label",
            "-m",
            "--milestone",
            "--reviewer",
            "--state",
            "--target-branch",
            "-t",
            "--title",
            "-u",
            "--unlabel",
        ],
        "merge" | "accept" => &[
            "-R",
            "--repo",
            "-m",
            "--message",
            "--sha",
            "--squash-message",
        ],
        "approve" => &["-R", "--repo", "-s", "--sha"],
        "view" | "show" => &[
            "-R",
            "--repo",
            "--jq",
            "-F",
            "--output",
            "-p",
            "--page",
            "-P",
            "--per-page",
        ],
        "checkout" => &[
            "-R",
            "--repo",
            "-b",
            "--branch",
            "-u",
            "--set-upstream-to",
            "--worktree",
        ],
        "diff" => &["-R", "--repo", "--color"],
        "approvers" => &["-R", "--repo", "--jq", "-F", "--output"],
        "note" => &[
            "-R",
            "--repo",
            "--attach",
            "--file",
            "--line",
            "-m",
            "--message",
            "--old-line",
            "--reply",
            "--jq",
            "-F",
            "--output",
            "--state",
            "-t",
            "--type",
        ],
        _ => &["-R", "--repo"],
    }
}

/// `glab mr note` subcommands whose first word is a note or discussion
/// id, before the merge request.
const NOTE_ID_FIRST: [&str; 4] = ["delete", "reopen", "resolve", "update"];

fn mr(args: &[String], place: &Place, found: &mut Found) {
    let Some((verb, rest)) = args.split_first() else {
        return;
    };
    if matches!(verb.as_str(), "create" | "new" | "for") {
        found.creates = true;
        return;
    }
    if !MR_VERBS.contains(&verb.as_str()) {
        return;
    }
    let repository = match flag_value(rest, "-R", "--repo") {
        Some(value) => match repository_of(value) {
            Some(repository) => Some(repository),
            // A project this reading cannot place is not the folder's.
            None => return,
        },
        None => None,
    };
    let positionals = positionals(rest, value_flags(verb));
    let target = if verb == "note" {
        match positionals.first().map(|word| word.as_str()) {
            Some("create" | "list") => positionals.get(1),
            Some(sub) if NOTE_ID_FIRST.contains(&sub) => positionals.get(2),
            Some("draft") => None,
            _ => positionals.first(),
        }
    } else {
        positionals.first()
    };
    let Some(target) = target else {
        return;
    };
    if let Some((repository, number)) = GITLAB.address(target) {
        found.push(KEY, Some(repository), number, None);
    } else if let Some(number) = pr_number(target.strip_prefix('!').unwrap_or(target)) {
        name(KEY, repository, number, place, found);
    }
}

/// The words that are not flags or a value-taking flag's value, in order;
/// everything after `--` is one.
fn positionals<'a>(args: &'a [String], value_flags: &[&str]) -> Vec<&'a String> {
    let mut out = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            out.extend(words);
            break;
        }
        if word.starts_with('-') && word.len() > 1 {
            if value_flags.contains(&word.as_str()) {
                words.next();
            }
            continue;
        }
        out.push(word);
    }
    out
}

/// `-R` / `--repo`: `group/project`, `group/sub/project`, its gitlab.com
/// address or its `ssh://` address. Another server's, or any address with a
/// user part, is none.
fn repository_of(value: &str) -> Option<String> {
    let value = value.trim();
    // A user part, a Git login's included, is never read (it can carry a
    // credential).
    if value.contains('@') {
        return None;
    }
    let path = if let Some(rest) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    {
        let (server, path) = rest.split_once('/')?;
        if !is_server(server) {
            return None;
        }
        path
    } else if let Some(rest) = value.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/')?;
        if !is_server(authority.split(':').next()?) {
            return None;
        }
        path
    } else {
        value.strip_prefix("gitlab.com/").unwrap_or(value)
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    project_path(path).map(|path| format!("{SERVER}/{path}"))
}

const API_VALUE_FLAGS: &[&str] = &[
    "-X",
    "--method",
    "-f",
    "--raw-field",
    "-F",
    "--field",
    "-H",
    "--header",
    "--input",
    "--form",
    "--hostname",
    "--jq",
    "--output",
];

/// `glab api projects/<project>/merge_requests/<iid>[/...]`, the project
/// being `:id` or `:fullpath` (the folder's), or its path with `/` written
/// `%2F`; and a POST to `projects/<project>/merge_requests`, which makes
/// one. A numeric project id names nothing.
fn api(args: &[String], place: &Place, found: &mut Found) {
    if flag_value(args, "--hostname", "--hostname").is_some_and(|server| !is_server(server)) {
        return;
    }
    let Some(endpoint) = first_positional(args, API_VALUE_FLAGS) else {
        return;
    };
    let path = endpoint
        .strip_prefix("https://gitlab.com/api/v4/")
        .unwrap_or(endpoint)
        .trim_start_matches('/');
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let ["projects", project, "merge_requests", rest @ ..] = parts.as_slice() else {
        return;
    };
    let repository = match *project {
        ":id" | ":fullpath" => None,
        project if project.bytes().all(|byte| byte.is_ascii_digit()) => return,
        project => {
            let decoded = project.replace("%2F", "/").replace("%2f", "/");
            let Some(path) = project_path(&decoded) else {
                return;
            };
            Some(format!("{SERVER}/{path}"))
        }
    };
    match rest.first() {
        None => {
            if crate::named_prs::api_posts(
                args,
                &["-f", "--raw-field", "-F", "--field", "--input", "--form"],
            ) {
                found.creates = true;
            }
        }
        Some(number) => {
            if let Some(number) = pr_number(number) {
                name(KEY, repository, number, place, found);
            }
        }
    }
}
