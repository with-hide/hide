//! The install kit's workers (PRD device-parity B1, B8, B10, B13).
//!
//! One thread, owned by the core, runs the kit of this machine's own node:
//! the launch pass, the operator's Reinstall, and periodic kit and hook reads
//! while Settings is on screen. It takes its job under the runtime lock and
//! makes the node call with the lock released. The node does every file
//! write, the engine call and child process (`hide_host::kit`). It does not live
//! on the session-sync coordinator, because that thread exists only while
//! this machine's engine answers, and the kit's other parts install whether or
//! not the engine is running.
//!
//! The core closes its own node when it stops, which ends a child the kit is
//! waiting on; dropping the pump then joins the thread (engineering rule 14).
//!
//! A device's kit runs on its helper (`hide_host::kit`), and a node's that
//! dials this core on that node, through its link; a worker per device makes
//! those calls while the device has kit work queued. The device's hook
//! diagnosis, which carries the versions of the agents its panes run, is read
//! by the device's own coordinator ([`DeviceDiagnosisReader`]), whether or not
//! any kit work is queued.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::handle::ChangeNotifier;
use crate::node_access::{NodeLink, call_as};
use crate::runtime::{
    DeviceKitAnswer, DeviceKitCall, DeviceKitWork, KitJob, ProfilesRead, Runtime,
};

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
    pub(crate) fn spawn(
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
        link: Arc<dyn NodeLink>,
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
                        let job = locked.take_local_kit_job(now).map(|job| {
                            let profiles = locked.profiles_due(node.as_str(), &job);
                            (job, locked.retirement_projects(node.as_str()), profiles)
                        });
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
                    let mut profiles_due = false;
                    let report = job.map(|(job, retirement_projects, profiles)| {
                        profiles_due = profiles;
                        let kit = hide_node_link::protocol::Call::Kit {
                            action: action_of(&job),
                            cli_dir: LOCAL_CLI_DIR.to_owned(),
                            retirement_projects,
                        };
                        let report =
                            call_as::<hide_kit::KitReport>(link.as_ref(), kit, KIT_TIMEOUT)
                                .map_err(|error| (unavailable_cause(&error), error.to_string()));
                        if let (Ok(report), KitJob::Apply(scope)) = (&report, &job) {
                            completed(node.as_str(), scope, report);
                            applied = true;
                        }
                        report
                    });
                    if cancelled.load(Ordering::Acquire) || link.closed_reason().is_some() {
                        break;
                    }
                    let profiles = (profiles_due && matches!(report, Some(Ok(_))))
                        .then(|| read_profiles(link.as_ref()));
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
                            Some(Err((cause, detail))) => {
                                // The node never ran the kit: a daemon
                                // outside the app, or a node with no home.
                                locked.set_local_kit_unavailable(*cause, detail);
                                true
                            }
                            None => false,
                        };
                        if let Some(profiles) = profiles {
                            changed |= locked.ingest_profiles(node.as_str(), profiles);
                        }
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

/// How long a device's answer stays fresh. What it carries that matters to
/// the doorbell is the installed agents' versions, which change only when the
/// operator updates one, and the node answers each from its own cache of the
/// program's file, so a minute asks the device little.
const DEVICE_DIAGNOSIS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Reads a registered device's hook diagnosis on a worker thread of its own,
/// so the agent versions its panes run reach the core: the doorbell reads a
/// pane's composer only for a version it knows, and a pane on a device has
/// no other way to say it (docs/delivery.md, Catching an agent's screen
/// change). The same `HookDiagnosis` call answers on every machine, from that
/// machine's own account and search, so no system has a form of its own.
///
/// The core's own node is not read here: the kit pump reads it with the kit.
pub(crate) struct DeviceDiagnosisReader {
    inner: crate::reader::BackgroundRead<(), Option<hide_agent_hooks::Diagnosis>>,
}

impl DeviceDiagnosisReader {
    pub(crate) fn new(runtime: Weak<Mutex<Runtime>>, device_id: String) -> Self {
        Self {
            inner: crate::reader::BackgroundRead::new(
                DEVICE_DIAGNOSIS_REFRESH_INTERVAL,
                Duration::ZERO,
                move |_: &()| read_device_diagnosis(&runtime, &device_id),
            ),
        }
    }

    /// The answer of a finished read, `Some(None)` when the device could not
    /// be asked or did not answer: that keeps the last answer, since an
    /// agent's version does not go with a link that dropped. Starts the next
    /// read once the last one is older than the refresh interval.
    pub(crate) fn read_if_due(&mut self) -> Option<Option<hide_agent_hooks::Diagnosis>> {
        self.inner.poll(())
    }
}

/// The device's diagnosis, or `None` while its helper is not connected or
/// when it did not answer (`hook_diagnosis` leaves the reason in the log).
/// The link is looked up under a short lock and asked with it released.
fn read_device_diagnosis(
    runtime: &Weak<Mutex<Runtime>>,
    device_id: &str,
) -> Option<hide_agent_hooks::Diagnosis> {
    let link = {
        let runtime = runtime.upgrade()?;
        let guard = runtime.lock().ok()?;
        guard.linked_node(device_id).ok()?.1
    };
    hook_diagnosis(link.as_ref(), device_id)
}

/// One record per install pass on any machine, naming each part's outcome;
/// the reasons are the operator's diagnostic detail (engineering rule 10).
pub(crate) fn completed(device_id: &str, scope: &hide_kit::Scope, report: &hide_kit::KitReport) {
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
        .name("hide-core-device-kit".into())
        .spawn(move || {
            loop {
                let Some(core) = runtime.upgrade() else {
                    return;
                };
                let Ok(call) = core.lock().map(|mut locked| {
                    locked.take_device_kit_call(&device_id).map(|call| {
                        let profiles = match &call.work {
                            DeviceKitWork::Job(job) => locked.profiles_due(&device_id, job),
                            DeviceKitWork::Remove => false,
                        };
                        (call, profiles)
                    })
                }) else {
                    return;
                };
                drop(core);
                let Some((call, profiles_due)) = call else {
                    return;
                };
                let answer = call_device(&call);
                let profiles = (profiles_due && matches!(answer, DeviceKitAnswer::Report(Ok(_))))
                    .then(|| read_profiles(call.channel.as_ref()));
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
                    if let DeviceKitAnswer::Report(report) = &answer {
                        locked.complete_device_connect_kit(
                            &device_id,
                            call.connect_attempt,
                            report,
                        );
                    }
                    let mut changed =
                        locked.ingest_device_kit_answer(&device_id, answer, call.daemon_off_run);
                    if let Some(profiles) = profiles {
                        changed |= locked.ingest_profiles(&device_id, profiles);
                    }
                    changed
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

/// A list of profiles is a file read and the kit's judgment of each.
const PROFILE_LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// The node's profiles (`Call::ProfileList`), with the runtime lock
/// released. A node that predates them answers the call unrun: one that
/// knows the call and keeps none (`Unsupported`), or one on an older
/// protocol that cannot read it.
pub(crate) fn read_profiles(link: &dyn NodeLink) -> ProfilesRead {
    use crate::node_access::LinkError;
    use hide_node_link::ErrorCode;
    let call = hide_node_link::protocol::Call::ProfileList { values: false };
    match call_as::<hide_node_link::profiles::ProfileList>(link, call, PROFILE_LIST_TIMEOUT) {
        Ok(list) => ProfilesRead::Listed(list),
        Err(LinkError::Refused(refusal))
            if refusal.code == ErrorCode::Unsupported
                || (refusal.code == ErrorCode::InvalidRequest
                    && link.predates_current_protocol()) =>
        {
            ProfilesRead::Unsupported
        }
        Err(error) => ProfilesRead::Unreadable(error.to_string()),
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

/// Why this machine's node did not run the kit. Only a daemon outside the
/// app refuses with the kit's own standalone sentence; anything else is a
/// node that did not answer, and its words go to the log.
fn unavailable_cause(error: &crate::node_access::LinkError) -> crate::model::KitUnavailable {
    use crate::model::KitUnavailable;
    match error {
        crate::node_access::LinkError::Refused(refusal)
            if refusal.code == hide_node_link::ErrorCode::Unsupported
                && refusal.message == hide_kit::STANDALONE_REASON =>
        {
            KitUnavailable::Standalone
        }
        _ => KitUnavailable::Unreadable,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_access::LinkError;
    use hide_node_link::protocol::Call;
    use hide_node_link::{ErrorCode, HostError, LinkAnswer};

    /// A node that answers every call the same way, on the current protocol
    /// or an older one the core still admits.
    struct Answering {
        answer: Result<serde_json::Value, ErrorCode>,
        older: bool,
    }

    impl NodeLink for Answering {
        fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
            assert!(
                matches!(call, Call::ProfileList { values: false }),
                "{call:?}"
            );
            match &self.answer {
                Ok(value) => Ok(value.clone().into()),
                Err(code) => Err(LinkError::Refused(HostError::new(*code, "refused"))),
            }
        }

        fn predates_current_protocol(&self) -> bool {
            self.older
        }
    }

    fn read(answer: Result<serde_json::Value, ErrorCode>, older: bool) -> ProfilesRead {
        read_profiles(&Answering { answer, older })
    }

    #[test]
    fn a_node_that_predates_profiles_reads_as_keeping_none() {
        assert!(matches!(
            read(Err(ErrorCode::InvalidRequest), true),
            ProfilesRead::Unsupported
        ));
        assert!(matches!(
            read(Err(ErrorCode::Unsupported), false),
            ProfilesRead::Unsupported
        ));
        // A current node that could not read the call is a failure, not a
        // machine with no profiles.
        assert!(matches!(
            read(Err(ErrorCode::InvalidRequest), false),
            ProfilesRead::Unreadable(_)
        ));
        assert!(matches!(
            read(Ok(json!({"version": 3})), false),
            ProfilesRead::Listed(list) if list.version == 3
        ));
    }
}
