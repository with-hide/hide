//! The `glab` commands the core may have its own node run with the
//! operator's gitlab.com login, and what one answers. The node refuses any
//! other command line ([`allowed`]); the core builds every command from the
//! builders below, so the two cannot drift apart.
//!
//! Every command names its server, gitlab.com, and never reads one from the
//! folder it runs in. A `glab api` line names its method and its endpoint,
//! which [`allowed`] checks together: `glab` turns a request with fields into
//! a POST on its own, so an endpoint alone does not say what the line does.
//! Every project read names the project by its validated path
//! ([`is_project_path`]), and every list it asks for has a fixed page whose
//! answer says when it was cut (`count` beside the page).

use serde::{Deserialize, Serialize};

use crate::gh::{GithubFailureCategory, is_commit};

/// The one server Hide reads with `glab`: gitlab.com, the operator's choice
/// for this round (self-managed GitLab is not read).
pub const HOST: &str = "gitlab.com";

/// The oldest `glab` whose flags every command line here parses on
/// (2025-11-21). `hide-host/src/glab.rs`'s floor test runs each line against
/// it, and `scripts/fetch-glab.sh` downloads it against pinned digests; the
/// three move together.
pub const MINIMUM_VERSION: GlabVersion = GlabVersion(1, 78, 0);

/// `glab --version`, read once per pass to refuse a release older than
/// [`MINIMUM_VERSION`] by name rather than by the flag it would refuse.
pub const VERSION: [&str; 1] = ["--version"];

/// Whether `glab` is signed in to gitlab.com: it exits non-zero when it is
/// not, or when the token it holds is refused.
pub const AUTH_STATUS: [&str; 4] = ["auth", "status", "--hostname", HOST];

/// The command an operator runs to sign `glab` in, for the access hint.
pub const SIGN_IN: &str = "glab auth login --hostname gitlab.com";

/// The most bytes of one answer a node keeps. A longer answer is cut there
/// and says so ([`GlabAnswer::Output::cut`]); a cut JSON answer cannot be
/// read, so the core treats it as a failed read and logs the cap.
pub const ANSWER_BYTE_LIMIT: usize = 8 * 1024 * 1024;

/// The most open merge requests one list read names; `count` beside them
/// says how many are open in all.
pub const OPEN_LIMIT: usize = 100;

/// The most recently updated merge requests of any state one list read
/// names, from which the merged and closed ones are taken: a checkout whose
/// branch merged long ago is past it, as a pull request past `gh`'s 200 is.
pub const RECENT_LIMIT: usize = 50;

/// The most jobs of one head pipeline a list read names; `count` says how
/// many it has, and a pipeline with more is judged by its own status alone.
pub const JOB_LIMIT: usize = 50;

/// The most reviewers and approvers of one merge request a list read names;
/// the reviewers' `count` says how many more there are.
pub const REVIEWER_LIMIT: usize = 20;

/// The most discussions one card read names.
pub const DISCUSSION_LIMIT: usize = 50;

/// The fields one merge request is read with in a list, beside its pipeline.
const CHANGE_FIELDS: &str = "iid title state draft webUrl sourceBranch targetBranch diffHeadSha sourceProjectId targetProjectId createdAt updatedAt mergedAt closedAt";

/// One read of a project's merge requests: the signed-in account, the
/// project's identity, the open merge requests with what places them in the
/// list (approvals, reviewers, author, merge status) and their head pipeline
/// with its jobs, and the most recently updated of any state. `glab mr list`
/// and the REST list carry no pipeline, which would cost one more call per
/// merge request. Kept under gitlab.com's query complexity of 250.
pub fn merge_requests_query() -> String {
    format!(
        "query=query HideGitlabMergeRequests($project:ID!){{currentUser{{username}} project(fullPath:$project){{id fullPath visibility webUrl repository{{rootRef}} open:mergeRequests(state:opened,first:{OPEN_LIMIT},sort:UPDATED_DESC){{count nodes{{{CHANGE_FIELDS} detailedMergeStatus approved approvalsRequired author{{username bot}} approvedBy(first:{REVIEWER_LIMIT}){{nodes{{username}}}} reviewers(first:{REVIEWER_LIMIT}){{count nodes{{username mergeRequestInteraction{{reviewState}}}}}} headPipeline{{sha status jobs(first:{JOB_LIMIT}){{count nodes{{name status allowFailure webPath}}}}}}}}}} settled:mergeRequests(state:all,first:{RECENT_LIMIT},sort:UPDATED_DESC){{nodes{{{CHANGE_FIELDS} author{{username bot}}}}}}}}}}"
    )
}

/// One merge request's card: its head, description, changed files and
/// discussions. `diffStats` takes no page; the node's byte cap bounds it,
/// and `diffStatsSummary` says how many files there are in all.
pub fn card_query() -> String {
    format!(
        "query=query HideGitlabMergeRequestCard($project:ID!,$iid:String!){{project(fullPath:$project){{mergeRequest(iid:$iid){{diffHeadSha description diffStatsSummary{{fileCount}} diffStats{{path additions deletions}} discussions(first:{DISCUSSION_LIMIT}){{nodes{{resolvable resolved notes(first:1){{nodes{{author{{username}} body position{{newPath newLine oldPath}}}}}}}}}}}}}}}}"
    )
}

/// The latest pipeline of each of `count` exact commits, `c0` to `cN`, each
/// commit a variable of its own so the query text is fixed for its count.
pub fn commit_checks_query(count: usize) -> String {
    let variables: String = (0..count).map(|index| format!(",$c{index}:String!")).collect();
    let reads: String = (0..count)
        .map(|index| format!(" c{index}:pipelines(sha:$c{index},first:1){{nodes{{sha status}}}}"))
        .collect();
    format!(
        "query=query HideGitlabCommitChecks($project:ID!{variables}){{project(fullPath:$project){{{} }}}}",
        reads.trim_start()
    )
}

/// `glab api --hostname gitlab.com -X POST graphql -f query=... -f k=v...`.
fn graphql(query: String, variables: &[(&str, &str)]) -> Vec<String> {
    let mut arguments: Vec<String> = ["api", "--hostname", HOST, "-X", "POST", "graphql", "-f"]
        .map(str::to_owned)
        .into();
    arguments.push(query);
    for (name, value) in variables {
        arguments.push("-f".to_owned());
        arguments.push(format!("{name}={value}"));
    }
    arguments
}

/// The list read of `project`.
pub fn merge_requests_arguments(project: &str) -> Option<Vec<String>> {
    is_project_path(project).then(|| graphql(merge_requests_query(), &[("project", project)]))
}

/// The card read of merge request `iid` of `project`.
pub fn card_arguments(project: &str, iid: u32) -> Option<Vec<String>> {
    (is_project_path(project) && iid > 0).then(|| {
        graphql(
            card_query(),
            &[("project", project), ("iid", &iid.to_string())],
        )
    })
}

/// The checks read of `commits` (distinct exact commit ids, at most
/// [`crate::worktrees::CHECKOUT_COMMIT_LIMIT`]) in `project`.
pub fn commit_checks_arguments(project: &str, commits: &[String]) -> Option<Vec<String>> {
    let distinct = commits
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        == commits.len();
    if !is_project_path(project)
        || commits.is_empty()
        || commits.len() > crate::worktrees::CHECKOUT_COMMIT_LIMIT
        || !distinct
        || commits.iter().any(|sha| !is_commit(sha))
    {
        return None;
    }
    let names: Vec<String> = (0..commits.len()).map(|index| format!("c{index}")).collect();
    let mut variables = vec![("project", project)];
    variables.extend(
        names
            .iter()
            .zip(commits)
            .map(|(name, sha)| (name.as_str(), sha.as_str())),
    );
    Some(graphql(commit_checks_query(commits.len()), &variables))
}

/// Whether `value` names a gitlab.com project as Hide takes it:
/// `group[/subgroup...]/project`, two to twenty path segments (gitlab.com's
/// nesting limit), each made of letters, digits, `.`, `_` and `-`, none
/// starting with `-` and none a path step (`.`, `..`). Nothing a shell, a
/// flag or `glab`'s own placeholders (`:fullpath`) or file fields (`@file`)
/// could read survives it.
pub fn is_project_path(value: &str) -> bool {
    let segments: Vec<&str> = value.split('/').collect();
    value.len() <= 255
        && (2..=20).contains(&segments.len())
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && !segment.starts_with('-')
                && *segment != "."
                && *segment != ".."
                && !segment.ends_with(".git")
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
}

/// Whether the node may run `glab` with `arguments`: the version, the
/// sign-in check, and `glab api` lines whose method and endpoint are allowed
/// together ([`api_allowed`]). Every shape named here has a line in
/// `hide-host/src/glab.rs`'s floor test, which runs it against
/// [`MINIMUM_VERSION`].
pub fn allowed(arguments: &[&str]) -> bool {
    match arguments {
        ["--version"] => true,
        ["auth", "status", "--hostname", HOST] => true,
        ["api", "--hostname", HOST, "-X", method, endpoint, fields @ ..] => {
            api_allowed(method, endpoint, fields, arguments)
        }
        _ => false,
    }
}

/// The method and endpoint pairs `glab api` may be run with. A pair not
/// named here is refused whatever its fields, so a read endpoint cannot be
/// sent as a write and a write endpoint cannot be reached at all.
fn api_allowed(method: &str, endpoint: &str, fields: &[&str], arguments: &[&str]) -> bool {
    match (method, endpoint) {
        // A GraphQL request is always a POST; only the three fixed queries
        // ride it, each rebuilt from its variables and compared whole.
        ("POST", "graphql") => graphql_allowed(fields, arguments),
        _ => false,
    }
}

/// Whether `fields` (`-f k=v` pairs) are one of the fixed queries with
/// valid variables: the line is rebuilt by its builder from the variables
/// it names and must equal it, so no other query, variable or flag rides.
fn graphql_allowed(fields: &[&str], arguments: &[&str]) -> bool {
    let mut pairs = Vec::new();
    let mut rest = fields;
    while let ["-f", pair, tail @ ..] = rest {
        let Some((name, value)) = pair.split_once('=') else {
            return false;
        };
        pairs.push((name, value));
        rest = tail;
    }
    if !rest.is_empty() {
        return false;
    }
    let value = |wanted: &str| {
        pairs
            .iter()
            .find(|(name, _)| *name == wanted)
            .map(|(_, value)| *value)
    };
    let (Some(query), Some(project)) = (value("query"), value("project")) else {
        return false;
    };
    let rebuilt = if query == merge_requests_query().trim_start_matches("query=") {
        merge_requests_arguments(project)
    } else if query == card_query().trim_start_matches("query=") {
        value("iid")
            .and_then(|iid| iid.parse::<u32>().ok().filter(|_| !iid.starts_with('0')))
            .and_then(|iid| card_arguments(project, iid))
    } else {
        let commits: Vec<String> = (0..)
            .map_while(|index| value(&format!("c{index}")).map(str::to_owned))
            .collect();
        commit_checks_arguments(project, &commits)
    };
    rebuilt.is_some_and(|rebuilt| rebuilt == arguments)
}

/// A `glab` release, compared part by part.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GlabVersion(pub u32, pub u32, pub u32);

impl GlabVersion {
    /// The release `glab --version` names on its first line: `glab 1.78.0
    /// (fb43432d)`, or `glab version 1.36.0 (2024-01-18)` from older
    /// releases; a suffix after the patch (`1.78.0-rc1`) is ignored. `None`
    /// for anything else, such as an unversioned build's `DEV`.
    pub fn of_output(output: &str) -> Option<Self> {
        let line = output.lines().next()?.strip_prefix("glab ")?;
        let version = line.strip_prefix("version ").unwrap_or(line);
        let version = version.split_whitespace().next()?;
        let version = version.strip_prefix('v').unwrap_or(version);
        let mut parts = version.splitn(3, '.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?;
        let digits = patch
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(patch.len());
        Some(Self(major, minor, patch[..digits].parse().ok()?))
    }
}

impl std::fmt::Display for GlabVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// What one `glab` command answered: its output, or why it gave none. The
/// failure categories are `gh`'s ([`GithubFailureCategory`]), read the same
/// way: not installed, too old, not signed in, or anything else.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum GlabAnswer {
    Output {
        stdout: String,
        /// The output passed [`ANSWER_BYTE_LIMIT`] and was cut there.
        #[serde(default)]
        cut: bool,
    },
    Failed {
        category: GithubFailureCategory,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECT: &str = "group/sub/project";
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn words(arguments: &[String]) -> Vec<&str> {
        arguments.iter().map(String::as_str).collect()
    }

    #[test]
    fn every_built_line_is_allowed_and_names_gitlab_com_and_the_project() {
        let lines = [
            merge_requests_arguments(PROJECT).unwrap(),
            card_arguments(PROJECT, 12).unwrap(),
            commit_checks_arguments(PROJECT, &[SHA.to_owned()]).unwrap(),
            commit_checks_arguments(PROJECT, &[SHA.to_owned(), "f".repeat(40)]).unwrap(),
        ];
        for line in &lines {
            let words = words(line);
            assert!(allowed(&words), "{words:?}");
            assert_eq!(words[..6], ["api", "--hostname", "gitlab.com", "-X", "POST", "graphql"]);
            assert!(words.contains(&"project=group/sub/project"), "{words:?}");
        }
        assert!(allowed(&VERSION));
        assert!(allowed(&AUTH_STATUS));
    }

    #[test]
    fn a_method_is_allowed_only_with_its_endpoint() {
        let line = merge_requests_arguments(PROJECT).unwrap();
        for (method, endpoint) in [
            ("GET", "graphql"),
            ("PUT", "graphql"),
            ("POST", "projects/1/merge_requests/2/merge"),
            ("GET", "projects/1"),
            ("POST", "user"),
        ] {
            let mut changed = line.clone();
            changed[4] = method.to_owned();
            changed[5] = endpoint.to_owned();
            assert!(!allowed(&words(&changed)), "{method} {endpoint}");
        }
    }

    #[test]
    fn a_line_off_its_builder_is_refused() {
        let line = merge_requests_arguments(PROJECT).unwrap();
        let mutate = |change: &dyn Fn(&mut Vec<String>)| {
            let mut changed = line.clone();
            change(&mut changed);
            allowed(&words(&changed))
        };
        // Another server, or none named.
        assert!(!mutate(&|line| line[2] = "gitlab.example.com".to_owned()));
        assert!(!mutate(&|line| {
            line.drain(1..3);
        }));
        // A query of another text.
        assert!(!mutate(&|line| line[7] = "query=query{currentUser{username}}".to_owned()));
        assert!(!mutate(&|line| line[7].push(' ')));
        // A project path that is no project, a placeholder or a file field.
        for project in [":fullpath", "@/etc/passwd", "one", "a/../b", "-x/y", "a/b c", "a/b.git"] {
            assert!(
                !mutate(&|line| line[9] = format!("project={project}")),
                "{project}"
            );
        }
        // An extra variable, an extra flag, a typed field in place of a raw one.
        assert!(!mutate(&|line| line.extend(["-f".to_owned(), "x=y".to_owned()])));
        assert!(!mutate(&|line| line.push("--paginate".to_owned())));
        assert!(!mutate(&|line| line[6] = "-F".to_owned()));
        assert!(!mutate(&|line| line[8] = "-F".to_owned()));
        // The project named twice.
        assert!(!mutate(&|line| line.extend(["-f".to_owned(), "project=a/b".to_owned()])));
        // Other commands.
        assert!(!allowed(&["auth", "status"]));
        assert!(!allowed(&["auth", "status", "--hostname", "gitlab.example.com"]));
        assert!(!allowed(&["mr", "list"]));
        assert!(!allowed(&["api", "user"]));
    }

    #[test]
    fn card_and_commit_lines_take_only_their_own_variables() {
        assert!(card_arguments(PROJECT, 0).is_none());
        let card = card_arguments(PROJECT, 12).unwrap();
        let mut leading_zero = card.clone();
        leading_zero[11] = "iid=012".to_owned();
        assert!(!allowed(&words(&leading_zero)));
        let mut not_number = card;
        not_number[11] = "iid=12 OR 1".to_owned();
        assert!(!allowed(&words(&not_number)));

        let commits = |count: usize| -> Vec<String> {
            (0..count).map(|index| format!("{index:040x}")).collect()
        };
        assert!(commit_checks_arguments(PROJECT, &[]).is_none());
        assert!(
            commit_checks_arguments(PROJECT, &commits(crate::worktrees::CHECKOUT_COMMIT_LIMIT))
                .is_some()
        );
        assert!(
            commit_checks_arguments(
                PROJECT,
                &commits(crate::worktrees::CHECKOUT_COMMIT_LIMIT + 1)
            )
            .is_none()
        );
        assert!(commit_checks_arguments(PROJECT, &[SHA.to_owned(), SHA.to_owned()]).is_none());
        assert!(commit_checks_arguments(PROJECT, &["main".to_owned()]).is_none());
        let mut skipped = commit_checks_arguments(PROJECT, &commits(2)).unwrap();
        skipped[11] = format!("c5={SHA}");
        assert!(!allowed(&words(&skipped)));
    }

    #[test]
    fn subgroup_paths_are_projects_and_other_shapes_are_not() {
        for path in ["a/b", "group/sub/sub2/project", "a.b/c_d-e", "acme/.dotfiles"] {
            assert!(is_project_path(path), "{path}");
        }
        let deep = vec!["g"; 21].join("/");
        for path in ["", "a", "a/", "/a/b", "a//b", "a/-b", "a/..", deep.as_str(), "a/b:1"] {
            assert!(!is_project_path(path), "{path}");
        }
    }

    #[test]
    fn a_version_is_read_from_either_first_line_glab_prints() {
        assert_eq!(
            GlabVersion::of_output("glab 1.78.0 (fb43432d)\n"),
            Some(GlabVersion(1, 78, 0))
        );
        assert_eq!(
            GlabVersion::of_output("glab version 1.36.0 (2024-01-18)"),
            Some(GlabVersion(1, 36, 0))
        );
        assert_eq!(GlabVersion::of_output("glab 1.80.2-rc1 (x)"), Some(GlabVersion(1, 80, 2)));
        assert_eq!(GlabVersion::of_output("glab DEV"), None);
        assert_eq!(GlabVersion::of_output("gh version 2.72.0"), None);
        assert!(GlabVersion(1, 77, 9) < MINIMUM_VERSION);
        assert!(GlabVersion(1, 131, 0) > MINIMUM_VERSION);
    }
}
