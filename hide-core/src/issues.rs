//! Bounded GitHub issue identities. These are references, never orchestration state.
pub(crate) use hide_node_link::gh::is_repository;
use serde::{Deserialize, Serialize};

pub const ISSUE_LIMIT: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize)]
pub struct IssueReference {
    pub repository: String,
    pub number: u32,
}

impl IssueReference {
    pub fn parse(value: &str, current_repository: Option<&str>) -> Result<Self, String> {
        let value = value.trim();
        let (repository, number) = if let Some(path) = value.strip_prefix("https://github.com/") {
            let (repository, number) = path
                .split_once("/issues/")
                .ok_or("a GitHub URL that is not an issue URL")?;
            (repository, number.trim_end_matches('/'))
        } else if let Some((repository, number)) = value.split_once('#') {
            (
                if repository.is_empty() {
                    current_repository
                        .ok_or("#N needs the current repository, which is not known")?
                } else {
                    repository
                },
                number,
            )
        } else {
            return Err("not owner/repo#N, #N or an issue URL".into());
        };
        Self::validated(repository, number)
    }

    fn validated(repository: &str, number: &str) -> Result<Self, String> {
        if !is_repository(repository) {
            return Err("the repository is not owner/repo".into());
        }
        let number = number
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or("the issue number is not a positive number")?;
        let reference = Self {
            repository: repository.to_owned(),
            number,
        };
        if reference.token().chars().count() > 80 {
            return Err("the issue reference is over 80 characters".into());
        }
        Ok(reference)
    }

    pub fn token(&self) -> String {
        format!("{}#{}", self.repository, self.number)
    }
    pub fn url(&self) -> String {
        format!(
            "https://github.com/{}/issues/{}",
            self.repository, self.number
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct ProjectIssuesSnapshot {
    pub repository: Option<String>,
    pub issues: Vec<IssueSnapshot>,
    pub overflow: bool,
    /// Why the issues' dependencies and sub-issues could not be read on the
    /// last pass, when they could not; each issue then keeps the blockers and
    /// sub-issues read before it.
    /// Reader provenance, not a wire field.
    #[serde(skip)]
    pub dependencies_failure: Option<String>,
    /// GitHub's id of the repository, which a rename keeps: the link record
    /// follows it (PRD link-graph D-35). Reader provenance, not a wire field.
    #[serde(skip)]
    pub repository_id: Option<String>,
    /// The branch GitHub names as the repository's default, which the
    /// worktree reader prefers over `origin/HEAD` (PRD git-freshness D-07).
    /// Reader provenance, not a wire field.
    #[serde(skip)]
    pub default_branch: Option<String>,
    /// Whether the repository is public, `None` until GitHub said
    /// (`공개`, PRD overview-device-tasks D-35).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct IssueSnapshot {
    pub reference: IssueReference,
    pub title: String,
    pub url: String,
    pub state: String,
    pub project_status: Option<String>,
    pub created_at_unix_ms: Option<u64>,
    pub updated_at_unix_ms: Option<u64>,
    pub closed_at_unix_ms: Option<u64>,
    /// The open issues GitHub records as blocking this one (its "blocked by"
    /// dependencies), in GitHub's order; a closed blocker no longer blocks.
    pub blocked_by: Vec<IssueReference>,
    /// GitHub's sub-issues of this issue; empty `total` for an issue with none.
    pub sub_issues: SubIssuesSnapshot,
    /// The issue's labels in GitHub's order, read with the list (or the
    /// linked-issue query) itself. Kept in the saved answer, so a restart
    /// shows them before the first read; an older saved answer has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<crate::tasks::TaskLabel>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub assignees: Vec<String>,
}

/// An issue's sub-issues as GitHub counts them (`Issue.subIssuesSummary`) and
/// lists them (`Issue.subIssues`). `total` and `completed` are GitHub's own
/// counts; `listed` is capped at GitHub's limit of 100 sub-issues per issue,
/// so it is never shorter than `total`.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct SubIssuesSnapshot {
    pub total: u32,
    pub completed: u32,
    pub listed: Vec<SubIssueSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SubIssueSnapshot {
    pub reference: IssueReference,
    pub title: String,
    pub open: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IssueLinkSnapshot {
    pub issue: IssueSnapshot,
    /// Where the link came from, as a code: `manual`, `pane_token`,
    /// `branch_setting`, `pr_body` or `branch_name`.
    pub source: String,
}

/// Only the two approved branch conventions match, never incidental digits.
pub fn branch_number(branch: &str) -> Option<u32> {
    let mut parts = branch.split('-');
    let first = parts.next()?;
    let candidate = if first.bytes().all(|c| c.is_ascii_digit()) {
        first
    } else if !first.is_empty() && first.bytes().all(|c| c.is_ascii_uppercase()) {
        parts.next()?
    } else {
        return None;
    };
    parts.next().filter(|suffix| !suffix.is_empty())?;
    candidate.parse::<u32>().ok().filter(|n| *n > 0)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedIssue {
    number: u32,
    title: String,
    url: String,
    state: String,
    #[serde(default)]
    project_items: Vec<ProjectItem>,
    #[serde(default)]
    labels: Vec<ListedLabel>,
    #[serde(default)]
    author: Option<ListedPerson>,
    #[serde(default)]
    assignees: Vec<ListedPerson>,
    updated_at: Option<String>,
    created_at: Option<String>,
    closed_at: Option<String>,
}
#[derive(Deserialize)]
struct ListedPerson {
    login: String,
}
#[derive(Deserialize)]
struct ListedLabel {
    name: String,
    #[serde(default)]
    color: Option<String>,
}
#[derive(Deserialize)]
struct ProjectItem {
    status: Option<ProjectStatus>,
}
#[derive(Deserialize)]
struct ProjectStatus {
    name: String,
}

pub fn parse_issues(output: &str) -> Result<Vec<IssueSnapshot>, String> {
    let listed: Vec<ListedIssue> = serde_json::from_str(output)
        .map_err(|error| format!("gh issue list returned output Hide could not read: {error}"))?;
    listed
        .into_iter()
        .map(|issue| {
            let reference = IssueReference::parse(&issue.url, None)?;
            if reference.number != issue.number
                || !matches!(issue.state.as_str(), "OPEN" | "CLOSED")
            {
                return Err("GitHub returned an inconsistent issue identity or state".into());
            }
            Ok(IssueSnapshot {
                author: issue.author.map(|person| person.login),
                assignees: issue
                    .assignees
                    .into_iter()
                    .map(|person| person.login)
                    .collect(),
                reference,
                title: issue.title,
                url: issue.url,
                state: issue.state,
                project_status: issue
                    .project_items
                    .into_iter()
                    .filter_map(|item| item.status)
                    .map(|status| status.name)
                    .find(|name| !name.trim().is_empty()),
                updated_at_unix_ms: issue
                    .updated_at
                    .as_deref()
                    .and_then(crate::github::parse_rfc3339_ms),
                created_at_unix_ms: issue
                    .created_at
                    .as_deref()
                    .and_then(crate::github::parse_rfc3339_ms),
                closed_at_unix_ms: issue
                    .closed_at
                    .as_deref()
                    .and_then(crate::github::parse_rfc3339_ms),
                blocked_by: Vec::new(),
                sub_issues: SubIssuesSnapshot::default(),
                labels: issue
                    .labels
                    .into_iter()
                    .map(|label| crate::tasks::TaskLabel::from_source(label.name, label.color))
                    .collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_github_issue_references_and_expands_local_numbers() {
        let expected = IssueReference {
            repository: "acme/project".into(),
            number: 42,
        };
        for input in [
            "acme/project#42",
            "#42",
            "https://github.com/acme/project/issues/42",
        ] {
            assert_eq!(
                IssueReference::parse(input, Some("acme/project")).unwrap(),
                expected
            );
        }
        for invalid in [
            "#0",
            "#-1",
            "#x",
            "a/b/c#2",
            "https://evil.test/a/b/issues/1",
            "https://github.com/a/b/pull/1",
            "a/b#1?x",
            "a/b#1\n#2",
        ] {
            assert!(
                IssueReference::parse(invalid, Some("acme/project")).is_err(),
                "{invalid}"
            );
        }
        assert!(IssueReference::parse("#42", None).is_err());
    }
    #[test]
    fn branch_conventions_require_a_number_prefix_and_a_suffix() {
        assert_eq!(branch_number("42-한글-작업"), Some(42));
        assert_eq!(branch_number("HOY-42-long-name"), Some(42));
        for branch in [
            "fix/42-name",
            "feature-42-name",
            "42",
            "ABC-42",
            "0-name",
            "42-",
        ] {
            assert_eq!(branch_number(branch), None, "{branch}");
        }
    }
    #[test]
    fn issue_state_and_project_status_come_from_the_response() {
        let issues = parse_issues(r#"[{"number":42,"title":"한국어 작업","url":"https://github.com/a/b/issues/42","state":"CLOSED","projectItems":[{"status":{"name":"Done","optionId":"1"},"title":"Roadmap"}],"updatedAt":"2026-09-20T00:00:00Z","createdAt":"2026-09-18T00:00:00Z","closedAt":"2026-09-19T00:00:00Z"}]"#).unwrap();
        assert_eq!(issues[0].state, "CLOSED");
        assert_eq!(
            issues[0].closed_at_unix_ms,
            crate::github::parse_rfc3339_ms("2026-09-19T00:00:00Z")
        );
        assert_ne!(issues[0].closed_at_unix_ms, issues[0].updated_at_unix_ms);
        assert_eq!(issues[0].project_status.as_deref(), Some("Done"));
        assert!(issues[0].created_at_unix_ms.is_some());
        assert_eq!(parse_issues(r#"[{"number":43,"title":"older","url":"https://github.com/a/b/issues/43","state":"OPEN"}]"#).unwrap()[0].created_at_unix_ms, None);
        assert!(parse_issues("not json").is_err());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IssueCandidate {
    pub reference: IssueReference,
    pub source: String,
}
