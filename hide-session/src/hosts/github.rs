//! GitHub's part of a session's reading: `gh pr <verb> <target>`,
//! `gh api <path>`, `gh browse <number>`, `github.com/<owner>/<name>/pull/N`
//! and the GitHub MCP server's create tool.

use serde_json::Value;

use super::{SessionHost, assigned, is_name};
use crate::named_prs::{Found, Place, first_positional, name, pr_number};

pub(super) struct Github;

pub(super) static GITHUB: Github = Github;

const KEY: &str = "github";

const SERVER: &str = "github.com/";

/// The keys a GitHub tool takes a pull request's number under.
const NUMBER_KEYS: [&str; 4] = ["pull_number", "pullNumber", "pr_number", "prNumber"];

impl SessionHost for Github {
    fn key(&self) -> &'static str {
        KEY
    }

    fn program(&self) -> &'static str {
        "gh"
    }

    fn command(&self, args: &[String], assignments: &[&str], place: &Place, found: &mut Found) {
        let variable = assigned(assignments, "GH_REPO").and_then(repository_of);
        gh(args, variable, place, found);
    }

    fn address(&self, word: &str) -> Option<(String, u64)> {
        let host = word
            .strip_prefix("https://")
            .or_else(|| word.strip_prefix("http://"))
            .unwrap_or(word);
        let host = host.strip_prefix("www.").unwrap_or(host);
        if !host.starts_with(SERVER) {
            return None;
        }
        self.addresses_in(word)
            .into_iter()
            .next()
            .map(|(_, repository, number)| (repository, number))
    }

    fn addresses_in(&self, text: &str) -> Vec<(usize, String, u64)> {
        addresses_at(text, "pull")
    }

    /// A GitHub tool given its owner, name and number.
    fn tool_arguments(&self, object: &serde_json::Map<String, Value>, found: &mut Found) {
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
            found.push(KEY, Some(format!("{owner}/{name}")), number, None);
        }
    }

    fn made_field(&self) -> &'static str {
        "html_url"
    }

    /// `create_pull_request`, as the GitHub MCP server names it, under any
    /// prefix.
    fn creating_tool(&self, name: &str) -> bool {
        name.ends_with("create_pull_request")
    }

    /// GitHub makes no pull request from a push.
    fn creating_push_option(&self, _option: &str) -> bool {
        false
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
            if matches!(verb.as_str(), "create" | "new") {
                found.creates = true;
                return;
            }
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
            let endpoint = first_positional(rest, API_VALUE_FLAGS);
            if let Some(endpoint) = endpoint
                && let Some((repository, number)) = api_path(endpoint)
            {
                name(KEY, repository, number, place, found);
            }
            if endpoint.is_some_and(is_pulls_path) && api_posts(rest) {
                found.creates = true;
            }
            return;
        }
        _ => return,
    };
    let Some(target) = target else {
        return;
    };
    if let Some((repository, number)) = GITHUB.address(target) {
        found.push(KEY, Some(repository), number, None);
    } else if let Some(number) = pr_number(target.strip_prefix('#').unwrap_or(target)) {
        name(KEY, repository.or(variable), number, place, found);
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

/// `-R owner/name`, `--repo owner/name`, `--repo=owner/name`, `-Rowner/name`.
fn flag_repository(args: &[String]) -> Option<String> {
    crate::named_prs::flag_value(args, "-R", "--repo").and_then(repository_of)
}

/// `owner/name`, `github.com/owner/name` or its address; another host is
/// not GitHub's repository, and names nothing here.
fn repository_of(value: &str) -> Option<String> {
    let value = value
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let value = value.strip_prefix(SERVER).unwrap_or(value);
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

/// `repos/<owner>/<name>/pulls`, the path a pull request is made at.
fn is_pulls_path(endpoint: &str) -> bool {
    let path = endpoint
        .strip_prefix("https://api.github.com/")
        .unwrap_or(endpoint)
        .trim_start_matches('/');
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    matches!(parts.as_slice(), ["repos", _, _, "pulls"])
}

/// Whether a `gh api` call sends a POST: `-X POST` or `--method POST`, or
/// with no method named, a field or an input, which `gh api` sends as one.
fn api_posts(args: &[String]) -> bool {
    crate::named_prs::api_posts(args, &["-f", "--raw-field", "-F", "--field", "--input"])
}

/// The issue addresses in `text`, each with where it starts: issues are
/// read on GitHub only.
pub(super) fn issue_addresses_in(text: &str) -> Vec<(usize, String, u64)> {
    addresses_at(text, "issues")
}

/// The first [`MAX_SIGHTINGS_PER_OUTPUT`] distinct
/// `github.com/<owner>/<repo>/<kind>/<number>` in `text`, each with where it
/// starts.
fn addresses_at(text: &str, kind: &str) -> Vec<(usize, String, u64)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(SERVER) {
        let start = from + at;
        from = start + SERVER.len();
        if let Some((repository, number)) = parse_address(&text[from..], kind)
            && !crate::sightings::keep_distinct(&mut found, (start, repository, number))
        {
            break;
        }
    }
    found
}

fn parse_address(after_host: &str, kind: &str) -> Option<(String, u64)> {
    let mut parts = after_host.splitn(4, '/');
    let owner = parts.next().filter(|part| is_name(part))?;
    let name = parts.next().filter(|part| is_name(part))?;
    if parts.next()? != kind {
        return None;
    }
    let digits: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let number = digits.parse::<u64>().ok().filter(|number| *number > 0)?;
    Some((format!("{owner}/{name}"), number))
}
