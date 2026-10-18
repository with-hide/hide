//! Controller regressions at the typed socket and node boundaries.
use super::*;
use crate::fake_herdr::FakeHerdr;
use hide_herdr_client::{ApiError, ApiStream, ConnectionShutdown, LocalSocketConnector};
use hide_node_link::{LinkAnswer, LinkError, NodeLink};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{Read, Write};
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
        }
    }

    fn last_saved(&self) -> Effect {
        serde_json::from_slice(self.saved.last().expect("effect saved before mutation")).unwrap()
    }

    fn pane_info(&self, id: &str, tab: &str) -> Value {
        json!({"pane_id":id,"terminal_id":self.identities[id].terminal_id,"workspace_id":"w1",
            "tab_id":tab,"focused":id == "w1:p1","cwd":"/fixture","agent_status":"unknown","revision":0})
    }

    fn geometry(&self, tab: &str) -> Value {
        let layout = &self.layouts[tab];
        let ids = pane_ids(&layout["root"]);
        json!({"workspace_id":"w1", "tab_id":tab, "zoomed":layout["zoomed"],
            "area":{"x":0,"y":0,"width":120,"height":40}, "focused_pane_id":layout["focused_pane_id"],
            "panes":ids.iter().map(|id| json!({"pane_id":id,"focused":id == "w1:p1",
                "rect":{"x":0,"y":0,"width":60,"height":40}})).collect::<Vec<_>>(), "splits":[]})
    }

    fn answer(&mut self, method: &str, params: &Value) -> Value {
        self.calls.push(method.into());
        match method {
            "layout.apply" => {
                let saved = self.last_saved();
                assert_eq!(saved.workspace_id, "w1");
                assert!(saved.binding.is_none());
                assert_eq!(params["focus"], false);
                assert_eq!(params["workspace_id"], "w1");
                assert!(params.get("tab_id").is_none());
                assert_eq!(params["root"]["label"], crate::start_claim::label(MARKER));
                let mut root = params["root"].clone();
                root["pane_id"] = json!("w1:p3");
                // Actual pinned layout.apply/export omits launch env.
                root.as_object_mut().unwrap().remove("env");
                let mut created = layout("w1:t2", root);
                created["focused_pane_id"] = json!("w1:p3");
                self.layouts.insert("w1:t2".into(), created.clone());
                json!({"type":"layout_apply", "layout":created})
            }
            "layout.export" => {
                let mut exported = self.layouts[params["tab_id"].as_str().unwrap()].clone();
                if self.wrong_export_owner {
                    exported["workspace_id"] = json!("w2");
                }
                json!({"type":"layout_export", "layout":exported})
            }
            "session.snapshot" => {
                let tabs: Vec<_> = self.layouts.iter().enumerate().map(|(n,(id,l))| json!({
                    "tab_id":id,"workspace_id":"w1","number":n+1,"label":"Terminal",
                    "focused":id == "w1:t1","pane_count":pane_ids(&l["root"]).len(),"agent_status":"unknown"
                })).collect();
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
                json!({"type":"session_snapshot","snapshot":{
                    "version":"fixture", "protocol":crate::herdr_contract::HERDR_PROTOCOL_REVISION,
                    "workspaces":[{"workspace_id":"w1","number":1,"label":"fixture","focused":true,
                        "pane_count":panes.len(),"tab_count":tabs.len(),"active_tab_id":"w1:t1","agent_status":"unknown"}],
                    "tabs":tabs,"panes":panes,"layouts":[],"agents":[],
                    "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p1"
                }})
            }
            "pane.zoom" => {
                assert_eq!(params, &json!({"pane_id":"w1:p1","mode":"off"}));
                assert!(self.last_saved().binding.is_some());
                self.layouts.get_mut("w1:t1").unwrap()["zoomed"] = json!(false);
                json!({"type":"pane_zoom","zoom":{"changed":true,"zoom_changed":true,"focus_changed":false,
                    "pane_id":"w1:p1","focused_pane_id":"w1:p1","zoomed":false,"layout":self.geometry("w1:t1")}})
            }
            "pane.move" => {
                let saved = self.last_saved();
                assert_eq!(saved.binding.as_ref().unwrap().identity, identity("w1:p3"));
                assert_eq!(saved.target.as_ref().unwrap().identity, identity("w1:p1"));
                assert_eq!(
                    params,
                    &json!({"pane_id":"w1:p3","focus":false,"destination":{
                    "type":"tab","tab_id":"w1:t1","target_pane_id":"w1:p1","split":"right","ratio":0.5}})
                );
                assert_eq!(self.layouts["w1:t1"]["zoomed"], false);
                let source = self.layouts.remove("w1:t2").unwrap();
                let target = self.layouts.get_mut("w1:t1").unwrap();
                target["root"]["first"] =
                    split(target["root"]["first"].clone(), source["root"].clone());
                json!({"type":"pane_move","move_result":{"changed":true,
                    "previous_pane_id":"w1:p3","previous_workspace_id":"w1","previous_tab_id":"w1:t2",
                    "pane":self.pane_info("w1:p3","w1:t1"), "target_layout":self.geometry("w1:t1"),
                    "closed_tab_id":"w1:t2","focused_pane_id":"w1:p1"}})
            }
            other => panic!("unexpected start request {other}"),
        }
    }
}

struct IdentityNode(Arc<Mutex<State>>);
impl NodeLink for IdentityNode {
    fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
        let Call::PaneInspect { pane_id } = call else {
            panic!("unexpected node call");
        };
        let mut state = self.0.lock().unwrap();
        state.inspections.push(pane_id.clone());
        if pane_id == "w1:p3"
            && state.respawn_at_inspection
                == Some(
                    state
                        .inspections
                        .iter()
                        .filter(|id| *id == &pane_id)
                        .count(),
                )
        {
            state.respawn_at_inspection = None;
            let identity = state.identities.get_mut(&pane_id).unwrap();
            identity.shell_pid += 1000;
            identity.shell_started += 1000;
            // A same-server respawn retains pane/terminal ids and manual
            // label, but clears launch argv in the pinned Herdr.
            state.layouts.get_mut("w1:t2").unwrap()["root"]
                .as_object_mut()
                .unwrap()
                .remove("command");
        }
        Ok(serde_json::to_value(&state.identities[&pane_id])
            .unwrap()
            .into())
    }
}

// Decorates the existing fake transport, without another socket/listener.
// The server's valid answer is consumed before the controller loses it.
struct LostAnswer {
    inner: LocalSocketConnector,
    method: &'static str,
    stopped: Arc<AtomicBool>,
}
impl ApiConnector for LostAnswer {
    fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
        Ok(Box::new(LostStream {
            inner: self.inner.connect()?,
            method: self.method,
            stopped: self.stopped.clone(),
            sent: vec![],
        }))
    }
}
struct LostStream {
    inner: Box<dyn ApiStream>,
    method: &'static str,
    stopped: Arc<AtomicBool>,
    sent: Vec<u8>,
}
impl LostStream {
    fn should_lose(&self) -> bool {
        let request: Value = serde_json::from_slice(&self.sent).unwrap();
        request["method"] == self.method
    }

    fn record_lost_success(&self, answer: &str) {
        let response: Value = serde_json::from_str(answer).unwrap();
        let request: Value = serde_json::from_slice(&self.sent).unwrap();
        assert_eq!(response["id"], request["id"]);
        assert!(response.get("result").is_some());
        assert!(response.get("error").is_none());
        self.stopped.store(true, Ordering::Release);
    }
}
impl Read for LostStream {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        // request_with_correlation_id uses BufReader's Read path, while
        // request_on_stream uses ApiStream::read_line_with_timeout below.
        if self.should_lose() {
            let answer = self
                .inner
                .read_line_with_timeout(Duration::from_secs(5))
                .expect("server must complete the effect and answer before loss");
            self.record_lost_success(&answer);
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "answer lost after effect",
            ));
        }
        self.inner.read(out)
    }
}
impl Write for LostStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(bytes)?;
        self.sent.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
impl ApiStream for LostStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError> {
        self.inner.set_read_timeout(timeout)
    }
    fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError> {
        self.inner.set_write_timeout(timeout)
    }
    fn shutdown_handle(&self) -> Result<Box<dyn ConnectionShutdown>, ApiError> {
        self.inner.shutdown_handle()
    }
    fn read_line_with_timeout(&mut self, timeout: Duration) -> Result<String, ApiError> {
        let answer = self.inner.read_line_with_timeout(timeout)?;
        if self.should_lose() {
            self.record_lost_success(&answer);
            Err(ApiError::Transport("answer lost after effect".into()))
        } else {
            Ok(answer)
        }
    }
}

struct Fixture {
    server: FakeHerdr,
    state: Arc<Mutex<State>>,
    stopped: Arc<AtomicBool>,
}
impl Fixture {
    fn new() -> Self {
        let state = Arc::new(Mutex::new(State::new()));
        let answer = state.clone();
        Self {
            server: FakeHerdr::start_with_errors("checkout-start", move |method, params| {
                let mut state = answer.lock().unwrap();
                if method == "pane.zoom" && state.refuse_unzoom {
                    state.calls.push(method.into());
                    return Err(("invalid_operation".into(), "unzoom refused".into()));
                }
                Ok(state.answer(method, params))
            }),
            state,
            stopped: Arc::new(AtomicBool::new(false)),
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
        let plain = self.server.connector();
        let lossy = LostAnswer {
            inner: self.server.connector(),
            method: lost.unwrap_or(""),
            stopped: self.stopped.clone(),
        };
        let node = IdentityNode(self.state.clone());
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
                connector: if lost.is_some() { &lossy } else { &plain },
                node: &node,
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
            .filter(|method| matches!(method.as_str(), "layout.apply" | "pane.move" | "pane.zoom"))
            .cloned()
            .collect()
    }
}

fn launch() -> ShellLaunch {
    ShellLaunch {
        command: vec!["/bin/zsh".into(), "-l".into()],
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
                        .run(&request, Some(&launch()), Some("layout.apply"))
                        .is_err()
                );
                fixture.run(&fixture.recover(&request), None, None)
            } else {
                fixture.run(&request, Some(&launch()), None)
            };
            assert!(result.unwrap_err().is_ambiguous());
            assert!(fixture.state.lock().unwrap().last_saved().binding.is_none());
            assert_eq!(fixture.mutations(), ["layout.apply"]);
        }
    }
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
    assert_eq!(fixture.mutations(), ["layout.apply", "pane.zoom"]);
    fixture.state.lock().unwrap().refuse_unzoom = false;
    let recovered = fixture.run(&fixture.recover(&request), None, None).unwrap();
    assert_eq!(recovered.created_tab_id.as_deref(), Some("w1:t1"));
    assert_eq!(
        fixture.mutations(),
        ["layout.apply", "pane.zoom", "pane.zoom", "pane.move"]
    );
}

#[test]
fn lost_create_answer_recovers_the_claim_without_creating_again() {
    let fixture = Fixture::new();
    let request = fixture.request(false);
    assert!(
        fixture
            .run(&request, Some(&launch()), Some("layout.apply"))
            .unwrap_err()
            .is_ambiguous()
    );
    assert!(fixture.state.lock().unwrap().last_saved().binding.is_none());
    let recovered = fixture.run(&fixture.recover(&request), None, None).unwrap();
    assert_eq!(recovered.pane_id, "w1:p3");
    assert_eq!(recovered.created_tab_id.as_deref(), Some("w1:t2"));
    assert_eq!(fixture.mutations(), ["layout.apply"]);
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
            .run(&request, Some(&launch()), Some("pane.move"))
            .unwrap_err()
            .is_ambiguous()
    );
    let saved = fixture.state.lock().unwrap().last_saved();
    assert_eq!(saved.binding.as_ref().unwrap().tab_id, "w1:t2");
    assert_eq!(saved.binding.as_ref().unwrap().identity, identity("w1:p3"));
    let recovered = fixture.run(&fixture.recover(&request), None, None).unwrap();
    assert_eq!(recovered.created_tab_id.as_deref(), Some("w1:t1"));
    assert_eq!(recovered.pane_id, "w1:p3");
    assert_eq!(fixture.mutations(), ["layout.apply", "pane.move"]);
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
            .run(&request, None, Some("layout.apply"))
            .unwrap_err()
            .is_ambiguous()
    );
    let error = fixture
        .run(&fixture.recover(&request), None, None)
        .unwrap_err();
    assert!(error.is_ambiguous());
    assert!(error.message().contains("continuity"));
    assert_eq!(fixture.mutations(), ["layout.apply"]);
    assert!(fixture.state.lock().unwrap().last_saved().binding.is_none());
}

#[test]
fn recovery_refuses_duplicate_claim_or_wrong_export_owner_without_mutation() {
    for duplicate in [true, false] {
        let fixture = Fixture::new();
        let request = fixture.request(false);
        assert!(
            fixture
                .run(&request, Some(&launch()), Some("layout.apply"))
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
        assert_eq!(fixture.mutations(), ["layout.apply"]);
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
                .run(&request, Some(&launch()), Some("pane.move"))
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
        assert_eq!(fixture.mutations(), ["layout.apply", "pane.move"]);
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
        ["layout.apply", "pane.zoom", "pane.move"]
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
    assert_eq!(fixture.mutations(), ["layout.apply"]);
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
            .run(&request, Some(&launch()), Some("layout.apply"))
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
    assert_eq!(fixture.mutations(), ["layout.apply"]);
    assert!(fixture.state.lock().unwrap().last_saved().binding.is_none());
}
