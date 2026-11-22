//! What a shell command asks a code host to do: merge a change request or
//! check one out, on GitHub (`gh`) or GitLab (`glab`, and a `git push` whose
//! push option arms a merge).
//!
//! One reading for the project guardrails (`git.rs`) and the mission
//! worker's merge bar (`plan_record`), so a form one of them catches the
//! other catches too.

/// The code host whose change request a call merges or checks out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Host {
    GitHub,
    GitLab,
}

impl Host {
    /// What the host calls a change request, in a refusal's sentence.
    pub(crate) fn change(self) -> &'static str {
        match self {
            Self::GitHub => "pull request",
            Self::GitLab => "merge request",
        }
    }
}

/// How a call merges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Route {
    /// The CLI's merge command: `gh pr merge`, `glab mr merge` or `accept`.
    Command,
    /// The host's REST merge endpoint through `gh api` or `glab api`.
    Api,
    /// `glab mr create --auto-merge` (or its alias `glab mr new`): the new
    /// merge request merges by itself
    /// once its checks pass.
    CreateAutoMerge,
    /// A `git push` carrying GitLab's `merge_request.auto_merge` or
    /// `merge_request.merge_when_pipeline_succeeds` push option.
    PushOption,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostCall {
    Merge { host: Host, route: Route },
    Checkout,
}

/// The sentence a project guardrail's refusal says a merging call did.
pub(crate) fn merge_sentence(host: Host, route: Route) -> String {
    let change = host.change();
    match (host, route) {
        (_, Route::Command) => format!("This call merges a {change}"),
        (Host::GitHub, Route::Api) => {
            format!("This call merges a {change} through GitHub's API")
        }
        (Host::GitLab, Route::Api) => {
            format!("This call merges a {change} through GitLab's API")
        }
        (_, Route::CreateAutoMerge) => {
            format!("This call opens a {change} set to merge by itself (--auto-merge)")
        }
        (_, Route::PushOption) => format!(
            "This call pushes with a GitLab push option that sets a {change} to merge by itself"
        ),
    }
}

/// What `program` (a name [`hide_shell::Syntax::program`] gave) run with
/// `args` asks a code host to do, if it merges or checks out a change
/// request. A `git` call's `args` start with Git's own options.
pub(crate) fn host_call(program: &str, args: &[String]) -> Option<HostCall> {
    match program {
        "gh" => cli(Host::GitHub, args),
        "glab" => cli(Host::GitLab, args),
        "git" => git_arms_merge(args).then_some(HostCall::Merge {
            host: Host::GitLab,
            route: Route::PushOption,
        }),
        _ => None,
    }
}

fn cli(host: Host, args: &[String]) -> Option<HostCall> {
    // `gh` and `glab` find their command as cobra does: a flag before it
    // without `=` (`--sha x`, `-y x`) takes the next word unless cobra knows
    // it as one that takes none. Both readings are judged, so a merge either
    // one finds is refused, never one that cobra finds and this does not.
    [true, false]
        .into_iter()
        .find_map(|takes| judge(host, args, &cli_words(args, takes)))
}

/// The first two command words of a `gh` or `glab` call and their places in
/// `args`. With `takes`, a flag without `=` that is `--name` or `-x` takes
/// the next word (cobra's reading of a flag its parent command does not
/// declare); without, only `-R`/`--repo` do.
fn cli_words(args: &[String], takes: bool) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        if words.len() == 2 || arg == "--" {
            break;
        }
        if arg.starts_with('-') && arg != "-" {
            let valued = matches!(arg.as_str(), "-R" | "--repo")
                || (takes
                    && !arg.contains('=')
                    && (arg.starts_with("--") || arg.len() == 2)
                    && !matches!(arg.as_str(), "-h" | "--help"));
            at += if valued { 2 } else { 1 };
            continue;
        }
        words.push((at, arg.as_str()));
        at += 1;
    }
    words
}

fn judge(host: Host, args: &[String], words: &[(usize, &str)]) -> Option<HostCall> {
    let merge = |route| Some(HostCall::Merge { host, route });
    let names: Vec<&str> = words.iter().map(|(_, word)| *word).collect();
    let places: Vec<usize> = words.iter().map(|(at, _)| *at).collect();
    match (host, names.as_slice()) {
        (_, ["api", ..]) if api_merges(host, args, places[0]) => merge(Route::Api),
        (Host::GitHub, ["pr", "merge"]) | (Host::GitLab, ["mr", "merge" | "accept"]) => {
            merge(Route::Command)
        }
        // `new` is `glab mr create`'s alias.
        (Host::GitLab, ["mr", "create" | "new"]) if create_auto_merges(args, &places) => {
            merge(Route::CreateAutoMerge)
        }
        // `glab mr co` is no alias: glab 1.130.0 answers it with `glab mr`'s
        // help.
        (Host::GitHub, ["pr", "checkout" | "co"]) | (Host::GitLab, ["mr", "checkout"]) => {
            Some(HostCall::Checkout)
        }
        _ => None,
    }
}

/// A value read from a word that still holds an expansion (`$opt`,
/// `$env:OPT`, a backtick), which only the shell knows.
fn unknown(value: &str) -> bool {
    value.contains(['$', '`'])
}

/// The options of `gh api` and `glab api` that take a value, the short ones
/// by letter; any other option takes none, so its value would stay a
/// positional word and be judged as an endpoint (more refused, never less).
const API_VALUE_OPTIONS: &[&str] = &[
    "--field",
    "--raw-field",
    "--header",
    "--hostname",
    "--input",
    "--form",
    "--output",
    "--placeholder",
    "--cache",
    "--jq",
    "--preview",
    "--template",
];
const API_VALUE_LETTERS: &[char] = &['F', 'f', 'H', 'q', 'p', 't'];

/// Whether `gh api` or `glab api` (the `api` word at `api_at` in `args`)
/// calls the host's merge endpoint with `PUT`: `repos/<owner>/<repo>/pulls/<n>/merge`
/// on GitHub, `projects/<id>/merge_requests/<iid>/merge` on GitLab, as a
/// positional word (an option's value, such as a field's, is no endpoint),
/// in any spelling of the method flag (`-X PUT`, `-XPUT`, `-X=PUT`, `-iX PUT`,
/// `--method PUT`, `--method=put`). A `GET` there only asks whether it is
/// merged. A method held in a variable is unknown, and read as `PUT`.
fn api_merges(host: Host, args: &[String], api_at: usize) -> bool {
    let collection = match host {
        Host::GitHub => "pulls",
        Host::GitLab => "merge_requests",
    };
    let mut endpoints = Vec::new();
    let mut methods = Vec::new();
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        let next = args.get(at + 1).map(String::as_str);
        at += 1;
        if at - 1 == api_at {
            continue;
        }
        if let Some(value) = arg.strip_prefix("--method=") {
            methods.push(value);
        } else if arg == "--method" {
            methods.extend(next);
            at += 1;
        } else if API_VALUE_OPTIONS.contains(&arg.as_str()) {
            at += 1;
        } else if let Some(letters) = arg.strip_prefix('-').filter(|rest| !rest.starts_with('-')) {
            // A short cluster: the first letter that takes a value takes the
            // rest of the cluster, or the next word.
            for (index, letter) in letters.char_indices() {
                if letter != 'X' && !API_VALUE_LETTERS.contains(&letter) {
                    continue;
                }
                let rest = &letters[index + letter.len_utf8()..];
                let value = match rest.strip_prefix('=').unwrap_or(rest) {
                    "" => {
                        at += 1;
                        next
                    }
                    value => Some(value),
                };
                if letter == 'X' {
                    methods.extend(value);
                }
                break;
            }
        } else if !arg.starts_with('-') {
            endpoints.push(arg.as_str());
        }
    }
    let endpoint = endpoints.iter().any(|arg| {
        let path = arg.split(['?', '#']).next().unwrap_or_default();
        let path = path.split_once("://").map_or(path, |(_, rest)| rest);
        let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        matches!(segments.as_slice(), [.., kind, _, "merge"] if *kind == collection)
    });
    endpoint
        && methods
            .iter()
            .any(|method| method.eq_ignore_ascii_case("PUT") || unknown(method))
}

/// The options of `glab mr create` that take a value, the short ones by
/// letter; a value is never read as an option (`-d --auto-merge` is a
/// description).
const CREATE_VALUE_OPTIONS: &[&str] = &[
    "--assignee",
    "--attach",
    "--description",
    "--description-file",
    "--head",
    "--label",
    "--milestone",
    "--related-issue",
    "--repo",
    "--reviewer",
    "--source-branch",
    "--target-branch",
    "--template",
    "--title",
];
const CREATE_VALUE_LETTERS: &[char] = &['a', 'd', 'H', 'l', 'm', 'i', 'R', 's', 'b', 't'];

/// Whether `glab mr create` with `args` (its command words at `places`) ends
/// with `--auto-merge` set: a boolean flag whose last spelling counts
/// (`--auto-merge=false` turns it off).
fn create_auto_merges(args: &[String], places: &[usize]) -> bool {
    let mut auto_merge = false;
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        at += 1;
        if places.contains(&(at - 1)) {
            continue;
        }
        if arg == "--" {
            break;
        }
        if let Some(value) = arg.strip_prefix("--auto-merge") {
            match value.strip_prefix('=') {
                None if value.is_empty() => auto_merge = true,
                None => {}
                Some(value) => {
                    auto_merge = unknown(value)
                        || !matches!(value.to_ascii_lowercase().as_str(), "false" | "f" | "0");
                }
            }
        } else if CREATE_VALUE_OPTIONS.contains(&arg.as_str()) {
            at += 1;
        } else if let Some(letters) = arg.strip_prefix('-').filter(|rest| !rest.starts_with('-')) {
            // A short cluster whose last letter takes a value takes the next
            // word; one that ends earlier holds its value.
            if letters.ends_with(CREATE_VALUE_LETTERS)
                && !letters[..letters.len() - 1].contains(CREATE_VALUE_LETTERS)
            {
                at += 1;
            }
        }
    }
    auto_merge
}

/// Whether a push option sets the merge request to merge by itself, or may:
/// one held in an expansion is unknown, and read as one that does.
fn arms(option: &str) -> bool {
    merging_option(option) || option.split('=').next().is_some_and(unknown)
}

fn push_option_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("push.pushOption")
}

/// Whether a `git` call (its args from Git's own options on) pushes with, or
/// stores for later pushes, a push option that makes GitLab merge the merge
/// request: `-o`/`--push-option` on `git push`, `-c push.pushOption=` or
/// `--config-env push.pushOption=<variable>` (a value only the shell knows)
/// on a push, or `git config` writing `push.pushOption`. An option that only
/// opens a merge request (`merge_request.create`) arms nothing, and neither
/// does one given to a command that pushes nothing (`git -c ... status`) or
/// a configuration read (`git config --get push.pushOption`).
fn git_arms_merge(args: &[String]) -> bool {
    let mut configured = false;
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        let next = args.get(at + 1).map(String::as_str);
        match arg.as_str() {
            "-c" => {
                configured |= next
                    .and_then(|value| value.split_once('='))
                    .is_some_and(|(key, value)| push_option_key(key) && arms(value));
                at += 2;
            }
            "--config-env" => {
                configured |= next
                    .and_then(|value| value.split_once('='))
                    .is_some_and(|(key, _)| push_option_key(key));
                at += 2;
            }
            other if other.starts_with("--config-env=") => {
                configured |= other["--config-env=".len()..]
                    .split_once('=')
                    .is_some_and(|(key, _)| push_option_key(key));
                at += 1;
            }
            "-C" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path"
            | "--super-prefix" => at += 2,
            other if other.starts_with('-') => at += 1,
            _ => break,
        }
    }
    let Some(subcommand) = args.get(at) else {
        return false;
    };
    let rest = &args[at + 1..];
    match subcommand.as_str() {
        "push" => configured || push_options(rest).any(arms),
        "config" => config_stores_merging_option(rest),
        _ => false,
    }
}

/// Whether `git config <args>` writes a push option that arms a merge into
/// `push.pushOption`: the key, then such a value after it, in a call that
/// neither reads, lists, unsets nor edits.
fn config_stores_merging_option(args: &[String]) -> bool {
    const NOT_WRITES: &[&str] = &[
        "get",
        "list",
        "unset",
        "remove-section",
        "rename-section",
        "edit",
        "--get",
        "--get-all",
        "--get-regexp",
        "--get-urlmatch",
        "-l",
        "--list",
        "--unset",
        "--unset-all",
        "--remove-section",
        "--rename-section",
        "-e",
        "--edit",
    ];
    if args.iter().any(|arg| NOT_WRITES.contains(&arg.as_str())) {
        return false;
    }
    args.iter()
        .position(|arg| push_option_key(arg))
        .is_some_and(|key| args[key + 1..].iter().any(|value| arms(value)))
}

/// The values of a `git push`'s `-o`/`--push-option` options.
fn push_options(args: &[String]) -> impl Iterator<Item = &str> {
    let end = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    let args = &args[..end];
    args.iter().enumerate().filter_map(|(at, arg)| {
        let next = || args.get(at + 1).map(String::as_str);
        if arg == "--push-option" {
            return next();
        }
        if let Some(value) = arg.strip_prefix("--push-option=") {
            return Some(value);
        }
        // `-o` takes the rest of its cluster, or the next word.
        let letters = arg
            .strip_prefix('-')
            .filter(|rest| !rest.starts_with('-'))?;
        let (_, value) = letters.split_once('o')?;
        if value.is_empty() {
            next()
        } else {
            Some(value)
        }
    })
}

/// Whether a GitLab push option sets the merge request to merge by itself:
/// `merge_request.auto_merge`, or `merge_request.merge_when_pipeline_succeeds`
/// (deprecated in GitLab 17.11), under either namespace GitLab reads
/// (`mr.` is its alias), with or without a value.
fn merging_option(option: &str) -> bool {
    let key = option
        .split('=')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let Some((namespace, name)) = key.split_once('.') else {
        return false;
    };
    matches!(namespace, "merge_request" | "mr")
        && matches!(name, "auto_merge" | "merge_when_pipeline_succeeds")
}
