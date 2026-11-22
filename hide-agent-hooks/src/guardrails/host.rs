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
    /// `glab mr create --auto-merge`: the new merge request merges by itself
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
    // The command's first two words, past `-R`/`--repo` and their value.
    let mut words: Vec<&str> = Vec::new();
    let mut rest = args.iter().map(String::as_str);
    while words.len() < 2
        && let Some(arg) = rest.next()
    {
        match arg {
            "-R" | "--repo" => {
                rest.next();
            }
            flag if flag.starts_with('-') => {}
            word => words.push(word),
        }
    }
    let merge = |route| Some(HostCall::Merge { host, route });
    match (host, words.as_slice()) {
        (_, ["api", ..]) if api_merges(host, args) => merge(Route::Api),
        (Host::GitHub, ["pr", "merge"]) | (Host::GitLab, ["mr", "merge" | "accept"]) => {
            merge(Route::Command)
        }
        (Host::GitLab, ["mr", "create"]) if create_auto_merges(args) => {
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

/// Whether `gh api` or `glab api` with `args` calls the host's merge
/// endpoint with `PUT`: `repos/<owner>/<repo>/pulls/<n>/merge` on GitHub,
/// `projects/<id>/merge_requests/<iid>/merge` on GitLab, in any spelling of
/// the method flag (`-X PUT`, `-XPUT`, `-X=PUT`, `--method PUT`,
/// `--method=put`). A `GET` there only asks whether it is merged. A method
/// held in a variable is unknown, and read as `PUT`.
fn api_merges(host: Host, args: &[String]) -> bool {
    let collection = match host {
        Host::GitHub => "pulls",
        Host::GitLab => "merge_requests",
    };
    let endpoint = args.iter().any(|arg| {
        let path = arg.split(['?', '#']).next().unwrap_or_default();
        let path = path.split_once("://").map_or(path, |(_, rest)| rest);
        let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        matches!(segments.as_slice(), [.., kind, _, "merge"] if *kind == collection)
    });
    endpoint
        && method_values(args)
            .any(|method| method.eq_ignore_ascii_case("PUT") || method.contains(['$', '`']))
}

/// Every value given to `-X`/`--method` in `args`.
fn method_values(args: &[String]) -> impl Iterator<Item = &str> {
    args.iter().enumerate().filter_map(|(at, arg)| {
        let next = || args.get(at + 1).map(String::as_str);
        if arg == "--method" {
            return next();
        }
        if let Some(value) = arg.strip_prefix("--method=") {
            return Some(value);
        }
        // A short cluster whose `X` takes the rest of it, or the next word
        // (`-X PUT`, `-XPUT`, `-X=PUT`, `-iX PUT`).
        let letters = arg
            .strip_prefix('-')
            .filter(|rest| !rest.starts_with('-'))?;
        let (_, value) = letters.split_once('X')?;
        match value.strip_prefix('=').unwrap_or(value) {
            "" => next(),
            value => Some(value),
        }
    })
}

/// Whether `glab mr create` with `args` sets `--auto-merge` (a boolean flag
/// that `--auto-merge=false` turns off).
fn create_auto_merges(args: &[String]) -> bool {
    args.iter()
        .take_while(|arg| *arg != "--")
        .filter_map(|arg| arg.strip_prefix("--auto-merge"))
        .any(|value| match value.strip_prefix('=') {
            None => value.is_empty(),
            Some(value) => !matches!(value.to_ascii_lowercase().as_str(), "false" | "f" | "0"),
        })
}

/// Whether a `git` call (its args from Git's own options on) pushes with, or
/// stores for later pushes, a push option that makes GitLab merge the merge
/// request: `-o`/`--push-option` on `git push`, `-c push.pushOption=` before
/// it, or `git config` writing `push.pushOption`. An option that only opens
/// a merge request (`merge_request.create`) arms nothing.
fn git_arms_merge(args: &[String]) -> bool {
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        match arg.as_str() {
            "-c" => {
                if args
                    .get(at + 1)
                    .and_then(|value| value.split_once('='))
                    .is_some_and(|(key, value)| {
                        key.eq_ignore_ascii_case("push.pushOption") && merging_option(value)
                    })
                {
                    return true;
                }
                at += 2;
            }
            "-C" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path" | "--config-env"
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
        "push" => push_options(rest).any(merging_option),
        "config" => {
            rest.iter()
                .any(|arg| arg.eq_ignore_ascii_case("push.pushOption"))
                && rest.iter().any(|arg| merging_option(arg))
        }
        _ => false,
    }
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
