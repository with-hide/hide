use super::*;

fn memory_open(workspace: Option<&str>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "memory_open",
        "payload": {"workspace_id": workspace},
    }))
    .expect("memory open event")
}

/// The 제안 tab opens and closes with the Memory tab, since the Hide AI
/// tool is one tool (PRD hide-ai-suggestions D-01), and rides its own delta
/// revision: a reader that has it is not sent it again.
#[test]
fn the_suggestions_section_opens_and_closes_with_the_hide_ai_tool() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let workspace = runtime.snapshot.navigator.workspaces[0].id.clone();
    let before = runtime.snapshot_delta_payload(0);
    assert!(
        before.suggestions.is_none(),
        "absent until the tool opens a Project"
    );

    assert!(runtime.dispatch_json(&memory_open(Some(&workspace))));
    let opened = runtime.snapshot_delta_payload(before.revision);
    let project = opened
        .suggestions
        .as_ref()
        .and_then(|section| section.project.as_ref())
        .expect("the opened Project's tab");
    assert_eq!(project.workspace_id, workspace);
    assert!(project.loading, "nothing read yet");
    assert_eq!(project.counts.rail, 0);

    let again = runtime.snapshot_delta_payload(opened.revision);
    assert!(
        again.suggestions.is_none(),
        "an unchanged section is not sent again"
    );

    assert!(runtime.dispatch_json(&memory_open(None)));
    let closed = runtime.snapshot_delta_payload(opened.revision);
    let section = closed.suggestions.expect("the closed section is sent");
    assert!(section.project.is_none());
}

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": payload,
    }))
    .expect("event")
}

fn opened_project(runtime: &mut Runtime) -> (String, String) {
    runtime.ingest_session(Ok(context_payload()));
    let workspace = &runtime.snapshot.navigator.workspaces[0];
    let opened = (workspace.id.clone(), workspace.path.clone());
    runtime.dispatch_json(&memory_open(Some(&opened.0)));
    opened
}

fn tab(runtime: &mut Runtime) -> crate::model::ProjectSuggestionsSnapshot {
    let delta = runtime.snapshot_delta_payload(0);
    delta
        .suggestions
        .and_then(|section| section.project.clone())
        .expect("the opened Project's tab")
}

/// What the worker reads is shown with what only the lock knows laid on
/// it: Hide AI's readiness, GitHub's answer, the switches that cannot
/// record here and the `GitHub 연결 필요` hold (B5, B18, B19).
#[test]
fn the_tab_shows_the_workers_read_with_hide_ai_and_github_laid_on_it() {
    use crate::model::{
        SignalUnavailable, SuggestionGroup, SuggestionHold, SuggestionRowSnapshot,
        SuggestionSettingsSnapshot, SuggestionSignalSnapshot, SuggestionSource, SuggestionState,
        SuggestionsAi, SuggestionsGithub,
    };
    use crate::suggestions::ledger::Signal;
    use crate::suggestions::notebook::Outcome;
    let mut runtime = runtime();
    let (workspace, root) = opened_project(&mut runtime);
    let row = |id: &str, outcome| SuggestionRowSnapshot {
        id: id.into(),
        source: SuggestionSource::Notebook,
        group: SuggestionGroup::Choose,
        state: SuggestionState::New,
        discomfort: None,
        cause: None,
        fix: None,
        outcome,
        basis: None,
        auto: false,
        hold: None,
        escalated: None,
        evidence: Vec::new(),
        attempt: None,
        watch: None,
    };
    runtime.ingest_suggestions_read(
        workspace.clone(),
        crate::model::ProjectSuggestionsSnapshot {
            workspace_id: workspace.clone(),
            enabled: true,
            settings: SuggestionSettingsSnapshot {
                signals: Signal::ALL
                    .into_iter()
                    .map(|signal| SuggestionSignalSnapshot {
                        signal,
                        enabled: true,
                        unavailable: None,
                    })
                    .collect(),
                ..Default::default()
            },
            rows: vec![row("s1", Outcome::Pr), row("s2", Outcome::Issue)],
            ..Default::default()
        },
    );
    let read = tab(&mut runtime);
    assert_eq!(
        read.ai,
        SuggestionsAi::NotReady,
        "no Hide AI choice read yet"
    );
    assert_eq!(read.github, SuggestionsGithub::Connected);
    assert_eq!(read.counts.suggestions, 2);
    assert!(read.rows.iter().all(|row| row.hold.is_none()));

    runtime.ai_settings = Some(hide_ai::AiSettings {
        provider: hide_ai::ProviderId::OPENCODE,
        chosen: true,
        enabled: true,
        ..hide_ai::AiSettings::default()
    });
    runtime
        .github
        .projects
        .push(crate::model::GithubProjectSnapshot {
            root_path: root,
            status: crate::model::GithubStatusSnapshot {
                failure_category: Some(hide_node_link::gh::GithubFailureCategory::NoGithubRemote),
                ..Default::default()
            },
            ..Default::default()
        });
    let read = tab(&mut runtime);
    assert_eq!(read.ai, SuggestionsAi::CannotInvestigate);
    assert_eq!(read.github, SuggestionsGithub::Missing);
    let holds = read
        .rows
        .iter()
        .map(|row| (row.id.as_str(), row.hold))
        .collect::<Vec<_>>();
    assert_eq!(
        holds,
        [("s1", Some(SuggestionHold::GithubNeeded)), ("s2", None)],
        "only a pull request needs GitHub"
    );
    let unavailable = read
        .settings
        .signals
        .iter()
        .map(|signal| (signal.signal, signal.unavailable))
        .collect::<Vec<_>>();
    assert_eq!(
        unavailable,
        [
            (Signal::FailedTool, None),
            (Signal::AgentWait, None),
            (Signal::Correction, Some(SignalUnavailable::MemoryOff)),
            (Signal::LabelMissed, None),
            (Signal::CheckFailed, Some(SignalUnavailable::NoGithub)),
            (Signal::ChangeRequest, Some(SignalUnavailable::NoGithub)),
        ]
    );
}

/// Each screen event becomes one command to the worker, and a settings
/// event naming two settings is refused whole.
#[test]
fn each_suggestion_event_hands_the_worker_one_command() {
    use crate::suggestions::notebook::LineId;
    use crate::suggestions::policy::Quantity;
    use crate::suggestions::worker::{Command, LineAction, SettingsChange, SuggestionClient};
    let mut runtime = runtime();
    let (client, commands) = SuggestionClient::for_test();
    runtime.suggestions_work.client = Some(client);
    let (workspace, root) = opened_project(&mut runtime);
    match commands.try_recv() {
        Ok(Command::Open {
            workspace_id,
            project,
        }) => {
            assert_eq!(workspace_id, workspace);
            assert_eq!(project.path, root);
        }
        _ => panic!("the tool's Project is opened on the worker"),
    }

    runtime.dispatch_json(&event(
        "suggestion_action",
        serde_json::json!({"workspace_id": workspace, "id": "s3", "action": "dismiss"}),
    ));
    assert!(matches!(
        commands.try_recv(),
        Ok(Command::Action {
            line: LineId(3),
            action: LineAction::Dismiss,
            ..
        })
    ));
    for (name, expected) in [("apply", LineAction::Apply), ("retry", LineAction::Retry)] {
        runtime.dispatch_json(&event(
            "suggestion_action",
            serde_json::json!({"workspace_id": workspace, "id": "s3", "action": name}),
        ));
        match commands.try_recv() {
            Ok(Command::Action {
                line: LineId(3),
                action,
                ..
            }) => assert_eq!(action, expected),
            _ => panic!("{name} reaches the worker"),
        }
    }

    runtime.dispatch_json(&event(
        "suggestion_settings",
        serde_json::json!({"workspace_id": workspace, "quantity": "many"}),
    ));
    assert!(matches!(
        commands.try_recv(),
        Ok(Command::Settings {
            change: SettingsChange::Quantity(Quantity::Many),
            ..
        })
    ));
    runtime.dispatch_json(&event(
        "suggestion_settings",
        serde_json::json!({"workspace_id": workspace, "quantity": "few", "suggestions_enabled": false}),
    ));
    assert!(commands.try_recv().is_err(), "two settings in one event");

    runtime.dispatch_json(&event(
        "hide_ai_switch",
        serde_json::json!({"workspace_id": workspace, "on": false}),
    ));
    assert!(matches!(
        commands.try_recv(),
        Ok(Command::Settings {
            change: SettingsChange::HideAiOff { memory_on: false },
            ..
        })
    ));

    runtime.dispatch_json(&event("suggestions_intro_seen", serde_json::json!({})));
    assert!(matches!(commands.try_recv(), Ok(Command::IntroSeen)));
    runtime.dispatch_json(&event(
        "hide_ai_wake",
        serde_json::json!({"cause": "unlock"}),
    ));
    assert!(matches!(commands.try_recv(), Ok(Command::Wake)));

    runtime.dispatch_json(&memory_open(None));
    assert!(matches!(commands.try_recv(), Ok(Command::Close)));
}

/// What GitHub's reads say went wrong in a Project's pull requests reaches
/// the worker once (B4, D-19): each check an open pull request failed, on
/// the head it failed on, and one that merged after a review asked for
/// changes.
#[test]
fn a_failed_check_and_a_merge_after_changes_are_handed_on_once() {
    use crate::model::{GithubProjectSnapshot, PrFailedCheck, PullRequestSnapshot, ReviewDecision};
    use crate::suggestions::ledger::Signal;
    use crate::suggestions::worker::{Command, SuggestionClient};
    let mut runtime = runtime();
    let (client, commands) = SuggestionClient::for_test();
    runtime.suggestions_work.client = Some(client);
    let pr = |number: u32| {
        let mut pr = PullRequestSnapshot {
            closing_issues: Vec::new(),
            title: format!("Pull request {number}"),
            checks: crate::model::PullRequestChecks::Failed,
            number,
            head_branch: format!("work-{number}"),
            base_branch: "main".into(),
            url: format!("https://github.com/acme/app/pull/{number}"),
            badge: crate::model::PullRequestBadge::Open,
            review: None,
            is_draft: false,
            merged_at_unix_ms: None,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            closed_at_unix_ms: None,
            head_oid: Some(format!("{number:040x}")),
            cross_repository: false,
            work: Default::default(),
        };
        pr.work.failed_checks = vec![PrFailedCheck {
            name: Some("verify".into()),
            url: None,
        }];
        pr
    };
    let mut reviewed = pr(8);
    reviewed.work.failed_checks.clear();
    reviewed.review = Some(ReviewDecision::ChangesRequested);
    runtime.github.projects = vec![GithubProjectSnapshot {
        root_path: "/repo".into(),
        pull_requests: vec![pr(7), reviewed.clone()],
        ..Default::default()
    }];
    let sent = |commands: &std::sync::mpsc::Receiver<Command>| match commands.try_recv() {
        Ok(Command::Observe { project, facts }) => {
            assert_eq!(project.path, "/repo");
            facts
                .into_iter()
                .map(|fact| {
                    (
                        fact.signal,
                        fact.tag,
                        fact.pr.map(|pr| pr.number),
                        fact.head,
                    )
                })
                .collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    runtime.feed_suggestion_prs();
    assert_eq!(
        sent(&commands),
        [(
            Signal::CheckFailed,
            "verify".to_owned(),
            Some(7),
            Some(format!("{:040x}", 7))
        )]
    );
    runtime.feed_suggestion_prs();
    assert!(
        sent(&commands).is_empty(),
        "the same read sends nothing again"
    );

    reviewed.merged_at_unix_ms = Some(1_000);
    reviewed.review = None;
    runtime.github.projects[0].pull_requests[1] = reviewed;
    runtime.feed_suggestion_prs();
    assert_eq!(
        sent(&commands),
        [(
            Signal::ChangeRequest,
            "changes_requested".to_owned(),
            Some(8),
            Some(format!("{:040x}", 8))
        )]
    );
}

/// A press on a Memory row (`m3`) is Memory's own action on that card
/// (D-02): apply promotes it, and nothing reaches the notebook's worker.
#[test]
fn a_press_on_a_memory_row_is_memorys_own_action() {
    use crate::suggestions::worker::SuggestionClient;
    let mut runtime = runtime();
    let (client, commands) = SuggestionClient::for_test();
    runtime.suggestions_work.client = Some(client);
    let (workspace, _) = opened_project(&mut runtime);
    while commands.try_recv().is_ok() {}
    runtime.dispatch_json(&event(
        "suggestion_action",
        serde_json::json!({"workspace_id": workspace, "id": "m3", "action": "apply"}),
    ));
    assert!(commands.try_recv().is_err(), "not a notebook line");
    let memory = runtime
        .snapshot
        .memory
        .as_ref()
        .and_then(|memory| memory.project.as_ref())
        .expect("the opened Project's Memory");
    assert_eq!(
        memory.action_failure.or(memory.pending_action),
        Some(crate::model::MemoryAction::Promote),
        "Memory took the promotion, here refused for want of its writer"
    );
}

/// B11: an attempt whose branch has a pull request in the PR catalog is
/// handed to the worker as open, once, with its home's facts kept for the
/// Needs You turn.
#[test]
fn a_followed_attempt_follows_its_branchs_pull_request() {
    use crate::model::{
        GithubProjectSnapshot, PullRequestBadge, PullRequestChecks, PullRequestSnapshot,
    };
    use crate::suggestions::apply::FollowedAttempt;
    use crate::suggestions::lifecycle::{AttemptMove, AttemptPhase};
    use crate::suggestions::notebook::{LineId, Outcome};
    use crate::suggestions::store::ProjectLocation;
    use crate::suggestions::worker::{Command, SuggestionClient};
    let mut runtime = runtime();
    let (client, commands) = SuggestionClient::for_test();
    runtime.suggestions_work.client = Some(client);
    let started = crate::runtime::unix_milliseconds();
    runtime.github.projects = vec![GithubProjectSnapshot {
        root_path: "/repo".into(),
        pull_requests: vec![PullRequestSnapshot {
            closing_issues: Vec::new(),
            title: "Fix the check".into(),
            checks: PullRequestChecks::Passing,
            number: 31,
            head_branch: "hide-suggestion-s2-x".into(),
            base_branch: "main".into(),
            url: "https://github.com/acme/app/pull/31".into(),
            badge: PullRequestBadge::Open,
            review: None,
            is_draft: false,
            merged_at_unix_ms: None,
            updated_at_unix_ms: None,
            created_at_unix_ms: Some(started),
            closed_at_unix_ms: None,
            head_oid: None,
            cross_repository: false,
            work: Default::default(),
        }],
        ..Default::default()
    }];
    let attempt = FollowedAttempt {
        project_id: "p".into(),
        location: ProjectLocation {
            node: runtime.node.as_str().to_owned(),
            path: "/repo".into(),
        },
        line: LineId(2),
        started_at_unix_ms: started,
        outcome: Outcome::Pr,
        phase: AttemptPhase::Working,
        branch: "hide-suggestion-s2-x".into(),
        agent_id: Some("agent-2".into()),
        repository: Some("acme/app".into()),
        pr: None,
        issue_url: None,
        issue_title: None,
        issue_since: None,
        reviewed: false,
    };
    runtime.follow_suggestion_attempts(vec![attempt], Vec::new());
    match commands.try_recv() {
        Ok(Command::Attempted {
            line,
            started_at_unix_ms,
            change: AttemptMove::PrOpen(pr),
            ..
        }) => {
            assert_eq!((line, started_at_unix_ms), (LineId(2), started));
            assert_eq!(pr.number, 31);
        }
        _ => panic!("the open pull request is handed on"),
    }
    let home = runtime
        .suggestions_work
        .apply
        .home_attempt("agent-2")
        .expect("the home's facts");
    assert_eq!(home.pull.map(|pull| pull.number), Some(31));
    runtime.track_suggestion_attempts();
    assert!(
        commands.try_recv().is_err(),
        "the same answer moves nothing again"
    );
}

/// B14: an issue attempt Hide never heard back from (it quit while `gh`
/// created) is looked for, never made again: found, it opens; not found, it
/// fails and `다시 시도` is offered. A retry looks again since the first try
/// before it creates, so a create whose answer was lost makes no second
/// issue.
#[test]
fn an_issue_whose_create_went_unanswered_is_found_before_another_is_made() {
    use super::backlog::FakeGh;
    use crate::suggestions::apply::{ApplyBrief, ApplyOrder, FollowedAttempt};
    use crate::suggestions::ledger::Signal;
    use crate::suggestions::lifecycle::{AttemptMove, AttemptPhase};
    use crate::suggestions::notebook::{LineId, Outcome};
    use crate::suggestions::store::ProjectLocation;
    use crate::suggestions::worker::{Command, SuggestionClient};
    let gh = FakeGh::new();
    let mut runtime = runtime();
    runtime.own_node = gh.clone();
    let (client, commands) = SuggestionClient::for_test();
    runtime.suggestions_work.client = Some(client);
    let location = ProjectLocation {
        node: runtime.node.as_str().to_owned(),
        path: "/repo".into(),
    };
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), ChangeNotifier::noop());
    let title = "CI check `verify` keeps failing";
    let started = crate::runtime::unix_milliseconds() - 5 * 60_000;
    let unanswered = |line: u64| FollowedAttempt {
        project_id: "p".into(),
        location: location.clone(),
        line: LineId(line),
        started_at_unix_ms: started,
        outcome: Outcome::Issue,
        phase: AttemptPhase::Starting,
        branch: format!("hide-suggestion-s{line}-x"),
        agent_id: None,
        repository: None,
        pr: None,
        issue_url: None,
        issue_title: Some(title.into()),
        issue_since: None,
        reviewed: false,
    };
    let retry = |line: u64| ApplyOrder {
        project_id: "p".into(),
        location: location.clone(),
        line: LineId(line),
        started_at_unix_ms: started + 1,
        auto: false,
        outcome: Outcome::Issue,
        branch: format!("hide-suggestion-s{line}-x"),
        agent_name: format!("suggestion-apply-s{line}-x"),
        brief: ApplyBrief {
            basis: Some((Signal::CheckFailed, "verify".into())),
            records: 3,
            ..Default::default()
        },
        issue_title: Some(title.into()),
        issue_since: Some(started),
    };
    let moved = |line: u64| loop {
        match commands.recv_timeout(Duration::from_secs(10)) {
            Ok(Command::Attempted {
                line: moved,
                change,
                ..
            }) if moved == LineId(line) => return change,
            Ok(_) => {}
            Err(_) => panic!("line {line}'s attempt moved"),
        }
    };
    let issue = |number: u32| AttemptMove::IssueOpen {
        url: format!("https://github.com/acme/project/issues/{number}"),
    };

    gh.reconcile.lock().unwrap().push((7, title.into(), true));
    shared
        .lock()
        .unwrap()
        .follow_suggestion_attempts(vec![unanswered(1)], Vec::new());
    assert_eq!(moved(1), issue(7), "the issue the lost create made");
    gh.reconcile.lock().unwrap().clear();
    shared
        .lock()
        .unwrap()
        .follow_suggestion_attempts(vec![unanswered(2)], Vec::new());
    assert_eq!(moved(2), AttemptMove::Failed("issue_failed".into()));

    gh.reconcile.lock().unwrap().push((7, title.into(), true));
    shared.lock().unwrap().start_suggestion_apply(retry(3));
    assert_eq!(moved(3), issue(7), "a retry adopts the earlier issue");
    assert!(
        gh.calls("issue create").is_empty(),
        "nothing was made again"
    );

    gh.reconcile.lock().unwrap().clear();
    shared.lock().unwrap().start_suggestion_apply(retry(4));
    assert_eq!(moved(4), issue(41), "none was found, so one is made");
    assert_eq!(
        *gh.create_repositories.lock().unwrap(),
        [Some("acme/project".to_owned())],
        "in the repository it looked in"
    );
}

/// B15: a watched line's pull request heads are asked about off the lock,
/// the merge commit once and then each head through GitHub's compare, and
/// the answers reach the worker. A pair already answered is not asked
/// again, and after a read fails the line waits before it is asked again.
#[test]
fn a_watched_line_asks_github_whether_each_head_has_the_merge() {
    use super::backlog::FakeGh;
    use crate::suggestions::ledger::PrRef;
    use crate::suggestions::notebook::LineId;
    use crate::suggestions::store::ProjectLocation;
    use crate::suggestions::watch::WatchedLine;
    use crate::suggestions::worker::{Command, SuggestionClient};
    use std::collections::{BTreeMap, BTreeSet};
    const MERGE: &str = "0123456789abcdef0123456789abcdef01234567";
    const WITH_FIX: &str = "1111111111111111111111111111111111111111";
    const WITHOUT_FIX: &str = "2222222222222222222222222222222222222222";
    const UNREAD: &str = "3333333333333333333333333333333333333333";
    let gh = FakeGh::new();
    gh.api.lock().unwrap().extend([
        ("repos/acme/app/pulls/12".to_owned(), format!("{MERGE}\n")),
        (
            format!("repos/acme/app/compare/{MERGE}...{WITH_FIX}"),
            "ahead\n".to_owned(),
        ),
        (
            format!("repos/acme/app/compare/{MERGE}...{WITHOUT_FIX}"),
            "diverged\n".to_owned(),
        ),
    ]);
    let mut runtime = runtime();
    runtime.own_node = gh.clone();
    let (client, commands) = SuggestionClient::for_test();
    runtime.suggestions_work.client = Some(client);
    let watched = |line: u64, merge_commit: Option<&str>, heads: &[&str]| WatchedLine {
        project_id: "p".into(),
        location: ProjectLocation {
            node: "local".into(),
            path: "/repo".into(),
        },
        line: LineId(line),
        merged_at_unix_ms: 1,
        base: Some("main".into()),
        holding: BTreeSet::new(),
        pr: PrRef {
            number: 12,
            url: "https://github.com/acme/app/pull/12".into(),
        },
        merge_commit: merge_commit.map(str::to_owned),
        heads: heads.iter().map(|head| (*head).to_owned()).collect(),
    };
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), ChangeNotifier::noop());
    let judged = |line: u64| loop {
        match commands.recv_timeout(Duration::from_secs(10)) {
            Ok(Command::HeadsJudged {
                line: judged,
                merge_commit,
                heads,
                ..
            }) if judged == LineId(line) => return (merge_commit, heads),
            Ok(_) => {}
            Err(_) => panic!("line {line}'s heads were judged"),
        }
    };
    let compares = || gh.calls("api --jq .status").len();

    shared.lock().unwrap().follow_suggestion_attempts(
        Vec::new(),
        vec![watched(3, None, &[WITH_FIX, WITHOUT_FIX, UNREAD])],
    );
    assert_eq!(
        judged(3),
        (
            MERGE.to_owned(),
            BTreeMap::from([(WITH_FIX.to_owned(), true), (WITHOUT_FIX.to_owned(), false)])
        ),
        "the heads read before the unread one are answered"
    );
    assert_eq!(gh.calls("api --jq .merge_commit_sha").len(), 1);
    assert_eq!(compares(), 3);

    shared
        .lock()
        .unwrap()
        .follow_suggestion_attempts(Vec::new(), vec![watched(3, Some(MERGE), &[UNREAD])]);
    assert_eq!(compares(), 3, "a line whose read failed waits");

    shared
        .lock()
        .unwrap()
        .follow_suggestion_attempts(Vec::new(), vec![watched(4, Some(MERGE), &[WITH_FIX])]);
    assert_eq!(
        judged(4),
        (
            MERGE.to_owned(),
            BTreeMap::from([(WITH_FIX.to_owned(), true)])
        ),
    );
    assert_eq!(compares(), 3, "an answered pair is not asked again");
    assert_eq!(gh.calls("api --jq .merge_commit_sha").len(), 1);
}

/// A day's run investigates only while Hide AI can (B19, D-39, D-49): with
/// no agent chosen, with OpenCode or with the worker's gate shut, it groups
/// by tag.
#[test]
fn a_run_investigates_only_while_hide_ai_can() {
    use crate::suggestions::analysis::StopFlag;
    use crate::suggestions::worker::{RunGate, RunPlan};
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let runtime = Arc::new(Mutex::new(runtime));
    let factory = super::super::suggestions::analysis_factory(Arc::downgrade(&runtime));
    let stop = StopFlag::default();
    let chosen = |ai_ready| {
        factory(&RunPlan {
            project_id: "project",
            gate: RunGate {
                ai_ready,
                usage_ok: true,
                language: hide_ai::OutputLanguage::English,
            },
            stop: &stop,
        })
        .name()
    };
    assert_eq!(chosen(true), "tag_grouping", "no agent chosen");
    let settings = |provider| hide_ai::AiSettings {
        provider,
        chosen: true,
        enabled: true,
        ..hide_ai::AiSettings::default()
    };
    runtime.lock().unwrap().ai_settings = Some(settings(hide_ai::ProviderId::CLAUDE));
    assert_eq!(chosen(true), "investigation");
    assert_eq!(chosen(false), "tag_grouping", "the gate shut");
    runtime.lock().unwrap().ai_settings = Some(settings(hide_ai::ProviderId::OPENCODE));
    assert_eq!(chosen(true), "tag_grouping", "OpenCode cannot investigate");
}
