//! A project's tasks in one shape whatever their source is (PRD
//! task-agents-views D-14). The web reads only this shape; each source has an
//! adapter here that projects its own answer into it: GitHub issues read
//! through the operator's `gh`, and Local issues kept on this Mac
//! (`local_issues.rs`). A project keeps its memos alongside GitHub issues.
//! The old source preference never hides or moves either source's records.
//!
//! Every value is additive on the wire and none is a string enum an older
//! reader decodes, so such a reader takes the snapshot exactly as before.

use serde::{Deserialize, Serialize};

use crate::issues::{IssueReference, IssueSnapshot, ProjectIssuesSnapshot};
use crate::local_issues::LocalIssueProject;
use crate::model::{GithubFailureCategory, GithubStatusSnapshot};

/// The source kind of a GitHub issue.
pub const GITHUB: &str = "github";
/// The source kind of an issue kept on this Mac.
pub const LOCAL: &str = "local";

/// One task. `key` names it across every source and project, so a checkout,
/// a card and a dependency edge can refer to it without knowing the source's
/// own identity scheme.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskSnapshot {
    pub key: String,
    /// The source kind (`github`, `local`). A plain string, so a shell that
    /// does not know a later source draws it as an unknown one instead of
    /// failing.
    pub source: String,
    /// The id the source shows for it (`#170`, `acme/other#12` for an issue of
    /// another repository, `L-3` for a local issue).
    pub id: Option<String>,
    pub url: Option<String>,
    pub title: String,
    pub open: bool,
    /// When the source last changed it, for the backlog's order and age.
    pub updated_at_unix_ms: Option<u64>,
    /// Creation time from the source, absent when that source did not provide it.
    pub created_at_unix_ms: Option<u64>,
    /// Actual closure, not the last edit time; absent when the source cannot prove it.
    pub closed_at_unix_ms: Option<u64>,
    /// The open tasks this one waits on, which may belong to another project
    /// (PRD task-agents-views D-09, D-10); the source records them.
    pub blocked_by: Vec<TaskRefSnapshot>,
    /// The task's sub-issues with GitHub's own progress count; `None` for a
    /// task that has none, as every Local issue does.
    pub sub_issues: Option<TaskSubIssuesSnapshot>,
    /// The source's labels in its order, read with the list itself so every
    /// card shows them; empty for a source without labels, as Local is, and
    /// then absent from the wire.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<TaskLabel>,
    pub author: Option<String>,
    pub assignees: Vec<String>,
}

/// GitHub's `completed` of `total` sub-issues and the sub-issues themselves.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskSubIssuesSnapshot {
    pub total: u32,
    pub completed: u32,
    pub items: Vec<TaskSubIssueSnapshot>,
}

/// One sub-issue, by the key a pull request's closing reference names it with.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskSubIssueSnapshot {
    pub key: String,
    pub id: Option<String>,
    pub title: String,
    pub open: bool,
}

/// Another task, named by its key and by the id this task's source shows for it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskRefSnapshot {
    pub key: String,
    pub id: Option<String>,
}

/// Where a project's tasks come from and how the last read went.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskSourceSnapshot {
    pub kind: String,
    /// What the operator calls the source (`GitHub`, `Local`).
    pub label: String,
    /// Which list of that source this is (`acme/app`), once it has answered.
    pub name: Option<String>,
    /// No read has answered yet.
    pub reading: bool,
    /// The last read failed and `tasks` are the answer before it. The detail
    /// is in the diagnostic log; this is the one sentence a tooltip shows.
    pub failure: Option<String>,
    pub last_read_at_unix_ms: Option<u64>,
    /// The operator chose this source in Settings rather than the default.
    pub chosen: bool,
    /// A GitHub source's repository is public; `None` when GitHub has not
    /// said, which the shell shows as `공개일 수 있음` (D-35). Absent for Local.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public: Option<bool>,
}

/// What an issue's panel reads when it opens, beyond the task itself (PRD
/// overview-lenses-issues D-40): the body, and for a GitHub issue its labels,
/// author, assignees and comments. A value the source does not have is empty
/// or absent, never invented: a Local issue has no labels, author or comments.
/// It rides only on the request's answer (`issue_work.detail`), never on the
/// snapshot's task list.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct TaskDetail {
    pub body: String,
    pub labels: Vec<TaskLabel>,
    pub author: Option<String>,
    pub created_at_unix_ms: Option<u64>,
    pub assignees: Vec<String>,
    /// How many comments the issue has; absent for a source without comments.
    pub comment_count: Option<u32>,
    /// One bounded page, oldest first; later pages are requested explicitly.
    pub comments: Vec<TaskComment>,
    pub comments_cursor: Option<String>,
    pub comments_has_more: bool,
    pub comments_limited: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TaskLabel {
    pub name: String,
    /// The source's colour as six hex digits, absent when it gave none.
    pub color: Option<String>,
}

impl TaskLabel {
    /// A label as a source gave it. The colour is drawn from data, so only
    /// six hex digits pass; anything else is no colour.
    pub fn from_source(name: String, color: Option<String>) -> Self {
        Self {
            name,
            color: color.filter(|color| {
                color.len() == 6 && color.bytes().all(|byte| byte.is_ascii_hexdigit())
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskComment {
    pub author: Option<String>,
    pub created_at_unix_ms: Option<u64>,
    /// At most `COMMENT_BODY_LIMIT` characters, ending in `…` when cut.
    pub body: String,
}

/// Comments one explicit page reads; later pages stay behind Load more.
pub const DETAIL_COMMENTS: usize = 50;
/// Most comments a detail retains across explicit page requests.
pub const DETAIL_COMMENT_LIMIT: usize = 200;
/// A shown comment's length; the whole comment is on the source.
pub const COMMENT_BODY_LIMIT: usize = 4_000;

/// `body` cut to `COMMENT_BODY_LIMIT` characters, marked when cut.
pub fn capped_comment(body: &str) -> String {
    let body = body.trim();
    match body.char_indices().nth(COMMENT_BODY_LIMIT) {
        Some(_) => {
            let end = body.char_indices().nth(COMMENT_BODY_LIMIT - 1).unwrap().0;
            format!("{}…", &body[..end])
        }
        None => body.to_owned(),
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectTasksSnapshot {
    /// Absent only for a device's project, whose issues this Mac does not read.
    pub source: Option<TaskSourceSnapshot>,
    /// Each source's independent freshness and failure; `source` is retained
    /// for older shells and saved project preferences.
    pub sources: Vec<TaskSourceSnapshot>,
    pub tasks: Vec<TaskSnapshot>,
    /// The source listed more than Hide keeps (`ISSUE_LIMIT`).
    pub overflow: bool,
}

/// The key of a GitHub issue.
pub fn github_key(reference: &IssueReference) -> String {
    format!("{GITHUB}:{}", reference.token())
}

/// The key of a local issue: the project's path names it across projects.
pub fn local_key(project_path: &str, number: u32) -> String {
    format!("{LOCAL}:{project_path}#{number}")
}

/// The project and number a local key names.
pub fn parse_local_key(key: &str) -> Option<(&str, u32)> {
    let rest = key.strip_prefix("local:")?;
    let (path, number) = rest.rsplit_once('#')?;
    Some((
        path,
        number.parse().ok().filter(|number: &u32| *number > 0)?,
    ))
}

/// The GitHub reference a GitHub key names.
pub fn parse_github_key(key: &str) -> Option<IssueReference> {
    IssueReference::parse(key.strip_prefix("github:")?, None).ok()
}

/// The token a checkout's issue link stores for a task key: `owner/repo#N`
/// for a GitHub issue, `L-N` for a local one. The linking chain reads both
/// back (`runtime/issues.rs`).
pub fn issue_token(key: &str) -> Option<String> {
    if let Some((_, number)) = parse_local_key(key) {
        return Some(crate::local_issues::display_id(number));
    }
    parse_github_key(key).map(|reference| reference.token())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    Github,
    Local,
}

/// The source a local project reads. `choice` is the operator's
/// (`ui_state.project_issue_sources`); without one, a Git repository reads
/// GitHub unless `gh` said it has no GitHub remote or is not installed, and a
/// folder reads Local. A GitHub read that fails for another reason (logged
/// out, offline) stays GitHub and says so, rather than moving the project's
/// issues somewhere else under the operator.
pub fn source_kind(
    choice: Option<&str>,
    git: bool,
    issues: &ProjectIssuesSnapshot,
    status: &GithubStatusSnapshot,
) -> SourceKind {
    match choice {
        Some(LOCAL) => SourceKind::Local,
        Some(GITHUB) if git => SourceKind::Github,
        _ if !git => SourceKind::Local,
        _ if issues.repository.is_some() => SourceKind::Github,
        _ if matches!(
            status.failure_category,
            Some(GithubFailureCategory::NotInstalled | GithubFailureCategory::NoGithubRemote)
        ) =>
        {
            SourceKind::Local
        }
        _ => SourceKind::Github,
    }
}

/// How the Local store stands for one project.
pub enum LocalRead<'a> {
    /// The store could not be read; its issues are unknown, not empty.
    Failed(&'a str),
    Ready(Option<&'a LocalIssueProject>),
}

/// The GitHub adapter: a local Git project's issues and the reader's status.
/// A read that has not answered yet is a source still reading; a failed one
/// keeps the tasks read before it and says so.
pub fn github_tasks(
    issues: &ProjectIssuesSnapshot,
    status: &GithubStatusSnapshot,
    chosen: bool,
) -> ProjectTasksSnapshot {
    let answered = issues.repository.is_some();
    let repository = issues.repository.as_deref();
    let failure = if status.stale || (!answered && !status.loading) {
        status.unavailable_reason.clone()
    } else {
        None
    };
    let source = TaskSourceSnapshot {
        kind: GITHUB.into(),
        label: "GitHub".into(),
        name: issues.repository.clone(),
        reading: !answered && failure.is_none(),
        failure: failure.or_else(|| {
            issues
                .dependencies_failure
                .as_ref()
                .map(|reason| format!("issue dependencies: {reason}"))
        }),
        last_read_at_unix_ms: status.last_success_at_unix_ms,
        chosen,
        public: issues.public,
    };
    ProjectTasksSnapshot {
        sources: vec![source.clone()],
        source: Some(source),
        tasks: issues
            .issues
            .iter()
            .map(|issue| github_task(issue, repository))
            .collect(),
        overflow: issues.overflow,
    }
}

/// The Local adapter: the project's issues in the store on this Mac.
pub fn local_tasks(project_path: &str, read: LocalRead<'_>, chosen: bool) -> ProjectTasksSnapshot {
    let (failure, project) = match read {
        LocalRead::Failed(reason) => (Some(reason.to_owned()), None),
        LocalRead::Ready(project) => (None, project),
    };
    let source = TaskSourceSnapshot {
        kind: LOCAL.into(),
        label: "Local".into(),
        name: None,
        reading: false,
        failure,
        last_read_at_unix_ms: None,
        chosen,
        public: None,
    };
    ProjectTasksSnapshot {
        sources: vec![source.clone()],
        source: Some(source),
        tasks: project
            .map(|project| {
                // Newest first, the order GitHub lists issues in.
                project
                    .issues
                    .iter()
                    .rev()
                    .map(|issue| TaskSnapshot {
                        key: local_key(project_path, issue.number),
                        source: LOCAL.into(),
                        id: Some(crate::local_issues::display_id(issue.number)),
                        url: None,
                        title: issue.title.clone(),
                        open: issue.open,
                        updated_at_unix_ms: Some(issue.updated_at_unix_ms),
                        created_at_unix_ms: Some(issue.created_at_unix_ms),
                        closed_at_unix_ms: issue.closed_at_unix_ms,
                        blocked_by: Vec::new(),
                        sub_issues: None,
                        labels: Vec::new(),
                        author: None,
                        assignees: Vec::new(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        overflow: false,
    }
}

/// `#170` in its own repository, `owner/repo#170` from another.
fn github_id(reference: &IssueReference, repository: Option<&str>) -> String {
    if Some(reference.repository.as_str()) == repository {
        format!("#{}", reference.number)
    } else {
        reference.token()
    }
}

fn github_task(issue: &IssueSnapshot, repository: Option<&str>) -> TaskSnapshot {
    let reference = &issue.reference;
    TaskSnapshot {
        key: github_key(reference),
        source: GITHUB.into(),
        id: Some(github_id(reference, repository)),
        url: Some(issue.url.clone()),
        title: issue.title.clone(),
        open: issue.state == "OPEN",
        updated_at_unix_ms: issue.updated_at_unix_ms,
        created_at_unix_ms: issue.created_at_unix_ms,
        closed_at_unix_ms: issue.closed_at_unix_ms,
        blocked_by: issue
            .blocked_by
            .iter()
            .map(|blocker| TaskRefSnapshot {
                key: github_key(blocker),
                id: Some(github_id(blocker, repository)),
            })
            .collect(),
        sub_issues: (issue.sub_issues.total > 0).then(|| TaskSubIssuesSnapshot {
            total: issue.sub_issues.total,
            completed: issue.sub_issues.completed,
            items: issue
                .sub_issues
                .listed
                .iter()
                .map(|sub| TaskSubIssueSnapshot {
                    key: github_key(&sub.reference),
                    id: Some(github_id(&sub.reference, repository)),
                    title: sub.title.clone(),
                    open: sub.open,
                })
                .collect(),
        }),
        labels: issue.labels.clone(),
        author: issue.author.clone(),
        assignees: issue.assignees.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B14: the visible ellipsis counts against the advertised 4,000
    /// character limit, including for Korean and supplementary characters.
    #[test]
    fn comment_truncation_includes_its_marker_in_the_cap() {
        let exact = "가".repeat(COMMENT_BODY_LIMIT);
        assert_eq!(capped_comment(&exact), exact);
        let cut = capped_comment(&"🙂".repeat(COMMENT_BODY_LIMIT + 1));
        assert_eq!(cut.chars().count(), COMMENT_BODY_LIMIT);
        assert!(cut.ends_with('…'));
        assert_eq!(
            cut.chars().filter(|character| *character == '🙂').count(),
            COMMENT_BODY_LIMIT - 1
        );
    }

    fn issue(repository: &str, number: u32, state: &str) -> IssueSnapshot {
        IssueSnapshot {
            reference: IssueReference {
                repository: repository.into(),
                number,
            },
            title: format!("Issue {number}"),
            url: format!("https://github.com/{repository}/issues/{number}"),
            state: state.into(),
            project_status: None,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            closed_at_unix_ms: None,
            blocked_by: Vec::new(),
            sub_issues: Default::default(),
            labels: Vec::new(),
            author: None,
            assignees: Vec::new(),
        }
    }

    fn answered() -> ProjectIssuesSnapshot {
        ProjectIssuesSnapshot {
            public: None,
            repository: Some("acme/app".into()),
            issues: vec![
                issue("acme/app", 170, "OPEN"),
                issue("acme/other", 12, "CLOSED"),
            ],
            overflow: true,
            dependencies_failure: None,
            repository_id: None,
            default_branch: None,
        }
    }

    fn healthy() -> GithubStatusSnapshot {
        GithubStatusSnapshot {
            available: true,
            last_success_at_unix_ms: Some(5),
            ..Default::default()
        }
    }

    #[test]
    fn issues_become_source_neutral_tasks() {
        let tasks = github_tasks(&answered(), &healthy(), false);
        let source = tasks.source.expect("connected");
        assert_eq!(source.name.as_deref(), Some("acme/app"));
        assert!(!source.reading && source.failure.is_none());
        assert!(tasks.overflow);
        assert_eq!(tasks.tasks[0].key, "github:acme/app#170");
        assert_eq!(tasks.tasks[0].id.as_deref(), Some("#170"));
        assert!(tasks.tasks[0].open);
        // Another repository's issue keeps its repository in the id.
        assert_eq!(tasks.tasks[1].id.as_deref(), Some("acme/other#12"));
        assert!(!tasks.tasks[1].open);
    }

    #[test]
    fn a_blocker_is_named_by_key_and_by_the_id_this_repository_shows() {
        let mut issues = answered();
        issues.issues[0].blocked_by = vec![
            IssueReference {
                repository: "acme/app".into(),
                number: 169,
            },
            IssueReference {
                repository: "acme/other".into(),
                number: 12,
            },
        ];
        let tasks = github_tasks(&issues, &healthy(), false);
        assert_eq!(
            tasks.tasks[0].blocked_by,
            vec![
                TaskRefSnapshot {
                    key: "github:acme/app#169".into(),
                    id: Some("#169".into())
                },
                TaskRefSnapshot {
                    key: "github:acme/other#12".into(),
                    id: Some("acme/other#12".into())
                },
            ]
        );
    }

    #[test]
    fn sub_issues_keep_githubs_counts_and_name_each_by_key() {
        let mut issues = answered();
        issues.issues[0].sub_issues = crate::issues::SubIssuesSnapshot {
            total: 5,
            completed: 3,
            listed: vec![crate::issues::SubIssueSnapshot {
                reference: IssueReference {
                    repository: "acme/app".into(),
                    number: 171,
                },
                title: "A piece".into(),
                open: false,
            }],
        };
        let tasks = github_tasks(&issues, &healthy(), false);
        let sub_issues = tasks.tasks[0].sub_issues.as_ref().expect("has sub-issues");
        assert_eq!((sub_issues.total, sub_issues.completed), (5, 3));
        assert_eq!(sub_issues.items[0].key, "github:acme/app#171");
        assert_eq!(sub_issues.items[0].id.as_deref(), Some("#171"));
        assert!(
            tasks.tasks[1].sub_issues.is_none(),
            "an issue with none has no progress to show"
        );
    }

    #[test]
    fn unread_dependencies_are_a_source_failure_beside_current_tasks() {
        let mut issues = answered();
        issues.dependencies_failure = Some("Field 'blockedBy' doesn't exist".into());
        let tasks = github_tasks(&issues, &healthy(), false);
        assert_eq!(tasks.tasks.len(), 2);
        assert_eq!(
            tasks.source.and_then(|source| source.failure).as_deref(),
            Some("issue dependencies: Field 'blockedBy' doesn't exist")
        );
    }

    #[test]
    fn a_failed_read_keeps_the_tasks_and_says_so() {
        let status = GithubStatusSnapshot {
            available: true,
            stale: true,
            unavailable_reason: Some("rate limited".into()),
            failure_category: Some(GithubFailureCategory::NetworkOrRateLimit),
            last_success_at_unix_ms: Some(5),
            ..Default::default()
        };
        let tasks = github_tasks(&answered(), &status, false);
        assert_eq!(tasks.tasks.len(), 2);
        assert_eq!(
            tasks.source.and_then(|source| source.failure).as_deref(),
            Some("rate limited")
        );
    }

    #[test]
    fn without_a_choice_a_repository_gh_cannot_see_on_github_reads_local() {
        let none = ProjectIssuesSnapshot::default();
        for (category, expected) in [
            (GithubFailureCategory::NotInstalled, SourceKind::Local),
            (GithubFailureCategory::NoGithubRemote, SourceKind::Local),
            // Logged out or offline is still a GitHub repository: it stays
            // GitHub and says why it could not be read.
            (GithubFailureCategory::NotLoggedIn, SourceKind::Github),
            (
                GithubFailureCategory::NetworkOrRateLimit,
                SourceKind::Github,
            ),
        ] {
            let status = GithubStatusSnapshot {
                unavailable_reason: Some(format!("gh: {}", category.english())),
                failure_category: Some(category),
                ..Default::default()
            };
            assert_eq!(
                source_kind(None, true, &none, &status),
                expected,
                "{category:?}"
            );
        }
        // A folder has no GitHub to read; a choice wins over the default.
        assert_eq!(
            source_kind(None, false, &none, &healthy()),
            SourceKind::Local
        );
        assert_eq!(
            source_kind(Some(LOCAL), true, &answered(), &healthy()),
            SourceKind::Local
        );
        assert_eq!(
            source_kind(Some(GITHUB), false, &none, &healthy()),
            SourceKind::Local
        );
    }

    #[test]
    fn a_github_read_that_never_answered_says_why_instead_of_reading_forever() {
        let status = GithubStatusSnapshot {
            unavailable_reason: Some("gh auth login".into()),
            failure_category: Some(GithubFailureCategory::NotLoggedIn),
            ..Default::default()
        };
        let tasks = github_tasks(&ProjectIssuesSnapshot::default(), &status, false);
        let source = tasks.source.expect("a GitHub source");
        assert!(!source.reading);
        assert_eq!(source.failure.as_deref(), Some("gh auth login"));
    }

    #[test]
    fn local_issues_are_tasks_keyed_by_project_newest_first() {
        let mut store = crate::local_issues::LocalIssueStore::default();
        store.create(None, "/p", "첫 이슈", "", 1).unwrap();
        store.create(None, "/p", "둘째", "", 2).unwrap();
        store.set_open(None, "/p", 1, false, 3);
        let tasks = local_tasks("/p", LocalRead::Ready(store.project(None, "/p")), true);
        let source = tasks.source.expect("local");
        assert_eq!(
            (source.kind.as_str(), source.chosen, source.reading),
            (LOCAL, true, false)
        );
        assert_eq!(tasks.tasks[0].id.as_deref(), Some("L-2"));
        assert_eq!(tasks.tasks[0].key, "local:/p#2");
        assert!(!tasks.tasks[1].open);
        assert_eq!(parse_local_key("local:/p#2"), Some(("/p", 2)));
        assert_eq!(
            local_tasks("/p", LocalRead::Failed("damaged"), false)
                .source
                .unwrap()
                .failure
                .as_deref(),
            Some("damaged")
        );
    }

    #[test]
    fn a_first_read_in_flight_is_a_source_still_reading() {
        let status = GithubStatusSnapshot {
            loading: true,
            ..Default::default()
        };
        let tasks = github_tasks(&ProjectIssuesSnapshot::default(), &status, false);
        assert!(tasks.source.expect("reading").reading);
    }
}
