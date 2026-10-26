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
/// the worker once (B4, D-19): each check an open pull request failed, and
/// one that merged after a review asked for changes.
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
            head_oid: None,
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
                .map(|fact| (fact.signal, fact.tag, fact.pr.map(|pr| pr.number)))
                .collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    runtime.feed_suggestion_prs();
    assert_eq!(
        sent(&commands),
        [(Signal::CheckFailed, "verify".to_owned(), Some(7))]
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
            Some(8)
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
