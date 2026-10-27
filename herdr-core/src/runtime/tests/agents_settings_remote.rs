use super::*;

#[test]
fn duplicate_inflight_remote_tab_creation_is_observable_and_ignored() {
    let mut runtime = runtime();
    let projected_workspace_id = "remote:mini:workspace:w1";
    // The checkout's owner on the device is w1.
    let mut owned = checkout(
        projected_workspace_id,
        &crate::device_catalog::checkout_id("mini", "/tmp/hide-remote-tab"),
        "/tmp/hide-remote-tab",
        None,
    );
    owned.owner_workspace_id = Some("w1".to_owned());
    let mut remote_workspace = workspace(
        projected_workspace_id,
        "Fixture",
        "/tmp/hide-remote-tab",
        vec![owned],
    );
    remote_workspace.remote_target_id = Some("mini".to_owned());
    remote_workspace.device_id = "mini".to_owned();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![remote_workspace],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: Some(projected_workspace_id.to_owned()),
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
    let connector: Arc<dyn hide_herdr_client::ApiConnector> = Arc::new(
        hide_herdr_client::LocalSocketConnector::new("/tmp/herdr-core-never-connect.sock"),
    );
    runtime.install_remote_control(RemoteControlContext::new(
        "mini",
        connector,
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    runtime.remote_tab_creations_in_flight.insert((
        "mini".to_owned(),
        "w1".to_owned(),
        "/tmp/hide-remote-tab".to_owned(),
        RemoteTabCreationIdentity::Label("New tab".to_owned()),
    ));

    assert!(runtime.request_remote_control(RemoteControlPayload {
        target_id: "mini".to_owned(),
        request_id: "request-2".to_owned(),
        report_pane_focus_outcome: false,
        focus_device: false,
        request: RemoteControlRequest::CreateTab {
            area_id: None,
            workspace_id: projected_workspace_id.to_owned(),
            checkout_id: None,
            cwd: "/tmp/hide-remote-tab".to_owned(),
            label: "New tab".to_owned(),
        },
    }));

    let diagnostic = runtime
        .snapshot
        .status
        .diagnostics
        .last()
        .expect("duplicate outcome is visible to the caller");
    assert_eq!(diagnostic.kind, "remote.control.duplicate_tab_ignored");
    assert!(runtime.snapshot.status.last_error.is_none());
}

#[test]
fn remote_session_sync_reconciles_target_scoped_structured_terminals() {
    let mut runtime = runtime();
    let terminals = record_terminals(&mut runtime);
    runtime.snapshot.navigator.devices.push(DeviceSnapshot {
        agent_scope: Default::default(),
        id: "mini".to_owned(),
        label: Some("Mac mini".to_owned()),
        kind: "remote".to_owned(),
        state: "unavailable".to_owned(),
        message: None,
        problem: None,
        ssh_alias: Some("mini".to_owned()),
        herdr_socket_path: None,
        agent_count: 0,
        test: None,
        host: Default::default(),
        kit: Default::default(),
        machine_name: None,
        dials_in: false,
        herdr_behind: None,
    });
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "not_connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime.snapshot.terminal.panes.push(TerminalPaneSnapshot {
        pane_id: "w-local:p1".to_owned(),
        ..TerminalPaneSnapshot::default()
    });
    let pane_id = "remote:mini:pane:w9:p1";
    let inactive_pane_id = "remote:mini:pane:w9:p2";
    let workspace_id = "remote:mini:workspace:w9";
    let checkout_id = "remote:mini:checkout:w9";
    let active_tab_id = "remote:mini:tab:w9:t1";
    let inactive_tab_id = "remote:mini:tab:w9:t2";
    let mut remote_checkout = checkout(
        workspace_id,
        checkout_id,
        "/tmp/herdr-remote-terminal",
        Some(pane(pane_id, "/tmp/herdr-remote-terminal")),
    );
    remote_checkout.tabs[0].id = Some(active_tab_id.to_owned());
    remote_checkout.tabs.push(TabSnapshot {
        agent: None,
        naming: Default::default(),
        id: Some(inactive_tab_id.to_owned()),
        workspace_id: Some(workspace_id.to_owned()),
        checkout_id: Some(checkout_id.to_owned()),
        label: Some("Inactive".to_owned()),
        empty: false,
        delegated: false,
        panes: vec![pane(inactive_pane_id, "/tmp/herdr-remote-terminal")],
    });
    let mut remote_workspace = workspace(
        workspace_id,
        "Remote fixture",
        "/tmp/herdr-remote-terminal",
        vec![remote_checkout],
    );
    remote_workspace.remote_target_id = Some("mini".to_owned());
    remote_workspace.device_id = "mini".to_owned();
    let session = RemoteSessionSnapshot {
        workspaces: vec![remote_workspace],
        agents: project_agents(serde_json::from_value(serde_json::json!({
            "agents": [{"pane_id": pane_id, "agent": "codex", "agent_status": "done", "state_change_seq": 1,
                "cwd": "/tmp/herdr-remote-terminal"}]
        })).unwrap()).agents,
        active_tab_ids: [(checkout_id.to_owned(), active_tab_id.to_owned())]
            .into_iter()
            .collect(),
        focused_workspace_id: Some(workspace_id.to_owned()),
        focused_checkout_id: Some(checkout_id.to_owned()),
        focused_tab_id: Some(active_tab_id.to_owned()),
        focused_pane_id: Some(pane_id.to_owned()),
        pane_layouts: Vec::new(),
        pane_hook_tokens: Default::default(),
        issue_tokens: Default::default(),
        issue_links: Default::default(),
        herdr_version: None,
    };

    assert!(runtime.ingest_remote_session("mini", Ok(session.clone())));
    assert!(terminals.attaches().is_empty());
    assert!(
        runtime
            .snapshot
            .terminal
            .panes
            .iter()
            .filter(|pane| pane.pane_id.starts_with("remote:mini:pane:"))
            .all(|pane| pane.transport_state == "idle")
    );

    // Focusing the device asks its node to attach the active tab's pane,
    // and only that one; the attach carries the size its view reported.
    runtime.terminal_sizes.insert(pane_id.to_owned(), (40, 120));
    let focus_remote = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_device",
        "payload": {"device_id": "mini"}
    }))
    .expect("focus remote event");
    assert!(runtime.dispatch_json(&focus_remote));
    let attached = terminals
        .take()
        .into_iter()
        .filter_map(|control| match control {
            TerminalControl::Attach { pane, size, .. } => Some((pane, size)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        attached,
        [(
            pane_id.to_owned(),
            Some(GridSize {
                rows: 40,
                cols: 120
            })
        )]
    );
    report_terminal(&mut runtime, pane_id, terminal_state("controlling", 1));
    assert_eq!(
        runtime
            .snapshot
            .terminal
            .panes
            .iter()
            .find(|pane| pane.pane_id == pane_id)
            .expect("the active pane is projected")
            .transport_state,
        "controlling"
    );
    assert_eq!(
        runtime
            .snapshot
            .terminal
            .panes
            .iter()
            .find(|pane| pane.pane_id == inactive_pane_id)
            .expect("inactive pane remains projected")
            .transport_state,
        "idle"
    );
    assert!(!runtime.ingest_remote_session("mini", Ok(session.clone())));
    assert!(
        terminals.attaches().is_empty(),
        "an unchanged session asks again"
    );
    assert!(runtime.resolve_session(
        pane_id,
        crate::agent_state::sessions::ResolveSource::Operator
    ));
    assert!(
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .agents[0]
            .resolved
            .is_some()
    );
    assert!(
        !runtime.ingest_remote_session("mini", Ok(session)),
        "an identical remote fetch retains resolution without publishing another snapshot"
    );

    // The device's panes leave its session: their node forgets them and so
    // does the projection, while this machine's pane stays.
    assert!(runtime.ingest_remote_session(
        "mini",
        Ok(RemoteSessionSnapshot {
            workspaces: Vec::new(),
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
        })
    ));
    assert!(terminals.take().contains(&TerminalControl::Forget {
        pane: pane_id.into()
    }));
    assert!(!runtime.terminal_states.contains_key(pane_id));
    assert!(
        runtime
            .snapshot
            .terminal
            .panes
            .iter()
            .any(|pane| pane.pane_id == "w-local:p1")
    );
    assert!(
        runtime
            .snapshot
            .terminal
            .panes
            .iter()
            .all(|pane| pane.pane_id != pane_id)
    );
}

/// A helper listing with these (name, is_directory) rows, in its order.
fn listing(rows: &[(&str, bool)]) -> hide_node_link::list::Listing {
    hide_node_link::list::Listing {
        entries: rows
            .iter()
            .map(|(name, is_directory)| hide_node_link::list::Entry {
                name: (*name).to_owned(),
                is_directory: *is_directory,
                inode: 1,
            })
            .collect(),
        truncated: false,
    }
}

#[test]
fn remote_file_results_are_scoped_and_generation_guarded() {
    let mut runtime = runtime();
    let root_path = "/private/tmp/herdr-remote-files";
    let workspace_id = "remote:mini:workspace:w9";
    let checkout_id = "remote:mini:checkout:w9";
    let mut remote_workspace = workspace(
        workspace_id,
        "Remote files",
        root_path,
        vec![checkout(workspace_id, checkout_id, root_path, None)],
    );
    remote_workspace.remote_target_id = Some("mini".to_owned());
    remote_workspace.device_id = "mini".to_owned();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![remote_workspace],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: Some(workspace_id.to_owned()),
            focused_checkout_id: Some(checkout_id.to_owned()),
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

    let request = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "remote_file_list",
        "payload": {"target_id": "mini", "root_path": root_path}
    }))
    .expect("remote file event");
    assert!(runtime.dispatch_json(&request));
    // No helper consent: the device lists nothing and says in place what
    // allowing it installs and runs, for the shell to ask with (B50).
    let files = &runtime.snapshot.status.remote[0].files;
    assert_eq!(files.state, "not_allowed");
    let message = files.message.as_deref().unwrap();
    assert!(message.contains(&runtime.host_helper_root()), "{message}");
    assert!(message.contains("only while Hide is connected over SSH"));

    runtime.snapshot.status.remote[0].files = RemoteFileListSnapshot {
        root_path: Some(root_path.to_owned()),
        state: "loading".to_owned(),
        entries: Vec::new(),
        message: None,
        generation: 7,
    };
    assert!(runtime.ingest_remote_file_list_result(
        "mini",
        root_path,
        7,
        Ok(listing(&[("Sources", true), ("zeta.txt", false)])),
    ));
    let files = &runtime.snapshot.status.remote[0].files;
    assert_eq!(files.state, "ready");
    assert_eq!(files.entries[0].name, "Sources");
    assert_eq!(files.entries[0].path, format!("{root_path}/Sources"));
    assert!(files.entries[0].is_directory);
    assert_eq!(files.entries[1].name, "zeta.txt");

    runtime.snapshot.status.remote[0].files = RemoteFileListSnapshot {
        root_path: Some(root_path.to_owned()),
        state: "loading".to_owned(),
        entries: Vec::new(),
        message: None,
        generation: 8,
    };
    assert!(runtime.ingest_remote_file_list_result("mini", root_path, 7, Ok(listing(&[]))));
    assert_eq!(runtime.snapshot.status.remote[0].files.state, "loading");
    assert_eq!(runtime.snapshot.status.remote[0].files.generation, 8);
    assert_eq!(
        runtime
            .snapshot
            .status
            .diagnostics
            .last()
            .expect("stale result is observable")
            .kind,
        "remote.files.stale"
    );

    runtime.next_remote_file_generation = 8;
    runtime.snapshot.status.remote[0].state = "stale".to_owned();
    assert!(runtime.dispatch_json(&request));
    assert_eq!(runtime.snapshot.status.remote[0].files.state, "unavailable");
    assert_eq!(runtime.snapshot.status.remote[0].files.generation, 9);
    assert!(runtime.ingest_remote_file_list_result("mini", root_path, 8, Ok(listing(&[]))));
    assert_eq!(runtime.snapshot.status.remote[0].files.state, "unavailable");
    assert_eq!(runtime.snapshot.status.remote[0].files.generation, 9);
}

/// D8: only the pane's detected kind permits an ordinary click report.
/// A title, a different pane's agent, and missing detection permit no bytes.
#[test]
fn ordinary_click_uses_detected_pane_agent_and_never_sends_enter() {
    let mut runtime = runtime();
    let terminals = record_terminals(&mut runtime);
    let pane_id = "w-click:p1";
    report_terminal(&mut runtime, pane_id, terminal_state("controlling", 1));
    let click = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "terminal_click",
        "payload": {"pane_id": pane_id, "column": 7, "row": 3, "modifiers": 3}
    }))
    .unwrap();
    let written = |terminals: &RecordedTerminals| {
        terminals
            .take()
            .into_iter()
            .filter_map(|control| match control {
                TerminalControl::Write { pane, data } if pane == pane_id => {
                    Some(hide_node_link::terminal::decode_base64(&data).unwrap())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    for (detected_pane, kind, expected) in [
        ("w-click:p2", "claude", false),
        (pane_id, "codex", false),
        (pane_id, "unknown", false),
        (pane_id, "claude", true),
    ] {
        let payload = crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": [{"pane_id": detected_pane, "agent": kind, "state_change_seq": 1}]
        }))
        .unwrap();
        runtime.snapshot.navigator.agents = crate::sidebar::project_agents(payload).agents;
        runtime.dispatch_json(&click);
        let writes = written(&terminals);
        assert_eq!(
            writes.len(),
            usize::from(expected),
            "{detected_pane} {kind}"
        );
        if expected {
            assert_eq!(writes[0], b"\x1b[<20;8;4M\x1b[<20;8;4m");
            assert!(!writes[0].contains(&b'\r') && !writes[0].contains(&b'\n'));
        }
    }
    runtime.snapshot.navigator.agents.clear();
    runtime.dispatch_json(&click);
    assert!(written(&terminals).is_empty());
}

#[test]
fn overview_tracks_live_checkout_panes_and_drops_retired_lineage() {
    let mut runtime = runtime();
    let mut payload = context_payload();
    runtime.ingest_session(Ok(payload.clone()));
    let project = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .find(|w| w.path.ends_with("zeta"))
        .unwrap()
        .clone();
    runtime.focus_checkout(&project.id, &project.checkouts[0].id);
    runtime.refresh_card();
    assert_eq!(runtime.snapshot.card.panes.len(), 1);
    assert_eq!(runtime.snapshot.card.panes[0].pane_id, "w2:p1");
    assert_eq!(
        runtime.snapshot.card.panes[0].session_id.as_deref(),
        Some("context-session")
    );
    assert_eq!(
        runtime.snapshot.card.panes[0].parent_pane_id.as_deref(),
        Some("w1:p1")
    );
    payload.agents.remove(0);
    payload.panes.remove(0);
    payload.layouts.remove(0);
    payload.tabs.remove(0);
    payload.workspaces.remove(0);
    runtime.ingest_session(Ok(payload.clone()));
    runtime.refresh_card();
    assert_eq!(runtime.snapshot.card.panes[0].parent_pane_id, None);
    payload.panes[0].cwd = Some("/tmp/hide-context-moved".into());
    payload.agents[0].cwd = Some("/tmp/hide-context-moved".into());
    runtime.ingest_session(Ok(payload));
    runtime.refresh_card();
    assert!(
        runtime.snapshot.card.panes.is_empty(),
        "A pane moved away is no longer checkout context"
    );
}

pub(super) fn ai_event(payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": 2, "kind": "ai_settings", "payload": payload
    }))
    .expect("the event encodes")
}

/// A read that found each named agent signed in and selectable.
pub(super) fn ready_read(ids: &[&str]) -> crate::model::BackgroundAiSnapshot {
    crate::model::BackgroundAiSnapshot {
        providers: ids
            .iter()
            .map(|id| {
                let provider = hide_ai::ProviderId::from_id(id).expect("a registered agent");
                crate::model::BackgroundAiProviderSnapshot {
                    state: "ready".to_owned(),
                    headline: "Signed in".to_owned(),
                    selectable: true,
                    models: vec!["m-1".to_owned(), "m-2".to_owned()],
                    ..crate::model::BackgroundAiProviderSnapshot::unread(provider)
                }
            })
            .collect(),
        ..crate::model::BackgroundAiSnapshot::default()
    }
}

/// Switches the named agents on in this Mac's kit snapshot, as the first-run
/// choice or Settings › Agents leaves them.
fn switch_on(runtime: &mut Runtime, ids: &[&str]) {
    let mut kit = runtime.kit_state(crate::node::TEST_NODE);
    kit.agents = hide_kit::agents::ADAPTERS
        .iter()
        .map(|adapter| {
            let piece = crate::model::KitPieceSnapshot {
                state: hide_kit::ComponentState::Installed,
                why: None,
                location: None,
            };
            crate::model::KitAgentSnapshot {
                id: adapter.id.to_owned(),
                label: adapter.label.to_owned(),
                availability: hide_kit::Availability::Available,
                enabled: ids.contains(&adapter.id),
                chosen: false,
                skill: piece.clone(),
                hook: None,
                herdr: None,
                partial: false,
                features: Vec::new(),
                sessions: None,
                update: None,
                doc_url: String::new(),
            }
        })
        .collect();
    runtime.set_kit_state(crate::node::TEST_NODE, kit);
}

/// The Hide AI tab's state travels on the ordinary snapshot, and nothing on
/// the way asks a provider anything under the mutex: this runtime has no
/// coordinator, and every assertion here still holds.
#[test]
fn the_snapshot_carries_the_choice_the_providers_and_their_models() {
    let mut runtime = runtime();
    let section = &runtime.snapshot.status.background_ai;
    assert_eq!(
        section.provider, None,
        "nothing is chosen before anything is read (B47)"
    );
    assert!(!section.chosen && section.fallback.is_empty());
    assert_eq!(
        section
            .providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "claude",
            "codex",
            "grok",
            "opencode",
            "pi",
            "omp",
            "antigravity"
        ],
        "every registered provider is a row, in the registry order"
    );
    assert!(
        section
            .providers
            .iter()
            .all(|provider| provider.state == "unread" && !provider.selectable),
        "a provider nobody has asked is unread and cannot be chosen, never a guessed state"
    );

    let read = ready_read(&["codex"]);
    assert!(runtime.ingest_background_ai(read.clone()));
    assert!(
        !runtime.ingest_background_ai(read),
        "an unchanged read publishes nothing"
    );
    let section = &runtime.snapshot.status.background_ai;
    let codex = &section.providers[1];
    assert_eq!((codex.state.as_str(), codex.selectable), ("ready", true));
    assert_eq!(codex.models, vec!["m-1".to_owned(), "m-2".to_owned()]);
    assert_eq!(
        section.providers[0].state, "unread",
        "an agent the read did not ask keeps its own row"
    );

    // The whole section survives the wire the shell actually reads.
    let encoded =
        serde_json::to_value(&runtime.snapshot.status.background_ai).expect("it serializes");
    assert_eq!(encoded["provider"], serde_json::Value::Null);
    assert_eq!(encoded["enabled"], true);
    assert_eq!(encoded["providers"][1]["agent"], "codex");
    assert_eq!(encoded["providers"][1]["selectable"], true);
    assert_eq!(encoded["providers"][1]["models"][1], "m-2");
    assert_eq!(encoded["refusal"], serde_json::Value::Null);
}

#[test]
fn a_chosen_agent_and_model_move_the_snapshot_and_queue_one_write() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_background_ai(ready_read(&["claude"]));
    assert!(
        runtime.take_ai_settings_save().is_none(),
        "nothing is written until the operator chooses"
    );
    assert!(!runtime.ai_request().observing, "nobody is looking yet");

    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"observing": true}))));
    assert!(
        runtime.ai_request().observing,
        "the tab being on screen is what lets the probe run"
    );
    assert!(
        runtime.take_ai_settings_save().is_none(),
        "looking at the screen is not a choice to save"
    );

    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "claude"}))));
    assert_eq!(
        runtime.snapshot.status.background_ai.provider.as_deref(),
        Some("claude"),
        "the choice is on the snapshot before the file write happens"
    );
    assert!(runtime.snapshot.status.background_ai.chosen);
    assert!(runtime.dispatch_json(&ai_event(
        serde_json::json!({"provider": "claude", "model": "sonnet"})
    )));
    assert_eq!(
        runtime.ai_request().models[&hide_ai::ProviderId::CLAUDE],
        "sonnet",
        "the probe asks about the model the operator chose"
    );

    let saved = runtime
        .take_ai_settings_save()
        .expect("the choice is queued for the coordinator to write");
    assert_eq!(saved.provider, hide_ai::ProviderId::CLAUDE);
    assert!(saved.chosen);
    assert_eq!(saved.model(hide_ai::ProviderId::CLAUDE), "sonnet");
    assert_eq!(
        saved.router_config().priority,
        vec![hide_ai::ProviderId::CLAUDE],
        "only the chosen provider is asked until the operator adds a fallback"
    );
    assert!(
        runtime.take_ai_settings_save().is_none(),
        "a taken write is not performed twice"
    );
}

/// D-15, B35: an agent that is not signed in, not installed or cannot be
/// asked without risk to files is refused where it is chosen, and the
/// refusal says which event and why.
#[test]
fn an_agent_that_cannot_be_chosen_is_refused_as_runs_on_and_as_a_fallback() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_background_ai(ready_read(&["claude"]));
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "claude"}))));
    runtime.take_ai_settings_save();

    for (payload, expected) in [
        (
            serde_json::json!({"provider": "opencode"}),
            "ai_settings.provider_not_selectable",
        ),
        (
            serde_json::json!({"fallback_add": "opencode"}),
            "ai_settings.provider_not_selectable",
        ),
        (
            serde_json::json!({"provider": "cursor"}),
            "ai_settings.unknown_provider",
        ),
        (
            serde_json::json!({"fallback_add": "cursor"}),
            "ai_settings.unknown_provider",
        ),
    ] {
        assert!(runtime.dispatch_json(&ai_event(payload)));
        assert_eq!(
            runtime
                .snapshot
                .status
                .last_error
                .as_ref()
                .expect("the refusal is visible")
                .kind,
            expected
        );
    }
    assert!(runtime.take_ai_settings_save().is_none());
    assert_eq!(
        runtime.snapshot.status.background_ai.provider.as_deref(),
        Some("claude")
    );
    assert!(runtime.snapshot.status.background_ai.fallback.is_empty());
}

/// D-16, D-17, D-27, B38, B39, B44: the fallback list is the operator's, in
/// the order they added, one model each, and choosing a listed agent as
/// Runs on takes it out of the list.
#[test]
fn the_fallback_list_keeps_the_operators_order_and_runs_on_leaves_it() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_background_ai(ready_read(&["claude", "codex", "grok"]));
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "claude"}))));
    for id in ["grok", "codex"] {
        assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"fallback_add": id}))));
    }
    let listed = |runtime: &Runtime| -> Vec<String> {
        runtime
            .snapshot
            .status
            .background_ai
            .fallback
            .iter()
            .map(|entry| entry.provider.clone())
            .collect()
    };
    assert_eq!(listed(&runtime), vec!["grok", "codex"]);

    assert!(runtime.dispatch_json(&ai_event(
        serde_json::json!({"provider": "grok", "model": "m-2"})
    )));
    let saved = runtime.take_ai_settings_save().expect("one queued write");
    assert_eq!(
        saved.fallback[0].model.as_deref(),
        Some("m-2"),
        "a model for a listed agent is that entry's model"
    );
    assert_eq!(
        saved.router_config().priority,
        vec![
            hide_ai::ProviderId::CLAUDE,
            hide_ai::ProviderId::GROK,
            hide_ai::ProviderId::CODEX
        ],
        "the order of adding is the order of trying"
    );

    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "codex"}))));
    assert_eq!(
        runtime.snapshot.status.background_ai.provider.as_deref(),
        Some("codex")
    );
    assert_eq!(
        listed(&runtime),
        vec!["grok"],
        "the agent that now answers first is not its own fallback"
    );

    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"fallback_remove": "grok"}))));
    assert!(listed(&runtime).is_empty());
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"fallback_add": "codex"}))));
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "ai_settings.fallback_is_runs_on"
    );
}

/// PRD core-host-node-move amendment 3: a core waiting for its move's link
/// asks no provider for labels, though the operator's switch stays on; the
/// labels are asked for once the hold is released.
#[test]
fn a_pending_core_asks_no_provider_for_labels_until_its_link_commits() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_background_ai(ready_read(&["claude"]));
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "claude"}))));
    runtime.hold_effects();
    assert!(runtime.agent_summary());
    assert!(!runtime.labels_asked());
    assert!(runtime.release_effects());
    assert!(runtime.labels_asked());
}

/// PRD overview-request-view D-11, B21: the agent-summary switch moves the
/// snapshot at once, keeps the provider choice, and is queued to be kept.
#[test]
fn the_agent_summary_switch_moves_the_snapshot_and_queues_one_write() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_background_ai(ready_read(&["claude"]));
    assert!(
        !runtime.agent_summary(),
        "no agent is chosen yet, so no label is asked for (B47)"
    );
    assert!(runtime.snapshot.status.background_ai.agent_summary);

    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "claude"}))));
    runtime.take_ai_settings_save();
    assert!(
        runtime.agent_summary(),
        "on until the operator turns it off"
    );
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"agent_summary": false}))));
    assert!(!runtime.agent_summary());
    assert!(!runtime.snapshot.status.background_ai.agent_summary);
    let saved = runtime
        .take_ai_settings_save()
        .expect("the switch is queued for the coordinator to write");
    assert!(!saved.agent_summary);
    assert_eq!(saved.provider, hide_ai::ProviderId::CLAUDE);

    runtime.dispatch_json(&ai_event(serde_json::json!({"agent_summary": false})));
    assert!(
        runtime.take_ai_settings_save().is_none(),
        "the same position writes nothing"
    );
}

/// D-14, B33: Use Hide AI off makes no model call anywhere, keeps every other
/// value, and turning it on resumes exactly where it was.
#[test]
fn use_hide_ai_off_stops_every_model_use_and_keeps_the_choice() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_background_ai(ready_read(&["claude", "codex"]));
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "claude"}))));
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"fallback_add": "codex"}))));
    runtime.take_ai_settings_save();
    assert!(runtime.ai_active() && runtime.agent_summary());

    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"enabled": false}))));
    assert!(!runtime.snapshot.status.background_ai.enabled);
    assert!(
        !runtime.ai_active() && !runtime.agent_summary(),
        "agent titles fall back to the session's own text"
    );
    let saved = runtime.take_ai_settings_save().expect("the switch is kept");
    assert!(!saved.enabled);
    assert!(
        !saved.router_config().enabled,
        "a router built from it asks no model anything"
    );
    let section = &runtime.snapshot.status.background_ai;
    assert_eq!(section.provider.as_deref(), Some("claude"));
    assert_eq!(section.fallback.len(), 1, "the rest is preserved");

    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"enabled": true}))));
    assert!(runtime.ai_active() && runtime.agent_summary());
    assert_eq!(
        runtime.snapshot.status.background_ai.fallback.len(),
        1,
        "turning it on resumes where it was"
    );
}

/// D-18, D-27, B45, B47, B48: with nobody chosen, the first agent of the
/// fixed order that is switched on and signed in is chosen with its default
/// model; with none signed in nothing is chosen; a stored choice is kept.
#[test]
fn the_first_signed_in_agent_that_is_switched_on_is_chosen_by_itself() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    switch_on(&mut runtime, &["codex", "opencode"]);
    let asked = runtime.ai_request();
    assert_eq!(
        asked.selecting.iter().copied().collect::<Vec<_>>(),
        vec![hide_ai::ProviderId::CODEX, hide_ai::ProviderId::OPENCODE],
        "only agents that are on are asked, while nobody has chosen"
    );

    // Claude Code is signed in but not switched on here: it is not used.
    assert!(runtime.ingest_background_ai(ready_read(&["claude"])));
    assert!(runtime.snapshot.status.background_ai.provider.is_none());
    assert!(runtime.take_ai_settings_save().is_none());

    // Login is noticed later with no setting touched (B47).
    assert!(runtime.ingest_background_ai(ready_read(&["codex", "opencode"])));
    assert_eq!(
        runtime.snapshot.status.background_ai.provider.as_deref(),
        Some("codex"),
        "the first of the fixed order among those on and signed in"
    );
    let saved = runtime
        .take_ai_settings_save()
        .expect("the choice is stored");
    assert!(saved.chosen && saved.fallback.is_empty());
    assert_eq!(
        saved.model(hide_ai::ProviderId::CODEX),
        hide_ai::settings::default_model(hide_ai::ProviderId::CODEX)
    );
    assert!(
        runtime.ai_request().selecting.is_empty(),
        "once chosen nothing selects"
    );

    // A stored choice is never replaced by the rule.
    let mut stored = hide_ai::AiSettings::default();
    stored.set_provider(hide_ai::ProviderId::CLAUDE);
    stored.set_model(hide_ai::ProviderId::CLAUDE, "haiku");
    runtime.ingest_ai_settings(stored, None);
    runtime.ingest_background_ai(ready_read(&["codex"]));
    assert_eq!(
        runtime.snapshot.status.background_ai.provider.as_deref(),
        Some("claude")
    );
    assert!(runtime.take_ai_settings_save().is_none());
}

/// PRD home-device-rail D-18: a start surface's model menu reads the provider
/// catalog while it shows, without the Settings tab's hook diagnosis, and the
/// two demands stop independently.
#[test]
fn a_start_surface_reads_the_catalog_without_the_settings_diagnosis() {
    let mut runtime = runtime();
    let event = |payload: serde_json::Value| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2, "kind": "ai_settings", "payload": payload
        }))
        .expect("the event encodes")
    };
    assert!(runtime.dispatch_json(&event(serde_json::json!({"start_observing": true}))));
    assert!(runtime.ai_request().observing);
    assert!(!runtime.settings_observed());

    assert!(runtime.dispatch_json(&event(serde_json::json!({"observing": true}))));
    assert!(runtime.dispatch_json(&event(serde_json::json!({"start_observing": false}))));
    assert!(runtime.ai_request().observing, "Settings is still looking");
    assert!(runtime.settings_observed());

    assert!(runtime.dispatch_json(&event(serde_json::json!({"observing": false}))));
    assert!(!runtime.ai_request().observing);
}

#[test]
fn an_unreadable_choice_and_a_failed_write_are_stated_rather_than_dropped() {
    let mut runtime = runtime();
    assert!(runtime.ingest_ai_settings(
        hide_ai::AiSettings::default(),
        Some("The saved choice could not be read; the defaults are in use".to_owned()),
    ));
    assert_eq!(
        runtime.snapshot.status.background_ai.provider, None,
        "no choice is invented from a file that could not be read"
    );
    assert!(
        runtime
            .snapshot
            .status
            .background_ai
            .unavailable_reason
            .is_some(),
        "and the reason travels with them rather than being swallowed"
    );

    assert!(runtime.report_ai_settings_failure("The choice could not be saved".to_owned()));
    assert_eq!(
        runtime
            .snapshot
            .status
            .background_ai
            .unavailable_reason
            .as_deref(),
        Some("The choice could not be saved")
    );
    assert!(
        !runtime.report_ai_settings_failure("The choice could not be saved".to_owned()),
        "the same failure publishes once"
    );
}

#[test]
fn an_unknown_provider_or_a_model_with_no_provider_is_refused() {
    let mut runtime = runtime();
    let event = |payload: serde_json::Value| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2, "kind": "ai_settings", "payload": payload
        }))
        .expect("the event encodes")
    };

    assert!(runtime.dispatch_json(&event(serde_json::json!({"provider": "gemini"}))));
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .expect("the refusal is visible")
            .kind,
        "ai_settings.unknown_provider"
    );
    assert_eq!(runtime.snapshot.status.background_ai.provider, None);
    assert!(runtime.take_ai_settings_save().is_none());

    assert!(runtime.dispatch_json(&event(serde_json::json!({"model": "sonnet"}))));
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .expect("the refusal is visible")
            .kind,
        "ai_settings.model_without_provider",
        "a model is never applied to whichever provider happens to be selected"
    );
    assert!(runtime.take_ai_settings_save().is_none());
}

#[test]
fn usage_activity_hints_are_accepted_without_persisting_ui_state() {
    let mut runtime = runtime();
    let state_path = runtime.state_path.clone();
    let _ = std::fs::remove_file(&state_path);
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "ui_state_update",
        "payload": {
            "expanded_paths": [],
            "selected_path": null,
            "selected_pane_id": null,
            "usage_window_visible": true,
            "usage_popover_open": true
        }
    }))
    .expect("usage activity event");

    assert!(runtime.dispatch_json(&event));
    assert_eq!(
        runtime.usage_activity(),
        crate::usage::UsageActivity {
            window_visible: true,
            popover_open_generation: 1,
            ..Default::default()
        }
    );
    assert!(
        !state_path.exists(),
        "an observation hint must not create persistent UI state"
    );
}

/// PRD B24. Remote relationship navigation keeps the established remote
/// focus path, but its visible result is correlated to the existing
/// remote-control request receipt rather than the shell's optimistic
/// navigation projection.
#[test]
fn remote_pane_focus_uses_its_existing_request_outcome() {
    let target_id = "mini";
    let source_pane_id = "w1:p2";
    let projected_pane_id = remote_pane_id_prefix(target_id) + source_pane_id;
    let projected_workspace_id = "remote:mini:workspace:w1";
    let projected_checkout_id = "remote:mini:checkout:w1";
    let projected_tab_id = "remote:mini:tab:w1:t1";
    let mut remote_checkout = checkout(
        projected_workspace_id,
        projected_checkout_id,
        "/tmp/hide-remote-focus",
        Some(pane(&projected_pane_id, "/tmp/hide-remote-focus")),
    );
    remote_checkout.tabs[0].id = Some(projected_tab_id.to_owned());
    let mut remote_workspace = workspace(
        projected_workspace_id,
        "Remote",
        "/tmp/hide-remote-focus",
        vec![remote_checkout],
    );
    remote_workspace.remote_target_id = Some(target_id.to_owned());
    remote_workspace.device_id = target_id.to_owned();

    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: target_id.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![remote_workspace],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: Some(projected_workspace_id.to_owned()),
            focused_checkout_id: Some(projected_checkout_id.to_owned()),
            focused_tab_id: Some(projected_tab_id.to_owned()),
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
    let connector: Arc<dyn hide_herdr_client::ApiConnector> =
        Arc::new(hide_herdr_client::LocalSocketConnector::new(
            "/tmp/herdr-core-remote-focus-never-connect.sock",
        ));
    runtime.install_remote_control(RemoteControlContext::new(
        target_id,
        connector,
        Weak::new(),
        ChangeNotifier::noop(),
    ));

    let request_id = "relationship-remote-1";
    assert!(runtime.request_remote_control(RemoteControlPayload {
        target_id: target_id.to_owned(),
        request_id: request_id.to_owned(),
        report_pane_focus_outcome: true,
        focus_device: false,
        request: RemoteControlRequest::FocusPane {
            pane_id: projected_pane_id.clone(),
        },
    }));
    let pending = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .expect("the remote focus request is projected");
    assert_eq!(pending.request_id, request_id);
    assert_eq!(pending.target_pane_id, projected_pane_id);
    assert_eq!(pending.phase, "pending");

    assert!(runtime.request_remote_control(RemoteControlPayload {
        target_id: target_id.to_owned(),
        request_id: "ordinary-remote-focus".to_owned(),
        report_pane_focus_outcome: false,
        focus_device: false,
        request: RemoteControlRequest::FocusPane {
            pane_id: projected_pane_id.clone(),
        },
    }));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .map(|request| request.request_id.as_str()),
        Some(request_id),
        "ordinary remote focus keeps its transport receipt without replacing B24's outcome"
    );

    runtime.set_error("pane.focus_failed", "another pane failed", true);
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .map(|request| request.phase.as_str()),
        Some("pending"),
        "an unrelated error is not the remote request's outcome"
    );

    assert!(runtime.ingest_remote_control_result(
        target_id,
        request_id,
        RemoteControlAction::Pane(PaneControlAction::Focus {
            pane_id: source_pane_id.to_owned(),
        }),
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: None,
            created_pane_id: None,
        }),
        7,
    ));
    let succeeded = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .expect("the matching remote outcome is projected");
    assert_eq!(succeeded.request_id, request_id);
    assert_eq!(succeeded.phase, "succeeded");
}

/// PRD B24. Only the matching remote request can end the pending intent,
/// and an explicit remote failure remains retryable at the relationship
/// control without inventing a shell timeout or rollback event.
#[test]
fn remote_pane_focus_ignores_unrelated_results_and_surfaces_failure() {
    let mut runtime = runtime();
    runtime.snapshot.status.pane_focus_request = Some(PaneFocusRequestSnapshot {
        request_id: "relationship-remote-2".to_owned(),
        target_pane_id: "remote:mini:pane:w1:p2".to_owned(),
        phase: "pending".to_owned(),
        message: None,
        retryable: false,
    });
    let action = RemoteControlAction::Pane(PaneControlAction::Focus {
        pane_id: "w1:p2".to_owned(),
    });

    assert!(runtime.ingest_remote_control_result(
        "mini",
        "unrelated-request",
        action.clone(),
        Err("unrelated refusal".to_owned()),
        4,
    ));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .map(|request| request.phase.as_str()),
        Some("pending")
    );

    assert!(runtime.ingest_remote_control_result(
        "mini",
        "relationship-remote-2",
        action,
        Err("remote pane refused focus".to_owned()),
        5,
    ));
    let failed = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .expect("the matching failure is projected");
    assert_eq!(failed.phase, "failed");
    assert_eq!(failed.message.as_deref(), Some("remote pane refused focus"));
    assert!(failed.retryable);
}

/// Two tabs with one agent each. The operator looks at the pane in tab 1
/// and switches to tab 2. The read record follows the keyboard: tab 2's
/// pane is read, and a state change in tab 1 afterwards is unread.
#[test]
fn read_record_follows_a_tab_switch_the_operator_made() {
    let checkout_path = "/private/tmp/hide-read-record-tab-switch";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let payload = |seq_t1: u64, seq_t2: u64| -> SessionSnapshotPayload {
        let tab = |tab_id: &str| {
            serde_json::json!({
                "workspace_id": "w-order",
                "tab_id": tab_id,
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": format!("{tab_id}:p"),
                "panes": [{
                    "pane_id": format!("{tab_id}:p"),
                    "rect": {"x": 0, "y": 0, "width": 80, "height": 24}
                }],
                "splits": []
            })
        };
        let agent = |tab_id: &str, seq: u64| {
            serde_json::json!({
                "pane_id": format!("{tab_id}:p"),
                "workspace_label": "order",
                "agent": "codex",
                "agent_status": "done",
                "state_change_seq": seq,
                "tokens": {"status_done_new": "\u{25cf}", "activity": "0000000000001"}
            })
        };
        crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": [agent("w-order:t1", seq_t1), agent("w-order:t2", seq_t2)],
            "focused_workspace_id": "w-order",
            "focused_pane_id": "w-order:t1:p",
            "workspaces": [{
                "workspace_id": "w-order", "label": "order", "active_tab_id": "w-order:t1"
            }],
            "tabs": [
                {"workspace_id": "w-order", "tab_id": "w-order:t1", "label": ""},
                {"workspace_id": "w-order", "tab_id": "w-order:t2", "label": ""}
            ],
            "panes": [
                {"pane_id": "w-order:t1:p", "cwd": checkout_path},
                {"pane_id": "w-order:t2:p", "cwd": checkout_path}
            ],
            "layouts": [tab("w-order:t1"), tab("w-order:t2")]
        }))
        .expect("two-tab agent payload")
    };
    runtime.ingest_session(Ok(payload(1, 1)));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    assert_eq!(unread_panes(&runtime), vec!["w-order:t2:p"]);

    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    assert!(
        unread_panes(&runtime).is_empty(),
        "the pane of the tab the operator switched to holds the keyboard and is read"
    );

    runtime.ingest_session(Ok(payload(2, 1)));
    assert_eq!(
        unread_panes(&runtime),
        vec!["w-order:t1:p"],
        "a change in the tab the operator left is unread; the record did not stay there"
    );
}

/// A checkout switch releases the read record without raising one. Two
/// checkouts at different paths; the operator reads pane A in the first,
/// then reaches pane B, in the second checkout's other tab, through the
/// sidebar, which sends a checkout focus and then a pane focus. Pane C,
/// on the tab the second checkout comes forward with, is not marked read
/// by the checkout switch, and a change on A afterwards is unread.
#[test]
fn read_record_is_released_and_not_raised_by_a_checkout_switch() {
    let mut runtime = live_runtime();
    let roots = scratch_dir("hide-checkout-switch-");
    let root_a = roots.path().join("a").to_string_lossy().into_owned();
    let root_b = roots.path().join("b").to_string_lossy().into_owned();
    std::fs::create_dir_all(&root_a).expect("checkout a");
    std::fs::create_dir_all(&root_b).expect("checkout b");
    // Each root is its own repository. The catalog keys a checkout by the
    // repository that contains it, so two plain directories under one
    // repository - which is what a temporary directory inside the project
    // is - collapse into a single checkout and this test loses the second
    // one before it starts.
    for root in [&root_a, &root_b] {
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(root)
                .status()
                .expect("git init runs")
                .success()
        );
    }
    runtime.snapshot.ui_state.workspace_registrations = vec![
        WorkspaceRegistration {
            primary_checkout_id: None,
            id: "workspace:a".to_owned(),
            label: "a".to_owned(),
            path: root_a.clone(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        },
        WorkspaceRegistration {
            primary_checkout_id: None,
            id: "workspace:b".to_owned(),
            label: "b".to_owned(),
            path: root_b.clone(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        },
    ];
    runtime.rebuild_catalog();
    let checkout_of = |runtime: &Runtime, workspace_id: &str| -> String {
        runtime
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .and_then(|workspace| workspace.checkouts.first())
            .map(|checkout| checkout.id.clone())
            .expect("registered checkout")
    };
    let checkout_a = checkout_of(&runtime, "workspace:a");
    let checkout_b = checkout_of(&runtime, "workspace:b");
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace:a".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_a.clone());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_a);
    runtime.reset_terminal_projection(None);

    let payload = |seq_a: u64| -> SessionSnapshotPayload {
        let layout = |workspace_id: &str, tab_id: &str, pane_ids: &[&str], focused: &str| {
            serde_json::json!({
                "workspace_id": workspace_id,
                "tab_id": tab_id,
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": focused,
                "panes": pane_ids.iter().enumerate().map(|(index, pane_id)| serde_json::json!({
                    "pane_id": pane_id,
                    "rect": {"x": index * 40, "y": 0, "width": 40, "height": 24}
                })).collect::<Vec<_>>(),
                "splits": []
            })
        };
        let agent = |pane_id: &str, seq: u64| {
            serde_json::json!({
                "pane_id": pane_id,
                "workspace_label": "fixture",
                "agent": "codex",
                "agent_status": "done",
                "state_change_seq": seq,
                "tokens": {"status_done_new": "\u{25cf}", "activity": format!("{seq:013}")}
            })
        };
        crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": [agent("wa:pA", seq_a), agent("wb:pB", 1), agent("wb:pC", 1)],
            "focused_workspace_id": "wa",
            "focused_pane_id": "wa:pA",
            "workspaces": [
                {"workspace_id": "wa", "label": "a", "active_tab_id": "wa:t1"},
                {"workspace_id": "wb", "label": "b", "active_tab_id": "wb:t1"}
            ],
            "tabs": [
                {"workspace_id": "wa", "tab_id": "wa:t1", "label": ""},
                {"workspace_id": "wb", "tab_id": "wb:t1", "label": ""},
                {"workspace_id": "wb", "tab_id": "wb:t2", "label": ""}
            ],
            "panes": [
                {"pane_id": "wa:pA", "cwd": root_a},
                {"pane_id": "wb:pB", "cwd": root_b},
                {"pane_id": "wb:pC", "cwd": root_b}
            ],
            "layouts": [
                layout("wa", "wa:t1", &["wa:pA"], "wa:pA"),
                layout("wb", "wb:t1", &["wb:pC"], "wb:pC"),
                layout("wb", "wb:t2", &["wb:pB"], "wb:pB")
            ]
        }))
        .expect("two-checkout payload")
    };
    runtime.ingest_session(Ok(payload(1)));
    assert!(runtime.dispatch_json(&operator_focus_event("wa:pA")));
    assert_eq!(unread_panes(&runtime), vec!["wb:pB", "wb:pC"]);

    let focus_b = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_checkout",
        "payload": {"workspace_id": "workspace:b", "checkout_id": checkout_b}
    }))
    .expect("focus checkout event");
    assert!(runtime.dispatch_json(&focus_b));
    assert_eq!(
        runtime.snapshot.navigator.focused_checkout_id.as_deref(),
        Some(checkout_b.as_str()),
        "error: {:?}",
        runtime.snapshot.status.last_error
    );
    assert_eq!(
        runtime.snapshot.ui_state.selected_pane_id.as_deref(),
        Some("wb:pC"),
        "the checkout comes forward on its remembered pane"
    );
    assert_eq!(
        unread_panes(&runtime),
        vec!["wb:pB", "wb:pC"],
        "the checkout coming forward raises no record for its remembered pane"
    );

    assert!(runtime.dispatch_json(&operator_focus_event("wb:pB")));
    assert_eq!(
        unread_panes(&runtime),
        vec!["wb:pC"],
        "only the pane the operator reached is read"
    );

    runtime.ingest_session(Ok(payload(2)));
    assert!(
        unread_panes(&runtime).contains(&"wa:pA".to_owned()),
        "a change on the pane the operator left is unread; the record did not stay there"
    );
    std::fs::remove_dir_all(&root_a).ok();
    std::fs::remove_dir_all(&root_b).ok();
}

/// Returning from a remote device left `remote:<target>:pane:<id>` in the
/// selection, and every local sync tick then compared it against local
/// layouts, never matched, and re-raised the same projection error. A
/// remote id names a pane this session can never hold, so it is not a
/// local selection that has gone missing.
#[test]
fn a_remote_pane_left_in_the_selection_does_not_block_local_projection() {
    let mut runtime = runtime();
    let checkout_path = "/tmp/hide-remote-selection-leak";
    let workspace_id = workspace::workspace_id_for_path(Path::new(checkout_path));
    let checkout_id = workspace::checkout_id_for_path(&workspace_id, Path::new(checkout_path));
    let registration = WorkspaceRegistration {
        primary_checkout_id: None,
        id: workspace_id.clone(),
        label: "Remote selection leak".to_owned(),
        path: checkout_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    };
    let local_pane = pane("wL:p1", checkout_path);
    let selected_workspace = workspace(
        &workspace_id,
        "Remote selection leak",
        checkout_path,
        vec![checkout(
            &workspace_id,
            &checkout_id,
            checkout_path,
            Some(local_pane),
        )],
    );
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.snapshot.navigator.workspaces = vec![selected_workspace.clone()];
    runtime.snapshot.navigator.focused_workspace_id = Some(workspace_id.clone());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id);
    runtime.snapshot.navigator.root_path = Some(checkout_path.to_owned());
    // What a trip to the remote device and back leaves behind.
    runtime.snapshot.ui_state.selected_pane_id = Some("remote:mini:pane:w59:p2".to_owned());
    runtime.snapshot.terminal.pane_id = Some("remote:mini:pane:w59:p2".to_owned());
    runtime.restore_hint_pending = false;
    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "panes": [{"pane_id": "wL:p1", "cwd": checkout_path}],
        "tabs": [{"workspace_id": "wL", "tab_id": "wL:t1", "label": ""}],
        "layouts": [{
            "workspace_id": "wL",
            "tab_id": "wL:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "wL:p1",
            "panes": [{"pane_id": "wL:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }]
    }))
    .expect("local session payload");
    let catalog = session_sync::PrecomputedCatalog {
        worktrees: runtime.worktree_catalog(),
        registrations: vec![registration],
        workspaces: vec![selected_workspace],
        paths: Default::default(),
    };

    assert!(runtime.ingest_session_with_catalog(Ok(payload), Some(catalog)));
    assert_eq!(runtime.snapshot().status.last_error, None);
    assert_eq!(
        runtime
            .snapshot()
            .active_pane_layout()
            .map(|layout| layout.focused_pane_id.as_str()),
        Some("wL:p1")
    );
}

#[test]
fn a_broken_sync_update_keeps_the_last_valid_agents_and_says_it_is_disconnected() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(working_payload()));
    assert_eq!(runtime.snapshot().navigator.agents.len(), 1);
    assert_eq!(runtime.snapshot().pet.pose, "carrying");
    assert_eq!(runtime.snapshot().pet.badges.working, 1);

    runtime.ingest_session(Err(SessionFetchError::SocketMissing(
        "Herdr socket file does not exist".to_owned(),
    )));
    let down = runtime.snapshot();
    assert_eq!(
        down.navigator.agents.len(),
        1,
        "the last valid agent list is retained rather than blanked"
    );
    assert_eq!(down.pet.pose, "disconnected");
    assert_eq!(down.pet.connection, "socket_missing");
    assert!(
        down.pet.connection_message.is_some(),
        "a missing socket is stated, never a silent idle"
    );
    assert_eq!(
        down.pet.badges.working, 0,
        "a server that stopped answering cannot keep a working badge lit"
    );
    assert_eq!(down.pet.badges.disconnected, 1);

    // The next valid sync update recovers on its own.
    runtime.ingest_session(Ok(working_payload()));
    assert_eq!(runtime.snapshot().pet.pose, "carrying");
    assert_eq!(runtime.snapshot().pet.connection, "connected");
}

/// AC2, AC4, SC3. An operator focus writes the pane's read record to the
/// store, and a pane Herdr stops reporting is gone from the file on the
/// next save, so the record cannot grow without bound.
#[test]
fn read_record_is_written_for_the_operator_focused_pane_and_evicted_when_it_disappears() {
    let mut runtime = live_runtime();
    let state_path = runtime.state_path.clone();
    runtime.ingest_session(Ok(working_payload()));
    assert!(
        runtime.snapshot().ui_state.pane_read_records.is_empty(),
        "the focus that arrived with the session is not a look the operator took"
    );

    assert!(runtime.dispatch_json(&operator_focus_event("w1:p1")));
    assert!(
        runtime
            .snapshot()
            .ui_state
            .pane_read_records
            .contains_key("w1:p1"),
        "the pane the operator chose is read"
    );
    let stored = std::fs::read_to_string(&state_path).expect("state file");
    assert!(stored.contains("w1:p1"), "the record reached the file");

    let empty: SessionSnapshotPayload =
        crate::sidebar::owned_label_fixture(serde_json::json!({"agents": []}))
            .expect("empty payload");
    runtime.ingest_session(Ok(empty));
    assert!(
        runtime.snapshot().ui_state.pane_read_records.is_empty(),
        "a pane Herdr stopped reporting leaves no record behind"
    );
    // The persisted selection may still name the pane, so the read
    // records are checked on their own.
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).expect("state file"))
            .expect("state file is JSON");
    assert!(
        stored["pane_read_records"]
            .as_object()
            .is_none_or(|records| !records.contains_key("w1:p1")),
        "the record left the file too"
    );
    let _ = std::fs::remove_file(&state_path);
}

/// A restarted server restores pane topology before agent detection has
/// necessarily caught up. The transient empty list must keep the pane's read
/// record, and the same stopped agent must not return to Done only because the
/// new server assigned a different process-local sequence.
#[test]
fn restored_pane_keeps_its_read_record_through_delayed_agent_detection() {
    let mut runtime = live_runtime();
    let original = [
        ("w1:p1", 6018_u64),
        ("w1:p2", 6019_u64),
        ("w1:p3", 6020_u64),
    ];
    runtime.ingest_session(Ok(finished_tab_payload(&original, "w1:p1")));
    assert!(runtime.dispatch_json(&operator_focus_event("w1:p1")));
    assert_eq!(unread_panes(&runtime), vec!["w1:p2", "w1:p3"]);

    runtime.begin_read_connection(None);
    let mut topology_first = finished_tab_payload(&original, "w1:p1");
    topology_first.agents.clear();
    runtime.ingest_session(Ok(topology_first));
    assert!(
        runtime
            .snapshot()
            .ui_state
            .pane_read_records
            .contains_key("w1:p1"),
        "an incomplete agent list cannot evict a live pane's record"
    );

    let restored = [("w1:p1", 3_u64), ("w1:p2", 4_u64), ("w1:p3", 5_u64)];
    runtime.ingest_session(Ok(finished_tab_payload(&restored, "w1:p1")));
    assert!(
        unread_panes(&runtime)
            .iter()
            .all(|pane_id| pane_id != "w1:p1"),
        "the restored stopped agent stays Seen after sequence rebasing"
    );
}

/// A device's Herdr server restarts the way this machine's does, and its
/// panes are judged the same way: a pane the operator read there stays read
/// through the restored server's steps, the mark Hide wants on it names the
/// device's own pane id, and work after the restore is news again.
#[test]
fn a_device_pane_read_before_its_server_restarts_stays_read() {
    let mut runtime = runtime();
    let pane_id = "remote:mini:pane:w9:p1";
    let workspace_id = "remote:mini:workspace:w9";
    let checkout_id = "remote:mini:checkout:w9";
    let tab_id = "remote:mini:tab:w9:t1";
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let session = |status: &str, seq: u64, focused: Option<&str>| {
        let mut remote_checkout = checkout(
            workspace_id,
            checkout_id,
            "/tmp/herdr-remote-read",
            Some(pane(pane_id, "/tmp/herdr-remote-read")),
        );
        remote_checkout.tabs[0].id = Some(tab_id.to_owned());
        let mut remote_workspace = workspace(
            workspace_id,
            "Remote",
            "/tmp/herdr-remote-read",
            vec![remote_checkout],
        );
        remote_workspace.remote_target_id = Some("mini".to_owned());
        remote_workspace.device_id = "mini".to_owned();
        RemoteSessionSnapshot {
            workspaces: vec![remote_workspace],
            agents: project_agents(
                serde_json::from_value(serde_json::json!({
                    "agents": [{"pane_id": pane_id, "agent": "codex", "agent_status": status,
                        "state_change_seq": seq, "agent_session": {"kind": "id", "value": "s1"},
                        "cwd": "/tmp/herdr-remote-read"}]
                }))
                .unwrap(),
            )
            .agents,
            active_tab_ids: [(checkout_id.to_owned(), tab_id.to_owned())]
                .into_iter()
                .collect(),
            focused_workspace_id: Some(workspace_id.to_owned()),
            focused_checkout_id: Some(checkout_id.to_owned()),
            focused_tab_id: Some(tab_id.to_owned()),
            focused_pane_id: focused.map(str::to_owned),
            pane_layouts: Vec::new(),
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
            herdr_version: None,
        }
    };
    let unread = |runtime: &Runtime| {
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .agents[0]
            .unread
    };

    runtime.begin_read_connection(Some("mini"));
    runtime.ingest_remote_session("mini", Ok(session("done", 40, Some(pane_id))));
    assert!(!unread(&runtime));
    let wanted = runtime.wanted_server_marks(Some("mini"));
    assert_eq!(
        wanted.keys().collect::<Vec<_>>(),
        ["w9:p1"],
        "the mark goes on the device's own pane id"
    );

    runtime.begin_read_connection(Some("mini"));
    for (status, seq) in [("idle", 0), ("unknown", 1), ("done", 5)] {
        runtime.ingest_remote_session("mini", Ok(session(status, seq, None)));
        assert!(!unread(&runtime), "{status} at {seq} stays read");
    }
    runtime.ingest_remote_session("mini", Ok(session("working", 6, None)));
    assert!(unread(&runtime), "work after the restore is news");
}

/// AC2, AC7, SC1. The defect the PRD was written against, from the other
/// side: one click on a Done row clears that row and nothing else, even
/// though Herdr reports the whole tab seen and names its own focused pane.
#[test]
fn read_record_follows_the_row_the_operator_clicked_and_no_other() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    assert_eq!(
        unread_panes(&runtime),
        vec!["w1:p1", "w1:p2", "w1:p3"],
        "nothing is read before the operator looks at anything"
    );

    assert!(runtime.dispatch_json(&operator_focus_event("w1:p3")));
    assert_eq!(
        unread_panes(&runtime),
        vec!["w1:p1", "w1:p2"],
        "only the clicked row leaves Done"
    );
}

/// AC2, SC1. Bringing a tab forward makes Herdr report the pane that tab
/// last had focused. Nobody chose that pane in this session, so no record
/// moves and the rows stay where they are.
#[test]
fn read_record_ignores_the_focus_a_tab_carries_when_it_comes_forward() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p2")));

    assert!(
        runtime.snapshot().ui_state.pane_read_records.is_empty(),
        "a focus Hide only inherited is not a look the operator took"
    );
    assert_eq!(unread_panes(&runtime), vec!["w1:p1", "w1:p2", "w1:p3"]);
}

/// AC4, SC3. A launch inherits Herdr's focus and puts the terminal back on
/// the pane the last session ended on. Neither is the operator looking at
/// anything, so an item that was unread before the quit is still unread.
#[test]
fn read_record_survives_a_launch_that_inherits_a_focus() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    assert!(runtime.dispatch_json(&restore_focus_event("w1:p1")));

    assert!(
        runtime.snapshot().ui_state.pane_read_records.is_empty(),
        "restoring the last session's selection reads nothing"
    );
    assert_eq!(unread_panes(&runtime), vec!["w1:p1", "w1:p2", "w1:p3"]);
}

/// AC3, R2. While the operator stays on the pane they chose, what the
/// agent does there is read as it happens, so the row does not come back.
#[test]
fn read_record_keeps_up_with_the_pane_the_operator_is_watching() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    assert!(runtime.dispatch_json(&operator_focus_event("w1:p3")));

    let moved = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6031)];
    runtime.ingest_session(Ok(finished_tab_payload(&moved, "w1:p3")));
    assert_eq!(
        unread_panes(&runtime),
        vec!["w1:p1", "w1:p2"],
        "the watched pane stays read as its agent moves"
    );
}

/// AC2, R2. Once Herdr moves focus off the pane the operator chose, that
/// pane stops counting as watched, so its next change comes back unread.
#[test]
fn read_record_stops_following_a_pane_herdr_moved_focus_away_from() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    assert!(runtime.dispatch_json(&operator_focus_event("w1:p3")));
    // The requested focus lands, then a spawned pane takes it away.
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p3")));
    super::session_navigation::finish_running_pane_focus(&mut runtime, Ok(()));
    assert!(runtime.pending_pane_focus.is_none());
    assert!(runtime.pane_focus_in_flight.is_none());
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 1);

    let moved = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6031)];
    runtime.ingest_session(Ok(finished_tab_payload(&moved, "w1:p1")));
    assert_eq!(
        unread_panes(&runtime),
        vec!["w1:p1", "w1:p2", "w1:p3"],
        "a pane nobody is watching comes back unread when it changes"
    );
}

/// AC2, AC3, AC4, R2. A pane is a pane: a remote pane earns a read record
/// from the focus its own server reports, exactly as a local pane earns one
/// from Hide's focus. The ledger is pruned by pane id namespace, so a local
/// sync cannot drop a remote record and one target cannot drop another's.
/// Before this, no record survived for a remote pane and the pane tree
/// could disagree with the agent row about an unresolved close demand.
#[test]
fn read_record_is_scoped_by_pane_id_namespace_across_servers() {
    let mut runtime = live_runtime();
    let state_path = runtime.state_path.clone();
    for target_id in ["mini", "build"] {
        runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
            target_id: target_id.to_owned(),
            state: "not_connected".to_owned(),
            message: None,
            herdr_version: None,
            session: None,
            files: RemoteFileListSnapshot::idle(),
            catalog: Default::default(),
            unreachable_since_unix_ms: None,
        });
    }
    let remote_session = |target_id: &str, pane_ids: &[&str], focused: Option<&str>| {
        let payload: SessionSnapshotPayload =
            crate::sidebar::owned_label_fixture(serde_json::json!({
                "agents": pane_ids
                    .iter()
                    .map(|pane_id| serde_json::json!({
                        "pane_id": pane_id,
                        "workspace_label": "Remote",
                        "agent": "codex",
                        "agent_status": "done",
                        "state_change_seq": 4,
                        "tokens": {"activity": "0000000000001"}
                    }))
                    .collect::<Vec<_>>()
            }))
            .expect("remote payload");
        let mut agents = project_agents(payload).agents;
        for agent in &mut agents {
            agent.pane_id = remote_pane_id_prefix(target_id) + &agent.pane_id;
            agent.id = agent.pane_id.clone();
        }
        // The pane tree arrives freshly projected with no read axis
        // applied, which is what a remote sync actually delivers.
        let panes = agents
            .iter()
            .map(|agent| {
                let mut projected = pane(&agent.pane_id, "/tmp/hide-remote-tree");
                projected.status_code = agent.status_code;
                projected.requires_close_confirmation = agent.requires_close_confirmation;
                projected
            })
            .collect::<Vec<_>>();
        let mut remote_workspace = workspace(
            "remote:ws",
            "Remote",
            "/tmp/hide-remote-tree",
            vec![checkout(
                "remote:ws",
                "remote:checkout",
                "/tmp/hide-remote-tree",
                None,
            )],
        );
        remote_workspace.checkouts[0].tabs = vec![TabSnapshot {
            agent: None,
            naming: Default::default(),
            id: Some("remote:tab".to_owned()),
            workspace_id: Some("remote:ws".to_owned()),
            checkout_id: Some("remote:checkout".to_owned()),
            label: Some("Session".to_owned()),
            empty: false,
            delegated: false,
            panes,
        }];
        RemoteSessionSnapshot {
            workspaces: vec![remote_workspace],
            agents,
            active_tab_ids: BTreeMap::new(),
            focused_workspace_id: None,
            focused_checkout_id: None,
            focused_tab_id: None,
            focused_pane_id: focused.map(|pane_id| remote_pane_id_prefix(target_id) + pane_id),
            pane_layouts: Vec::new(),
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
            herdr_version: None,
        }
    };
    let stored_agents = |runtime: &Runtime, target_id: &str| {
        runtime
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == target_id)
            .and_then(|status| status.session.as_ref())
            .map(|session| {
                session
                    .agents
                    .iter()
                    .map(|agent| {
                        (
                            agent.pane_id.clone(),
                            (
                                agent.status_code,
                                agent.requires_close_confirmation,
                                agent.group.clone(),
                            ),
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .expect("remote session")
    };
    let tree_panes = |runtime: &Runtime, target_id: &str| {
        runtime
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == target_id)
            .and_then(|status| status.session.as_ref())
            .map(|session| {
                session
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .flat_map(|checkout| checkout.tabs.iter())
                    .flat_map(|tab| tab.panes.iter())
                    .map(|pane| {
                        (
                            pane.id.clone(),
                            (pane.status_code, pane.requires_close_confirmation),
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .expect("remote session")
    };

    runtime.ingest_session(Ok(working_payload()));
    assert!(runtime.dispatch_json(&operator_focus_event("w1:p1")));
    assert!(
        runtime
            .snapshot
            .ui_state
            .pane_read_records
            .contains_key("w1:p1")
    );

    runtime.ingest_remote_session(
        "mini",
        Ok(remote_session("mini", &["w9:p1", "w9:p2"], Some("w9:p1"))),
    );
    assert!(
        runtime
            .snapshot
            .ui_state
            .pane_read_records
            .contains_key("remote:mini:pane:w9:p1"),
        "the pane the remote server focuses gets a read record like any other"
    );
    let mini = stored_agents(&runtime, "mini");
    assert_eq!(
        mini["remote:mini:pane:w9:p1"],
        (AgentStatusCode::Idle, false, "seen".to_owned()),
        "a stopped remote pane the operator has read closes without a prompt"
    );
    assert_eq!(
        mini["remote:mini:pane:w9:p2"],
        (AgentStatusCode::Stopped, false, "done".to_owned()),
        "an unread completion does not create a close prompt"
    );
    // The pane tree is what the remote pane surface reads, so the read
    // axis has to reach it and not only the agent rows.
    let tree = tree_panes(&runtime, "mini");
    assert_eq!(
        tree["remote:mini:pane:w9:p1"],
        (AgentStatusCode::Idle, false),
        "the remote pane tree carries the read pane's answer, not the projected one"
    );
    assert_eq!(
        tree["remote:mini:pane:w9:p2"],
        (AgentStatusCode::Stopped, false),
        "the pane tree carries the completion protection decision"
    );

    runtime.ingest_session(Ok(working_payload()));
    assert!(
        runtime
            .snapshot
            .ui_state
            .pane_read_records
            .contains_key("remote:mini:pane:w9:p1"),
        "a local sync never prunes a remote record"
    );

    runtime.ingest_remote_session(
        "build",
        Ok(remote_session("build", &["w2:p1"], Some("w2:p1"))),
    );
    assert!(
        runtime
            .snapshot
            .ui_state
            .pane_read_records
            .contains_key("remote:mini:pane:w9:p1"),
        "one target's sync never prunes another target's record"
    );

    runtime.ingest_remote_session("mini", Ok(remote_session("mini", &["w9:p2"], None)));
    let records = runtime
        .snapshot
        .ui_state
        .pane_read_records
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        records,
        vec!["remote:build:pane:w2:p1".to_owned(), "w1:p1".to_owned(),],
        "a target prunes only its own namespace"
    );
    let _ = std::fs::remove_file(&state_path);
}

/// The pane tree is projected separately from the navigator's agent rows.
/// It published `Done` and demanded a close confirmation for every pane the
/// operator had already read, because that projection never saw the record
/// ledger.
#[test]
fn read_record_reaches_the_pane_tree_and_not_only_the_agent_rows() {
    let mut runtime = live_runtime();
    let state_path = runtime.state_path.clone();
    let idle = |pane_id: &str| {
        serde_json::json!({
            "pane_id": pane_id,
            "workspace_label": "Fixture",
            "agent": "codex",
            "agent_status": "idle",
            "tokens": {"status_idle": "\u{25cb}", "activity": "0000000000001"}
        })
    };
    let done = |pane_id: &str| {
        serde_json::json!({
            "pane_id": pane_id,
            "workspace_label": "Fixture",
            "agent": "codex",
            "agent_status": "done",
            "tokens": {"status_done_new": "●", "activity": "0000000000001"}
        })
    };
    let checkout_path = "/private/tmp/hide-read-record-pane-tree";
    runtime.snapshot.ui_state.workspace_registrations = vec![WorkspaceRegistration {
        primary_checkout_id: None,
        id: "workspace:read-record".to_owned(),
        label: "read-record".to_owned(),
        path: checkout_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    }];
    runtime.rebuild_catalog();
    let checkout_id =
        workspace::checkout_id_for_path("workspace:read-record", Path::new(checkout_path));
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace:read-record".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id);
    runtime.snapshot.navigator.root_path = Some(checkout_path.to_owned());
    runtime.reset_terminal_projection(None);
    let tab = |index: u8| {
        serde_json::json!({
            "workspace_id": "herdr-workspace",
            "tab_id": format!("herdr-workspace:t{index}"),
            "label": index.to_string()
        })
    };
    let layout = |index: u8| {
        let pane_id = format!("plain:p{index}");
        serde_json::json!({
            "workspace_id": "herdr-workspace",
            "tab_id": format!("herdr-workspace:t{index}"),
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": pane_id,
            "panes": [{"pane_id": pane_id,
                       "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        })
    };
    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [idle("plain:p1"), done("plain:p2")],
        "focused_pane_id": "plain:p1",
        "panes": [
            {"pane_id": "plain:p1", "cwd": checkout_path},
            {"pane_id": "plain:p2", "cwd": checkout_path}
        ],
        "tabs": [tab(1), tab(2)],
        "layouts": [layout(1), layout(2)]
    }))
    .expect("session payload");
    runtime.ingest_session(Ok(payload));
    assert!(runtime.dispatch_json(&operator_focus_event("plain:p1")));

    let snapshot = runtime.snapshot();
    let panes = snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .flat_map(|checkout| checkout.tabs.iter())
        .flat_map(|tab| tab.panes.iter())
        .map(|pane| {
            (
                pane.id.as_str(),
                pane.status_code,
                pane.requires_close_confirmation,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        panes,
        vec![
            ("plain:p1", AgentStatusCode::Idle, false),
            ("plain:p2", AgentStatusCode::Stopped, false)
        ],
        "the read pane and the unread completion both close without a prompt"
    );

    for agent in &snapshot.navigator.agents {
        let pane = panes
            .iter()
            .find(|(id, _, _)| *id == agent.pane_id)
            .expect("every agent pane is in the tree");
        assert_eq!(
            (pane.1, pane.2),
            (agent.status_code, agent.requires_close_confirmation),
            "pane {} disagrees with its agent row",
            agent.pane_id
        );
    }
    let _ = std::fs::remove_file(&state_path);
}

fn remote_purpose_runtime(version: &str) -> Runtime {
    let mut runtime = runtime();
    let workspace_id = "remote:mini:workspace:w1";
    let checkout_id = "remote:mini:checkout:w1";
    let mut remote_workspace = workspace(
        workspace_id,
        "Remote fixture",
        "/fixture/remote",
        vec![checkout(workspace_id, checkout_id, "/fixture/remote", None)],
    );
    remote_workspace.remote_target_id = Some("mini".to_owned());
    remote_workspace.device_id = "mini".to_owned();
    remote_workspace.session_workspace_ids = vec!["w1".to_owned()];
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some(version.to_owned()),
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![remote_workspace],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: Some(workspace_id.to_owned()),
            focused_checkout_id: Some(checkout_id.to_owned()),
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
    runtime
}

fn begin_remote_purpose_operation(runtime: &mut Runtime) -> u64 {
    let id = runtime
        .begin_task_operation(
            "checkout_purpose",
            Some("/fixture/remote".to_owned()),
            None,
            None,
            None,
        )
        .expect("purpose operation");
    runtime.purpose_operation_target = Some(PurposeOperationTarget {
        id,
        checkout_id: "remote:mini:checkout:w1".to_owned(),
        remote_target_id: Some("mini".to_owned()),
    });
    id
}

/// B20. A remote save updates the exact target-scoped checkout instead of
/// looking only in the local navigator. Clearing and refusal keep the same
/// caller-visible task-operation contract as a local save.
#[test]
fn remote_purpose_results_update_clear_and_preserve_the_exact_checkout() {
    let mut runtime = remote_purpose_runtime("0.9.1");

    let id = begin_remote_purpose_operation(&mut runtime);
    assert!(runtime.ingest_purpose_operation_result(
        id,
        Ok(live::PurposeTaskOutcome::Saved {
            purpose: "Remote updated".to_owned(),
            token_written: true,
        })
    ));
    let purpose = runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .unwrap()
        .workspaces[0]
        .checkouts[0]
        .purpose
        .as_ref()
        .expect("saved purpose");
    assert_eq!(purpose.text, "Remote updated");
    assert_eq!(purpose.origin, crate::model::CheckoutPurposeOrigin::Token);

    let id = begin_remote_purpose_operation(&mut runtime);
    assert!(runtime.ingest_purpose_operation_result(
        id,
        Ok(live::PurposeTaskOutcome::Saved {
            purpose: String::new(),
            token_written: true,
        })
    ));
    assert!(
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .workspaces[0]
            .checkouts[0]
            .purpose
            .is_none()
    );

    runtime.snapshot.status.remote[0]
        .session
        .as_mut()
        .unwrap()
        .workspaces[0]
        .checkouts[0]
        .purpose = Some(crate::model::CheckoutPurposeSnapshot {
        text: "Keep me".to_owned(),
        origin: crate::model::CheckoutPurposeOrigin::Token,
    });
    let id = begin_remote_purpose_operation(&mut runtime);
    assert!(runtime.ingest_purpose_operation_result(id, Err("injected refusal".to_owned())));
    assert_eq!(
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .workspaces[0]
            .checkouts[0]
            .purpose
            .as_ref()
            .unwrap()
            .text,
        "Keep me"
    );
    let operation = runtime.snapshot.task_operation.as_ref().unwrap();
    assert_eq!(operation.phase, "failed");
    assert_eq!(
        operation.failure.as_deref(),
        Some("checkout_purpose.token_failed")
    );
}

/// B20. The remote row resolver reaches the projected checkout, then refuses
/// an unsupported server through the sheet's task operation instead of a
/// detached global error.
#[test]
fn remote_purpose_resolver_reports_an_unsupported_server_in_the_sheet() {
    let mut runtime = remote_purpose_runtime("0.9.0");

    assert!(runtime.set_checkout_purpose(SetCheckoutPurposePayload {
        checkout_id: "remote:mini:checkout:w1".to_owned(),
        text: "Remote purpose".to_owned(),
    }));

    let operation = runtime.snapshot.task_operation.as_ref().expect("operation");
    assert_eq!(operation.kind, "checkout_purpose");
    assert_eq!(operation.phase, "failed");
    assert_eq!(
        operation.failure.as_deref(),
        Some("checkout_purpose.remote_unsupported")
    );
    assert!(
        runtime
            .snapshot
            .status
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.kind == "checkout_purpose.remote_unsupported" })
    );
}

#[test]
fn window_readers_rest_while_no_window_draws_the_snapshot() {
    let mut runtime = runtime();
    let event = |attached: bool| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2, "kind": "ui_attached", "payload": {"attached": attached}
        }))
        .expect("the event encodes")
    };
    assert!(
        runtime.ui_attached(),
        "a core no daemon told otherwise is drawn, so every reader runs"
    );
    assert!(
        !runtime.dispatch_json(&event(false)),
        "the last window leaving is known and changes nothing on screen"
    );
    assert!(!runtime.ui_attached());
    assert!(!runtime.dispatch_json(&event(true)));
    assert!(runtime.ui_attached());
}

/// A Memory analysis builds its router once; one already running must not
/// keep sending session text to an agent the operator has since switched off
/// or taken out of the list, and an unrelated switch must not end it.
#[test]
fn a_running_memory_analysis_stops_when_hide_ai_turns_off_or_its_agents_change() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_background_ai(ready_read(&["claude", "codex", "grok"]));
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "claude"}))));
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"fallback_add": "codex"}))));

    let running = |runtime: &mut Runtime| runtime.pretend_memory_pass();
    let stopped_by = |payload: serde_json::Value, runtime: &mut Runtime| {
        let cancel = running(runtime);
        assert!(runtime.dispatch_json(&ai_event(payload)));
        cancel.is_cancelled()
    };

    assert!(
        !stopped_by(serde_json::json!({"agent_summary": false}), &mut runtime),
        "a switch that changes nobody who may be asked leaves the run alone"
    );
    assert!(
        stopped_by(serde_json::json!({"fallback_add": "grok"}), &mut runtime),
        "an added fallback agent is not the list the run was built for"
    );
    assert!(
        stopped_by(serde_json::json!({"fallback_remove": "grok"}), &mut runtime),
        "a removed fallback agent stops receiving session text"
    );
    assert!(
        stopped_by(
            serde_json::json!({"provider": "codex", "model": "m-2"}),
            &mut runtime
        ),
        "a different Runs on agent or model ends the run"
    );
    assert!(
        stopped_by(serde_json::json!({"enabled": false}), &mut runtime),
        "Use Hide AI off stops a model call already under way"
    );
}
