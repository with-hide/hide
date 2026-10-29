//! An SSH device's sign-in through the core (PRD system-ssh-device-link
//! D-05, D-06, B6-B8, B10, B14): what ssh asks reaches the device row, the
//! operator's answer reaches the attempt that asked and nothing else, and an
//! attempt of Hide's own asks nobody. The transport is the test's: each
//! attempt asks one question and fails the way ssh would after it.

use super::*;
use crate::remote::{
    Attempt, Cancel, CapabilityReport, DeviceConnector, DeviceTransport, DialedTransport,
    EstablishError, Established, SshFailure, SshHostListing, SshProblem, Wake, WslListing,
};
use hide_node_link::device::{
    Answer, HostConsent, Refusal, RemoteHostIdentity, SignInWait, SnapshotCheck,
};
use std::sync::atomic::{AtomicUsize, Ordering};

const DEVICE: &str = "mini";

/// The line ssh ends a failed attempt with in this test, naming the account
/// and server as a refused sign-in's does.
const RAW_LINE: &str = "alice@mini: Permission denied (publickey).";

/// What each attempt was given and what its question got.
#[derive(Default)]
struct Asked {
    wakes: Mutex<Vec<Wake>>,
    answers: Mutex<Vec<Result<String, Refusal>>>,
    cancels: Mutex<Vec<Cancel>>,
    /// Set, an attempt whose question ended waits on it before it returns,
    /// as one does while ssh is ended and reaped.
    ending: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    running: AtomicUsize,
    most_running: AtomicUsize,
    /// Set, an attempt asks nothing and fails as ssh does with this class.
    fails_with: Mutex<Option<SshProblem>>,
    /// Set, each attempt's `ssh` outlives its return until the test reaps
    /// it, as a real one's reaper holds its share until the reap.
    unreaped: Mutex<Option<Vec<hide_node_link::device::Held>>>,
}

struct Devices(Arc<Asked>);

impl DeviceConnector for Devices {
    fn transport(
        &self,
        _home: &std::path::Path,
        _node: &str,
        alias: &str,
        _herdr_socket: Option<String>,
    ) -> Result<Arc<dyn DialedTransport>, String> {
        assert!(alias.starts_with(DEVICE), "{alias}");
        Ok(Arc::new(AskingDevice(Arc::clone(&self.0))))
    }

    fn ssh_hosts(
        &self,
        _home: &std::path::Path,
        _registered: &[String],
        _stop: &std::sync::atomic::AtomicBool,
    ) -> SshHostListing {
        SshHostListing::default()
    }

    fn herdr_profiles(
        &self,
        _home: &std::path::Path,
        _registered: &[String],
        _stop: &std::sync::atomic::AtomicBool,
    ) -> Result<hide_node_link::device::HerdrProfileListing, String> {
        Ok(hide_node_link::device::HerdrProfileListing::default())
    }

    fn wsl_transport(
        &self,
        _node: &str,
        _distro: &str,
    ) -> Result<Arc<dyn DialedTransport>, String> {
        Err("this test has no WSL".to_owned())
    }

    fn wsl_distributions(&self) -> WslListing {
        Ok(Vec::new())
    }
}

/// Asks for a password; an answer is refused by the server, and no answer
/// ends the attempt as ssh does when its askpass gives none.
struct AskingDevice(Arc<Asked>);

impl DeviceTransport for AskingDevice {
    fn herdr_api_connector(&self) -> Arc<dyn hide_herdr_client::ApiConnector> {
        Arc::new(hide_herdr_client::LocalSocketConnector::new(
            "/tmp/herdr-core-never-connect.sock",
        ))
    }

    fn cached_herdr_version(&self) -> Option<String> {
        None
    }
}

impl DialedTransport for AskingDevice {
    fn capability_test(&self, operation_id: &str, _check: SnapshotCheck<'_>) -> CapabilityReport {
        CapabilityReport::new(
            operation_id,
            RemoteHostIdentity {
                host_id: DEVICE.to_owned(),
                alias: DEVICE.to_owned(),
                hostname: DEVICE.to_owned(),
                port: 22,
            },
        )
    }

    fn establish(
        &self,
        _consent: &HostConsent,
        _retirement_projects: &[String],
        attempt: Attempt,
    ) -> Result<Established, EstablishError> {
        let running = self.0.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.0.most_running.fetch_max(running, Ordering::SeqCst);
        self.0.wakes.lock().unwrap().push(attempt.wake);
        self.0.cancels.lock().unwrap().push(attempt.cancel.clone());
        if let Some(unreaped) = self.0.unreaped.lock().unwrap().as_mut() {
            unreaped.push(attempt.cancel.hold());
        }
        if let Some(problem) = *self.0.fails_with.lock().unwrap() {
            if let Some(ending) = self.0.ending.lock().unwrap().take() {
                let _ = ending.recv();
            }
            self.0.running.fetch_sub(1, Ordering::SeqCst);
            return Err(EstablishError::Ssh(SshFailure {
                problem,
                detail: RAW_LINE.to_owned(),
            }));
        }
        let answer = attempt.sign_in.ask(SignInWait::Secret {
            prompt: "alice@mini's password:".to_owned(),
            key_file: None,
        });
        let problem = match &answer {
            Ok(_) => SshProblem::SignInRefused,
            Err(_) => SshProblem::SignInNeeded,
        };
        self.0
            .answers
            .lock()
            .unwrap()
            .push(answer.map(|answer| match answer {
                Answer::Text(secret) => secret.expose().to_owned(),
                Answer::Accept => "yes".to_owned(),
            }));
        if let Some(ending) = self.0.ending.lock().unwrap().take() {
            let _ = ending.recv();
        }
        self.0.running.fetch_sub(1, Ordering::SeqCst);
        Err(EstablishError::Ssh(SshFailure {
            problem,
            detail: "Permission denied".to_owned(),
        }))
    }
}

fn ssh_runtime() -> (Arc<Mutex<Runtime>>, Arc<Asked>) {
    let mut runtime = super::devices::runtime_with_home();
    let asked = Arc::new(Asked::default());
    runtime.devices = Arc::new(Devices(Arc::clone(&asked)));
    let consent = runtime.new_host_consent();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: DEVICE.to_owned(),
            label: "Mini".to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: DEVICE.to_owned(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent: Some(consent),
        });
    runtime.rebuild_device_rows();
    let shared = Arc::new(Mutex::new(runtime));
    shared.lock().unwrap().install_worker_context(
        Arc::downgrade(&shared),
        crate::handle::ChangeNotifier::noop(),
    );
    (shared, asked)
}

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": payload,
    }))
    .unwrap()
}

fn row(runtime: &Runtime) -> &crate::model::DeviceSnapshot {
    runtime
        .snapshot()
        .navigator
        .devices
        .iter()
        .find(|device| device.id == DEVICE)
        .expect("the device row")
}

/// The device connected as the core starting connects it, whose attempt
/// asks nobody, and then retried by the operator.
fn retry(shared: &Arc<Mutex<Runtime>>) {
    shared.lock().unwrap().connect_registered_devices();
    settled(shared);
    let retry = event(
        "device_host_retry",
        serde_json::json!({"device_id": DEVICE}),
    );
    assert!(shared.lock().unwrap().dispatch_json(&retry));
}

/// The question the row shows, once it shows one.
fn question(shared: &Arc<Mutex<Runtime>>) -> u64 {
    super::wait(shared, "the row to ask", |runtime| {
        row(runtime).sign_in.is_some()
    });
    let runtime = shared.lock().unwrap();
    let sign_in = row(&runtime).sign_in.clone().unwrap();
    assert_eq!(
        sign_in.wait,
        SignInWait::Secret {
            prompt: "alice@mini's password:".to_owned(),
            key_file: None,
        }
    );
    sign_in.question
}

fn settled(shared: &Arc<Mutex<Runtime>>) {
    super::wait(shared, "the attempt to settle", |runtime| {
        !runtime.device_host_connecting(DEVICE)
    });
}

fn answers(asked: &Asked) -> Vec<Result<String, Refusal>> {
    asked.answers.lock().unwrap().clone()
}

fn no_retry_waits(runtime: &Runtime) -> bool {
    runtime
        .device_host_retries
        .get(DEVICE)
        .is_none_or(|retry| retry.at_unix_ms.is_none())
}

/// B6, B7, B10: the operator's attempt puts ssh's question on the row; the
/// answer reaches that attempt and appears nowhere in the snapshot, and a
/// server that refuses it ends as a refused sign-in Hide does not retry by
/// itself.
#[test]
fn an_answer_reaches_the_attempt_that_asked_and_nothing_else() {
    let (shared, asked) = ssh_runtime();
    retry(&shared);
    let question = question(&shared);
    let answered = event(
        "device_sign_in",
        serde_json::json!({
            "device_id": DEVICE,
            "question": question,
            "reply": "text",
            "text": "hunter2",
        }),
    );
    assert!(shared.lock().unwrap().dispatch_json(&answered));
    settled(&shared);
    assert_eq!(
        answers(&asked),
        [Err(Refusal::NotAllowed), Ok("hunter2".to_owned())]
    );
    let runtime = shared.lock().unwrap();
    let device = row(&runtime);
    assert_eq!(device.sign_in, None);
    assert_eq!(device.problem.as_deref(), Some("sign_in_refused"));
    assert!(
        no_retry_waits(&runtime),
        "a refused sign-in is retried by the operator only"
    );
    let wire = serde_json::to_string(runtime.snapshot()).unwrap();
    assert!(!wire.contains("hunter2"), "the answer reached the snapshot");
}

/// B7, B8: an answer for a question ssh no longer asks reaches nothing, and
/// Cancel ends the attempt as needing a sign-in, with the row's sign-in
/// button left.
#[test]
fn a_stale_answer_reaches_nothing_and_cancel_ends_the_attempt() {
    let (shared, asked) = ssh_runtime();
    retry(&shared);
    let question = question(&shared);
    let stale = event(
        "device_sign_in",
        serde_json::json!({
            "device_id": DEVICE,
            "question": question + 1,
            "reply": "text",
            "text": "for another question",
        }),
    );
    assert!(!shared.lock().unwrap().dispatch_json(&stale));
    assert!(row(&shared.lock().unwrap()).sign_in.is_some());
    let cancel = event(
        "device_sign_in",
        serde_json::json!({"device_id": DEVICE, "question": question, "reply": "cancel"}),
    );
    assert!(shared.lock().unwrap().dispatch_json(&cancel));
    settled(&shared);
    assert_eq!(
        answers(&asked),
        [Err(Refusal::NotAllowed), Err(Refusal::Cancelled)]
    );
    let runtime = shared.lock().unwrap();
    assert_eq!(row(&runtime).problem.as_deref(), Some("sign_in_needed"));
    assert!(no_retry_waits(&runtime));
}

/// D-06, B14, B10: Hide's own attempt asks nobody and shows nothing; it
/// stops as needing a sign-in without a retry of its own, and the app
/// opening tries again with leave to ask.
#[test]
fn hides_own_attempt_asks_nobody_and_the_app_opening_may_ask() {
    let (shared, asked) = ssh_runtime();
    shared.lock().unwrap().connect_registered_devices();
    settled(&shared);
    {
        let runtime = shared.lock().unwrap();
        assert_eq!(answers(&asked), [Err(Refusal::NotAllowed)]);
        let device = row(&runtime);
        assert_eq!(device.sign_in, None);
        assert_eq!(device.problem.as_deref(), Some("sign_in_needed"));
        assert!(no_retry_waits(&runtime));
    }
    let app_open = event("hide_ai_wake", serde_json::json!({"cause": "app_open"}));
    shared.lock().unwrap().dispatch_json(&app_open);
    question(&shared);
    assert_eq!(
        *asked.wakes.lock().unwrap(),
        [Wake::NotAllowed, Wake::Allowed]
    );
}

/// The device's removal ends the question its attempt waits on and tells
/// the attempt to end (B26).
#[test]
fn removing_the_device_ends_its_question() {
    let (shared, asked) = ssh_runtime();
    retry(&shared);
    question(&shared);
    let remove = event("remove_device", serde_json::json!({"device_id": DEVICE}));
    shared.lock().unwrap().dispatch_json(&remove);
    super::wait_for("the question to end", || answers(&asked).len() == 2);
    assert_eq!(
        answers(&asked),
        [Err(Refusal::NotAllowed), Err(Refusal::Ended)]
    );
    assert!(asked.cancels.lock().unwrap()[1].is_cancelled());
}

/// B14: once a sign-in is needed, nothing Hide does by itself starts ssh
/// again, so the server sees no sign-in nobody typed; the operator's retry
/// does.
#[test]
fn once_a_sign_in_is_needed_hides_own_work_starts_no_ssh() {
    let (shared, asked) = ssh_runtime();
    shared.lock().unwrap().connect_registered_devices();
    settled(&shared);
    {
        let mut runtime = shared.lock().unwrap();
        assert_eq!(row(&runtime).problem.as_deref(), Some("sign_in_needed"));
        // The core starting again, a read that needs the link, a due retry
        // and the device's session answering again.
        runtime.connect_registered_devices();
        assert!(!runtime.start_device_host(DEVICE, hosts::HostAttempt::Background));
        runtime.tick_device_hosts(u64::MAX);
        runtime.device_reachable_again(DEVICE);
        assert!(!runtime.device_host_connecting(DEVICE));
        assert_eq!(row(&runtime).problem.as_deref(), Some("sign_in_needed"));
    }
    assert_eq!(*asked.wakes.lock().unwrap(), [Wake::NotAllowed]);
    let retry = event(
        "device_host_retry",
        serde_json::json!({"device_id": DEVICE}),
    );
    assert!(shared.lock().unwrap().dispatch_json(&retry));
    question(&shared);
    assert_eq!(
        *asked.wakes.lock().unwrap(),
        [Wake::NotAllowed, Wake::Allowed]
    );
}

/// B26: withdrawing consent ends the running attempt, and an attempt the
/// consent given again starts reaches ssh only once the one before it has
/// ended, its `ssh` reaped included; a waiting attempt that is itself
/// cancelled gives up without the device's slot forgetting the earlier one,
/// so the attempt after it still waits for that `ssh`.
#[test]
#[allow(clippy::disallowed_methods)] // a window in which the next attempt must not start: no state reports a start that did not happen
fn an_attempt_reaches_ssh_only_once_the_ones_before_it_are_reaped() {
    let (shared, asked) = ssh_runtime();
    shared.lock().unwrap().connect_registered_devices();
    settled(&shared);
    *asked.unreaped.lock().unwrap() = Some(Vec::new());
    let retry = event(
        "device_host_retry",
        serde_json::json!({"device_id": DEVICE}),
    );
    assert!(shared.lock().unwrap().dispatch_json(&retry));
    question(&shared);
    let consent = |allow: bool| {
        event(
            "device_host_consent",
            serde_json::json!({"device_id": DEVICE, "allow": allow}),
        )
    };
    shared.lock().unwrap().dispatch_json(&consent(false));
    super::wait_for("the question to end", || answers(&asked).len() == 2);
    assert_eq!(answers(&asked)[1], Err(Refusal::Ended));
    let first = asked.cancels.lock().unwrap()[1].clone();
    assert!(first.is_cancelled());
    super::wait_for("the attempt to return", || {
        asked.running.load(Ordering::SeqCst) == 0
    });
    assert!(!first.ended(), "its ssh is not reaped yet");

    let window = || std::thread::sleep(std::time::Duration::from_millis(300));
    shared.lock().unwrap().dispatch_json(&consent(true));
    super::wait(&shared, "the next attempt to start", |runtime| {
        runtime.device_host_connecting(DEVICE)
    });
    window();
    assert_eq!(asked.wakes.lock().unwrap().len(), 2);
    // The waiting attempt is cancelled in turn and gives up.
    shared.lock().unwrap().dispatch_json(&consent(false));
    super::wait(&shared, "the waiting attempt to give up", |runtime| {
        !runtime.device_host_connecting(DEVICE)
    });
    shared.lock().unwrap().dispatch_json(&consent(true));
    super::wait(&shared, "the third attempt to start", |runtime| {
        runtime.device_host_connecting(DEVICE)
    });
    window();
    assert_eq!(
        asked.wakes.lock().unwrap().len(),
        2,
        "an attempt reached ssh beside one whose ssh still runs"
    );

    // The first attempt's ssh is reaped.
    asked.unreaped.lock().unwrap().take();
    super::wait_for("the third attempt to reach ssh", || {
        asked.wakes.lock().unwrap().len() == 3
    });
    assert!(
        !asked.cancels.lock().unwrap()[2].is_cancelled(),
        "the attempt that gave up reached ssh"
    );
    assert!(first.ended());
    assert_eq!(asked.most_running.load(Ordering::SeqCst), 1);
}

/// B13, B18: a changed server key stays the row's reason and action. Hide's
/// own work starts nothing for it, the device's session saying its old link
/// ended does not replace it, and while the operator's retry runs the row
/// still says it.
#[test]
fn a_changed_server_key_stays_the_rows_reason_until_an_attempt_ends_otherwise() {
    let (shared, asked) = ssh_runtime();
    *asked.fails_with.lock().unwrap() = Some(SshProblem::HostKeyChanged);
    shared.lock().unwrap().connect_registered_devices();
    settled(&shared);
    let reason = |runtime: &Runtime| {
        let device = row(runtime);
        (device.problem.clone(), device.message.clone())
    };
    let changed = reason(&shared.lock().unwrap());
    assert_eq!(changed.0.as_deref(), Some("host_key_changed"));
    {
        let mut runtime = shared.lock().unwrap();
        let status = runtime
            .snapshot
            .status
            .remote
            .iter_mut()
            .find(|status| status.target_id == DEVICE)
            .expect("the device's session status");
        status.message = Some(
            "the node's Herdr could not be reached: the node's link has ended (the helper's output closed)"
                .to_owned(),
        );
        runtime.start_device_host(DEVICE, hosts::HostAttempt::Background);
        runtime.tick_device_hosts(u64::MAX);
        runtime.refresh_device_snapshots();
        assert_eq!(reason(&runtime), changed);
    }
    assert_eq!(asked.wakes.lock().unwrap().len(), 1);
    let (release, ending) = std::sync::mpsc::channel();
    *asked.ending.lock().unwrap() = Some(ending);
    let retry = event(
        "device_host_retry",
        serde_json::json!({"device_id": DEVICE}),
    );
    assert!(shared.lock().unwrap().dispatch_json(&retry));
    super::wait_for("the operator's attempt to reach ssh", || {
        asked.running.load(Ordering::SeqCst) == 1
    });
    assert_eq!(reason(&shared.lock().unwrap()), changed);
    release.send(()).unwrap();
    settled(&shared);
    assert_eq!(reason(&shared.lock().unwrap()), changed);
}

/// B26: a run leaves the device's slot as nothing of it runs any more, and
/// a device's slot with it, so attempts of removed devices are not kept
/// until the core stops.
#[test]
fn ended_attempts_leave_no_slot_for_their_devices() {
    let (shared, asked) = ssh_runtime();
    let devices = ["mini-a", "mini-b", "mini-c"];
    {
        let mut runtime = shared.lock().unwrap();
        for id in devices {
            let consent = runtime.new_host_consent();
            runtime
                .snapshot
                .ui_state
                .device_registrations
                .push(crate::model::DeviceRegistration {
                    id: id.to_owned(),
                    label: id.to_owned(),
                    origin: crate::model::LinkOrigin::Dialed {
                        ssh_alias: id.to_owned(),
                    },
                    herdr_socket_path: None,
                    herdr_session: None,
                    host_consent: Some(consent),
                });
        }
        runtime.rebuild_device_rows();
    }
    // Each attempt's `ssh` stays unreaped until the test reaps it.
    *asked.unreaped.lock().unwrap() = Some(Vec::new());
    shared.lock().unwrap().connect_registered_devices();
    super::wait_for("every device's attempt to reach ssh", || {
        asked.unreaped.lock().unwrap().as_ref().unwrap().len() == devices.len() + 1
    });
    super::wait(&shared, "the attempts to settle", |runtime| {
        devices
            .iter()
            .chain([&DEVICE])
            .all(|id| !runtime.device_host_connecting(id))
    });
    assert_eq!(
        shared.lock().unwrap().device_attempts.len(),
        devices.len() + 1
    );
    for id in devices {
        assert!(shared.lock().unwrap().dispatch_json(&event(
            "remove_device",
            serde_json::json!({"device_id": id}),
        )));
    }
    assert!(
        asked
            .cancels
            .lock()
            .unwrap()
            .iter()
            .filter(|run| run.is_cancelled())
            .count()
            >= devices.len()
    );
    assert_eq!(
        shared.lock().unwrap().device_attempts.len(),
        devices.len() + 1,
        "a run whose ssh still runs keeps its slot"
    );

    // Every `ssh` is reaped.
    asked.unreaped.lock().unwrap().take();
    super::wait(&shared, "every slot to go", |runtime| {
        runtime.device_attempts.is_empty()
    });
}

fn device_command(
    shared: &Arc<Mutex<Runtime>>,
    command: crate::device_command::DeviceCommand,
) -> serde_json::Value {
    shared
        .lock()
        .unwrap()
        .device_command(crate::node::TEST_NODE, command)
        .expect("answered")
        .0
}

fn retry_command() -> crate::device_command::DeviceCommand {
    crate::device_command::DeviceCommand::Retry {
        device_id: DEVICE.to_owned(),
    }
}

/// B29-B31: `hide device` reads a stopped sign-in as the operator's to do,
/// its retry starts the row's attempt that may ask, and while the row asks
/// the answer names what to ask the operator for without ssh's words, and
/// another retry leaves the question the operator is answering alone.
#[test]
fn hide_device_retry_starts_the_rows_attempt_and_leaves_its_question_alone() {
    use crate::device_command::DeviceCommand;
    let (shared, asked) = ssh_runtime();
    shared.lock().unwrap().connect_registered_devices();
    settled(&shared);
    let list = device_command(&shared, DeviceCommand::List);
    let device = &list["devices"][0];
    assert_eq!(device["state"], "failed");
    assert_eq!(
        device["failure"]["action"], "sign_in",
        "a sign-in nobody was asked for is signed in again"
    );
    assert_eq!(device["failure"]["cause"], "sign_in_needed");

    let started = device_command(&shared, retry_command());
    assert_eq!(started["started"], true);
    assert_eq!(started["device"]["state"], "connecting");
    question(&shared);
    let waiting = device_command(&shared, retry_command());
    assert_eq!(waiting["started"], false);
    assert_eq!(waiting["device"]["state"], "operator_turn");
    assert_eq!(
        waiting["device"]["waits_on"],
        serde_json::json!({"kind": "sign_in", "input": "secret"})
    );
    assert!(
        waiting["device"]["next_action"]
            .as_str()
            .is_some_and(|next| next.contains("password or code")),
        "{waiting}"
    );
    assert!(
        !waiting.to_string().contains("alice"),
        "ssh's question reached the answer: {waiting}"
    );
    assert_eq!(
        *asked.wakes.lock().unwrap(),
        [Wake::NotAllowed, Wake::Allowed],
        "the second retry started an attempt"
    );
}

/// B29, operator decision 2026-10-30: a failure reads as the row's one
/// action for it, with the core's finer cause beside it; a changed server
/// key is checked in a terminal, and a refusing access policy is retried
/// once changed, not signed in again. ssh's last line comes along only for
/// a retry, where it names what to fix: a refused sign-in's names the
/// account, in the JSON and in the lines for a person alike.
#[test]
fn hide_device_reads_a_failure_as_the_rows_action() {
    for (problem, action, said, detail) in [
        (
            SshProblem::SignInRefused,
            "sign_in",
            "hide device retry mini --wait",
            None,
        ),
        (
            SshProblem::HostKeyChanged,
            "check_host_key",
            "ssh mini",
            None,
        ),
        (
            SshProblem::PolicyRefused,
            "retry",
            "hide device retry mini",
            Some(RAW_LINE),
        ),
        (
            SshProblem::Unreachable,
            "retry",
            "hide device retry mini",
            Some(RAW_LINE),
        ),
    ] {
        let (shared, asked) = ssh_runtime();
        *asked.fails_with.lock().unwrap() = Some(problem);
        shared.lock().unwrap().connect_registered_devices();
        settled(&shared);
        let list = device_command(&shared, crate::device_command::DeviceCommand::List);
        let device = &list["devices"][0];
        assert_eq!(device["state"], "failed", "{problem:?}");
        assert_eq!(device["failure"]["action"], action, "{problem:?}");
        assert_eq!(device["failure"]["cause"], problem.code());
        assert_eq!(device["failure"]["detail"].as_str(), detail, "{problem:?}");
        if detail.is_none() {
            assert!(!list.to_string().contains("alice"), "{list}");
        }
        assert!(
            device["next_action"]
                .as_str()
                .is_some_and(|next| next.contains(said)),
            "{device}"
        );
    }
}

/// `hide contract` publishes the retry's answer from its type, and the
/// answer the core gives has each field the schema requires, of its type.
#[test]
fn hide_device_retry_answers_as_its_published_schema() {
    let (shared, _asked) = ssh_runtime();
    shared.lock().unwrap().connect_registered_devices();
    settled(&shared);
    let answer = device_command(&shared, retry_command());
    let schema = crate::device_command::retried_schema();
    let required = schema["required"].as_array().expect("required fields");
    assert!(!required.is_empty());
    for field in required {
        let field = field.as_str().unwrap();
        let value = &answer[field];
        let fits = match schema["properties"][field]["type"].as_str() {
            Some("boolean") => value.is_boolean(),
            Some("object") => value.is_object(),
            Some("string") => value.is_string(),
            Some(other) => panic!("{field} is declared {other}, which this check does not read"),
            // `true`: any value.
            None => !value.is_null(),
        };
        assert!(fits, "{field} = {value} does not fit {schema}");
    }
    assert_eq!(schema["properties"]["started"]["type"], "boolean");
}
