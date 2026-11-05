//! Controller regressions at the typed socket and node boundaries.
use super::*;
use crate::engine::fake::{FakeEngine, Reply, closed_layout, refused, reply, session, unanswered};
use crate::live::ControlFailure;
use crate::worktree_control::{CheckoutStart, CheckoutTabRequest, WorktreeTaskOutcome};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const MARKER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn identity(pane: &str) -> PaneIdentity {
    let n = pane.rsplit('p').next().unwrap().parse::<i32>().unwrap();
    PaneIdentity {
        terminal_id: format!("terminal-{n}"),
        shell_pid: 100 + n,
        shell_started: 500 + n as u64,
    }
}

fn pane(id: &str) -> Value {
    json!({"type":"pane", "pane_id":id, "cwd":"/fixture"})
}

fn split(first: Value, second: Value) -> Value {
    json!({"type":"split", "direction":"right", "ratio":0.5, "first":first, "second":second})
}

fn layout(tab: &str, root: Value) -> Value {
    json!({"workspace_id":"w1", "tab_id":tab, "zoomed":false, "focused_pane_id":"w1:p1", "root":root})
}

fn pane_ids(root: &Value) -> Vec<String> {
    if root["type"] == "pane" {
        vec![root["pane_id"].as_str().unwrap().into()]
    } else {
        [pane_ids(&root["first"]), pane_ids(&root["second"])].concat()
    }
}

struct State {
    layouts: BTreeMap<String, Value>,
    identities: BTreeMap<String, PaneIdentity>,
    saved: Vec<Vec<u8>>,
    calls: Vec<String>,
    inspections: Vec<String>,
    wrong_export_owner: bool,
    refuse_binding_save: bool,
    respawn_at_inspection: Option<usize>,
    refuse_unzoom: bool,
    /// The engine names no pane's shell.
    no_identity: bool,
    /// The call whose answer is lost after its effect, and the flag that
    /// stops the controller once it is.
    lost: Option<&'static str>,
    stopped: Arc<AtomicBool>,
}

impl State {
    fn new() -> Self {
        Self {
            layouts: BTreeMap::from([(
                "w1:t1".into(),
                layout("w1:t1", split(pane("w1:p1"), pane("w1:p2"))),
            )]),
            identities: ["w1:p1", "w1:p2", "w1:p3"]
                .into_iter()
                .map(|id| (id.into(), identity(id)))
                .collect(),
            saved: vec![],
            calls: vec![],
            inspections: vec![],
            wrong_export_owner: false,
            refuse_binding_save: false,
            respawn_at_inspection: None,
            refuse_unzoom: false,
            no_identity: false,
            lost: None,
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    fn last_saved(&self) -> Effect {
        serde_json::from_slice(self.saved.last().expect("effect saved before mutation")).unwrap()
    }

    fn pane_info(&self, id: &str, tab: &str) -> Value {
        json!({"pane_id":id,"terminal_id":self.identities[id].terminal_id,"workspace_id":"w1",
            "tab_id":tab,"cwd":"/fixture","agent_status":"unknown"})
    }

    /// The engine's answer to `method`; a lost answer is the effect done
    /// and no answer read.
    fn answer(&mut self, method: &'static str, params: &Value) -> Reply {
        if method == "set_zoom" && self.refuse_unzoom {
            self.calls.push(method.into());
            return refused("invalid_operation", "unzoom refused");
        }
        let answer = self.effect(method, params);
        if self.lost == Some(method) {
            self.stopped.store(true, Ordering::Release);
            return unanswered("answer lost after effect");
        }
        answer
    }

    fn effect(&mut self, method: &str, params: &Value) -> Reply {
        if method != "inspect_pane" {
            self.calls.push(method.into());
        }
        match method {
            "launch_checkout_tab" => {
                let saved = self.last_saved();
                assert_eq!(saved.workspace_id, "w1");
                assert!(saved.binding.is_none());
                assert_eq!(params["workspace"], "w1");
                assert!(params["seed_tab"].is_null());
                assert_eq!(params["claim"], crate::start_claim::label(MARKER));
                let mut root = json!({"type":"pane", "pane_id":"w1:p3",
                    "label":params["claim"], "cwd":params["cwd"]});
                if !params["command"].is_null() {
                    root["command"] = params["command"].clone();
                }
                let mut created = layout("w1:t2", root);
                created["focused_pane_id"] = json!("w1:p3");
                self.layouts.insert("w1:t2".into(), created);
                reply(("w1:t2".to_owned(), "w1:p3".to_owned()))
            }
            "export_layout" => {
                let mut exported = self.layouts[params["tab"].as_str().unwrap()].clone();
                if self.wrong_export_owner {
                    exported["workspace_id"] = json!("w2");
                }
                reply(closed_layout(exported))
            }
            "session" => {
                let tabs: Vec<_> = self
                    .layouts
                    .keys()
                    .enumerate()
                    .map(|(n, id)| {
                        json!({
                            "tab_id":id,"workspace_id":"w1","number":n+1,"label":"Terminal"
                        })
                    })
                    .collect();
                let panes: Vec<_> = self
                    .layouts
                    .iter()
                    .flat_map(|(tab, l)| {
                        pane_ids(&l["root"])
                            .into_iter()
                            .map(|id| self.pane_info(&id, tab))
                            .collect::<Vec<_>>()
                    })
                    .collect();
                reply(session(json!({
                    "workspaces":[{"workspace_id":"w1","label":"fixture","active_tab_id":"w1:t1"}],
                    "tabs":tabs,"panes":panes,
                    "focused_workspace_id":"w1","focused_pane_id":"w1:p1"
                })))
            }
            "set_zoom" => {
                assert_eq!(params, &json!({"pane":"w1:p1","zoom":false}));
                assert!(self.last_saved().binding.is_some());
                self.layouts.get_mut("w1:t1").unwrap()["zoomed"] = json!(false);
                reply(())
            }
            "move_pane_beside" => {
                let saved = self.last_saved();
                assert_eq!(saved.binding.as_ref().unwrap().identity, identity("w1:p3"));
                assert_eq!(saved.target.as_ref().unwrap().identity, identity("w1:p1"));
                assert_eq!(
                    params,
                    &json!({"pane":"w1:p3","target_tab":"w1:t1","target_pane":"w1:p1","down":false})
                );
                assert_eq!(self.layouts["w1:t1"]["zoomed"], false);
                let source = self.layouts.remove("w1:t2").unwrap();
                let target = self.layouts.get_mut("w1:t1").unwrap();
                target["root"]["first"] =
                    split(target["root"]["first"].clone(), source["root"].clone());
                reply(crate::engine::PaneJoinResult {
                    changed: true,
                    reason: None,
                    workspace_id: "w1".into(),
                    tab_id: "w1:t1".into(),
                    pane_id: "w1:p3".into(),
                    terminal_id: self.identities["w1:p3"].terminal_id.clone(),
                    previous_workspace_id: "w1".into(),
                    previous_tab_id: "w1:t2".into(),
                    previous_pane_id: "w1:p3".into(),
                    closed_tab_id: Some("w1:t2".into()),
                    closed_workspace_id: None,
                    created_tab_id: None,
                })
            }
            "close_tab" => {
                self.layouts.remove(params["tab"].as_str().unwrap());
                reply(())
            }
            "inspect_pane" => self.inspect(params["pane"].as_str().unwrap()),
            other => panic!("unexpected start request {other}"),
        }
    }

    /// The pane's shell as the engine names it.
    fn inspect(&mut self, pane_id: &str) -> Reply {
        self.inspections.push(pane_id.to_owned());
        if self.no_identity {
            return refused("unsupported", "the engine names no pane's shell");
        }
        if pane_id == "w1:p3"
            && self.respawn_at_inspection
                == Some(self.inspections.iter().filter(|id| *id == pane_id).count())
        {
            self.respawn_at_inspection = None;
            let identity = self.identities.get_mut(pane_id).unwrap();
            identity.shell_pid += 1000;
            identity.shell_started += 1000;
            // A same-engine respawn keeps the pane, its terminal id and its
            // claim, but not the command it was launched with.
            self.layouts.get_mut("w1:t2").unwrap()["root"]
                .as_object_mut()
                .unwrap()
                .remove("command");
        }
        reply(self.identities[pane_id].clone())
    }
}

struct Fixture {
    server: Arc<FakeEngine<'static>>,
    state: Arc<Mutex<State>>,
    stopped: Arc<AtomicBool>,
}
impl Fixture {
    fn new() -> Self {
        let state = Arc::new(Mutex::new(State::new()));
        let answer = state.clone();
        let stopped = state.lock().unwrap().stopped.clone();
        Self {
            server: FakeEngine::start(move |method, params| {
                answer.lock().unwrap().answer(method, params)
            }),
            state,
            stopped,
        }
    }
    fn request(&self, add_pane: bool) -> CheckoutTabRequest {
        CheckoutTabRequest {
            start: Some(CheckoutStart {
                marker: MARKER.into(),
                recovery_only: false,
                tab_id: add_pane.then(|| "w1:t1".into()),
                settled_tab_id: None,
                agent_already_attempted: false,
                agent_phase: None,
                confirm_only: false,
                effect: None,
            }),
            id: 1,
            agent_kind: None,
            checkout_path: "/fixture".into(),
            label: "Terminal".into(),
            host: crate::checkout_owner::TabHost::Workspace("w1".into()),
            focus: false,
            resume_scope: None,
            resume_reference: None,
            cwd: "/fixture".into(),
        }
    }
    fn run(
        &self,
        request: &CheckoutTabRequest,
        launch: Option<&ShellLaunch>,
        lost: Option<&'static str>,
    ) -> Result<WorktreeTaskOutcome, ControlFailure> {
        let engine = self.server.engine();
        self.state.lock().unwrap().lost = lost;
        let save = |effect: &Effect| {
            let mut state = self.state.lock().unwrap();
            if state.refuse_binding_save && effect.binding.is_some() {
                return Err("binding was not saved".into());
            }
            state.saved.push(serde_json::to_vec(effect).unwrap());
            Ok(())
        };
        let check = || {
            if self.stopped.load(Ordering::Acquire) {
                Err("controller stopped".into())
            } else {
                Ok(())
            }
        };
        let result = run(
            &Control {
                engine: engine.as_ref(),
                save: &save,
                check: &check,
            },
            request,
            launch,
        );
        if lost.is_some() {
            assert!(
                self.stopped.load(Ordering::Acquire),
                "the requested successful answer was not intercepted"
            );
        }
        self.state.lock().unwrap().lost = None;
        result
    }
    fn recover(&self, request: &CheckoutTabRequest) -> CheckoutTabRequest {
        self.stopped.store(false, Ordering::Release);
        let mut next = request.clone();
        let start = next.start.as_mut().unwrap();
        start.recovery_only = true;
        start.effect = Some(self.state.lock().unwrap().last_saved());
        next
    }
    fn mutations(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|method| {
                matches!(
                    method.as_str(),
                    "launch_checkout_tab" | "move_pane_beside" | "set_zoom"
                )
            })
            .cloned()
            .collect()
    }
}

fn launch() -> ShellLaunch {
    ShellLaunch {
        command: vec!["/bin/zsh".into(), "-l".into()],
        readiness: hide_node_link::protocol::ShellReadiness::Integrated,
        env: BTreeMap::from([("SHELL_INTEGRATION".into(), "1".into())]),
    }
}

#[test]
fn respawn_after_discovery_cannot_become_the_first_binding_with_or_without_an_ack() {
    for lost_ack in [false, true] {
        for inspection in [1, 2] {
            let fixture = Fixture::new();
            fixture.state.lock().unwrap().respawn_at_inspection = Some(inspection);
            let request = fixture.request(false);
            let result = if lost_ack {
                assert!(
                    fixture
                        .run(&request, Some(&launch()), Some("launch_checkout_tab"))
                        .is_err()
                );
                fixture.run(&fixture.recover(&request), None, None)
            } else {
                fixture.run(&request, Some(&launch()), None)
            };
            assert!(result.unwrap_err().is_ambiguous());
            assert!(fixture.state.lock().unwrap().last_saved().binding.is_none());
            assert_eq!(fixture.mutations(), ["launch_checkout_tab"]);
        }
    }
}

/// A node that can name no pane's shell (the core's own engine until pane
/// identity lands) fails the start for certain, and the tab the start made
/// goes with it in the same event.
#[test]
fn a_start_no_node_can_confirm_fails_and_closes_the_tab_it_made() {
    let fixture = Fixture::new();
    fixture.state.lock().unwrap().no_identity = true;
    let error = fixture
        .run(&fixture.request(false), Some(&launch()), None)
        .unwrap_err();
    assert!(!error.is_ambiguous(), "{error:?}");
    assert_eq!(error.message(), "agent_start.terminal_unchecked");
    let state = fixture.state.lock().unwrap();
    assert_eq!(
        state
            .calls
            .iter()
            .filter(|method| matches!(method.as_str(), "launch_checkout_tab" | "close_tab"))
            .collect::<Vec<_>>(),
        ["launch_checkout_tab", "close_tab"]
    );
    assert!(
        !state.layouts.contains_key("w1:t2"),
        "the tab it made stays"
    );
    assert!(state.layouts.contains_key("w1:t1"));
}

#[test]
fn refused_unzoom_retains_the_created_terminal_for_confirmation() {
    let fixture = Fixture::new();
    {
        let mut state = fixture.state.lock().unwrap();
        state.layouts.get_mut("w1:t1").unwrap()["zoomed"] = json!(true);
        state.refuse_unzoom = true;
    }
    let request = fixture.request(true);
    let error = fixture.run(&request, Some(&launch()), None).unwrap_err();
    assert!(error.is_ambiguous(), "creation already succeeded");
    assert!(fixture.state.lock().unwrap().last_saved().binding.is_some());
    assert_eq!(fixture.mutations(), ["launch_checkout_tab", "set_zoom"]);
    fixture.state.lock().unwrap().refuse_unzoom = false;
    let recovered = fixture.run(&fixture.recover(&request), None, None).unwrap();
    assert_eq!(recovered.created_tab_id.as_deref(), Some("w1:t1"));
    assert_eq!(
        fixture.mutations(),
        [
            "launch_checkout_tab",
            "set_zoom",
            "set_zoom",
            "move_pane_beside"
        ]
    );
}

#[test]
fn lost_create_answer_recovers_the_claim_without_creating_again() {
    let fixture = Fixture::new();
    let request = fixture.request(false);
    assert!(
        fixture
            .run(&request, Some(&launch()), Some("launch_checkout_tab"))
            .unwrap_err()
            .is_ambiguous()
    );
    assert!(fixture.state.lock().unwrap().last_saved().binding.is_none());
    let recovered = fixture.run(&fixture.recover(&request), None, None).unwrap();
    assert_eq!(recovered.pane_id, "w1:p3");
    assert_eq!(recovered.created_tab_id.as_deref(), Some("w1:t2"));
    assert_eq!(fixture.mutations(), ["launch_checkout_tab"]);
    assert_eq!(
        fixture
            .state
            .lock()
            .unwrap()
            .last_saved()
            .binding
            .unwrap()
            .identity,
        identity("w1:p3")
    );
}

#[test]
fn lost_move_answer_recovers_saved_incarnation_without_moving_again() {
    let fixture = Fixture::new();
    let request = fixture.request(true);
    assert!(
        fixture
            .run(&request, Some(&launch()), Some("move_pane_beside"))
            .unwrap_err()
            .is_ambiguous()
    );
    let saved = fixture.state.lock().unwrap().last_saved();
    assert_eq!(saved.binding.as_ref().unwrap().tab_id, "w1:t2");
    assert_eq!(saved.binding.as_ref().unwrap().identity, identity("w1:p3"));
    let recovered = fixture.run(&fixture.recover(&request), None, None).unwrap();
    assert_eq!(recovered.created_tab_id.as_deref(), Some("w1:t1"));
    assert_eq!(recovered.pane_id, "w1:p3");
    assert_eq!(
        fixture.mutations(),
        ["launch_checkout_tab", "move_pane_beside"]
    );
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.layouts.len(), 1);
    assert_eq!(
        pane_ids(&state.layouts["w1:t1"]["root"]),
        ["w1:p1", "w1:p3", "w1:p2"]
    );
    assert_eq!(state.identities["w1:p1"], identity("w1:p1"));
    assert_eq!(state.identities["w1:p2"], identity("w1:p2"));
}

#[test]
fn unobserved_default_shell_stays_unknown_after_lost_create_answer() {
    let fixture = Fixture::new();
    let request = fixture.request(false);
    assert!(
        fixture
            .run(&request, Some(&launch()), Some("launch_checkout_tab"))
            .unwrap_err()
            .is_ambiguous()
    );
    // A receipt from before explicit normal-shell recipes were available
    // has no command proof. It must never adopt a matching retained label.
    let mut legacy = fixture.recover(&request);
    legacy
        .start
        .as_mut()
        .unwrap()
        .effect
        .as_mut()
        .unwrap()
        .expected_command = None;
    let error = fixture.run(&legacy, None, None).unwrap_err();
    assert!(error.is_ambiguous());
    assert!(error.message().contains("continuity"));
    assert_eq!(fixture.mutations(), ["launch_checkout_tab"]);
    assert!(fixture.state.lock().unwrap().last_saved().binding.is_none());
}

#[test]
fn a_missing_normal_shell_refuses_before_terminal_creation() {
    let fixture = Fixture::new();
    let error = fixture
        .run(&fixture.request(false), None, None)
        .unwrap_err();
    assert!(!error.is_ambiguous());
    assert!(error.message().contains("no terminal was started"));
    assert!(fixture.mutations().is_empty());
    assert!(fixture.state.lock().unwrap().saved.is_empty());
}

#[test]
fn an_explicit_plain_shell_recovers_a_lost_answer_without_another_creation() {
    let fixture = Fixture::new();
    let request = fixture.request(false);
    let plain = ShellLaunch {
        readiness: hide_node_link::protocol::ShellReadiness::Plain,
        ..launch()
    };
    assert!(
        fixture
            .run(&request, Some(&plain), Some("launch_checkout_tab"))
            .unwrap_err()
            .is_ambiguous()
    );
    let saved = fixture.state.lock().unwrap().last_saved();
    assert_eq!(saved.shell_env, plain.env);
    assert_eq!(saved.shell_readiness, plain.readiness);
    let recovered = fixture.run(&fixture.recover(&request), None, None).unwrap();
    assert!(matches!(
        recovered.shell,
        crate::agent_start::ShellStart::Plain
    ));
    assert_eq!(recovered.pane_id, "w1:p3");
    assert_eq!(fixture.mutations(), ["launch_checkout_tab"]);
}

#[test]
fn recovery_refuses_duplicate_claim_or_wrong_export_owner_without_mutation() {
    for duplicate in [true, false] {
        let fixture = Fixture::new();
        let request = fixture.request(false);
        assert!(
            fixture
                .run(&request, Some(&launch()), Some("launch_checkout_tab"))
                .is_err()
        );
        {
            let mut state = fixture.state.lock().unwrap();
            if duplicate {
                let mut copy = state.layouts["w1:t2"]["root"].clone();
                copy["pane_id"] = json!("w1:p1");
                state.layouts.get_mut("w1:t1").unwrap()["root"]["first"] = copy;
            } else {
                state.wrong_export_owner = true;
            }
        }
        let error = fixture
            .run(&fixture.recover(&request), None, None)
            .unwrap_err();
        assert!(error.is_ambiguous());
        assert!(error.message().contains(if duplicate {
            "More than one"
        } else {
            "owner changed"
        }));
        assert_eq!(fixture.mutations(), ["launch_checkout_tab"]);
        assert!(fixture.state.lock().unwrap().last_saved().binding.is_none());
    }
}

#[test]
fn saved_terminal_binding_refuses_each_changed_incarnation_field() {
    for field in ["terminal", "pid", "started"] {
        let fixture = Fixture::new();
        let request = fixture.request(true);
        assert!(
            fixture
                .run(&request, Some(&launch()), Some("move_pane_beside"))
                .is_err()
        );
        {
            let mut state = fixture.state.lock().unwrap();
            let changed = state.identities.get_mut("w1:p3").unwrap();
            match field {
                "terminal" => changed.terminal_id = "replacement".into(),
                "pid" => changed.shell_pid += 1,
                _ => changed.shell_started += 1,
            }
        }
        let error = fixture
            .run(&fixture.recover(&request), None, None)
            .unwrap_err();
        assert!(error.is_ambiguous());
        assert!(
            error
                .message()
                .contains("terminal or its placement changed")
        );
        assert_eq!(
            fixture.mutations(),
            ["launch_checkout_tab", "move_pane_beside"]
        );
    }
}

#[test]
fn adding_to_a_zoomed_target_unzooms_then_preserves_its_existing_panes() {
    let fixture = Fixture::new();
    fixture
        .state
        .lock()
        .unwrap()
        .layouts
        .get_mut("w1:t1")
        .unwrap()["zoomed"] = json!(true);
    let outcome = fixture
        .run(&fixture.request(true), Some(&launch()), None)
        .unwrap();
    assert_eq!(outcome.created_tab_id.as_deref(), Some("w1:t1"));
    assert_eq!(
        fixture.mutations(),
        ["launch_checkout_tab", "set_zoom", "move_pane_beside"]
    );
    let state = fixture.state.lock().unwrap();
    assert_eq!(
        pane_ids(&state.layouts["w1:t1"]["root"]),
        ["w1:p1", "w1:p3", "w1:p2"]
    );
    assert_eq!(state.layouts["w1:t1"]["focused_pane_id"], "w1:p1");
    assert_eq!(state.identities["w1:p1"], identity("w1:p1"));
    assert_eq!(state.identities["w1:p2"], identity("w1:p2"));
    assert_eq!(
        state.saved.len(),
        2,
        "owner/target saved before create, incarnation saved before join"
    );
}

#[test]
fn joining_waits_for_the_incarnation_save_to_succeed() {
    let fixture = Fixture::new();
    fixture.state.lock().unwrap().refuse_binding_save = true;
    let request = fixture.request(true);
    let error = fixture.run(&request, Some(&launch()), None).unwrap_err();
    assert!(error.is_ambiguous());
    assert_eq!(error.message(), "binding was not saved");
    assert_eq!(fixture.mutations(), ["launch_checkout_tab"]);
    let state = fixture.state.lock().unwrap();
    assert!(state.last_saved().binding.is_none());
    assert_eq!(
        pane_ids(&state.layouts["w1:t1"]["root"]),
        ["w1:p1", "w1:p2"]
    );
    assert_eq!(pane_ids(&state.layouts["w1:t2"]["root"]), ["w1:p3"]);
}

#[test]
fn a_restored_claim_without_the_original_command_is_not_bound() {
    let fixture = Fixture::new();
    let request = fixture.request(false);
    assert!(
        fixture
            .run(&request, Some(&launch()), Some("launch_checkout_tab"))
            .is_err()
    );
    fixture
        .state
        .lock()
        .unwrap()
        .layouts
        .get_mut("w1:t2")
        .unwrap()["root"]
        .as_object_mut()
        .unwrap()
        .remove("command");
    let error = fixture
        .run(&fixture.recover(&request), None, None)
        .unwrap_err();
    assert!(error.is_ambiguous());
    assert!(error.message().contains("continuity"));
    assert_eq!(fixture.mutations(), ["launch_checkout_tab"]);
    assert!(fixture.state.lock().unwrap().last_saved().binding.is_none());
}
