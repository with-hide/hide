use super::*;
use crate::model::{
    ArchiveDetailSnapshot, MemoryAction, MemoryUnavailableSnapshot, ProjectMemorySnapshot,
    RightPanelSection, SessionRowSnapshot, SessionsProviderFilter,
};

fn session(id: &str, provider: &str, request: &str, updated_at_unix_ms: u64) -> SessionRowSnapshot {
    SessionRowSnapshot {
        id: id.to_owned(),
        provider: provider.to_owned(),
        provider_label: if provider == "codex" {
            "Codex"
        } else {
            "Claude Code"
        }
        .to_owned(),
        locator: format!("/fixture/{id}.jsonl"),
        checkout_path: "/fixture/project".to_owned(),
        first_human_request: Some(request.to_owned()),
        started_at_unix_ms: Some(updated_at_unix_ms.saturating_sub(1)),
        updated_at_unix_ms,
        title: None,
        unavailable_reason: None,
    }
}

/// A runtime with the fixture's first Project's Memory open, as its first
/// read leaves it, with Memory's Project id `project-1`.
fn opened_project() -> (Runtime, String) {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let workspace = runtime.snapshot.navigator.workspaces[0].id.clone();
    assert!(runtime.open_project_memory(Some(workspace.clone())));
    runtime.project_memory.project_id = Some("project-1".to_owned());
    runtime.snapshot.status.last_error = None;
    (runtime, workspace)
}

fn opened(runtime: &Runtime) -> Option<&ProjectMemorySnapshot> {
    runtime.snapshot.memory.as_ref()?.project.as_ref()
}

fn action(workspace: &str, action: MemoryAction) -> crate::runtime::events::MemoryActionPayload {
    crate::runtime::events::MemoryActionPayload {
        workspace_id: workspace.to_owned(),
        action,
        number: None,
        numbers: Vec::new(),
        change: None,
    }
}

#[test]
fn focusing_a_checkout_refreshes_an_already_visible_sessions_panel() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    runtime.snapshot.ui_state.right_panel_visible = true;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::Sessions;
    runtime.snapshot.sessions.unavailable_reason =
        Some("Choose a local Project to view sessions".to_owned());
    let project = runtime.snapshot.navigator.workspaces[0].clone();

    assert!(runtime.focus_checkout(&project.id, &project.checkouts[0].id));

    assert_eq!(
        runtime.snapshot.sessions.unavailable_reason.as_deref(),
        Some("Session reader is unavailable")
    );
}

#[test]
fn a_catalog_change_refreshes_an_already_visible_sessions_panel() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let project = runtime.snapshot.navigator.workspaces[0].clone();
    runtime.focus_checkout(&project.id, &project.checkouts[0].id);
    runtime.snapshot.ui_state.right_panel_visible = true;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::Sessions;
    runtime.snapshot.sessions.unavailable_reason =
        Some("Choose a local Project to view sessions".to_owned());

    assert!(runtime.refresh_sessions_after_catalog_change(true));

    assert_eq!(
        runtime.snapshot.sessions.unavailable_reason.as_deref(),
        Some("Session reader is unavailable")
    );
}

#[test]
fn the_agent_sessions_tool_does_not_activate_the_memory_reader() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let project = runtime.snapshot.navigator.workspaces[0].clone();
    runtime.focus_checkout(&project.id, &project.checkouts[0].id);
    runtime.snapshot.ui_state.right_panel_visible = true;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::AgentSessions;
    runtime.snapshot.sessions.unavailable_reason = Some("Memory has not been opened".to_owned());

    assert!(!runtime.refresh_sessions_after_catalog_change(true));
    assert_eq!(
        runtime.snapshot.sessions.unavailable_reason.as_deref(),
        Some("Memory has not been opened")
    );
}

#[test]
fn session_tabs_share_the_preview_slot_and_promote_in_place() {
    let mut runtime = runtime();
    let archive = |kind: &str, id: &str, title: &str| ArchiveDetailSnapshot {
        id: id.to_owned(),
        kind: kind.to_owned(),
        title: title.to_owned(),
        provider: None,
        unavailable_reason: None,
        events: Vec::new(),
    };

    runtime.show_diff_tab("workspace", "checkout", "/fixture/notes.md", false, true);
    assert_eq!(runtime.snapshot.editor.tabs[0].kind, EditorTabKind::Diff);
    runtime.show_archive_tab(
        "workspace",
        "checkout",
        archive("session", "s1", "Codex · Project Memory"),
        true,
    );
    assert_eq!(runtime.snapshot.editor.tabs.len(), 1);
    assert_eq!(runtime.snapshot.editor.tabs[0].kind, EditorTabKind::Session);
    assert!(runtime.snapshot.editor.tabs[0].preview);

    runtime.show_archive_tab(
        "workspace",
        "checkout",
        archive("session", "s1", "Codex · Project Memory"),
        false,
    );
    assert!(!runtime.snapshot.editor.tabs[0].preview);
    runtime.show_archive_tab(
        "workspace",
        "checkout",
        archive("session", "s2", "Claude Code · Review"),
        true,
    );
    assert_eq!(runtime.snapshot.editor.tabs.len(), 2);
    assert!(!runtime.snapshot.editor.tabs[0].preview);
    assert!(runtime.snapshot.editor.tabs[1].preview);
}

#[test]
fn session_filter_and_query_keep_the_catalogs_existing_order() {
    let mut runtime = runtime();
    runtime.session_catalog_rows = vec![
        session("new-codex", "codex", "Alpha decision", 30),
        session("claude", "claude", "Alpha review", 20),
        session("old-codex", "codex", "Beta work", 10),
    ];

    assert!(runtime.set_sessions_filter("codex", "alpha".to_owned()));
    assert_eq!(
        runtime.snapshot.sessions.provider_filter,
        SessionsProviderFilter::Codex
    );
    assert_eq!(
        runtime
            .snapshot
            .sessions
            .rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["new-codex"]
    );

    assert!(runtime.set_sessions_filter("all", "alpha".to_owned()));
    assert_eq!(
        runtime
            .snapshot
            .sessions
            .rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["new-codex", "claude"]
    );
}

#[test]
fn memory_opens_for_a_project_without_its_sessions_and_closes() {
    // The Memory screen names its own Project and waits for no Sessions
    // history read; opening another leaves nothing of the first.
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let workspace = runtime.snapshot.navigator.workspaces[0].id.clone();

    assert!(runtime.open_project_memory(Some(workspace.clone())));
    let project = opened(&runtime).expect("Memory is open");
    assert_eq!(project.workspace_id, workspace);
    assert_eq!(project.unavailable, None);

    assert!(runtime.open_project_memory(Some("gone-project".to_owned())));
    let project = opened(&runtime).unwrap();
    assert_eq!(project.workspace_id, "gone-project");
    assert_eq!(
        project.unavailable,
        Some(MemoryUnavailableSnapshot::Unregistered)
    );
    assert_eq!(runtime.named_memory_project(), None);

    assert!(runtime.open_project_memory(None));
    assert!(opened(&runtime).is_none());
    assert!(!runtime.open_project_memory(None));
}

#[test]
fn expanding_or_searching_a_project_memory_does_not_open_shows_nothing() {
    let (mut runtime, workspace) = opened_project();

    assert!(!runtime.expand_memory_card("another-project", Some(3)));
    assert!(!runtime.search_project_memory("another-project", "lint".to_owned()));
    assert!(runtime.expand_memory_card(&workspace, Some(3)));
    assert!(
        !runtime.expand_memory_card(&workspace, Some(3)),
        "already shown"
    );
    assert!(runtime.search_project_memory(&workspace, " lint ".to_owned()));
    assert!(!runtime.search_project_memory(&workspace, "lint".to_owned()));
    assert!(runtime.search_project_memory(&workspace, "x".repeat(2_000)));
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "memory.search_too_long"
    );
}

#[test]
fn a_memory_action_for_a_project_no_longer_registered_says_so() {
    let (mut runtime, _) = opened_project();

    assert!(runtime.apply_memory_action(action("another-project", MemoryAction::Disable)));
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "memory.project_unavailable"
    );
    assert!(runtime.project_memory.action.is_none());
}

#[test]
fn a_forgotten_card_offers_its_undo_on_its_project_until_undone() {
    // B24: "mN 지움 · 되돌리기", from the tab or an agent's `hide memory forget`.
    use crate::runtime::memory_analysis::MemoryChange;
    let (mut runtime, _) = opened_project();
    let shown = |runtime: &Runtime| opened(runtime).and_then(|project| project.forgotten);

    runtime.note_memory_change("another-project", MemoryChange::Forgot(2));
    assert_eq!(shown(&runtime), None);

    runtime.note_memory_change("project-1", MemoryChange::Forgot(5));
    assert_eq!(shown(&runtime), Some(5));
    runtime.note_memory_change("project-1", MemoryChange::Undid(4));
    assert_eq!(shown(&runtime), Some(5), "another card's undo leaves it");
    runtime.note_memory_change("project-1", MemoryChange::Undid(5));
    assert_eq!(shown(&runtime), None);
}

#[test]
fn a_session_its_pane_let_go_is_analyzed_by_the_next_pass_at_once() {
    // B2: the label worker's live set is the pane-close signal.
    let mut runtime = runtime();
    let session = |id: &str| ("claude", id.to_owned());
    let handoff = |live: &[&str]| crate::labels::worker::MemoryHandoff {
        receipts: Vec::new(),
        live_sessions: Some(live.iter().map(|id| session(id)).collect()),
    };
    let own = runtime.node.as_str().to_owned();
    runtime.take_memory_handoff(&own, handoff(&["a", "b"]));
    runtime.memory_work.next_pass_unix_ms = u64::MAX;

    runtime.take_memory_handoff(&own, handoff(&["a"]));

    assert_eq!(
        runtime
            .memory_work
            .closed_sessions
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![session("b")]
    );
    assert_eq!(
        runtime.memory_work.next_pass_unix_ms, 0,
        "the next pass starts at once"
    );
}

#[test]
fn analysis_event_groups_preserve_every_event_exactly_once() {
    let events = (0..3)
        .map(|index| {
            serde_json::json!({
                "offset": index,
                "kind": "human",
                "text": format!("{index}{}", "x".repeat(18 * 1024)),
            })
        })
        .collect::<Vec<_>>();

    let groups =
        crate::runtime::memory_analysis::event_groups(&events).expect("bounded events group");
    let flattened = groups.into_iter().flatten().collect::<Vec<_>>();

    assert_eq!(flattened, events);
}

#[test]
fn one_analysis_event_over_the_group_budget_is_split_without_text_loss() {
    let text = format!("{}끝", "x".repeat(37 * 1024));
    let events = vec![serde_json::json!({
        "offset": 1,
        "kind": "human",
        "text": text,
    })];

    let groups = crate::runtime::memory_analysis::event_groups(&events).unwrap();
    let chunks = groups.into_iter().flatten().collect::<Vec<_>>();
    assert!(chunks.len() > 1);
    assert_eq!(
        chunks
            .iter()
            .filter_map(|event| event["text"].as_str())
            .collect::<String>(),
        text
    );
    assert!(chunks.iter().all(|event| event["offset"] == 1));
}

#[test]
fn an_oversized_event_and_a_later_turn_are_both_preserved() {
    let events = vec![
        serde_json::json!({"offset": 1, "text": "x".repeat(40 * 1024)}),
        serde_json::json!({"offset": 2, "text": "Keep later turns analyzable"}),
    ];
    let groups = crate::runtime::memory_analysis::event_groups(&events).unwrap();
    let flattened = groups.into_iter().flatten().collect::<Vec<_>>();
    assert!(flattened.iter().any(|event| event["offset"] == 1));
    assert!(flattened.iter().any(|event| event["offset"] == 2));
}

#[test]
fn one_event_over_the_analysis_cap_preserves_every_chunk_with_bounded_groups() {
    // A 256 KiB Korean message keeps its entire text. The worker's persisted
    // four-group quantum bounds calls per pass, not the total source length.
    let text = "가".repeat(256 * 1024 / 3);
    let events = vec![serde_json::json!({
        "offset": 1,
        "kind": "human",
        "text": text,
    })];

    let groups = crate::runtime::memory_analysis::event_groups(&events).unwrap();
    assert!(groups.len() > hide_memory::ANALYSIS_GROUP_QUANTUM);
    assert_eq!(
        groups
            .iter()
            .flatten()
            .map(|event| event["text"].as_str().unwrap())
            .collect::<String>(),
        text
    );
    for group in &groups {
        assert!(serde_json::to_vec(group).unwrap().len() <= 36 * 1024);
    }
}

#[test]
fn the_largest_comparison_set_and_event_group_fit_one_request() {
    // B6, D-13: thirty Korean cards near the body cap and one full event
    // group still make one request under the analysis input cap.
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().to_path_buf();
    let mut store = hide_memory::MemoryStore::open(&root.join("memory.sqlite3")).unwrap();
    let project_id = "project:request-budget";
    store
        .ensure_project(project_id, &root, crate::node::TEST_NODE)
        .unwrap();
    store.set_enabled(project_id, true, true).unwrap();
    store
        .apply_candidates(
            &hide_memory::AnalysisBatch {
                id: "request-budget-batch".into(),
                project_id: project_id.into(),
                provider: "codex".into(),
                analysis_provider: "codex".into(),
                session_id: "source".into(),
                content_hash: "request-budget-hash".into(),
                created_at_unix_ms: 1,
            },
            &(0..40)
                .map(|index| hide_memory::Candidate {
                    text: format!("규칙 {index} 테스트 {}", "가".repeat(3_980)),
                    title: String::new(),
                    aliases: vec!["테스트".to_owned()],
                    kind: hide_memory::CandidateKind::Rule,
                    confidence: 1.0,
                    salience: 1.0 - index as f64 / 100.0,
                    source_offsets: vec![index],
                    human_offsets: vec![index],
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: Vec::new(),
                    relation: hide_memory::CandidateRelation::New,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let events = vec![serde_json::json!({
        "offset": 1,
        "kind": "human",
        "text": format!("테스트 {}", "x".repeat(35 * 1024)),
    })];
    let group = crate::runtime::memory_analysis::event_groups(&events)
        .unwrap()
        .remove(0);
    // The reporting session's three held cards, each near the cap, are
    // charged against the same budget (PRD memory-stale-cards D-06).
    for number in 1..=3 {
        store
            .record_receipt(
                project_id,
                "codex",
                "session",
                &format!("event:{number}"),
                &format!("m{number}@1"),
                crate::runtime::memory::now_ms(),
            )
            .unwrap();
        assert!(matches!(
            store
                .report_stale(
                    project_id,
                    number,
                    &hide_memory::StaleReporter {
                        runtime: "codex",
                        session_id: "session",
                        report_offset: Some(1),
                    },
                    "reversed",
                    crate::runtime::memory::now_ms(),
                )
                .unwrap(),
            hide_memory::StaleOutcome::Held(_)
        ));
    }
    let root_path = root.to_string_lossy().into_owned();
    let set = crate::runtime::memory_analysis::comparison_set(
        &store,
        &crate::runtime::memory_analysis::BatchSource {
            project_id,
            provider: "codex",
            session_id: "session",
            checkout_path: &root_path,
            evidence_root: &root_path,
            new_card_limit: None,
            deadline: None,
        },
        &group,
    )
    .unwrap();
    // One near-cap card fills the 16 KiB set; the other two are left out
    // rather than pushing the request past its cap.
    let similar = set.cards;
    assert_eq!(similar.len(), 1);
    assert!(similar[0].reported);
    let unsent = set
        .left_out
        .iter()
        .map(|card| card.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(set.left_out.len(), 2);
    // Saving the cursor past the report ends only the hold the analysis was
    // shown; the two it never saw stay held.
    let ended = store
        .save_cursor(
            hide_memory::CursorReader::Analysis,
            &hide_memory::SessionCursorRecord {
                project_id: project_id.to_owned(),
                provider: "codex".to_owned(),
                session_id: "session".to_owned(),
                byte_offset: 2,
                checkpoint: Vec::new(),
                last_content_hash: None,
                updated_at_unix_ms: crate::runtime::memory::now_ms(),
                operator_events: 0,
            },
            &unsent,
        )
        .unwrap();
    assert_eq!(
        ended.iter().map(|hold| hold.number).collect::<Vec<_>>(),
        [similar[0].number]
    );
    for card in &set.left_out {
        assert!(
            store
                .hold(project_id, card.number, crate::runtime::memory::now_ms())
                .unwrap()
                .is_some()
        );
    }

    let request = hide_memory::HideNativeAnalyzer
        .request(
            "request-budget",
            project_id,
            "session",
            &serde_json::to_string(&group).unwrap(),
            &similar,
            &[],
            hide_ai::OutputLanguage::English,
        )
        .unwrap();

    assert!(request.input.len() <= hide_memory::ANALYSIS_INPUT_LIMIT_BYTES);
    let input: serde_json::Value = serde_json::from_str(&request.input).unwrap();
    assert!(input["new_events"].is_array());
    assert!(!input["similar_memories"].as_array().unwrap().is_empty());
}

#[test]
fn turning_memory_on_updates_the_hooks_first_and_then_turns_it_on() {
    // D-17, B4: the switch is the operator's word; no separate hook button.
    let (runtime, workspace) = opened_project();
    let runtime = Arc::new(Mutex::new(runtime));
    let mut guard = runtime.lock().unwrap();
    guard.install_worker_context(
        Arc::downgrade(&runtime),
        crate::handle::ChangeNotifier::noop(),
    );
    let diagnosis = |status| hide_agent_hooks::Diagnosis {
        runtimes: vec![hide_agent_hooks::diagnosis::RuntimeDiagnosis {
            runtime: hide_agent_hooks::AgentRuntime::Codex,
            label: "Codex".to_owned(),
            path: "/fixture/.codex/hooks.json".to_owned(),
            status,
            current_version: hide_agent_hooks::HOOK_ENTRY_VERSION,
            memory_compatibility: hide_agent_hooks::MemoryCompatibility::Supported {
                version: "999.0.0".to_owned(),
            },
        }],
        last_report_failure: None,
    };
    guard.ingest_hook_diagnosis(diagnosis(hide_agent_hooks::HookStatus::NotInstalled));
    guard.ingest_kit_report(
        crate::node::TEST_NODE,
        &hide_kit::KitReport {
            components: vec![hide_kit::ComponentReport {
                id: hide_kit::ComponentId::CodexHook,
                state: hide_kit::ComponentState::NotInstalled,
                reason: None,
                why: None,
                location: None,
            }],
            agents: Vec::new(),
            held_for_onboarding: false,
            labels_retirement: Default::default(),
            legacy_retirement: Default::default(),
            codex_daemon: None,
            codex_daemon_on: None,
            codex_daemon_running: None,
            codex_daemon_unreadable: None,
            codex_capability_unread: None,
            codex_daemon_off: None,
            herdr: None,
            refused: None,
        },
    );
    let pending = |runtime: &Runtime| opened(runtime).and_then(|project| project.pending_action);

    guard.apply_memory_action(action(&workspace, MemoryAction::Enable));
    assert_eq!(
        guard.take_local_kit_job(std::time::Instant::now()),
        Some(crate::runtime::KitJob::Apply(hide_kit::Scope::reinstall([
            hide_kit::ComponentId::CodexHook
        ])))
    );
    assert_eq!(
        pending(&guard),
        Some(MemoryAction::Enable),
        "Memory waits for the hooks"
    );
    assert!(opened(&guard).unwrap().preparing_hooks);

    guard.ingest_hook_diagnosis(diagnosis(hide_agent_hooks::HookStatus::Installed {
        version: hide_agent_hooks::HOOK_ENTRY_VERSION,
    }));
    assert_eq!(
        pending(&guard),
        Some(MemoryAction::Enable),
        "the switch turns on once the hooks carry Memory"
    );
    guard.ingest_hook_diagnosis(diagnosis(hide_agent_hooks::HookStatus::Installed {
        version: hide_agent_hooks::HOOK_ENTRY_VERSION,
    }));
    assert_eq!(
        guard.snapshot.status.last_error, None,
        "a second hook read starts no second enable"
    );
}

#[test]
fn a_switch_pressed_before_the_hooks_are_read_turns_memory_on_once_they_are() {
    // Issue 1086: a press at launch waits for the first hook read instead of
    // being refused.
    let (runtime, workspace) = opened_project();
    let runtime = Arc::new(Mutex::new(runtime));
    let mut guard = runtime.lock().unwrap();
    guard.install_worker_context(
        Arc::downgrade(&runtime),
        crate::handle::ChangeNotifier::noop(),
    );
    let pending = |runtime: &Runtime| opened(runtime).and_then(|project| project.pending_action);

    guard.apply_memory_action(action(&workspace, MemoryAction::Enable));
    assert_eq!(
        guard.snapshot.status.last_error, None,
        "the press is held, not refused"
    );
    assert_eq!(
        pending(&guard),
        Some(MemoryAction::Enable),
        "the switch shows the held press"
    );

    guard.ingest_hook_diagnosis(hide_agent_hooks::Diagnosis {
        runtimes: vec![hide_agent_hooks::diagnosis::RuntimeDiagnosis {
            runtime: hide_agent_hooks::AgentRuntime::Codex,
            label: "Codex".to_owned(),
            path: "/fixture/.codex/hooks.json".to_owned(),
            status: hide_agent_hooks::HookStatus::Installed {
                version: hide_agent_hooks::HOOK_ENTRY_VERSION,
            },
            current_version: hide_agent_hooks::HOOK_ENTRY_VERSION,
            memory_compatibility: hide_agent_hooks::MemoryCompatibility::Supported {
                version: "999.0.0".to_owned(),
            },
        }],
        last_report_failure: None,
    });
    assert_eq!(pending(&guard), Some(MemoryAction::Enable));
}

#[test]
fn a_switch_held_for_a_hook_read_that_fails_is_refused() {
    let (mut runtime, workspace) = opened_project();
    runtime.apply_memory_action(action(&workspace, MemoryAction::Enable));
    assert!(runtime.hook_diagnosis_failed());
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("memory.hooks_unread")
    );
    assert_eq!(
        opened(&runtime).and_then(|project| project.pending_action),
        None
    );
    assert!(!runtime.hook_diagnosis_failed(), "nothing else was held");
}

#[test]
fn one_unsupported_runtime_does_not_disable_another_supported_runtime() {
    let mut runtime = runtime();
    runtime.ingest_hook_diagnosis(hide_agent_hooks::Diagnosis {
        runtimes: vec![
            hide_agent_hooks::RuntimeDiagnosis {
                runtime: hide_agent_hooks::AgentRuntime::ClaudeCode,
                label: "Claude Code".to_owned(),
                path: "/fixture/.claude/settings.json".to_owned(),
                status: hide_agent_hooks::HookStatus::Installed {
                    version: hide_agent_hooks::HOOK_ENTRY_VERSION,
                },
                current_version: hide_agent_hooks::HOOK_ENTRY_VERSION,
                memory_compatibility: hide_agent_hooks::MemoryCompatibility::UpdateRequired {
                    installed_version: Some("2.1.277".to_owned()),
                    minimum_version: "2.1.278".to_owned(),
                },
            },
            hide_agent_hooks::RuntimeDiagnosis {
                runtime: hide_agent_hooks::AgentRuntime::Codex,
                label: "Codex".to_owned(),
                path: "/fixture/.codex/hooks.json".to_owned(),
                status: hide_agent_hooks::HookStatus::Installed {
                    version: hide_agent_hooks::HOOK_ENTRY_VERSION,
                },
                current_version: hide_agent_hooks::HOOK_ENTRY_VERSION,
                memory_compatibility: hide_agent_hooks::MemoryCompatibility::Supported {
                    version: "0.155.1".to_owned(),
                },
            },
        ],
        last_report_failure: None,
    });

    assert!(runtime.hooks_support_memory());
    for row in &mut runtime.hook_diagnosis.as_mut().unwrap().runtimes {
        row.status = hide_agent_hooks::HookStatus::Installed {
            version: hide_agent_hooks::HOOK_VERSION,
        };
    }
    assert!(
        !runtime.hooks_support_memory(),
        "the pane protocol version does not prove current installed commands"
    );
}

#[test]
fn a_registered_device_project_keeps_its_memory_scope_while_the_link_is_down() {
    // A disconnected device retains its node-scoped Project, awaiting its link.
    let mut runtime = runtime();
    let mut project = workspace(
        "remote:mini:workspace:w",
        "api",
        "/srv/api",
        vec![checkout(
            "remote:mini:workspace:w",
            "remote:mini:checkout:c",
            "/srv/api",
            None,
        )],
    );
    project.device_id = "mini".into();
    project.remote_target_id = Some("mini".into());

    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: "mini".into(),
            label: "mini".into(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: "mini-host".into(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent: None,
        });
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".into(),
        state: "connected".into(),
        message: None,
        herdr_version: None,
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![project],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: None,
            focused_checkout_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            pane_layouts: Vec::new(),
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
            herdr_version: None,
        }),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });

    assert!(runtime.open_project_memory(Some("remote:mini:workspace:w".into())));
    assert_eq!(opened(&runtime).unwrap().unavailable, None);
}

#[test]
fn memory_enable_and_disable_failures_stay_with_the_requested_project() {
    let (mut runtime, workspace) = opened_project();
    // The press waits for the first hook read; that read failing refuses it.
    runtime.apply_memory_action(action(&workspace, MemoryAction::Enable));
    assert_eq!(opened(&runtime).unwrap().action_failure, None);
    assert!(runtime.hook_diagnosis_failed());
    assert_eq!(
        opened(&runtime).unwrap().action_failure,
        Some(MemoryAction::Enable)
    );
    assert_eq!(opened(&runtime).unwrap().pending_action, None);
    runtime.apply_memory_action(action(&workspace, MemoryAction::Disable));
    assert_eq!(
        opened(&runtime).unwrap().action_failure,
        Some(MemoryAction::Disable)
    );
    assert_eq!(opened(&runtime).unwrap().pending_action, None);
    runtime.open_project_memory(Some("another-project".into()));
    runtime.sync_project_memory();
    assert_eq!(opened(&runtime).unwrap().action_failure, None);
    runtime.open_project_memory(Some(workspace));
    runtime.sync_project_memory();
    assert_eq!(
        opened(&runtime).unwrap().action_failure,
        Some(MemoryAction::Disable)
    );
}
