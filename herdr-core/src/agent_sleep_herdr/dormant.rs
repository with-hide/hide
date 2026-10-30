//! Worker boundaries for a sleeping session whose old pane no longer exists.
use super::*;
use crate::agent_sleep::{DormantRecord, SleepId};
use crate::checkout_owner::OwnerOpen;
use crate::engine::Gate;
use crate::live::LiveContext;
use crate::sidebar::SessionSnapshotPayload;

#[derive(Clone)]
pub(crate) struct DormantWork {
    pub id: SleepId,
    pub record: DormantRecord,
}

pub(crate) enum DormantStartOutcome {
    Started,
    NotStarted,
    Unknown,
}

pub(crate) enum DormantTabOutcome {
    Created(String, String, Box<SessionSnapshotPayload>),
    NotCreated(&'static str),
    Unknown,
}

pub(crate) struct DormantStatus {
    pub snapshot: SessionSnapshotPayload,
    pub wake_tab: Result<Option<(String, String)>, String>,
}

/// Observes current topology and exact intent markers. Never creates,
/// repairs, moves, closes or starts anything, including after a restart.
pub(crate) fn spawn_dormant_status(
    context: LiveContext,
    work: DormantWork,
    generation: u64,
    owner: Option<OwnerOpen>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("hide-sleep-status".into())
        .spawn(move || {
            let result = crate::live::fetch_session(context.engine.as_ref())
                .map_err(|_| {
                    crate::diagnostic!(serde_json::json!({
                        "component": "agent_sleep", "kind": "agent_sleep.status_failed",
                        "sleep_id": work.id.as_str(),
                    }));
                    "The current session could not be read".to_owned()
                })
                .map(|snapshot| {
                    let wake_tab = if work.record.closed {
                        owner
                            .as_ref()
                            .ok_or_else(|| "The saved checkout is unavailable".to_owned())
                            .and_then(|owner| {
                                crate::live::inspect_sleep_tab(
                                    context.engine.as_ref(),
                                    work.id.as_str(),
                                    &work.record.context,
                                    owner,
                                    &snapshot,
                                )
                            })
                    } else {
                        Ok(None)
                    };
                    DormantStatus { snapshot, wake_tab }
                });
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .ingest_dormant_status(&work, generation, result);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|_| "The sleeping-session status worker could not start".into())
}

pub(crate) fn spawn_dormant_tab(
    context: LiveContext,
    work: DormantWork,
    owner: OwnerOpen,
) -> Result<(), String> {
    thread::Builder::new()
        .name("hide-sleep-tab".into())
        .spawn(move || {
            if !work_is_current(&context, &work) {
                return;
            }
            let route = crate::live::confirm_session_launch(
                context.node.as_ref(),
                &work.record.kind,
                &work.record.native_session_id,
                Some(&work.record.cwd),
                work.record.source_reference.as_ref(),
            );
            let result = if route.is_err() {
                DormantTabOutcome::NotCreated("The saved conversation has no confirmed native route. Check its records before retrying; no tab was created.")
            } else if !crate::node_access::is_directory(context.node.as_ref(), &work.record.cwd) {
                DormantTabOutcome::NotCreated("The saved working folder is unavailable. Restore it before retrying; no tab was created.")
            } else {
                match crate::live::create_sleep_tab(
                    context.engine.gated(wake_gate(&context, &work)).as_ref(),
                    context.node.as_ref(),
                    work.id.as_str(),
                    &work.record.context,
                    &owner,
                    &work.record.cwd,
                ) {
                    Ok((tab, pane, snapshot)) => {
                        DormantTabOutcome::Created(tab, pane, Box::new(snapshot))
                    }
                    Err(_) => {
                        crate::diagnostic!(serde_json::json!({
                            "component": "agent_sleep", "kind": "agent_sleep.tab_unconfirmed",
                            "sleep_id": work.id.as_str(),
                        }));
                        DormantTabOutcome::Unknown
                    }
                }
            };
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .ingest_dormant_tab(&work, result);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|_| "The sleeping-session tab worker could not start".into())
}

pub(crate) fn spawn_dormant_start(
    context: LiveContext,
    work: DormantWork,
    args: Vec<String>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("hide-sleep-resume".into())
        .spawn(move || {
            if !work_is_current(&context, &work) {
                return;
            }
            let outcome = start_dormant(
                context.engine.gated(wake_gate(&context, &work)).as_ref(),
                context.node.as_ref(),
                &work,
                args,
            );
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .ingest_dormant_start(&work, outcome);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|_| "The sleeping-session resume worker could not start".into())
}

fn work_is_current(context: &LiveContext, work: &DormantWork) -> bool {
    context.runtime.upgrade().is_some_and(|runtime| {
        runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .dormant_work_is_current(work)
    })
}

/// Shell readiness and topology reads may block. Check the exact saved
/// intent again at every request, once Herdr is reached and just before the
/// request is written, rather than only at worker entry. The check is
/// released before I/O and never holds the runtime mutex over it.
#[derive(Clone)]
enum CurrentIntent {
    Wake(Arc<DormantWork>),
    Close(Arc<crate::live::CloseEffectRequest>),
}

fn wake_gate(context: &LiveContext, work: &DormantWork) -> Gate<'static> {
    let context = context.clone();
    let intent = CurrentIntent::Wake(Arc::new(work.clone()));
    Gate::new(|| Ok(()), move || still_current(&context, &intent))
}

/// A saved close goes on only while its sleeping session still holds it and
/// the session's route is confirmed, checked before Herdr is reached; and
/// only while the close is still the current intent, checked again before
/// each request is written.
pub(crate) fn dormant_close_gate(
    context: &LiveContext,
    effect: &crate::live::CloseEffectRequest,
) -> Gate<'static> {
    let before = context.clone();
    let key = effect.key.clone();
    let at_send = context.clone();
    let intent = CurrentIntent::Close(Arc::new(effect.clone()));
    Gate::new(
        move || close_route_confirmed(&before, &key),
        move || still_current(&at_send, &intent),
    )
}

fn close_route_confirmed(context: &LiveContext, key: &str) -> Result<(), EngineError> {
    let record = context
        .runtime
        .upgrade()
        .and_then(|runtime| {
            runtime
                .lock()
                .ok()?
                .snapshot()
                .ui_state
                .agent_sleep
                .dormant
                .values()
                .find(|record| record.close_key.as_deref() == Some(key))
                .cloned()
        })
        .ok_or_else(|| EngineError::Refused {
            code: "sleep_intent_superseded".into(),
            message: "Sleeping session changed".into(),
        })?;
    crate::live::confirm_session_launch(
        context.node.as_ref(),
        &record.kind,
        &record.native_session_id,
        Some(&record.cwd),
        record.source_reference.as_ref(),
    )
    .map_err(|reason| EngineError::Refused {
        code: reason,
        message: "Session route is unconfirmed".into(),
    })
}

/// A superseded intent stops the request as its write would have failed:
/// nothing reached Herdr, and the caller reads the result as unknown.
fn still_current(context: &LiveContext, intent: &CurrentIntent) -> Result<(), EngineError> {
    if intent_is_current(context, intent) {
        Ok(())
    } else {
        Err(EngineError::Transport(
            "request could not be written: sleep intent superseded".into(),
        ))
    }
}

fn intent_is_current(context: &LiveContext, intent: &CurrentIntent) -> bool {
    context.runtime.upgrade().is_some_and(|runtime| {
        let runtime = runtime.lock().unwrap_or_else(|error| error.into_inner());
        match intent {
            CurrentIntent::Wake(work) => runtime.dormant_work_is_current(work),
            CurrentIntent::Close(effect) => runtime.dormant_close_effect_is_current(effect),
        }
    })
}

/// The name a dormant session wakes under. One that held a registration
/// wakes under the name Herdr knew it by, so the registration the wake
/// continues still names its agent; one that held none is a new execution of
/// the conversation, named as a fork.
pub(crate) fn wake_name(work: &DormantWork) -> String {
    work.record.registration.as_ref().map_or_else(
        || crate::fork::fork_name("sleep", work.id.as_str()),
        |registration| registration.actor.name.clone(),
    )
}

fn start_dormant(
    engine: &dyn Engine,
    node: &dyn NodeLink,
    work: &DormantWork,
    args: Vec<String>,
) -> DormantStartOutcome {
    let Some(pane) = work.record.wake_pane_id.as_deref() else {
        return DormantStartOutcome::NotStarted;
    };
    if !crate::node_access::is_directory(node, &work.record.cwd) {
        return DormantStartOutcome::NotStarted;
    }
    let name = wake_name(work);
    let start = match engine.prepare_start(AgentLaunch {
        pane_id: pane.to_owned(),
        name,
        kind: work.record.kind.clone(),
        args,
        codex_daemon: crate::codex_launch::CodexDaemon::Unsupported,
    }) {
        Ok(start) => start,
        Err(_) => return DormantStartOutcome::NotStarted,
    };
    let confirm = || {
        crate::live::confirm_session_launch(
            node,
            &work.record.kind,
            &work.record.native_session_id,
            Some(&work.record.cwd),
            work.record.source_reference.as_ref(),
        )
    };
    let correlation = format!("hide:{}:resume", work.id.as_str());
    let answer_timeout = Duration::from_millis(AGENT_START_TIMEOUT_MS + 5_000);
    // The old pane closed a moment ago, and Herdr refuses its agent's name
    // until it has forgotten that agent.
    let started = if work.record.registration.is_some() {
        crate::agent_start::start_at_shell_reusing_name(
            engine,
            Some(node),
            &correlation,
            crate::agent_start::ShellStart::Plain,
            &start,
            answer_timeout,
            &confirm,
        )
    } else {
        crate::agent_start::start_at_shell_checked(
            engine,
            Some(node),
            &correlation,
            crate::agent_start::ShellStart::Plain,
            &start,
            answer_timeout,
            &confirm,
        )
    };
    match started {
        // An answer in a shape the start does not promise is still Herdr
        // taking the start.
        Ok(_) | Err(StartError::Herdr(EngineError::Answer(_))) => {
            match super::report_resumed_session(
                engine,
                pane,
                &work.record.kind,
                &work.record.native_session_id,
            ) {
                Ok(()) => DormantStartOutcome::Started,
                Err(error) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "agent_sleep", "kind": "agent_sleep.session_report_failed",
                        "sleep_id": work.id.as_str(), "pane_id": pane, "message": error.to_string(),
                    }));
                    DormantStartOutcome::Unknown
                }
            }
        }
        Err(StartError::NotStarted { .. })
        | Err(StartError::ShellStarting(_))
        | Err(StartError::Herdr(EngineError::Refused { .. })) => DormantStartOutcome::NotStarted,
        Err(StartError::Herdr(_)) => {
            crate::diagnostic!(serde_json::json!({
                "component": "agent_sleep", "kind": "agent_sleep.resume_unconfirmed",
                "sleep_id": work.id.as_str(),
            }));
            DormantStartOutcome::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_sleep::{DormantRegistration, fixture_record};
    use crate::fake_herdr::FakeHerdr;
    use serde_json::json;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Runs one dormant start against a Herdr that refuses the first
    /// `refusals` starts with `agent_name_taken` and returns the outcome with
    /// the name each `agent.start` carried.
    fn wake(registered: bool, refusals: usize) -> (DormantStartOutcome, Vec<String>) {
        let folder = tempfile::tempdir().unwrap();
        let mut record = fixture_record("w1:p1");
        record.cwd = folder.path().to_str().unwrap().into();
        record.wake_pane_id = Some("w1:p1".into());
        if registered {
            let actor = crate::delivery::Actor {
                pane_id: "w1:p1".into(),
                name: "reviewer".into(),
                kind: "claude".into(),
                device_id: "test-node".into(),
                session: Some("digest".into()),
            };
            record.registration = Some(DormantRegistration {
                id: "agent-2".into(),
                actor,
            });
        }
        let names = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&names);
        let starts = Arc::new(AtomicUsize::new(0));
        let herdr =
            FakeHerdr::start_with_errors("dormant-wake-name", move |method, params| match method {
                "pane.process_info" => Ok(json!({"type":"pane_process_info","process_info":{
                    "pane_id":"w1:p1","shell_pid":42,"foreground_process_group_id":42,
                    "foreground_processes":[{"pid":42,"name":"zsh"}]
                }})),
                "agent.start" => {
                    seen.lock()
                        .unwrap()
                        .push(params["name"].as_str().unwrap().to_owned());
                    if starts.fetch_add(1, Ordering::SeqCst) < refusals {
                        Err((
                            "agent_name_taken".into(),
                            "agent name reviewer is already used".into(),
                        ))
                    } else {
                        Ok(json!({"type":"agent_started","argv":[],"agent":{
                            "pane_id":"w1:p1","terminal_id":"term_1","workspace_id":"w1",
                            "tab_id":"w1:t1","focused":false,"agent_status":"idle","revision":1
                        }}))
                    }
                }
                // A name Herdr still holds after it is read is its refusal.
                "agent.get" => Ok(json!({"type":"agent_info","agent":{
                    "pane_id":"w1:p2","terminal_id":"term_2","workspace_id":"w1",
                    "tab_id":"w1:t2","name":"held","focused":false,
                    "agent_status":"unknown","revision":0,"launch_pending":true
                }})),
                other => panic!("unexpected {other}"),
            });
        let work = DormantWork {
            id: SleepId::new().unwrap(),
            record,
        };
        let node = hide_node::Local::of_process();
        let outcome = start_dormant(
            herdr.engine().as_ref(),
            &node,
            &work,
            vec!["--resume".into(), work.record.native_session_id.clone()],
        );
        let names = names.lock().unwrap().clone();
        (outcome, names)
    }

    #[test]
    fn a_session_that_held_a_registration_wakes_under_its_name_and_waits_for_herdr_to_release_it() {
        let (outcome, names) = wake(true, 2);
        assert!(matches!(outcome, DormantStartOutcome::Started));
        assert_eq!(names, ["reviewer", "reviewer", "reviewer"]);
    }

    #[test]
    fn a_session_that_held_none_wakes_as_a_fork_and_takes_a_refusal_as_the_answer() {
        let (outcome, names) = wake(false, 0);
        assert!(matches!(outcome, DormantStartOutcome::Started));
        assert!(names[0].starts_with("fork-sleep-"), "{names:?}");
        let (outcome, names) = wake(false, 1);
        assert!(matches!(outcome, DormantStartOutcome::NotStarted));
        assert_eq!(names.len(), 1);
    }

    /// What `report_resumed_session` does for a record of `kind` against a
    /// Herdr that accepts or refuses session reports, with the requests it got.
    fn report_of(kind: &str, accepts: bool) -> (Result<(), EngineError>, Vec<serde_json::Value>) {
        let mut record = fixture_record("w1:p1");
        record.kind = kind.into();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let reported = Arc::clone(&reports);
        let herdr =
            FakeHerdr::start_with_errors("dormant-session-report", move |method, params| {
                match method {
                    "pane.report_agent_session" => {
                        reported.lock().unwrap().push(params.clone());
                        if accepts {
                            Ok(json!({"type":"ok"}))
                        } else {
                            Err(("invalid_request".into(), "session refused".into()))
                        }
                    }
                    other => panic!("unexpected {other}"),
                }
            });
        let work = DormantWork {
            id: SleepId::new().unwrap(),
            record,
        };
        let result = super::super::report_resumed_session(
            herdr.engine().as_ref(),
            "w1:p9",
            &work.record.kind,
            &work.record.native_session_id,
        );
        let reports = reports.lock().unwrap().clone();
        (result, reports)
    }

    #[test]
    fn a_retired_cursor_session_is_never_reported_as_a_new_live_session() {
        for accepts in [true, false] {
            let (result, reports) = report_of("cursor", accepts);
            result.unwrap();
            assert!(
                reports.is_empty(),
                "a retired session must not acquire a live integration"
            );
        }
    }

    #[test]
    fn an_agent_whose_resume_reports_its_own_session_is_never_reported_for() {
        for kind in ["claude", "codex", "grok", "opencode", "pi", "omp"] {
            let (result, reports) = report_of(kind, true);
            result.unwrap();
            assert!(reports.is_empty(), "{kind}: {reports:?}");
        }
    }

    #[test]
    fn an_unknown_session_kind_has_no_live_integration_report() {
        let (result, reports) = report_of("future-agent", false);
        result.unwrap();
        assert!(reports.is_empty());
    }
}
