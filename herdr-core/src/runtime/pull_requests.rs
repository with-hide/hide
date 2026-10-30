//! The Overview's PRs tab (PRD overview-lenses-prs): which of a project's
//! pull requests reach the wire, a pull request linked to an issue, and an
//! agent handed a pull request. Every `gh` call runs on a worker off the
//! lock (`github.rs`); the answers land in `pr_work` or, for an agent's
//! start, in the task-operation slot every start reports through.

use super::*;
use crate::model::{
    CheckoutSnapshot, CodedFailure, PR_CARD_LIMIT, PrCardSnapshot, PrFeedbackSnapshot,
    PrLinkSnapshot, PrWriteSnapshot, PullRequestBadge, PullRequestSnapshot,
};

/// How long a merged pull request with no worktree here stays on the tab (D-52).
const RECENT_MERGE_MS: u64 = 14 * 24 * 60 * 60 * 1000;

/// The pull requests a project's PRs tab can draw (D-32, D-52): every open
/// one, and a merged one while a worktree of its branch is recorded here or
/// for 14 days after it merged. A pull request closed without merging never
/// shows.
pub(super) fn shown_pull_requests(
    all: &[PullRequestSnapshot],
    checkouts: &[CheckoutSnapshot],
    now: u64,
) -> Vec<PullRequestSnapshot> {
    let recorded: BTreeSet<&str> = checkouts
        .iter()
        .filter(|checkout| checkout.is_worktree)
        .filter_map(|checkout| checkout.branch.as_deref())
        .collect();
    crate::github::preferred_per_branch(all)
        .into_iter()
        .filter(|pull_request| match pull_request.badge {
            PullRequestBadge::Closed => false,
            PullRequestBadge::Merged => {
                recorded.contains(pull_request.head_branch.as_str())
                    || pull_request
                        .merged_at_unix_ms
                        .is_some_and(|at| now.saturating_sub(at) <= RECENT_MERGE_MS)
            }
            PullRequestBadge::Review | PullRequestBadge::Open => true,
        })
        .cloned()
        .collect()
}

/// The worktree that holds `pull_request`'s work, the one Hide's issue link
/// lives on.
fn branch_worktree<'a>(
    workspace: &'a WorkspaceSnapshot,
    pull_request: &PullRequestSnapshot,
) -> Option<&'a CheckoutSnapshot> {
    workspace.checkouts.iter().find(|checkout| {
        checkout.is_worktree && checkout.exists && holds_pull_request(checkout, pull_request)
    })
}

fn holds_pull_request(checkout: &CheckoutSnapshot, pull_request: &PullRequestSnapshot) -> bool {
    crate::github::belongs_to_checkout(
        pull_request,
        checkout.branch.as_deref(),
        checkout.head_sha(),
    )
}

/// The checkout a pull request's card opens in when the operator did not ask
/// from one of the project's: the one its agent is in, else the one on its
/// branch, else the project's own (PRD work-tab-pr-lens B22).
fn card_checkout<'a>(
    project: &'a WorkspaceSnapshot,
    pull_request: &PullRequestSnapshot,
    agent_pane: Option<&str>,
    asked_from: Option<&str>,
) -> Option<&'a CheckoutSnapshot> {
    let in_agent = agent_pane.and_then(|pane| {
        project.checkouts.iter().find(|checkout| {
            checkout
                .tabs
                .iter()
                .flat_map(|tab| &tab.panes)
                .any(|found| found.id == pane)
        })
    });
    in_agent
        .or_else(|| {
            asked_from.and_then(|id| project.checkouts.iter().find(|checkout| checkout.id == id))
        })
        .or_else(|| {
            project
                .checkouts
                .iter()
                .find(|checkout| checkout.exists && holds_pull_request(checkout, pull_request))
        })
        .or_else(|| {
            project
                .checkouts
                .iter()
                .find(|checkout| checkout.is_primary)
        })
        .or_else(|| project.checkouts.first())
}

impl Runtime {
    /// `pr_open`: the pull request's card in the lane's 보기 slot, read
    /// again, with the Workspace, its Agent tab and the keyboard where they
    /// are (PRD sidebar-task-lens D-43, D-50). Asked from Home, the pull
    /// request's Workspace opens too and its live agent comes forward (B17).
    /// One event, so a refusal partway cannot leave the screen half moved.
    pub(super) fn open_pull_request_card(&mut self, payload: PrOpenPayload) -> bool {
        if !self.separate_view_areas() {
            self.set_error(
                "pr_open.unsupported",
                "This shell does not draw View areas",
                false,
            );
            return true;
        }
        let Some((_, project)) = self
            .issue_project(&payload.workspace_id)
            .filter(|(_, project)| project.is_git)
        else {
            self.set_error(
                "pr_open.unknown_project",
                "the project is not a Git project this core knows",
                false,
            );
            return true;
        };
        let Some(pull_request) = project
            .pull_requests
            .iter()
            .find(|pull_request| pull_request.number == payload.pr_number)
        else {
            self.set_error(
                "pr_open.unknown_pull_request",
                format!(
                    "pull request #{} is not in this project's list",
                    payload.pr_number
                ),
                false,
            );
            return true;
        };
        let number = payload.pr_number;
        let workspace_id = project.id.clone();
        if payload.open_workspace {
            let lead = project
                .agent_scope
                .prs
                .rows
                .iter()
                .find(|row| row.number == number)
                .and_then(|row| row.agents.first())
                .map(|agent| agent.pane_id.clone());
            let Some(target) = card_checkout(
                &project,
                pull_request,
                lead.as_deref(),
                payload.checkout_id.as_deref(),
            ) else {
                self.set_error(
                    "pr_open.no_checkout",
                    "the project has no checkout to open the card in",
                    false,
                );
                return true;
            };
            let checkout_id = target.id.clone();
            match lead {
                Some(pane) => self.focus_pane(pane, PaneFocusOrigin::Operator, None),
                None => {
                    self.focus_checkout(&workspace_id, &checkout_id);
                }
            }
        }
        crate::diagnostic!(serde_json::json!({
            "component": "pull_requests", "kind": "pr_open",
            "project": workspace_id, "pr": number,
            "open_workspace": payload.open_workspace,
        }));
        self.open_detail(crate::model::DetailItem::PullRequest {
            workspace_id: payload.workspace_id,
            number,
        })
    }

    /// A Git project of this machine or of a node that dials this core,
    /// with the node it is on, and one of its open pull requests, or why not.
    /// A node's pull request is linked and handed on as this machine's is
    /// (PRD core-host-node-move D-29); a device this core dials has none.
    fn open_pull_request(
        &self,
        workspace_id: &str,
        number: u32,
    ) -> Result<(Option<String>, WorkspaceSnapshot, PullRequestSnapshot), String> {
        self.find_pull_request(workspace_id, number, true)
    }

    /// A pull request the list shows, open or settled: a card of a merged
    /// pull request reads and draws like any other (B14, B20); only a write
    /// needs it open.
    fn listed_pull_request(
        &self,
        workspace_id: &str,
        number: u32,
    ) -> Result<(Option<String>, WorkspaceSnapshot, PullRequestSnapshot), String> {
        self.find_pull_request(workspace_id, number, false)
    }

    fn find_pull_request(
        &self,
        workspace_id: &str,
        number: u32,
        open_only: bool,
    ) -> Result<(Option<String>, WorkspaceSnapshot, PullRequestSnapshot), String> {
        let (node, workspace) = self
            .issue_project(workspace_id)
            .filter(|(_, workspace)| workspace.is_git)
            .ok_or_else(|| "the project is not a Git project this core knows".to_owned())?;
        let pull_request = workspace
            .pull_requests
            .iter()
            .find(|pull_request| {
                pull_request.number == number && !(open_only && pull_request.badge.is_settled())
            })
            .cloned()
            .ok_or_else(|| format!("pull request #{number} is not open"))?;
        Ok((node, workspace, pull_request))
    }

    /// `detail` is the failure in the core's own words, for the log; the
    /// card words `failure`.
    fn fail_pr_link(
        &mut self,
        mut link: PrLinkSnapshot,
        step: &str,
        failure: CodedFailure,
        detail: impl std::fmt::Display,
    ) -> bool {
        crate::diagnostic!(serde_json::json!({
            "component": "pull_requests", "kind": "pr_link.failed",
            "request": link.request_id, "pr": link.pr_number, "step": step,
            "code": failure.code, "reason": detail.to_string(),
        }));
        link.step = step.into();
        link.phase = "failed".into();
        link.failure = Some(failure);
        self.snapshot.pr_work.link = Some(link);
        true
    }

    /// `pr_link_issue` (D-13, D-31, D-34): a GitHub issue is linked in Hide
    /// when a worktree holds the pull request's branch, and `Closes #N` is
    /// written into the pull request's body on a worker; a new GitHub issue
    /// is made first, on the same worker. A Local issue, made in the store
    /// first when it is new, is linked in Hide only and GitHub is not written.
    pub(super) fn link_pr_issue(&mut self, payload: PrLinkIssuePayload) -> bool {
        if self
            .snapshot
            .pr_work
            .link
            .as_ref()
            .is_some_and(|slot| slot.phase == "working")
        {
            self.set_error(
                "pr_link.busy",
                "Another pull request is still being linked",
                true,
            );
            return true;
        }
        let link = PrLinkSnapshot {
            request_id: payload.request_id,
            workspace_id: payload.workspace_id,
            pr_number: payload.pr_number,
            step: "link".into(),
            phase: "working".into(),
            issue_key: None,
            issue_id: None,
            created: false,
            failure: None,
        };
        let (machine, workspace, pull_request) =
            match self.open_pull_request(&link.workspace_id, link.pr_number) {
                Ok(target) => target,
                Err(message) => {
                    return self.fail_pr_link(
                        link,
                        "link",
                        CodedFailure::new("pr_link.pr_unknown"),
                        message,
                    );
                }
            };
        crate::diagnostic!(serde_json::json!({
            "component": "pull_requests", "kind": "pr_link.requested",
            "request": link.request_id, "project": workspace.path, "pr": link.pr_number,
            "node": machine, "new_issue": payload.new_issue.is_some(),
        }));
        let local = self.project_source_kind(&workspace) == crate::tasks::SourceKind::Local;
        match (payload.issue_key, payload.new_issue) {
            (Some(key), None) if local => self.link_local_pr(link, &workspace, &pull_request, key),
            (Some(key), None) => {
                self.link_github_pr(link, machine.as_deref(), &workspace, &pull_request, &key)
            }
            (None, Some(new)) => {
                let checked = crate::local_issues::validated_title(&new.title).and_then(|title| {
                    crate::local_issues::validated_body(&new.body).map(|body| (title, body))
                });
                let (title, body) = match checked {
                    Ok(checked) => checked,
                    Err(error) => return self.fail_pr_link(link, "create", error.into(), error),
                };
                if local {
                    return match self.create_local_issue(
                        machine.as_deref(),
                        &workspace.path,
                        &title,
                        &body,
                    ) {
                        Err((failure, detail)) => {
                            self.fail_pr_link(link, "create", failure, detail)
                        }
                        Ok(number) => {
                            let link = PrLinkSnapshot {
                                created: true,
                                ..link
                            };
                            let key = crate::tasks::local_key(&workspace.path, number);
                            self.link_local_pr(link, &workspace, &pull_request, key)
                        }
                    };
                }
                let place = match self.github_place(machine.as_deref(), &workspace.path) {
                    Ok(place) => place,
                    Err(message) => {
                        return self.fail_pr_link(
                            link,
                            "create",
                            CodedFailure::new("pr_link.repository_unknown"),
                            message,
                        );
                    }
                };
                let request_id = link.request_id.clone();
                self.snapshot.pr_work.link = Some(PrLinkSnapshot {
                    step: "create".into(),
                    ..link
                });
                let number = pull_request.number;
                let node = self.own_node();
                let spawned = self.spawn_issue_worker(
                    "pr-issue-create",
                    move || {
                        let created =
                            crate::github::create_issue(node.as_ref(), &place, &title, &body);
                        let written = created.as_ref().ok().map(|issue| {
                            crate::github::write_closing_line(
                                node.as_ref(),
                                &place,
                                number,
                                issue.reference.number,
                            )
                        });
                        (created, written)
                    },
                    move |runtime, (created, written)| {
                        runtime.ingest_pr_issue_created(&request_id, created, written)
                    },
                );
                match spawned {
                    Ok(()) => true,
                    Err(message) => {
                        let link = self.snapshot.pr_work.link.take().expect("slot set above");
                        self.fail_pr_link(
                            link,
                            "create",
                            CodedFailure::new("pr_link.worker_unavailable"),
                            message,
                        )
                    }
                }
            }
            _ => self.fail_pr_link(
                link,
                "link",
                CodedFailure::new("pr_link.request_invalid"),
                "the request names neither one issue to link nor one new issue",
            ),
        }
    }

    /// A Local issue linked to a pull request: Hide's link on the worktree of
    /// its branch, nothing on GitHub (D-34). The link write's answer settles
    /// the slot (`settle_pr_link_write`).
    fn link_local_pr(
        &mut self,
        mut link: PrLinkSnapshot,
        workspace: &WorkspaceSnapshot,
        pull_request: &PullRequestSnapshot,
        key: String,
    ) -> bool {
        let Some(number) = crate::tasks::parse_local_key(&key)
            .filter(|(path, _)| *path == workspace.path)
            .map(|(_, number)| number)
        else {
            return self.fail_pr_link(
                link,
                "link",
                CodedFailure::new("pr_link.issue_not_in_project"),
                format!("{key} is not an issue of this project"),
            );
        };
        let id = crate::local_issues::display_id(number);
        link.issue_key = Some(key);
        link.issue_id = Some(id.clone());
        let Some(checkout) = branch_worktree(workspace, pull_request) else {
            return self.fail_pr_link(
                link,
                "link",
                CodedFailure::new("pr_link.no_worktree"),
                "no worktree holds the pull request's branch",
            );
        };
        let checkout_id = checkout.id.clone();
        link.step = "link".into();
        self.snapshot.pr_work.link = Some(link);
        // Set before the write starts: a core without a live connection
        // answers the write at once, inside `link_checkout_issue`.
        self.pr_link_checkout = Some(checkout_id.clone());
        if let Err((_, message)) = self.link_checkout_issue(&checkout_id, &id) {
            self.pr_link_checkout = None;
            let link = self.snapshot.pr_work.link.take().expect("slot set above");
            return self.fail_pr_link(
                link,
                "link",
                CodedFailure::new("pr_link.link_not_saved"),
                message,
            );
        }
        true
    }

    /// The Hide link a Local `pr_link_issue` waited on has been written, or
    /// not; a GitHub link is settled by the body write instead.
    pub(super) fn settle_pr_link_write(&mut self, error: Option<String>) {
        let Some(link) = self
            .snapshot
            .pr_work
            .link
            .take_if(|slot| slot.phase == "working" && slot.step == "link")
        else {
            return;
        };
        match error {
            Some(message) => {
                self.fail_pr_link(
                    link,
                    "link",
                    CodedFailure::new("pr_link.link_not_saved"),
                    message,
                );
            }
            None => {
                self.snapshot.pr_work.link = Some(PrLinkSnapshot {
                    phase: "ready".into(),
                    ..link
                })
            }
        }
    }

    /// A GitHub issue the project has, linked to a pull request: Hide's link
    /// on the worktree of its branch at once, and `Closes #N` in the body on
    /// a worker. A failed body write keeps the link (D-53).
    fn link_github_pr(
        &mut self,
        mut link: PrLinkSnapshot,
        machine: Option<&str>,
        workspace: &WorkspaceSnapshot,
        pull_request: &PullRequestSnapshot,
        key: &str,
    ) -> bool {
        let reference = crate::tasks::parse_github_key(key).filter(|reference| {
            workspace.home_issues.repository.as_deref() == Some(reference.repository.as_str())
        });
        let Some(reference) = reference else {
            return self.fail_pr_link(
                link,
                "link",
                CodedFailure::new("pr_link.issue_not_in_project"),
                format!("{key} is not an issue of this repository"),
            );
        };
        let place = match self.github_place(machine, &workspace.path) {
            Ok(place) => place,
            Err(message) => {
                return self.fail_pr_link(
                    link,
                    "body",
                    CodedFailure::new("pr_link.repository_unknown"),
                    message,
                );
            }
        };
        link.issue_key = Some(key.to_owned());
        link.issue_id = Some(format!("#{}", reference.number));
        link.step = "body".into();
        let request_id = link.request_id.clone();
        self.snapshot.pr_work.link = Some(link);
        self.link_pr_branch(workspace, pull_request, &reference);
        let number = pull_request.number;
        let issue = reference.number;
        let node = self.own_node();
        let spawned = self.spawn_issue_worker(
            "pr-body-write",
            move || crate::github::write_closing_line(node.as_ref(), &place, number, issue),
            move |runtime, written| runtime.finish_pr_body(&request_id, &reference, written),
        );
        if let Err(message) = spawned {
            let link = self.snapshot.pr_work.link.take().expect("slot set above");
            return self.fail_pr_link(
                link,
                "body",
                CodedFailure::new("pr_link.worker_unavailable"),
                message,
            );
        }
        true
    }

    /// Hide's link for a GitHub issue on the worktree of the pull request's
    /// branch, when there is one. Its failure is the diagnostic log's: the
    /// body's `Closes` line links the two for GitHub and Hide alike.
    fn link_pr_branch(
        &mut self,
        workspace: &WorkspaceSnapshot,
        pull_request: &PullRequestSnapshot,
        reference: &crate::issues::IssueReference,
    ) {
        let Some(checkout) = branch_worktree(workspace, pull_request) else {
            return;
        };
        let checkout_id = checkout.id.clone();
        if let Err((kind, message)) = self.link_checkout_issue(&checkout_id, &reference.token()) {
            self.push_diagnostic(kind, message);
        }
    }

    pub(crate) fn ingest_pr_issue_created(
        &mut self,
        request_id: &str,
        created: Result<crate::issues::IssueSnapshot, String>,
        written: Option<Result<bool, String>>,
    ) -> bool {
        let Some(link) = self
            .snapshot
            .pr_work
            .link
            .take_if(|slot| slot.request_id == request_id && slot.phase == "working")
        else {
            return false;
        };
        let issue = match created {
            Ok(issue) => issue,
            Err(message) => {
                return self.fail_pr_link(
                    link,
                    "create",
                    CodedFailure::new("pr_link.create_failed"),
                    message,
                );
            }
        };
        let reference = issue.reference.clone();
        let target = self.open_pull_request(&link.workspace_id, link.pr_number);
        if let Some((machine, workspace)) = self.issue_project(&link.workspace_id) {
            self.remember_created_issue(machine.as_deref(), &workspace.path, issue);
        }
        self.snapshot.pr_work.link = Some(PrLinkSnapshot {
            step: "body".into(),
            issue_key: Some(crate::tasks::github_key(&reference)),
            issue_id: Some(format!("#{}", reference.number)),
            created: true,
            ..link
        });
        if let Ok((_, workspace, pull_request)) = &target
            && crate::links::repository_of(&pull_request.url).as_deref()
                == Some(reference.repository.as_str())
        {
            self.link_pr_branch(workspace, pull_request, &reference);
        }
        let written =
            written.unwrap_or_else(|| Err("the pull request's body was not written".to_owned()));
        self.finish_pr_body(request_id, &reference, written);
        true
    }

    /// The body write's answer (D-31, D-53): the pull request closes the
    /// issue from now on, shown at once and confirmed by the next read; a
    /// failure keeps the issue and the link and says why, for `본문 다시 쓰기`.
    pub(crate) fn finish_pr_body(
        &mut self,
        request_id: &str,
        reference: &crate::issues::IssueReference,
        written: Result<bool, String>,
    ) -> bool {
        let Some(link) = self
            .snapshot
            .pr_work
            .link
            .take_if(|slot| slot.request_id == request_id && slot.phase == "working")
        else {
            return false;
        };
        let wrote = match written {
            Ok(wrote) => wrote,
            Err(message) => {
                return self.fail_pr_link(
                    link,
                    "body",
                    CodedFailure::new("pr_link.body_not_written"),
                    message,
                );
            }
        };
        crate::diagnostic!(serde_json::json!({
            "component": "pull_requests", "kind": "pr_link.body_written",
            "request": link.request_id, "pr": link.pr_number, "issue": reference.number,
            "already_there": !wrote,
        }));
        let machine = self.issue_project(&link.workspace_id);
        if let Some((machine, workspace)) = &machine {
            let github = match machine {
                None => Some(&mut self.github),
                Some(node) => self.device_github.get_mut(node),
            };
            if let Some(project) = github.and_then(|github| {
                github.projects.iter_mut().find(|project| {
                    project.root_path == workspace.path
                        && project.issues.repository.as_deref()
                            == Some(reference.repository.as_str())
                })
            }) {
                for pull_request in &mut project.pull_requests {
                    if pull_request.number == link.pr_number
                        && !pull_request.closing_issues.contains(reference)
                    {
                        pull_request.closing_issues.push(reference.clone());
                    }
                }
            }
            match machine {
                None => self.refresh_pull_requests(&workspace.path),
                Some(node) => self.reread_node_github(node, &workspace.path),
            }
        }
        self.snapshot.pr_work.link = Some(PrLinkSnapshot {
            phase: "ready".into(),
            ..link
        });
        self.refresh_issue_rows(machine.and_then(|(machine, _)| machine).as_deref());
        true
    }

    /// `pr_feedback_read`: what the pull request's failed checks and change
    /// requests say, for the first prompt of the agent it is handed to (D-46).
    pub(super) fn read_pr_feedback(&mut self, payload: PrFeedbackReadPayload) -> bool {
        let mut slot = PrFeedbackSnapshot {
            request_id: payload.request_id,
            device_id: String::new(),
            root: String::new(),
            pr_number: payload.pr_number,
            phase: "reading".into(),
            body: None,
            failed_checks: Vec::new(),
            change_requests: Vec::new(),
            failure: None,
        };
        let place = self
            .open_pull_request(&payload.workspace_id, payload.pr_number)
            .and_then(|(machine, workspace, _)| {
                slot.device_id = machine
                    .clone()
                    .unwrap_or_else(|| self.node.as_str().to_owned());
                slot.root = workspace.path.clone();
                self.github_place(machine.as_deref(), &workspace.path)
            });
        let place = match place {
            Ok(place) => place,
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "pull_requests", "kind": "pr_feedback.failed",
                    "request": slot.request_id, "pr": slot.pr_number, "reason": message,
                }));
                slot.phase = "failed".into();
                slot.failure = Some("pr_feedback.read_failed".to_owned());
                self.snapshot.pr_work.feedback = Some(slot);
                return true;
            }
        };
        let request_id = slot.request_id.clone();
        self.snapshot.pr_work.feedback = Some(slot);
        let number = payload.pr_number;
        let answered = request_id.clone();
        let node = self.own_node();
        if let Err(message) = self.spawn_issue_worker(
            "pr-feedback",
            move || crate::github::pr_feedback(node.as_ref(), &place, number),
            move |runtime, answer| runtime.ingest_pr_feedback(&answered, answer),
        ) {
            self.ingest_pr_feedback(&request_id, Err(message));
        }
        true
    }

    pub(crate) fn ingest_pr_feedback(
        &mut self,
        request_id: &str,
        answer: Result<crate::github::PrFeedback, String>,
    ) -> bool {
        let Some(slot) = self
            .snapshot
            .pr_work
            .feedback
            .as_mut()
            .filter(|slot| slot.request_id == request_id && slot.phase == "reading")
        else {
            return false;
        };
        match answer {
            Ok(feedback) => {
                slot.phase = "ready".into();
                slot.body = Some(feedback.body);
                slot.failed_checks = feedback.failed_checks;
                slot.change_requests = feedback.change_requests;
            }
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "pull_requests", "kind": "pr_feedback.failed",
                    "request": request_id, "pr": slot.pr_number, "reason": message,
                }));
                slot.phase = "failed".into();
                slot.failure = Some("pr_feedback.read_failed".to_owned());
            }
        }
        true
    }

    /// `pr_delegate` (D-12, D-46): the agent starts in the checkout of the
    /// pull request's branch with the operator's first prompt, or, with no
    /// such checkout, in a new worktree of that existing branch, on the
    /// machine the project is on. Both report through the task-operation
    /// slot, as every start does.
    pub(super) fn delegate_pr(&mut self, mut payload: PrDelegatePayload) -> bool {
        let Some(provider) = hide_agent_adapter::start_kind(&payload.provider) else {
            self.set_error(
                "pr_delegate.unknown_provider",
                format!("No agent provider named {}", payload.provider),
                false,
            );
            return true;
        };
        payload.provider = provider.to_owned();
        let (machine, workspace, pull_request) =
            match self.open_pull_request(&payload.workspace_id, payload.pr_number) {
                Ok(target) => target,
                Err(message) => {
                    self.set_error("pr_delegate.unknown_pull_request", message, false);
                    return true;
                }
            };
        let prompt = Some(payload.prompt.trim().to_owned()).filter(|prompt| !prompt.is_empty());
        let checkout = workspace
            .checkouts
            .iter()
            .find(|checkout| checkout.exists && holds_pull_request(checkout, &pull_request));
        crate::diagnostic!(serde_json::json!({
            "component": "pull_requests", "kind": "pr_delegate.requested",
            "project": workspace.path, "pr": pull_request.number, "node": machine,
            "checkout": checkout.map(|checkout| checkout.id.clone()),
        }));
        match checkout {
            Some(checkout) => self.agent_start_in_checkout(AgentStartInCheckoutPayload {
                target: None,
                confirm_only: false,
                checkout_path: Some(checkout.path.clone()),
                device_id: machine,
                home: false,
                provider: payload.provider,
                prompt,
                model: payload.model,
                request_id: payload.request_id,
                resume_session_id: None,
                resume_session_path: None,
                stay_behind: false,
            }),
            None => self.start_answered_worktree_task(
                CreateWorktreePayload {
                    request_id: payload.request_id,
                    device_id: machine,
                    repository_root: workspace.path.clone(),
                    branch: pull_request.head_branch.clone(),
                    base_branch: None,
                    agent_kind: Some(payload.provider),
                    purpose: None,
                    task_key: None,
                    prompt,
                    model: payload.model,
                },
                true,
            ),
        }
    }
}

impl Runtime {
    /// The card of one pull request, made when it is new; the cards kept are
    /// capped and the one asked for least recently goes first (B50).
    pub(super) fn pr_card_slot(&mut self, workspace_id: &str, number: u32) -> &mut PrCardSnapshot {
        let cards = &mut self.snapshot.pr_work.cards;
        match cards
            .iter()
            .position(|card| card.workspace_id == workspace_id && card.number == number)
        {
            Some(index) => {
                let card = cards.remove(index);
                cards.push(card);
            }
            None => {
                if cards.len() >= PR_CARD_LIMIT {
                    cards.remove(0);
                }
                cards.push(PrCardSnapshot {
                    workspace_id: workspace_id.to_owned(),
                    number,
                    ..PrCardSnapshot::default()
                });
            }
        }
        cards.last_mut().expect("a card was just pushed")
    }

    fn existing_pr_card(&mut self, workspace_id: &str, number: u32) -> Option<&mut PrCardSnapshot> {
        self.snapshot
            .pr_work
            .cards
            .iter_mut()
            .find(|card| card.workspace_id == workspace_id && card.number == number)
    }

    /// `pr_card_read` from the operator (Look again): a failed write's notice
    /// is the card of a pull request they have now looked at again, so it is
    /// dismissed, and the list and the card are read.
    pub(super) fn look_again_pr_card(&mut self, payload: PrCardReadPayload) -> bool {
        let dismissed = self
            .existing_pr_card(&payload.workspace_id, payload.pr_number)
            .is_some_and(|card| {
                let failed = card
                    .write
                    .as_ref()
                    .is_some_and(|write| write.phase == "failed");
                if failed {
                    card.write = None;
                }
                failed
            });
        // The operator looking again after a refusal wants the list read too:
        // the refusal may be the list lagging the card (D-22).
        if dismissed
            && let Ok((machine, workspace, _)) =
                self.listed_pull_request(&payload.workspace_id, payload.pr_number)
        {
            match machine.as_deref() {
                None => self.refresh_pull_requests(&workspace.path),
                Some(node) => self.reread_node_github(node, &workspace.path),
            }
        }
        self.read_pr_card(payload) || dismissed
    }

    /// The card's read: the parts of the card the list does not carry, read on
    /// a worker. The last read's parts stay drawn while a new one runs; a
    /// read already running is not started twice (D-06, B52).
    pub(super) fn read_pr_card(&mut self, payload: PrCardReadPayload) -> bool {
        let number = payload.pr_number;
        let (machine, project) = match self.listed_pull_request(&payload.workspace_id, number) {
            Ok((machine, workspace, _)) => (machine, workspace),
            Err(message) => {
                self.set_error("pr_card_read.unknown_pull_request", message, false);
                return true;
            }
        };
        let place = match self.github_place(machine.as_deref(), &project.path) {
            Ok(place) => place,
            Err(message) => {
                let card = self.pr_card_slot(&payload.workspace_id, number);
                card.reading = false;
                card.failed = vec!["body", "files", "threads"];
                crate::diagnostic!(serde_json::json!({
                    "component": "pull_requests", "kind": "pr_card.unplaced",
                    "project": payload.workspace_id, "pr": number, "reason": message,
                }));
                return true;
            }
        };
        let card = self.pr_card_slot(&payload.workspace_id, number);
        if card.reading {
            return false;
        }
        card.reading = true;
        let workspace_id = payload.workspace_id;
        let answered = workspace_id.clone();
        let node = self.own_node();
        if let Err(message) = self.spawn_issue_worker(
            "pr-card",
            move || crate::github::card::read(node.as_ref(), &place, number),
            move |runtime, answer| runtime.ingest_pr_card(&answered, number, answer),
        ) {
            self.ingest_pr_card(&workspace_id, number, Err(message));
        }
        true
    }

    pub(crate) fn ingest_pr_card(
        &mut self,
        workspace_id: &str,
        number: u32,
        answer: Result<crate::github::card::CardRead, String>,
    ) -> bool {
        let Some(card) = self.existing_pr_card(workspace_id, number) else {
            return false;
        };
        card.reading = false;
        match answer {
            Ok(read) => {
                card.head = read.head.or_else(|| card.head.take());
                if read.body.is_some() {
                    card.body = read.body;
                }
                if read.files.is_some() {
                    card.files = read.files;
                    card.files_total = read.files_total;
                }
                if read.threads.is_some() {
                    card.threads = read.threads;
                }
                if read.merge.is_some() {
                    card.merge = read.merge;
                }
                card.checks_completed_at_unix_ms = read.checks_completed_at;
                card.review_at_unix_ms = read.review_at;
                if !read.times_unread.is_empty() {
                    // The card is drawn without the times; only the log says why (B21).
                    crate::diagnostic!(serde_json::json!({
                        "component": "pull_requests", "kind": "pr_card.times_unread",
                        "project": workspace_id, "pr": number, "parts": read.times_unread,
                    }));
                }
                card.failed = read.failed;
            }
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "pull_requests", "kind": "pr_card.failed",
                    "project": workspace_id, "pr": number, "reason": message,
                }));
                card.failed = vec!["body", "files", "threads"];
            }
        }
        true
    }

    /// `pr_merge`: Merge from the card. The only event that writes a merge,
    /// and it is the operator's own (D-22). A second ask while one is
    /// working is the same intent and is not sent again; asking again after
    /// the pull request merged succeeds without a second write.
    pub(super) fn merge_pull_request(&mut self, payload: PrMergePayload) -> bool {
        let number = payload.pr_number;
        let write = PrWriteSnapshot {
            action: "merge",
            head: payload.head.clone(),
            method: payload.method,
            phase: "working",
            reason: None,
        };
        self.start_pr_write(
            payload.workspace_id,
            number,
            write,
            crate::github::card::Write::Merge {
                number,
                method: payload.method,
                head: payload.head,
            },
        )
    }

    /// `pr_update_branch`: brings the branch up to its base (D-22).
    pub(super) fn update_pull_request_branch(&mut self, payload: PrUpdateBranchPayload) -> bool {
        let number = payload.pr_number;
        let write = PrWriteSnapshot {
            action: "update_branch",
            head: payload.head.clone(),
            method: None,
            phase: "working",
            reason: None,
        };
        self.start_pr_write(
            payload.workspace_id,
            number,
            write,
            crate::github::card::Write::UpdateBranch {
                number,
                head: payload.head,
            },
        )
    }

    fn start_pr_write(
        &mut self,
        workspace_id: String,
        number: u32,
        write: PrWriteSnapshot,
        request: crate::github::card::Write,
    ) -> bool {
        let action = write.action;
        let (machine, project, pull_request) = match self.open_pull_request(&workspace_id, number) {
            Ok((machine, workspace, pull_request)) => (machine, workspace, pull_request),
            Err(message) => {
                self.set_error(format!("pr_{action}.unknown_pull_request"), message, false);
                return true;
            }
        };
        let place = match self.github_place(machine.as_deref(), &project.path) {
            Ok(place) => place,
            Err(message) => {
                self.set_error(format!("pr_{action}.unplaced"), message, false);
                return true;
            }
        };
        let card = self.pr_card_slot(&workspace_id, number);
        if card.write.as_ref().is_some_and(|w| w.phase == "working") {
            return false;
        }
        // GitHub only proves the head is its current one. The state that
        // enabled the button (checks, merge state) was judged on the list's
        // head, so a card read at another head is refused and both are read
        // again before anything is written (D-22, B31).
        if pull_request
            .head_oid
            .as_deref()
            .is_some_and(|listed| !listed.eq_ignore_ascii_case(&write.head))
        {
            crate::diagnostic!(serde_json::json!({
                "component": "pull_requests", "kind": "pr_write.head_differs_from_list",
                "project": workspace_id, "pr": number, "action": action,
            }));
            let mut refused = write;
            refused.phase = "failed";
            refused.reason = Some("head_moved");
            card.write = Some(refused);
            self.reread_pull_request(machine.as_deref(), &project.path, &workspace_id, number);
            return true;
        }
        card.write = Some(write);
        crate::diagnostic!(serde_json::json!({
            "component": "pull_requests", "kind": "pr_write.requested",
            "project": workspace_id, "pr": number, "action": action, "node": machine,
        }));
        let root = project.path;
        let (answered, answered_machine) = (workspace_id.clone(), machine);
        let node = self.own_node();
        let spawned = self.spawn_issue_worker(
            "pr-write",
            move || crate::github::card::run_write(node.as_ref(), &place, &request),
            {
                let root = root.clone();
                let machine = answered_machine.clone();
                move |runtime, answer| {
                    runtime.ingest_pr_write(&answered, number, machine.as_deref(), &root, answer)
                }
            },
        );
        if let Err(message) = spawned {
            self.ingest_pr_write(
                &workspace_id,
                number,
                answered_machine.as_deref(),
                &root,
                Err(crate::github::card::WriteFailure {
                    reason: "failed",
                    detail: message,
                }),
            );
        }
        true
    }

    pub(crate) fn ingest_pr_write(
        &mut self,
        workspace_id: &str,
        number: u32,
        machine: Option<&str>,
        root: &str,
        answer: Result<(), crate::github::card::WriteFailure>,
    ) -> bool {
        let Some(card) = self.existing_pr_card(workspace_id, number) else {
            return false;
        };
        let Some(write) = card.write.as_mut().filter(|w| w.phase == "working") else {
            return false;
        };
        let action = write.action;
        let reread = match answer {
            Ok(()) => {
                write.phase = "done";
                // A merge with a method is made now; one asked of a merge
                // queue only joins it, and GitHub's list says when it lands.
                // A node's project is read from that node and waits for its
                // poll, so the overlay is for this machine's own projects.
                let merged_now = action == "merge" && write.method.is_some() && machine.is_none();
                if merged_now {
                    self.merged_here
                        .insert((root.to_owned(), number), unix_milliseconds());
                    let mut github = self.github.clone();
                    self.read_merges_made_here(&mut github);
                    self.github = github;
                    self.apply_pull_requests();
                }
                true
            }
            Err(failure) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "pull_requests", "kind": "pr_write.failed",
                    "project": workspace_id, "pr": number, "action": action,
                    "code": failure.reason, "detail": failure.detail,
                }));
                write.phase = "failed";
                write.reason = Some(failure.reason);
                // A moved head means the card shows an old PR, and a branch
                // update that conflicted has made the row a conflict: read it again.
                failure.reason == "head_moved"
                    || (action == "update_branch" && failure.reason == "conflict")
            }
        };
        if reread {
            self.reread_pull_request(machine, root, workspace_id, number);
        }
        true
    }

    /// The list of the project's pull requests and the card of one, read again.
    fn reread_pull_request(
        &mut self,
        machine: Option<&str>,
        root: &str,
        workspace_id: &str,
        number: u32,
    ) {
        match machine {
            None => self.refresh_pull_requests(root),
            Some(node) => self.reread_node_github(node, root),
        }
        self.read_pr_card(PrCardReadPayload {
            workspace_id: workspace_id.to_owned(),
            pr_number: number,
            look_again: false,
        });
    }
}
