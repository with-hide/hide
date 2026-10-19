//! The install kit's workers (PRD device-parity B1, B8, B10, B13).
//!
//! One thread, owned by the core, runs the kit of this machine's own node:
//! the launch pass, the operator's Reinstall, and periodic kit and hook reads
//! while Settings is on screen. It takes its job under the runtime lock and
//! makes the node call with the lock released. The node does every file
//! write, Herdr call and child process (`hide_host::kit`). It does not live
//! on the session-sync coordinator, because that thread exists only while
//! this machine's Herdr answers, and the kit's other parts install whether or
//! not Herdr is running.
//!
//! The core closes its own node when it stops, which ends a child the kit is
//! waiting on; dropping the pump then joins the thread (engineering rule 14).
//!
//! A device's kit runs on its helper (`hide_host::kit`), and a node's that
//! dials this core on that node, through its link; a worker per device makes
//! those calls while the device has kit work queued.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::handle::ChangeNotifier;
use crate::node_access::{NodeLink, call_as};
use crate::runtime::{DeviceKitAnswer, DeviceKitCall, DeviceKitWork, KitJob, Runtime};

const PUMP_TICK: Duration = Duration::from_millis(250);

/// The longest a kit call may take: bounded daemon retirement, login-agent
/// removal, plugin uninstall and the remaining kit components.
const KIT_TIMEOUT: Duration = Duration::from_secs(180);

/// Where the core's own machine links `hide`, under its node's home.
const LOCAL_CLI_DIR: &str = "~/.local/bin";

/// The hook diagnosis is two small file reads.
const HOOK_DIAGNOSIS_TIMEOUT: Duration = Duration::from_secs(10);
const HOOK_DIAGNOSIS_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct KitPump {
    stop: mpsc::Sender<()>,
    stopping: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl KitPump {
    /// `herdr_socket` is the Herdr server the embedder named for this
    /// machine; without one the node uses the one Herdr would for its home.
    pub(crate) fn spawn(
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
        link: Arc<dyn NodeLink>,
        herdr_socket: Option<String>,
        node: crate::node::NodeId,
    ) -> std::io::Result<Self> {
        let (stop, receiver) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&stopping);
        let worker = thread::Builder::new()
            .name("kit-pump".into())
            .spawn(move || {
                // This worker is the sole local diagnosis owner. A read and
                // an install cannot overtake each other, and a due refresh
                // is one deadline, never a queue of elapsed ticks.
                let mut next_diagnosis = None;
                while matches!(
                    receiver.recv_timeout(PUMP_TICK),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    if cancelled.load(Ordering::Acquire) || link.closed_reason().is_some() {
                        break;
                    }
                    let Some(core) = runtime.upgrade() else {
                        break;
                    };
                    let now = Instant::now();
                    let Ok((job, diagnosis_due)) = core.lock().map(|mut locked| {
                        let job = locked
                            .take_local_kit_job(now)
                            .map(|job| (job, locked.retirement_projects(node.as_str())));
                        let due = next_diagnosis.is_none()
                            || (locked.ui_attached()
                                && locked.settings_observed()
                                && next_diagnosis.is_some_and(|next| now >= next));
                        (job, due)
                    }) else {
                        break;
                    };
                    drop(core);
                    let mut applied = false;
                    let report = job.map(|(job, retirement_projects)| {
                        let kit = hide_node_link::protocol::Call::Kit {
                            action: action_of(&job),
                            cli_dir: LOCAL_CLI_DIR.to_owned(),
                            herdr_socket: herdr_socket.clone(),
                            preserve_herdr_server: false,
                            retirement_projects,
                        };
                        let report =
                            call_as::<hide_kit::KitReport>(link.as_ref(), kit, KIT_TIMEOUT)
                                .map_err(|error| error.to_string());
                        if let (Ok(report), KitJob::Apply(scope)) = (&report, &job) {
                            completed(node.as_str(), scope, report);
                            applied = true;
                        }
                        report
                    });
                    if cancelled.load(Ordering::Acquire) || link.closed_reason().is_some() {
                        break;
                    }
                    // The hook diagnosis reads the same files, so it is read
                    // again here: Memory's "update hooks" and the agent rows
                    // follow what the kit just wrote.
                    let diagnosis = (diagnosis_due || applied)
                        .then(|| hook_diagnosis(link.as_ref(), node.as_str()));
                    if diagnosis_due || applied {
                        next_diagnosis = Some(Instant::now() + HOOK_DIAGNOSIS_REFRESH_INTERVAL);
                    }
                    let Some(core) = runtime.upgrade() else {
                        break;
                    };
                    let Ok(changed) = core.lock().map(|mut locked| {
                        // Core stops this owner before closing its fixed node.
                        // No reply from that retiring connection may publish.
                        if cancelled.load(Ordering::Acquire) || link.closed_reason().is_some() {
                            return false;
                        }
                        let mut changed = match &report {
                            Some(Ok(report)) => locked.ingest_kit_report(node.as_str(), report),
                            Some(Err(reason)) => {
                                // The node never ran the kit: a daemon
                                // outside the app, or a node with no home.
                                locked.set_local_kit_unavailable(reason);
                                true
                            }
                            None => false,
                        };
                        match diagnosis {
                            Some(Some(diagnosis)) => {
                                changed |= locked.ingest_hook_diagnosis(diagnosis);
                            }
                            Some(None) => changed |= locked.hook_diagnosis_failed(),
                            None => {}
                        }
                        changed
                    }) else {
                        break;
                    };
                    drop(core);
                    if changed {
                        notifier.notify();
                    }
                }
            })?;
        Ok(Self {
            stop,
            stopping,
            worker: Some(worker),
        })
    }

    /// Stop admission before Core closes the node to cancel its current call.
    pub(crate) fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        let _ = self.stop.send(());
    }
}

/// The node's hook diagnosis, or `None` with the reason logged.
pub(crate) fn hook_diagnosis(
    link: &dyn NodeLink,
    device_id: &str,
) -> Option<hide_agent_hooks::Diagnosis> {
    call_as(
        link,
        hide_node_link::protocol::Call::HookDiagnosis,
        HOOK_DIAGNOSIS_TIMEOUT,
    )
    .map_err(|error| {
        crate::diagnostic!(json!({
            "component": "kit",
            "kind": "hook_diagnosis.failed",
            "device_id": device_id,
            "reason": error.to_string(),
        }));
    })
    .ok()
}

/// One record per install pass on any machine, naming each part's outcome;
/// the reasons are the operator's diagnostic detail (engineering rule 10).
fn completed(device_id: &str, scope: &hide_kit::Scope, report: &hide_kit::KitReport) {
    crate::diagnostic!(json!({
        "component": "kit",
        "kind": "apply.completed",
        "device_id": device_id,
        "scope": if scope.is_automatic() { "automatic" } else { "operator" },
        "restore": scope.restore.iter().map(|id| id.code()).collect::<Vec<_>>(),
        "agents_on": scope.agent_on,
        "agents_off": scope.agent_off,
        "components": report.components.iter().map(|part| json!({
            "id": part.id.code(),
            "state": part.state,
            "reason": part.reason,
        })).collect::<Vec<_>>(),
        "refused": report.refused,
    }));
}

/// Runs a device's queued kit calls on its helper, one at a time, until
/// nothing is queued or the helper is gone (`Runtime::take_device_kit_call`).
/// Each call is bounded by [`KIT_TIMEOUT`], and the worker ends with
/// the core, like the device's other helper workers.
pub(crate) fn spawn_device_worker(
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    device_id: String,
) -> std::io::Result<()> {
    thread::Builder::new()
        .name("herdr-core-device-kit".into())
        .spawn(move || {
            loop {
                let Some(core) = runtime.upgrade() else {
                    return;
                };
                let Ok(call) = core
                    .lock()
                    .map(|mut locked| locked.take_device_kit_call(&device_id))
                else {
                    return;
                };
                drop(core);
                let Some(call) = call else {
                    return;
                };
                let answer = call_device(&call);
                if let (
                    DeviceKitAnswer::Report(Ok(report)),
                    DeviceKitWork::Job(KitJob::Apply(scope)),
                ) = (&answer, &call.work)
                {
                    completed(&device_id, scope, report);
                }
                // A removed device's helper connection was kept open only
                // for this call.
                if call.work == DeviceKitWork::Remove {
                    call.channel.close("device removed");
                }
                let Some(core) = runtime.upgrade() else {
                    return;
                };
                let Ok(changed) = core.lock().map(|mut locked| {
                    locked.ingest_device_kit_answer(&device_id, answer, call.daemon_off_run)
                }) else {
                    return;
                };
                drop(core);
                if changed {
                    notifier.notify();
                }
            }
        })
        .map(|_| ())
}

fn call_device(call: &DeviceKitCall) -> DeviceKitAnswer {
    use hide_node_link::protocol::KitAction;
    let action = match &call.work {
        DeviceKitWork::Job(job) => action_of(job),
        DeviceKitWork::Remove => KitAction::Remove,
    };
    let removing = action == KitAction::Remove;
    let kit = hide_node_link::protocol::Call::Kit {
        action,
        cli_dir: call.cli_dir.clone(),
        herdr_socket: call.herdr_socket.clone(),
        preserve_herdr_server: call.preserve_herdr_server,
        retirement_projects: call.retirement_projects.clone(),
    };
    if removing {
        DeviceKitAnswer::Removed(
            call_as(call.channel.as_ref(), kit, KIT_TIMEOUT).map_err(|error| error.to_string()),
        )
    } else {
        DeviceKitAnswer::Report(
            call_as(call.channel.as_ref(), kit, KIT_TIMEOUT).map_err(|error| error.to_string()),
        )
    }
}

/// What the node's kit is asked for `job`.
fn action_of(job: &KitJob) -> hide_node_link::protocol::KitAction {
    use hide_node_link::protocol::KitAction;
    match job {
        KitJob::Apply(scope) if scope.is_automatic() => KitAction::Apply,
        KitJob::Apply(scope) => KitAction::Reinstall {
            components: scope.restore.iter().copied().collect(),
            agents_on: scope.agent_on.iter().cloned().collect(),
            agents_off: scope.agent_off.iter().cloned().collect(),
            codex_daemon_off: scope.codex_daemon_off,
        },
        KitJob::Status => KitAction::Status,
    }
}

impl Drop for KitPump {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            crate::diagnostic!(json!({"component":"kit", "kind":"worker.join_failed"}));
        }
    }
}
