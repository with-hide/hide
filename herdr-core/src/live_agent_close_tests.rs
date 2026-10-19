use super::*;
use crate::fake_herdr::FakeHerdr;

/// B5: authorization never follows a failed pane into another tab or covers
/// a new/now-active peer in the tab the user selected.
#[test]
fn failed_tab_close_rechecks_membership_and_live_peers() {
    for (
        failed_tab,
        peer_status,
        extra,
        approved_terminal,
        approved_session,
        current_session,
        allowed,
    ) in [
        ("w1:t1", "idle", false, None, None, None, true),
        ("w1:t2", "idle", false, None, None, None, false),
        ("w1:t1", "working", false, None, None, None, false),
        ("w1:t1", "unknown", false, None, None, None, false),
        ("w1:t1", "blocked", false, None, None, None, false),
        ("w1:t1", "idle", true, None, None, None, false),
        ("w1:t1", "working", false, Some("live"), None, None, true),
        ("w1:t1", "blocked", false, Some("live"), None, None, true),
        ("w1:t1", "unknown", false, Some("live"), None, None, false),
        ("w1:t1", "working", false, Some("old"), None, None, false),
        (
            "w1:t1",
            "working",
            false,
            Some("live"),
            Some("original"),
            Some("original"),
            true,
        ),
        (
            "w1:t1",
            "working",
            false,
            Some("live"),
            Some("original"),
            Some("new"),
            false,
        ),
    ] {
        let herdr = FakeHerdr::start("failed-tab-scope", move |method, _| match method {
            "session.snapshot" => {
                let mut value = snapshot(false);
                let mut panes = vec![
                    json!({"pane_id":"w1:p1","terminal_id":"failed","workspace_id":"w1","tab_id":failed_tab,"focused":false,"agent_status":"unknown","revision":1,"restore_error":"ENOENT"}),
                    json!({"pane_id":"w1:p2","terminal_id":"live","workspace_id":"w1","tab_id":"w1:t1","focused":false,"agent_status":peer_status,"revision":1}),
                ];
                if extra {
                    panes.push(json!({"pane_id":"w1:p3","terminal_id":"new","workspace_id":"w1","tab_id":"w1:t1","focused":false,"agent_status":"idle","revision":1}));
                }
                if let Some(session) = current_session {
                    let mut peer = panes[1].clone();
                    peer["agent"] = json!("claude");
                    peer["agent_session"] = json!({"source":"herdr:claude", "agent":"claude", "kind":"id", "value":session});
                    value["snapshot"]["agents"] = json!([peer]);
                }
                value["snapshot"]["panes"] = json!(panes);
                value
            }
            "tab.close" => json!({"type":"ok"}),
            other => panic!("unexpected {other}"),
        });
        let result = run_close_effect(
            &herdr.connector(),
            &CloseEffectRequest {
                safety: Some(CloseSafety {
                    approved_peers: approved_terminal
                        .map(|terminal| {
                            vec![ClosePeer {
                                pane_id: "w1:p2".into(),
                                terminal_id: terminal.into(),
                                session: approved_session.map(|value| {
                                    crate::sidebar::SessionAgentSessionPayload {
                                        kind: "id".into(),
                                        value: value.into(),
                                    }
                                }),
                            }]
                        })
                        .unwrap_or_default(),
                    pane_ids: vec!["w1:p1".into(), "w1:p2".into()],
                    deadline: Instant::now() + Duration::from_secs(5),
                }),
                failed_terminals: vec![("w1:p1".into(), "failed".into())],
                retain_for_reopen: false,
                allow_replacement_create: false,
                replacement: None,
                key: "scope".into(),
                connection_generation: 1,
                target: CloseCaptureTarget::Tab {
                    tab_id: "w1:t1".into(),
                },
            },
        );
        assert_eq!(result.is_ok(), allowed);
        assert_eq!(herdr.methods().contains(&"tab.close".into()), allowed);
    }
}

#[test]
fn failed_pane_close_preserves_a_new_working_descendant_in_another_tab() {
    let herdr = FakeHerdr::start("failed-descendant", |method, params| match method {
        "session.snapshot" => {
            let mut value = snapshot(false);
            let parent = json!({"pane_id":"w1:p1","terminal_id":"original","workspace_id":"w1","tab_id":"w1:t1","focused":false,"agent":"claude","agent_status":"unknown","revision":1,"restore_error":"ENOENT",
                "agent_session":{"source":"herdr:claude","agent":"claude","kind":"id","value":"parent"}});
            let child = json!({"pane_id":"w1:p2","terminal_id":"child","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent":"claude","agent_status":"working","revision":1,
                "agent_session":{"source":"herdr:claude","agent":"claude","kind":"id","value":"child"},
                "tokens":{"parent_pane":"w1:p1","child_session":wire::session_digest("child"),"parent_session":wire::session_digest("parent")}});
            value["snapshot"]["panes"] = json!([parent.clone(), child.clone()]);
            value["snapshot"]["agents"] = json!([parent, child]);
            value
        }
        "pane.close" => {
            assert_eq!(
                params["pane_id"], "w1:p1",
                "Close only this must leave the child running"
            );
            json!({"type":"ok"})
        }
        other => panic!("unexpected {other}"),
    });
    let result = run_close_effect(
        &herdr.connector(),
        &CloseEffectRequest {
            safety: Some(CloseSafety {
                approved_peers: Vec::new(),
                pane_ids: vec!["w1:p1".into()],
                deadline: Instant::now() + Duration::from_secs(5),
            }),
            failed_terminals: vec![("w1:p1".into(), "original".into())],
            retain_for_reopen: false,
            allow_replacement_create: false,
            replacement: None,
            key: "descendant".into(),
            connection_generation: 1,
            target: CloseCaptureTarget::Pane {
                pane_id: "w1:p1".into(),
            },
        },
    );
    assert!(result.is_ok());
    assert_eq!(herdr.methods(), ["session.snapshot", "pane.close"]);
}

/// B6: time spent obtaining permission cannot become extra mutation time.
#[test]
fn an_expired_failed_close_sends_no_snapshot_or_mutation() {
    let herdr = FakeHerdr::start("expired-close", |method, _| {
        panic!("expired close sent {method}")
    });
    let result = run_close_effect(
        &herdr.connector(),
        &CloseEffectRequest {
            safety: Some(CloseSafety {
                approved_peers: Vec::new(),
                pane_ids: vec!["w1:p1".into()],
                deadline: Instant::now(),
            }),
            failed_terminals: vec![("w1:p1".into(), "failed".into())],
            retain_for_reopen: false,
            allow_replacement_create: false,
            replacement: None,
            key: "expired".into(),
            connection_generation: 1,
            target: CloseCaptureTarget::Pane {
                pane_id: "w1:p1".into(),
            },
        },
    );
    assert_eq!(result.unwrap_err().code(), Some("close_deadline_expired"));
    assert!(herdr.methods().is_empty());
}

/// The snapshot answers successfully, but its read used up the admission.
#[test]
fn failed_close_cannot_mutate_after_its_fresh_read_consumes_the_deadline() {
    let herdr = FakeHerdr::start("slow-fresh-close", |method, _| match method {
        "session.snapshot" => {
            let mut value = snapshot(false);
            value["snapshot"]["panes"] = json!([{
                "pane_id":"w1:p1","terminal_id":"original","workspace_id":"w1","tab_id":"w1:t1",
                "focused":false,"agent_status":"unknown","revision":1,"restore_error":"ENOENT"
            }]);
            value
        }
        other => panic!("the expired close sent {other}"),
    });
    let start = Instant::now();
    let reads = std::cell::Cell::new(0);
    let clock = || {
        let read = reads.get();
        reads.set(read + 1);
        start + Duration::from_secs(if read == 0 { 0 } else { 6 })
    };
    let result = run_close_effect_with_clock(
        &herdr.connector(),
        &CloseEffectRequest {
            safety: Some(CloseSafety {
                approved_peers: Vec::new(),
                pane_ids: vec!["w1:p1".into()],
                deadline: start + Duration::from_secs(5),
            }),
            failed_terminals: vec![("w1:p1".into(), "original".into())],
            retain_for_reopen: false,
            allow_replacement_create: false,
            replacement: None,
            key: "slow".into(),
            connection_generation: 1,
            target: CloseCaptureTarget::Pane {
                pane_id: "w1:p1".into(),
            },
        },
        &clock,
    );
    assert_eq!(result.unwrap_err().code(), Some("close_deadline_expired"));
    assert_eq!(herdr.methods(), ["session.snapshot"]);
}

/// Coexistence B4-B6: a fresh restoration failure can close, but the same
/// pane id with a replacement terminal or a recovered terminal cannot use it.
#[test]
fn failed_terminal_close_checks_incarnation_and_retains_refusals() {
    for (terminal, failed, refuses, expected) in [
        ("original", true, false, true),
        ("replacement", true, false, false),
        ("original", false, false, false),
        ("original", true, true, false),
    ] {
        let herdr = FakeHerdr::start_with_errors("restore-close", move |method, _| match method {
            "session.snapshot" => {
                let mut value = snapshot(false);
                value["snapshot"]["panes"] = json!([{
                    "pane_id":"w1:p1", "terminal_id":terminal, "workspace_id":"w1", "tab_id":"w1:t1",
                    "focused":false, "agent_status":"idle", "revision":1,
                    "restore_error": if failed { Some("ENOENT") } else { None },
                }]);
                Ok(value)
            }
            "pane.close" if refuses => Err(("fixture_refusal".into(), "close refused".into())),
            "pane.close" => Ok(json!({"type":"ok"})),
            other => panic!("unexpected request {other}"),
        });
        let result = run_close_effect(
            &herdr.connector(),
            &CloseEffectRequest {
                safety: None,
                failed_terminals: vec![("w1:p1".into(), "original".into())],
                retain_for_reopen: false,
                allow_replacement_create: false,
                replacement: None,
                key: "restore-close".into(),
                connection_generation: 1,
                target: CloseCaptureTarget::Pane {
                    pane_id: "w1:p1".into(),
                },
            },
        );
        assert_eq!(result.is_ok(), expected);
        assert_eq!(
            herdr.methods().contains(&"pane.close".to_owned()),
            terminal == "original" && failed
        );
    }
}

fn context() -> ClosedContext {
    ClosedContext {
        workspace_id: "w1".into(),
        workspace_label: "primary".into(),
        workspace_ids_before_close: vec!["w1".into(), "w2".into()],
        tab_ids_before_close: vec!["w1:t1".into()],
        pane_ids_before_close: vec!["w1:p1".into()],
        checkout_id: "checkout".into(),
        checkout_path: "/tmp".into(),
        tab_id: "w1:t1".into(),
        tab_label: "old".into(),
        tab_index: 0,
        agent_area: Some(("a1".into(), 0)),
        agent_group: None,
        replacement_shell: true,
        replacement_in_tab: false,
        tab_name: Default::default(),
    }
}
fn closed_tab() -> CloseCaptureTarget {
    CloseCaptureTarget::Tab {
        tab_id: "w1:t1".into(),
    }
}
fn tab(id: &str) -> Value {
    json!({"tab_id":id,"workspace_id":"w1","number":1,"label":"shell","focused":false,"pane_count":1,"agent_status":"idle"})
}
fn snapshot(created: bool) -> Value {
    let mut tabs = vec![tab("w1:t1")];
    if created {
        tabs.push(tab("w1:t2"));
    }
    json!({"type":"session_snapshot","snapshot":{
        "version":"fixture","protocol":HERDR_PROTOCOL_REVISION,
        "workspaces":[{"workspace_id":"w1","number":1,"label":"primary","focused":false,"pane_count":tabs.len(),"tab_count":tabs.len(),"active_tab_id":"w1:t1","agent_status":"idle"}],
        "tabs":tabs,"panes":[],"layouts":[],"agents":[]
    }})
}

#[test]
fn replacement_creation_failure_sends_no_close() {
    let herdr = FakeHerdr::start_with_errors("replacement-failure", |method, _| match method {
        "session.snapshot" => Ok(snapshot(false)),
        "tab.create" => Err(("fixture_create_refused".into(), "private diagnostic".into())),
        other => panic!("unexpected effect {other}"),
    });
    let error = prepare_close_replacement(
        &herdr.connector(),
        "intent",
        &context(),
        &closed_tab(),
        true,
    )
    .unwrap_err();
    assert!(error.message.contains("fixture_create_refused"));
    assert_eq!(herdr.methods(), ["session.snapshot", "tab.create"]);
}

#[test]
fn replacement_retry_after_close_refusal_adopts_its_shell_without_another_create() {
    let mut created = false;
    let herdr = FakeHerdr::start_with_errors(
        "replacement-retry",
        move |method, params| match method {
            "session.snapshot" => Ok(snapshot(created)),
            "tab.create" => {
                assert!(!created);
                assert_eq!(params["cwd"], "/tmp");
                assert_eq!(params["focus"], false);
                assert_eq!(params["env"][REOPEN_INTENT_ENV], "intent:close-replacement");
                created = true;
                Ok(json!({"type":"tab_created","tab":tab("w1:t2"),"root_pane":{
                    "pane_id":"w1:p2","terminal_id":"terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":0
                }}))
            }
            "layout.export" => Ok(json!({"type":"layout_export","layout":{
                "workspace_id":"w1","tab_id":"w1:t2","zoomed":false,"focused_pane_id":"w1:p2",
                "root":{"type":"pane","pane_id":"w1:p2","cwd":"/tmp","env":{(REOPEN_INTENT_ENV):"intent:close-replacement"}}
            }})),
            "tab.close" => Err(("fixture_close_refused".into(), "private diagnostic".into())),
            other => panic!("unexpected effect {other}"),
        },
    );
    let connector = herdr.connector();
    assert_eq!(
        prepare_close_replacement(&connector, "intent", &context(), &closed_tab(), true)
            .unwrap()
            .0,
        "w1:t2"
    );
    let close = CloseEffectRequest {
        safety: None,
        failed_terminals: Vec::new(),
        retain_for_reopen: true,
        allow_replacement_create: true,
        key: "intent".into(),
        connection_generation: 0,
        target: CloseCaptureTarget::Tab {
            tab_id: "w1:t1".into(),
        },
        replacement: Some(context()),
    };
    assert!(run_close_effect(&connector, &close).is_err());
    assert_eq!(
        prepare_close_replacement(&connector, "intent", &context(), &closed_tab(), false)
            .unwrap()
            .0,
        "w1:t2"
    );
    assert_eq!(
        herdr
            .methods()
            .iter()
            .filter(|method| method.as_str() == "tab.create")
            .count(),
        1
    );
    assert_eq!(
        herdr
            .methods()
            .iter()
            .filter(|method| method.as_str() == "tab.close")
            .count(),
        1
    );
}

/// Herdr names a new pane's folder from its process, which is the server's
/// own until the shell has started: the snapshot the close ingests must still
/// place the replacement at the checkout it was created for.
#[test]
fn replacement_shell_is_placed_at_its_checkout_while_herdr_still_names_the_servers_folder() {
    let mut created = false;
    let herdr = FakeHerdr::start("replacement-early-cwd", move |method, _| match method {
        "session.snapshot" => {
            let mut value = snapshot(created);
            if created {
                let body = &mut value["snapshot"];
                body["panes"] = json!([
                    {"workspace_id":"w1","tab_id":"w1:t1","pane_id":"w1:p1","terminal_id":"t1","focused":false,"revision":0,"agent_status":"idle","cwd":"/tmp"},
                    {"workspace_id":"w1","tab_id":"w1:t2","pane_id":"w1:p2","terminal_id":"t2","focused":false,"revision":0,"agent_status":"idle","cwd":"/srv/herdr-server"}
                ]);
                body["layouts"] = json!([{
                    "workspace_id":"w1","tab_id":"w1:t2","zoomed":false,
                    "area":{"x":0,"y":0,"width":120,"height":60},"focused_pane_id":"w1:p2",
                    "panes":[{"pane_id":"w1:p2","focused":false,"rect":{"x":0,"y":0,"width":120,"height":60}}],
                    "splits":[]
                }]);
            }
            value
        }
        "tab.create" => {
            created = true;
            json!({"type":"tab_created","tab":tab("w1:t2"),"root_pane":{
                "pane_id":"w1:p2","terminal_id":"t2","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":0
            }})
        }
        "layout.export" => json!({"type":"layout_export","layout":{
            "workspace_id":"w1","tab_id":"w1:t1","zoomed":false,"focused_pane_id":"w1:p1",
            "root":{"type":"pane","pane_id":"w1:p1","cwd":"/tmp","env":{}}
        }}),
        other => panic!("unexpected {other}"),
    });
    let (tab_id, payload) = prepare_close_replacement(
        &herdr.connector(),
        "intent",
        &context(),
        &closed_tab(),
        true,
    )
    .unwrap();
    assert_eq!(tab_id, "w1:t2");
    let cwd = |pane: &str| {
        payload
            .panes
            .iter()
            .find(|candidate| candidate.pane_id == pane)
            .and_then(|candidate| candidate.cwd.as_deref())
    };
    assert_eq!(cwd("w1:p2"), Some("/tmp"));
    assert_eq!(cwd("w1:p1"), Some("/tmp"));
}

#[test]
fn reuse_only_retry_cannot_create_when_its_shell_disappears_before_worker_reads() {
    let herdr = FakeHerdr::start("replacement-reuse-only", |method, _| match method {
        "session.snapshot" => snapshot(false),
        other => panic!("reuse-only retry must not send {other}"),
    });
    let error = prepare_close_replacement(
        &herdr.connector(),
        "intent",
        &context(),
        &closed_tab(),
        false,
    )
    .unwrap_err();
    assert!(error.message.contains("disappeared"));
    assert_eq!(herdr.methods(), ["session.snapshot"]);
}

#[test]
fn malformed_create_ack_is_recovered_by_its_marker_without_repeating_the_effect() {
    let mut marker = String::new();
    let herdr = FakeHerdr::start("create-recover", move |method, params| match method {
        "tab.create" => {
            assert!(marker.is_empty(), "creation must not be repeated");
            marker = params["env"][REOPEN_INTENT_ENV]
                .as_str()
                .unwrap()
                .to_owned();
            json!({"type":"tab_created","tab":tab(""),"root_pane":{
                "pane_id":"w1:p2","terminal_id":"terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":0
            }})
        }
        "session.snapshot" => snapshot(true),
        "layout.export" => {
            let id = params["tab_id"].as_str().unwrap();
            json!({"type":"layout_export","layout":{
                "workspace_id":"w1","tab_id":id,"zoomed":false,"focused_pane_id":"w1:p2",
                "root":{"type":"pane","pane_id":"w1:p2","cwd":"/tmp","env":{
                    (REOPEN_INTENT_ENV): if id == "w1:t2" {marker.clone()} else {"other-intent".into()}
                }}
            }})
        }
        other => panic!("unexpected {other}"),
    });
    let outcome = execute_local_control(
        &herdr.connector(),
        &RemoteControlAction::CreateTab {
            workspace_id: "w1".into(),
            cwd: "/tmp".into(),
            label: "New".into(),
            area_id: Some("a1".into()),
            group_id: None,
            admission_id: Some(91),
        },
    )
    .unwrap();
    assert!(
        matches!(outcome, RemoteControlOutcome::Acknowledged { created_tab_id: Some(id), .. } if id == "w1:t2")
    );
    assert_eq!(
        herdr.methods(),
        [
            "tab.create",
            "session.snapshot",
            "layout.export",
            "layout.export"
        ]
    );
}

fn named_tab_context() -> ClosedContext {
    ClosedContext {
        replacement_in_tab: true,
        ..context()
    }
}
fn closing_agent() -> CloseCaptureTarget {
    CloseCaptureTarget::Pane {
        pane_id: "w1:p1".into(),
    }
}
/// The named tab w1:t1 as Herdr reads it: the agent's pane, and once split
/// the shell beside it, whose folder Herdr still names as the server's.
fn named_tab_snapshot(shell: bool) -> Value {
    let pane = |id: &str, cwd: &str| {
        json!({"workspace_id":"w1","tab_id":"w1:t1","pane_id":id,"terminal_id":format!("{id}-terminal"),
            "focused":false,"revision":0,"agent_status":"idle","cwd":cwd})
    };
    let rect = |x: u32, width: u32| json!({"x":x,"y":0,"width":width,"height":60});
    let (panes, layout_panes) = if shell {
        (
            json!([pane("w1:p1", "/tmp"), pane("w1:p2", "/srv/herdr-server")]),
            json!([{"pane_id":"w1:p1","focused":true,"rect":rect(0, 60)},
                {"pane_id":"w1:p2","focused":false,"rect":rect(60, 60)}]),
        )
    } else {
        (
            json!([pane("w1:p1", "/tmp")]),
            json!([{"pane_id":"w1:p1","focused":true,"rect":rect(0, 120)}]),
        )
    };
    let mut value = snapshot(false);
    value["snapshot"]["panes"] = panes;
    value["snapshot"]["layouts"] = json!([{"workspace_id":"w1","tab_id":"w1:t1","zoomed":false,
        "area":rect(0, 120),"focused_pane_id":"w1:p1","panes":layout_panes,"splits":[]}]);
    value
}
fn named_tab_export(shell: bool) -> Value {
    let agent = json!({"type":"pane","pane_id":"w1:p1","cwd":"/tmp","env":{}});
    let root = if shell {
        json!({"type":"split","direction":"right","ratio":0.5,"first":agent,
            "second":{"type":"pane","pane_id":"w1:p2","cwd":"/tmp","env":{(REOPEN_INTENT_ENV):"intent:close-replacement"}}})
    } else {
        agent
    };
    json!({"type":"layout_export","layout":{"workspace_id":"w1","tab_id":"w1:t1","zoomed":false,
        "focused_pane_id":"w1:p1","root":root}})
}

/// A named tab's last agent leaves a shell in the same tab: split beside
/// the closing pane at the checkout, unfocused, with the close's marker; a
/// refused close keeps it, and the retry adopts it instead of making another.
#[test]
fn a_named_tabs_shell_is_split_into_its_tab_and_a_retry_adopts_it() {
    let mut split = false;
    let herdr = FakeHerdr::start_with_errors(
        "named-tab-shell",
        move |method, params| match method {
            "layout.export" => Ok(named_tab_export(split)),
            "pane.split" => {
                assert!(!split, "the shell is made once");
                assert_eq!(params["target_pane_id"], "w1:p1");
                assert_eq!(params["cwd"], "/tmp");
                assert_eq!(params["focus"], false);
                assert_eq!(params["env"][REOPEN_INTENT_ENV], "intent:close-replacement");
                split = true;
                Ok(
                    json!({"type":"pane_info","pane":{"pane_id":"w1:p2","terminal_id":"w1:p2-terminal",
                "workspace_id":"w1","tab_id":"w1:t1","focused":false,"agent_status":"idle","revision":1}}),
                )
            }
            "session.snapshot" => Ok(named_tab_snapshot(split)),
            "pane.close" => Err(("fixture_close_refused".into(), "private diagnostic".into())),
            other => panic!("unexpected effect {other}"),
        },
    );
    let connector = herdr.connector();
    let (shell, payload) = prepare_close_replacement(
        &connector,
        "intent",
        &named_tab_context(),
        &closing_agent(),
        true,
    )
    .unwrap();
    assert_eq!(shell, "w1:p2");
    let cwd = |pane: &str| {
        payload
            .panes
            .iter()
            .find(|candidate| candidate.pane_id == pane)
            .and_then(|candidate| candidate.cwd.clone())
    };
    assert_eq!(
        cwd("w1:p2").as_deref(),
        Some("/tmp"),
        "placed at its checkout"
    );
    let close = CloseEffectRequest {
        safety: None,
        failed_terminals: Vec::new(),
        retain_for_reopen: true,
        allow_replacement_create: true,
        key: "intent".into(),
        connection_generation: 0,
        target: closing_agent(),
        replacement: Some(named_tab_context()),
    };
    assert!(run_close_effect(&connector, &close).is_err());
    let (adopted, _) = prepare_close_replacement(
        &connector,
        "intent",
        &named_tab_context(),
        &closing_agent(),
        false,
    )
    .unwrap();
    assert_eq!(adopted, "w1:p2");
    let count = |name: &str| {
        herdr
            .methods()
            .iter()
            .filter(|method| *method == name)
            .count()
    };
    assert_eq!(
        (
            count("pane.split"),
            count("pane.close"),
            count("tab.create")
        ),
        (1, 1, 0)
    );
}

#[test]
fn a_named_tab_whose_shell_cannot_be_made_closes_nothing() {
    let herdr = FakeHerdr::start_with_errors("named-tab-refused", |method, _| match method {
        "layout.export" => Ok(named_tab_export(false)),
        "pane.split" => Err(("fixture_split_refused".into(), "private diagnostic".into())),
        other => panic!("unexpected effect {other}"),
    });
    let error = prepare_close_replacement(
        &herdr.connector(),
        "intent",
        &named_tab_context(),
        &closing_agent(),
        true,
    )
    .unwrap_err();
    assert!(error.message.contains("fixture_split_refused"));
    assert_eq!(error.shell_in_tab, None, "Herdr made no shell");
    assert_eq!(
        herdr.methods(),
        ["layout.export", "pane.split", "layout.export"]
    );
}

/// A split Herdr made whose answer was lost still leaves its shell in the
/// tab; the failure names it by its marker so the retry closes beside it.
#[test]
fn a_named_tabs_shell_whose_split_answer_is_lost_is_named_by_its_marker() {
    let mut split = false;
    let herdr = FakeHerdr::start_with_errors("named-tab-lost-ack", move |method, _| match method {
        "layout.export" => Ok(named_tab_export(split)),
        "pane.split" => {
            assert!(!split, "the shell is made once");
            split = true;
            Err(("fixture_answer_lost".into(), "private diagnostic".into()))
        }
        other => panic!("unexpected effect {other}"),
    });
    let error = prepare_close_replacement(
        &herdr.connector(),
        "intent",
        &named_tab_context(),
        &closing_agent(),
        true,
    )
    .unwrap_err();
    assert_eq!(error.shell_in_tab.as_deref(), Some("w1:p2"));
    assert_eq!(
        herdr.methods(),
        ["layout.export", "pane.split", "layout.export"]
    );
}
