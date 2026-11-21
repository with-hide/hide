//! `hide links`: the record read for an agent, answered as one JSON value
//! (PRD link-graph D-19, D-23, B41-B43).
//!
//! The caller's Project is the default scope. A target that only another
//! Project holds is refused with `other_project`, so a request text or a
//! conversation path of another Project leaves only through
//! `--all-projects`. Nothing here writes.

use super::store::{LinkStore, LocalFiles, PrRow};
use super::{LinkedIssue, LinkedSession};
use crate::node_access::NodeLink;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

/// What `hide links` asks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinksQuery {
    pub target: QueryTarget,
    #[serde(default)]
    pub all_projects: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum QueryTarget {
    Pr { number: u64 },
    Issue { number: u64 },
    Branch { name: String },
    Session { id: String },
}

impl QueryTarget {
    /// A target an agent could not have meant: empty, overlong, or with
    /// control characters.
    pub fn valid(&self) -> bool {
        let plain = |text: &str| {
            !text.is_empty() && text.len() <= 256 && !text.chars().any(char::is_control)
        };
        match self {
            Self::Pr { number } | Self::Issue { number } => *number > 0,
            Self::Branch { name } => plain(name),
            Self::Session { id } => plain(id),
        }
    }
}

/// The registered Projects as the runtime knows them when the query came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub store: PathBuf,
    pub local_device: String,
    /// The record key of the caller checkout's Project.
    pub caller: String,
    pub projects: Vec<ScopeProject>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeProject {
    pub key: String,
    pub workspace_id: String,
    pub label: String,
    pub root: String,
    pub device_id: String,
    /// Its checkouts now: a recorded worktree not among them was removed.
    pub checkouts: Vec<String>,
}

/// A refusal: the reason code and the next action the CLI prints.
pub type Refusal = (&'static str, &'static str);

/// Answers `query` from the record; `own` is the node `scope.local_device`
/// names, asked which of this machine's session files are still there.
pub fn run(scope: &Scope, query: &LinksQuery, own: &dyn NodeLink) -> Result<Value, Refusal> {
    if !query.target.valid() {
        return Err(("invalid_request", "Check hide links arguments and retry"));
    }
    let store = LinkStore::open_read_only(&scope.store).map_err(|code| {
        crate::diagnostic!(json!({"component": "links", "kind": "cli.read_failed", "code": code}));
        if code == "links_store_missing" {
            (
                "links_unavailable",
                "Hide has not recorded links yet; retry after it has read the session files",
            )
        } else {
            (
                "links_unavailable",
                "Retry; the reason is in Hide's diagnostic log",
            )
        }
    })?;
    let failed = |code: String| {
        crate::diagnostic!(json!({"component": "links", "kind": "cli.read_failed", "code": code}));
        (
            "links_unavailable",
            "Retry; the reason is in Hide's diagnostic log",
        )
    };
    let mut results = Vec::new();
    if query.all_projects {
        for project in &scope.projects {
            if let Some(found) = find(&store, scope, project, &query.target, own).map_err(failed)? {
                results.push(found);
            }
        }
    } else {
        let caller = scope
            .projects
            .iter()
            .find(|project| project.key == scope.caller)
            .ok_or((
                "workspace_unavailable",
                "Run hide links from a checkout of a registered Project",
            ))?;
        match find(&store, scope, caller, &query.target, own).map_err(failed)? {
            Some(found) => results.push(found),
            None => {
                for project in scope.projects.iter().filter(|p| p.key != scope.caller) {
                    if find(&store, scope, project, &query.target, own)
                        .map_err(failed)?
                        .is_some()
                    {
                        return Err((
                            "other_project",
                            "The target is in another Project; add --all-projects to read it",
                        ));
                    }
                }
            }
        }
    }
    if results.is_empty() {
        return Err((
            "not_found",
            "Check the number, branch or session id; the record holds only what it has seen",
        ));
    }
    Ok(json!({
        "target": query.target,
        "scope": if query.all_projects { "all_projects" } else { "project" },
        "results": results,
    }))
}

fn find(
    store: &LinkStore,
    scope: &Scope,
    project: &ScopeProject,
    target: &QueryTarget,
    own: &dyn NodeLink,
) -> Result<Option<Value>, String> {
    let Some(row) = store.project(&project.key)? else {
        return Ok(None);
    };
    let present = |asked: &[String]| super::files_present(own, asked);
    let local = Some(LocalFiles {
        device: &scope.local_device,
        present: &present,
    });
    let (prs, sessions): (Vec<Value>, Vec<LinkedSession>) = match target {
        QueryTarget::Pr { number } => {
            let Some(links) = store.pr_panel(&project.key, *number, local)? else {
                return Ok(None);
            };
            let pr = pr_value(
                &links.pr,
                &links.issues,
                &links.worktrees,
                project,
                &links.sessions,
            );
            (vec![pr], Vec::new())
        }
        QueryTarget::Issue { number } => {
            let mut keys = vec![format!("local:{}#{number}", project.root)];
            if let Some(name) = row.name.as_deref() {
                keys.push(format!("github:{name}#{number}"));
            }
            let mut found = None;
            for key in keys {
                let links = store.issue_panel(&project.key, &key, local)?;
                if !links.prs.is_empty() {
                    found = Some(links);
                    break;
                }
            }
            let Some(links) = found else {
                return Ok(None);
            };
            let mut prs = Vec::new();
            for number in &links.prs {
                if let Some(pr) = store.pr_panel(&project.key, *number, local)? {
                    prs.push(pr_value(
                        &pr.pr,
                        &pr.issues,
                        &pr.worktrees,
                        project,
                        &pr.sessions,
                    ));
                }
            }
            (prs, links.sessions)
        }
        QueryTarget::Branch { name } => {
            let Some((rows, sessions)) = store.branch_links(&project.key, name, local)? else {
                return Ok(None);
            };
            let mut prs = Vec::new();
            for pr in rows {
                if let Some(links) = store.pr_panel(&project.key, pr.number, local)? {
                    prs.push(pr_value(
                        &links.pr,
                        &links.issues,
                        &links.worktrees,
                        project,
                        &links.sessions,
                    ));
                }
            }
            (prs, sessions)
        }
        QueryTarget::Session { id } => {
            let Some((line, linked)) = store.session_links(&project.key, id, local)? else {
                return Ok(None);
            };
            let mut prs = Vec::new();
            for (pr, line) in linked {
                let issues = store.pr_issues(&pr)?;
                let worktrees = store.pr_worktrees(&project.key, &pr.branch)?;
                prs.push(pr_value(&pr, &issues, &worktrees, project, &[line]));
            }
            (prs, vec![line])
        }
    };
    Ok(Some(json!({
        "project": {
            "workspace_id": project.workspace_id,
            "label": project.label,
            "root": project.root,
            "device_id": project.device_id,
        },
        "repository": row.name,
        "prs": prs,
        "sessions": sessions,
    })))
}

fn pr_value(
    pr: &PrRow,
    issues: &[LinkedIssue],
    worktrees: &[String],
    project: &ScopeProject,
    sessions: &[LinkedSession],
) -> Value {
    let state = match pr.badge() {
        crate::model::PullRequestBadge::Merged => "merged",
        crate::model::PullRequestBadge::Closed => "closed",
        _ => "open",
    };
    json!({
        "number": pr.number,
        "title": pr.title,
        "url": pr.url,
        "branch": pr.branch,
        "state": state,
        "created_at_unix_ms": pr.created_at,
        "closed_at_unix_ms": pr.closed_at,
        "merged_at_unix_ms": pr.merged_at,
        "issues": issues,
        "worktrees": worktrees.iter().map(|path| json!({
            "path": path,
            "removed": !project.checkouts.contains(path),
        })).collect::<Vec<_>>(),
        "sessions": sessions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::{IssueSource, PaneFact, PrFact, ProjectFacts, WorktreeFact};

    const NOW: u64 = 1_790_000_000_000;

    fn project(key: &str, root: &str, repository: &str, prs: Vec<PrFact>) -> ProjectFacts {
        ProjectFacts {
            key: key.into(),
            device_id: "local".into(),
            workspace_id: format!("ws-{key}"),
            root: root.into(),
            repository: Some(repository.into()),
            repository_id: None,
            connection_epoch: 0,
            worktrees: vec![WorktreeFact {
                path: format!("{root}/feat"),
                branch: Some("feat".into()),
                created_at_unix_ms: None,
            }],
            prs,
            prs_read: true,
            outputs: Vec::new(),
        }
    }

    fn pr(repository: &str, number: u64, closes: Option<u64>) -> PrFact {
        PrFact {
            repository: repository.into(),
            number,
            branch: "feat".into(),
            title: format!("PR {number}"),
            url: format!("https://github.com/{repository}/pull/{number}"),
            created_at: Some(NOW - 1_000),
            closed_at: None,
            merged_at: None,
            issues: closes
                .map(|issue| (format!("github:{repository}#{issue}"), IssueSource::Closes))
                .into_iter()
                .collect(),
            hide_issue_known: false,
        }
    }

    fn scope_project(key: &str, root: &str) -> ScopeProject {
        ScopeProject {
            key: key.into(),
            workspace_id: format!("ws-{key}"),
            label: key.into(),
            root: root.into(),
            device_id: "local".into(),
            // The worktree `feat` was removed since it was recorded.
            checkouts: vec![root.into()],
        }
    }

    /// Two projects: `app` holds PR 7 closing issue 3, `lib` holds PR 9;
    /// one pane session worked on each.
    fn fixture() -> (tempfile::TempDir, Scope) {
        let state = tempfile::tempdir().unwrap();
        let path = state.path().join("links.sqlite3");
        let (mut store, _) = LinkStore::open(&path).unwrap();
        store
            .apply_project(
                &project(
                    "app",
                    "/work/app",
                    "Acme/App",
                    vec![pr("acme/app", 7, Some(3))],
                ),
                NOW,
            )
            .unwrap();
        store
            .apply_project(
                &project(
                    "lib",
                    "/work/lib",
                    "acme/lib",
                    vec![pr("acme/lib", 9, None)],
                ),
                NOW,
            )
            .unwrap();
        let pane = |id: &str, cwd: &str| PaneFact {
            device_id: "local".into(),
            agent: "claude".into(),
            session_id: id.into(),
            cwd: cwd.into(),
            branch: Some("feat".into()),
        };
        store
            .apply_panes(
                &[
                    pane("s-app", "/work/app/feat"),
                    pane("s-lib", "/work/lib/feat"),
                ],
                NOW,
            )
            .unwrap();
        let scope = Scope {
            store: path,
            local_device: "local".into(),
            caller: "app".into(),
            projects: vec![
                scope_project("app", "/work/app"),
                scope_project("lib", "/work/lib"),
            ],
        };
        (state, scope)
    }

    fn query(target: QueryTarget, all_projects: bool) -> LinksQuery {
        LinksQuery {
            target,
            all_projects,
        }
    }

    #[test]
    fn a_pull_request_of_the_callers_project_answers_with_its_sessions_and_links() {
        let (_state, scope) = fixture();
        let answer = run(
            &scope,
            &query(QueryTarget::Pr { number: 7 }, false),
            &hide_node::Local::of_process(),
        )
        .unwrap();

        assert_eq!(answer["scope"], "project");
        let result = &answer["results"][0];
        assert_eq!(result["repository"], "acme/app");
        let pr = &result["prs"][0];
        assert_eq!(pr["state"], "open");
        assert_eq!(
            pr["issues"][0],
            json!({"key": "github:acme/app#3", "source": "closes"})
        );
        assert_eq!(
            pr["worktrees"][0],
            json!({"path": "/work/app/feat", "removed": true})
        );
        assert_eq!(pr["sessions"][0]["id"], "s-app");
        assert_eq!(pr["sessions"][0]["role"], "worked");
        assert_eq!(pr["sessions"][0]["device_id"], "local");
    }

    #[test]
    fn another_projects_target_is_refused_unless_all_projects_is_asked() {
        let (_state, scope) = fixture();
        assert_eq!(
            run(
                &scope,
                &query(QueryTarget::Pr { number: 9 }, false),
                &hide_node::Local::of_process()
            )
            .unwrap_err()
            .0,
            "other_project"
        );
        let answer = run(
            &scope,
            &query(QueryTarget::Pr { number: 9 }, true),
            &hide_node::Local::of_process(),
        )
        .unwrap();
        assert_eq!(answer["scope"], "all_projects");
        assert_eq!(answer["results"].as_array().unwrap().len(), 1);
        assert_eq!(answer["results"][0]["project"]["workspace_id"], "ws-lib");
        assert_eq!(answer["results"][0]["prs"][0]["sessions"][0]["id"], "s-lib");
    }

    #[test]
    fn what_the_record_never_saw_is_not_found() {
        let (_state, scope) = fixture();
        for target in [
            QueryTarget::Pr { number: 404 },
            QueryTarget::Issue { number: 404 },
            QueryTarget::Branch {
                name: "nowhere".into(),
            },
            QueryTarget::Session {
                id: "s-unknown".into(),
            },
        ] {
            assert_eq!(
                run(
                    &scope,
                    &query(target.clone(), true),
                    &hide_node::Local::of_process()
                )
                .unwrap_err()
                .0,
                "not_found",
                "{target:?}"
            );
        }
    }

    #[test]
    fn an_issue_a_branch_and_a_session_read_the_same_record() {
        let (_state, scope) = fixture();
        let issue = run(
            &scope,
            &query(QueryTarget::Issue { number: 3 }, false),
            &hide_node::Local::of_process(),
        )
        .unwrap();
        assert_eq!(issue["results"][0]["prs"][0]["number"], 7);
        assert_eq!(issue["results"][0]["sessions"][0]["pr"], 7);

        let branch = run(
            &scope,
            &query(
                QueryTarget::Branch {
                    name: "feat".into(),
                },
                false,
            ),
            &hide_node::Local::of_process(),
        )
        .unwrap();
        assert_eq!(branch["results"][0]["prs"][0]["number"], 7);
        assert_eq!(branch["results"][0]["sessions"][0]["id"], "s-app");

        let session = run(
            &scope,
            &query(QueryTarget::Session { id: "s-app".into() }, false),
            &hide_node::Local::of_process(),
        )
        .unwrap();
        assert_eq!(session["results"][0]["sessions"][0]["id"], "s-app");
        assert_eq!(session["results"][0]["prs"][0]["number"], 7);
    }

    #[test]
    fn a_missing_record_is_unavailable_rather_than_empty() {
        let (state, mut scope) = fixture();
        scope.store = state.path().join("absent.sqlite3");
        assert_eq!(
            run(
                &scope,
                &query(QueryTarget::Pr { number: 7 }, false),
                &hide_node::Local::of_process()
            )
            .unwrap_err()
            .0,
            "links_unavailable"
        );
    }
}
