//! The one place label analyses run (PRD labels-in-hided D-05).
//!
//! One thread for the whole core takes jobs in arrival order and runs them
//! one at a time through a `hide-ai` router built from the operator's
//! current provider and model choice. Each engine's worker hands in at
//! most one job at a time, so arrival order is a round robin between servers
//! and a busy server cannot starve another. The router's own one-in-flight
//! budget is therefore never exceeded, which would otherwise park a second
//! server's pane for the ten-minute provider wait.
//!
//! A changed choice rebuilds the router before the next job; the job already
//! running finishes on the router it started with (B17). A profile the
//! router asks that is removed or edited lets it go at once, even with no
//! job to come, so its backend ends with it (PRD agent-profiles-phase2 B21). `shutdown` cancels
//! the running request, which ends its provider child, and joins the thread,
//! so no request or provider process outlives the daemon (B22). With agent
//! summaries off (PRD overview-request-view D-11) a queued job is answered
//! as stopped without running, and `cancel_running` ends the one running.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use hide_ai::{AiRequest, AiResult, AiRouter, AiSettings, CancelToken, ProviderId};
use serde_json::json;

use super::analysis::{Analysis, AnalysisFailure, AnalysisPhase};
use super::context_label;
use crate::ai::RouterFacts;

pub(crate) type AnalysisResult = Result<(ProviderId, Analysis), AnalysisFailure>;

pub(crate) struct AnalysisJob {
    /// The turn boundary the request asks about, which decides how its
    /// answer is read.
    pub(crate) phase: AnalysisPhase,
    pub(crate) request: AiRequest,
    /// Called once on the analyzer thread with the outcome, including when
    /// the analyzer shuts down before running it.
    pub(crate) done: Box<dyn FnOnce(AnalysisResult) + Send>,
}

pub(crate) type SettingsSource = Box<dyn Fn() -> AiSettings + Send>;
pub(crate) type RouterFactory = Box<dyn Fn(&AiSettings) -> Arc<AiRouter> + Send>;
/// What a router of the settings would be told besides them
/// (`crate::ai::AiMachines::facts`).
pub(crate) type Reach = Box<dyn Fn(&AiSettings) -> RouterFacts + Send>;

/// What the analyzer thread is handed: a job, or word that the profiles a
/// router may ask moved.
enum Message {
    Job(Box<AnalysisJob>),
    ProfilesMoved,
}

pub(crate) struct LabelAnalyzer {
    queue: Mutex<Option<Sender<Message>>>,
    cancel: CancelToken,
    /// The running job's own token, cancelled on shutdown too.
    running: Arc<Mutex<Option<CancelToken>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl LabelAnalyzer {
    pub(crate) fn spawn(
        settings: SettingsSource,
        standing: Arc<crate::ai::AiStanding>,
        machines: crate::ai::AiMachines,
        gate: Arc<hide_ai::UsageLimits>,
        usage: Arc<dyn hide_ai::AiUsageSink>,
    ) -> Result<Self, String> {
        let reach = machines.clone();
        Self::spawn_with(
            settings,
            Box::new(move |settings| {
                Arc::new(crate::ai::labels_router(&machines, settings, &gate, &usage))
            }),
            Box::new(move |settings| reach.facts(settings)),
            standing,
        )
    }

    pub(crate) fn spawn_with(
        settings: SettingsSource,
        router: RouterFactory,
        reach: Reach,
        standing: Arc<crate::ai::AiStanding>,
    ) -> Result<Self, String> {
        let (sender, receiver) = channel();
        let cancel = CancelToken::new();
        let running = Arc::new(Mutex::new(None));
        let thread = Running {
            stopping: cancel.clone(),
            job: Arc::clone(&running),
        };
        let worker = std::thread::Builder::new()
            .name("hide-core-labels-analyzer".to_owned())
            .spawn(move || run(receiver, settings, router, reach, standing, thread))
            .map_err(|error| format!("label analyzer could not be started: {error}"))?;
        Ok(Self {
            queue: Mutex::new(Some(sender)),
            cancel,
            running,
            worker: Mutex::new(Some(worker)),
        })
    }

    /// Queues a job. After shutdown the job is answered at once as stopped,
    /// so its pane never stays in flight.
    pub(crate) fn submit(&self, job: AnalysisJob) {
        let queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        let refused = match queue.as_ref() {
            Some(sender) => match sender.send(Message::Job(Box::new(job))) {
                Ok(()) => None,
                Err(error) => match error.0 {
                    Message::Job(job) => Some(*job),
                    Message::ProfilesMoved => None,
                },
            },
            None => Some(job),
        };
        drop(queue);
        if let Some(job) = refused {
            (job.done)(Err(AnalysisFailure::Stopped));
        }
    }

    /// Says a profile moved: the thread lets its router go when that
    /// router asks it, so the backend it kept is released there, off the
    /// runtime's lock. Only a send, so it may be called under that lock.
    pub(crate) fn profiles_moved(&self) {
        if let Some(sender) = self
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            let _ = sender.send(Message::ProfilesMoved);
        }
    }

    /// Ends the request running now, if any; it is answered as stopped.
    pub(crate) fn cancel_running(&self) {
        if let Some(job) = lock(&self.running).as_ref() {
            job.cancel();
        }
    }

    /// Cancels the running request, answers the queued ones and joins.
    pub(crate) fn shutdown(&self) {
        self.cancel.cancel();
        self.cancel_running();
        self.queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(worker) = worker
            && worker.join().is_err()
        {
            crate::diagnostic!(json!({"component": "labels", "kind": "analyzer.join_failed"}));
        }
    }
}

impl Drop for LabelAnalyzer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// What the analyzer thread is stopped by: the daemon stopping, or the one
/// job it runs being cancelled.
struct Running {
    stopping: CancelToken,
    job: Arc<Mutex<Option<CancelToken>>>,
}

fn run(
    receiver: Receiver<Message>,
    settings: SettingsSource,
    build: RouterFactory,
    reach: Reach,
    standing: Arc<crate::ai::AiStanding>,
    running: Running,
) {
    let mut router: Option<(AiSettings, RouterFacts, Arc<AiRouter>)> = None;
    while let Ok(message) = receiver.recv() {
        let job = match message {
            Message::Job(job) => *job,
            Message::ProfilesMoved => {
                // Dropped here, on this thread: its backends' release
                // calls never run under the runtime's lock.
                if router
                    .as_ref()
                    .is_some_and(|(built_for, built_facts, _)| reach(built_for) != *built_facts)
                {
                    router = None;
                    crate::diagnostic!(json!({
                        "component": "labels",
                        "kind": "analyzer.router_released",
                        "reason": "profile_moved",
                    }));
                }
                continue;
            }
        };
        let current = settings();
        if running.stopping.is_cancelled()
            || !current.agent_summary
            || !current.enabled
            || !current.chosen
        {
            (job.done)(Err(AnalysisFailure::Stopped));
            continue;
        }
        // The switch is not the router's: turning it does not rebuild one.
        // Everything the router was built from is: the agents it asks, in
        // order (Runs on, then the fallback list), and the model each is
        // asked for. A removed fallback agent must stop receiving
        // conversation text on the next job, and an added one must be tried.
        // So is the link of each device its agents run on: a router keeps
        // what it was told of a device that was away for its availability
        // window, so a device on a new link, even one that dropped and came
        // back between two jobs, is asked at the next job rather than after
        // that window. So is each profile item's version: a removed profile
        // is passed over and an edited one runs as it is now.
        let reached = reach(&current);
        let same_router = |built_for: &AiSettings, built_reach: &RouterFacts| {
            built_for.routes_like(&current) && *built_reach == reached
        };
        let active = match router.as_ref() {
            Some((built_for, built_reach, router)) if same_router(built_for, built_reach) => {
                Arc::clone(router)
            }
            _ => {
                if router.is_some() {
                    crate::diagnostic!(json!({
                        "component": "labels",
                        "kind": "analyzer.settings_changed",
                        "provider": current.runs_on.provider.to_string(),
                        "profile": current.runs_on.profile_id(),
                    }));
                }
                let built = build(&current);
                router = Some((current, reached, Arc::clone(&built)));
                built
            }
        };
        let cancel = CancelToken::new();
        *lock(&running.job) = Some(cancel.clone());
        // A shutdown or the switch turned off between the checks above and
        // the token's install found no token to cancel; look once more.
        let latest = settings();
        if running.stopping.is_cancelled()
            || !latest.agent_summary
            || !latest.enabled
            || !latest.chosen
        {
            cancel.cancel();
        }
        // The outcome must arrive whatever happens on this thread: a panic
        // that escaped would leave the pane in flight forever.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            analyze(&active, job.phase, &job.request, &cancel)
        }))
        .unwrap_or_else(|_| {
            Err(AnalysisFailure::Worker(
                "analysis_worker_panicked".to_owned(),
            ))
        });
        lock(&running.job).take();
        standing.observe(&active);
        (job.done)(result);
    }
}

fn analyze(
    router: &AiRouter,
    phase: AnalysisPhase,
    request: &AiRequest,
    cancel: &CancelToken,
) -> AnalysisResult {
    let AiResult { provider, value } = router.execute(request, cancel).map_err(|error| {
        if cancel.is_cancelled() {
            AnalysisFailure::Stopped
        } else {
            AnalysisFailure::Provider(error)
        }
    })?;
    // Every label request names its language; one without it is a bug in
    // the request's maker, not an answer to judge.
    let language = request
        .output_language
        .ok_or_else(|| AnalysisFailure::Worker("label_request_without_language".to_owned()))?;
    let analysis = context_label::parse(phase, language, value)
        .map_err(|error| AnalysisFailure::Invalid(format!("{error:#}")))?;
    Ok((provider, analysis))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use hide_ai::{
        AiBackend, AiError, AiResponse, Availability, FallbackEntry, ModelCatalog, NoopLogSink,
    };

    use super::*;

    /// A provider that records that it was asked and cannot answer, so the
    /// router moves on to the next agent of its list.
    struct Down {
        id: ProviderId,
        asked: Arc<Mutex<Vec<(ProviderId, String)>>>,
        model: String,
    }

    impl AiBackend for Down {
        fn id(&self) -> ProviderId {
            self.id
        }
        fn availability(&self) -> Availability {
            Availability::Ready
        }
        fn models(&self) -> ModelCatalog {
            ModelCatalog::plain([self.model.clone()])
        }
        fn execute(&self, _: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
            lock(&self.asked).push((self.id, self.model.clone()));
            Err(AiError::ProviderUnavailable("down".to_owned()))
        }
    }

    struct Fixture {
        settings: Arc<Mutex<AiSettings>>,
        asked: Arc<Mutex<Vec<(ProviderId, String)>>>,
        analyzer: LabelAnalyzer,
    }

    impl Fixture {
        fn new(start: AiSettings) -> Self {
            let settings = Arc::new(Mutex::new(start));
            let asked = Arc::new(Mutex::new(Vec::new()));
            let source = Arc::clone(&settings);
            let log = Arc::clone(&asked);
            let analyzer = LabelAnalyzer::spawn_with(
                Box::new(move || lock(&source).clone()),
                Box::new(move |settings| {
                    let models = settings.default_models();
                    let backends: Vec<Arc<dyn AiBackend>> = hide_ai::PROVIDERS
                        .iter()
                        .map(|provider| {
                            Arc::new(Down {
                                id: *provider,
                                asked: Arc::clone(&log),
                                model: models[provider].clone(),
                            }) as Arc<dyn AiBackend>
                        })
                        .collect();
                    Arc::new(AiRouter::new(
                        backends,
                        settings.router_config(),
                        Arc::new(NoopLogSink),
                    ))
                }),
                Box::new(|_| RouterFacts::default()),
                Arc::new(crate::ai::AiStanding::default()),
            )
            .unwrap();
            Self {
                settings,
                asked,
                analyzer,
            }
        }

        /// Runs one job to its outcome and returns the agents it asked, in
        /// order, each with the model it was asked for.
        fn job(&self) -> Vec<(ProviderId, String)> {
            lock(&self.asked).clear();
            let (done, outcome) = channel();
            self.analyzer.submit(AnalysisJob {
                phase: AnalysisPhase::TurnEnd,
                request: context_label::request(
                    AnalysisPhase::TurnEnd,
                    hide_ai::OutputLanguage::English,
                    "pane",
                    format!("job-{}", rand_id()),
                    "context",
                ),
                done: Box::new(move |result| {
                    let _ = done.send(result.is_err());
                }),
            });
            outcome
                .recv_timeout(Duration::from_secs(30))
                .expect("the job is answered");
            lock(&self.asked).clone()
        }

        fn change(&self, edit: impl FnOnce(&mut AiSettings)) {
            edit(&mut lock(&self.settings));
        }
    }

    /// Where a test's device link is found, with its generation; `None`
    /// while the device is not connected.
    type Slot = Arc<Mutex<Option<crate::ai::DeviceLink>>>;

    /// The line of the label a device's agent makes.
    const DEVICE_LINE: &str = "Made on the device";

    /// A device's node that answers its agent as signed in and counts the
    /// requests it ran.
    #[derive(Default)]
    struct DeviceNode {
        executed: std::sync::atomic::AtomicUsize,
        /// When set, the link drops while it runs a request: its slot
        /// empties and the request ends unanswered.
        drops: Mutex<Option<Slot>>,
    }

    impl DeviceNode {
        fn executed(&self) -> usize {
            self.executed.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// Puts this node in `slot` as the device's link `generation`.
        fn connect(self: &Arc<Self>, slot: &Slot, generation: u64) {
            *lock(slot) = Some(crate::ai::DeviceLink {
                generation,
                link: Arc::clone(self) as Arc<dyn crate::node_access::NodeLink>,
            });
        }
    }

    impl crate::node_access::NodeLink for DeviceNode {
        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            _timeout: Duration,
        ) -> Result<hide_node_link::link::LinkAnswer, crate::node_access::LinkError> {
            use hide_node_link::protocol::Call;
            let value = match call {
                Call::AiAvailability { .. } => serde_json::to_value(hide_node_link::ai::Logged {
                    value: Availability::Ready,
                    log: Vec::new(),
                }),
                Call::AiExecute { .. } => {
                    if let Some(slot) = lock(&self.drops).take() {
                        lock(&slot).take();
                        return Err(crate::node_access::LinkError::NotConnected(
                            "the device's link dropped".to_owned(),
                        ));
                    }
                    self.executed
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    serde_json::to_value(hide_node_link::ai::Logged {
                        value: Ok::<AiResponse, AiError>(AiResponse {
                            value: serde_json::json!({
                                "goal": "Device label", "goal_update": "replace",
                                "closed_result": "", "requests": [], "line": DEVICE_LINE,
                                "end": "done",
                                "work": {"missed": [], "resolved": [], "artifacts": []},
                            }),
                            usage: hide_ai::AiUsage::default(),
                        }),
                        log: Vec::new(),
                    })
                }
                _ => Ok(serde_json::Value::Null),
            };
            Ok(hide_node_link::link::LinkAnswer::Parsed(value.unwrap()))
        }
    }

    /// A label analyzer whose agent runs on the device found in `slot`, and
    /// the core's own node, which no test expects to be asked.
    fn device_analyzer(slot: &Slot) -> (LabelAnalyzer, Arc<DeviceNode>) {
        let own = Arc::new(DeviceNode::default());
        let links = Arc::clone(slot);
        let machines = crate::ai::AiMachines {
            own: Arc::clone(&own) as Arc<dyn crate::node_access::NodeLink>,
            devices: Arc::new(move |_| lock(&links).clone()),
            profiles: Arc::default(),
        };
        let mut settings = AiSettings::default();
        settings.set_runs_on(ProviderId::CODEX.into());
        settings.place(&ProviderId::CODEX.into(), Some("wsl-ubuntu".to_owned()));
        settings.chosen = true;
        let analyzer = LabelAnalyzer::spawn(
            Box::new(move || settings.clone()),
            Arc::new(crate::ai::AiStanding::default()),
            machines,
            Arc::new(hide_ai::UsageLimits::new()),
            crate::usage_ledger::ai_usage::AiUsageQueue::new() as Arc<dyn hide_ai::AiUsageSink>,
        )
        .unwrap();
        (analyzer, own)
    }

    /// Runs one label job of `analyzer` and answers the agent that made
    /// its label and the label's line, or why none was made.
    fn run_job(analyzer: &LabelAnalyzer) -> Result<(ProviderId, String), String> {
        let (done, outcome) = channel();
        analyzer.submit(AnalysisJob {
            phase: AnalysisPhase::TurnEnd,
            request: context_label::request(
                AnalysisPhase::TurnEnd,
                hide_ai::OutputLanguage::English,
                "pane",
                format!("job-{}", rand_id()),
                "context",
            ),
            done: Box::new(move |result| {
                let _ = done.send(
                    result
                        .map(|(provider, analysis)| (provider, analysis.line))
                        .map_err(|failure| format!("{failure:?}")),
                );
            }),
        });
        outcome
            .recv_timeout(Duration::from_secs(30))
            .expect("the job is answered")
    }

    /// PRD wsl-device B13: a label job that found the device its agent runs
    /// on away runs there at the next job once the device is back, with
    /// nothing chosen again. The router built while it was away keeps that
    /// answer for its availability window; the device's new link builds it
    /// again.
    #[test]
    fn a_label_job_runs_on_its_device_at_the_next_job_once_the_device_is_back() {
        let slot = Slot::default();
        let (analyzer, own) = device_analyzer(&slot);
        assert!(run_job(&analyzer).is_err(), "away: no label is made");
        let device = Arc::new(DeviceNode::default());
        device.connect(&slot, 1);
        assert_eq!(
            run_job(&analyzer),
            Ok((ProviderId::CODEX, DEVICE_LINE.to_owned())),
            "the next job's label is made on the device"
        );
        assert_eq!(device.executed(), 1);
        assert_eq!(own.executed(), 0, "nothing ran on the core's machine");
    }

    /// PRD wsl-device B13: a device's link that drops while a label job runs
    /// on it, and is back before the next job, is asked at that next job on
    /// its new link. The analyzer found the device connected at both jobs;
    /// what tells the two links apart is their generation.
    #[test]
    fn a_label_job_runs_on_a_device_that_dropped_mid_job_and_came_back_before_the_next() {
        let slot = Slot::default();
        let first = Arc::new(DeviceNode::default());
        *lock(&first.drops) = Some(Arc::clone(&slot));
        first.connect(&slot, 1);
        let (analyzer, own) = device_analyzer(&slot);
        assert!(run_job(&analyzer).is_err(), "the dropped job made no label");
        assert!(lock(&slot).is_none(), "the link dropped during the job");
        assert_eq!(first.executed(), 0);
        let next = Arc::new(DeviceNode::default());
        next.connect(&slot, 2);
        assert_eq!(
            run_job(&analyzer),
            Ok((ProviderId::CODEX, DEVICE_LINE.to_owned())),
            "the next job's label is made on the new link"
        );
        assert_eq!(next.executed(), 1);
        assert_eq!(own.executed(), 0, "nothing ran on the core's machine");
    }

    /// The account Claude Code's built-in default runs in on [`ProfileNode`].
    const DEFAULT_ACCOUNT: &str = "default";

    /// A node that runs Claude Code's profiles as `accounts` lists them now,
    /// as a node does: a backend is built from its profile when its
    /// instance is first asked, and a profile the node no longer lists is
    /// refused as gone. It records the account each request ran in and each
    /// instance released.
    #[derive(Default)]
    struct ProfileNode {
        accounts: Mutex<std::collections::BTreeMap<String, String>>,
        built: Mutex<std::collections::BTreeMap<u64, String>>,
        ran: Mutex<Vec<String>>,
        released: Mutex<Vec<u64>>,
        /// Woken at each release.
        release: std::sync::Condvar,
    }

    impl ProfileNode {
        /// The instances built for an account, in the order they were.
        fn built_for(&self, account: &str) -> Vec<u64> {
            lock(&self.built)
                .iter()
                .filter(|(_, built)| built.as_str() == account)
                .map(|(instance, _)| *instance)
                .collect()
        }

        /// Waits, up to ten seconds, until `instance` was released.
        fn wait_released(&self, instance: u64) -> bool {
            let (released, _) = self
                .release
                .wait_timeout_while(lock(&self.released), Duration::from_secs(10), |released| {
                    !released.contains(&instance)
                })
                .unwrap_or_else(|error| error.into_inner());
            released.contains(&instance)
        }
    }

    impl crate::node_access::NodeLink for ProfileNode {
        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            _timeout: Duration,
        ) -> Result<hide_node_link::link::LinkAnswer, crate::node_access::LinkError> {
            use hide_node_link::protocol::Call;
            let answer =
                |value: serde_json::Value| Ok(hide_node_link::link::LinkAnswer::Parsed(value));
            let spec = match &call {
                Call::AiRelease { instance } => {
                    lock(&self.released).push(*instance);
                    self.release.notify_all();
                    return answer(serde_json::Value::Null);
                }
                Call::AiAvailability { backend } | Call::AiExecute { backend, .. } => {
                    backend.clone()
                }
                _ => return answer(serde_json::Value::Null),
            };
            let built = lock(&self.built).get(&spec.instance).cloned();
            let account = match built {
                Some(account) => account,
                None => {
                    let account =
                        match &spec.profile {
                            None => DEFAULT_ACCOUNT.to_owned(),
                            Some(profile) => lock(&self.accounts)
                                .get(&profile.0)
                                .cloned()
                                .ok_or_else(|| {
                                    crate::node_access::LinkError::Refused(
                                        hide_node_link::HostError::new(
                                            hide_node_link::ErrorCode::NotFound,
                                            format!("This machine has no profile {}", profile.0),
                                        ),
                                    )
                                })?,
                        };
                    lock(&self.built).insert(spec.instance, account.clone());
                    account
                }
            };
            let value = match call {
                Call::AiExecute { .. } => {
                    lock(&self.ran).push(account.clone());
                    serde_json::to_value(hide_node_link::ai::Logged {
                        value: Ok::<AiResponse, AiError>(AiResponse {
                            value: serde_json::json!({
                                "goal": "Profile label", "goal_update": "replace",
                                "closed_result": "", "requests": [], "line": account,
                                "end": "done",
                                "work": {"missed": [], "resolved": [], "artifacts": []},
                            }),
                            usage: hide_ai::AiUsage::default(),
                        }),
                        log: Vec::new(),
                    })
                }
                _ => serde_json::to_value(hide_node_link::ai::Logged {
                    value: Availability::Ready,
                    log: Vec::new(),
                }),
            };
            answer(value.unwrap())
        }
    }

    /// What the core's own machine lists of `node`'s profiles: `p-work` at
    /// `version` in an account folder of its own, and nothing once deleted.
    fn list_work(machines: &crate::ai::AiMachines, version: Option<u64>) {
        let work = hide_ai::Item::of(
            ProviderId::CLAUDE,
            Some(hide_agent_adapter::ProfileId("p-work".to_owned())),
        );
        machines.profiles.set(
            version
                .map(|version| {
                    let listed = crate::ai::ItemProfile {
                        version,
                        account: Some(hide_agent_adapter::AccountKey::Folder(
                            "/accounts/work/".to_owned(),
                        )),
                    };
                    ((None, work), listed)
                })
                .into_iter()
                .collect(),
        );
    }

    /// A label analyzer with Runs on Claude Code's `p-work` profile and the
    /// built-in default as its fallback, both on `node`, which keeps the
    /// account folder `/accounts/work`.
    fn profile_analyzer() -> (
        LabelAnalyzer,
        Arc<ProfileNode>,
        crate::ai::AiMachines,
        Arc<crate::ai::AiStanding>,
    ) {
        let node = Arc::new(ProfileNode::default());
        lock(&node.accounts).insert("p-work".to_owned(), "/accounts/work".to_owned());
        let machines =
            crate::ai::AiMachines::own(Arc::clone(&node) as Arc<dyn crate::node_access::NodeLink>);
        list_work(&machines, Some(1));
        let mut settings = AiSettings::default();
        settings.set_runs_on(hide_ai::Item::of(
            ProviderId::CLAUDE,
            Some(hide_agent_adapter::ProfileId("p-work".to_owned())),
        ));
        settings
            .add_fallback(ProviderId::CLAUDE.into(), None)
            .unwrap();
        settings.chosen = true;
        let standing = Arc::new(crate::ai::AiStanding::default());
        let analyzer = LabelAnalyzer::spawn(
            Box::new(move || settings.clone()),
            Arc::clone(&standing),
            machines.clone(),
            Arc::new(hide_ai::UsageLimits::new()),
            crate::usage_ledger::ai_usage::AiUsageQueue::new() as Arc<dyn hide_ai::AiUsageSink>,
        )
        .unwrap();
        (analyzer, node, machines, standing)
    }

    /// PRD agent-profiles-phase2 B21, engineering 14: a profile Runs on used
    /// and the operator then deleted leaves the analyzer's router at once,
    /// with no job to come, so the backend its node kept is released; the
    /// next job passes over it as a missing profile and the fallback answers,
    /// while the settings still name it.
    #[test]
    fn a_profile_deleted_after_use_releases_its_backend_and_the_next_job_passes_it_over() {
        let (analyzer, node, machines, standing) = profile_analyzer();
        assert_eq!(
            run_job(&analyzer),
            Ok((ProviderId::CLAUDE, "/accounts/work".to_owned()))
        );
        let work = node.built_for("/accounts/work");
        assert_eq!(work.len(), 1);

        lock(&node.accounts).remove("p-work");
        list_work(&machines, None);
        analyzer.profiles_moved();
        assert!(
            node.wait_released(work[0]),
            "the deleted profile's backend is released with no job to come"
        );
        assert_eq!(lock(&node.ran).len(), 1, "nothing was asked meanwhile");

        assert_eq!(
            run_job(&analyzer),
            Ok((ProviderId::CLAUDE, DEFAULT_ACCOUNT.to_owned())),
            "the fallback answers the next job"
        );
        assert_eq!(
            node.built_for("/accounts/work"),
            work,
            "the gone profile is not built again"
        );
        let standing = standing.current().expect("the job left a standing");
        assert_eq!(
            standing.selected,
            hide_ai::Item::of(
                ProviderId::CLAUDE,
                Some(hide_agent_adapter::ProfileId("p-work".to_owned()))
            )
        );
        assert_eq!(standing.reason, crate::ai::PROFILE_MISSING);
        assert_eq!(standing.using, Some(ProviderId::CLAUDE.into()));
    }

    /// PRD agent-profiles-phase2 B21: a profile edited after use, here moved
    /// to another account folder, is asked as it is now at the next job:
    /// the old backend is released and a new one runs in the new account.
    #[test]
    fn a_profile_edited_after_use_runs_in_its_new_account_at_the_next_job() {
        let (analyzer, node, machines, _) = profile_analyzer();
        assert_eq!(
            run_job(&analyzer),
            Ok((ProviderId::CLAUDE, "/accounts/work".to_owned()))
        );
        let before = node.built_for("/accounts/work");

        lock(&node.accounts).insert("p-work".to_owned(), "/accounts/work-2".to_owned());
        list_work(&machines, Some(2));
        analyzer.profiles_moved();
        assert!(node.wait_released(before[0]), "the old backend is released");

        assert_eq!(
            run_job(&analyzer),
            Ok((ProviderId::CLAUDE, "/accounts/work-2".to_owned())),
            "the next job runs in the edited profile's account"
        );
        assert_eq!(node.built_for("/accounts/work-2").len(), 1);
        assert_eq!(
            *lock(&node.ran),
            ["/accounts/work", "/accounts/work-2"],
            "nothing ran in the old account after the edit"
        );
    }

    /// Word of a profile the router does not ask leaves it as it is.
    #[test]
    fn a_profile_change_the_router_does_not_ask_keeps_its_backend() {
        let (analyzer, node, machines, _) = profile_analyzer();
        run_job(&analyzer).unwrap();
        let work = node.built_for("/accounts/work");
        let other = (
            (
                None,
                hide_ai::Item::of(
                    ProviderId::CLAUDE,
                    Some(hide_agent_adapter::ProfileId("p-other".to_owned())),
                ),
            ),
            crate::ai::ItemProfile {
                version: 1,
                account: None,
            },
        );
        let mut listed = std::collections::BTreeMap::from([other]);
        listed.insert(
            (
                None,
                hide_ai::Item::of(
                    ProviderId::CLAUDE,
                    Some(hide_agent_adapter::ProfileId("p-work".to_owned())),
                ),
            ),
            crate::ai::ItemProfile {
                version: 1,
                account: Some(hide_agent_adapter::AccountKey::Folder(
                    "/accounts/work/".to_owned(),
                )),
            },
        );
        assert!(machines.profiles.set(listed));
        analyzer.profiles_moved();
        run_job(&analyzer).unwrap();
        assert!(lock(&node.released).is_empty(), "the router was kept");
        assert_eq!(node.built_for("/accounts/work"), work);
    }

    fn rand_id() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    fn chosen_with_fallback() -> AiSettings {
        let mut settings = AiSettings::default();
        settings.set_runs_on(ProviderId::CLAUDE.into());
        settings
            .add_fallback(ProviderId::CODEX.into(), Some("first".to_owned()))
            .unwrap();
        settings
    }

    #[test]
    fn a_removed_fallback_agent_is_not_asked_by_the_next_job() {
        let fixture = Fixture::new(chosen_with_fallback());
        let before: Vec<ProviderId> = fixture.job().iter().map(|(id, _)| *id).collect();
        assert_eq!(before, [ProviderId::CLAUDE, ProviderId::CODEX]);
        fixture.change(|settings| settings.remove_fallback(&ProviderId::CODEX.into()));
        let after: Vec<ProviderId> = fixture.job().iter().map(|(id, _)| *id).collect();
        assert_eq!(after, [ProviderId::CLAUDE]);
    }

    #[test]
    fn an_added_fallback_agent_is_tried_by_the_next_job() {
        let mut start = AiSettings::default();
        start.set_runs_on(ProviderId::CLAUDE.into());
        let fixture = Fixture::new(start);
        let before: Vec<ProviderId> = fixture.job().iter().map(|(id, _)| *id).collect();
        assert_eq!(before, [ProviderId::CLAUDE]);
        fixture.change(|settings| {
            settings
                .add_fallback(ProviderId::GROK.into(), None)
                .unwrap();
        });
        let after: Vec<ProviderId> = fixture.job().iter().map(|(id, _)| *id).collect();
        assert_eq!(after, [ProviderId::CLAUDE, ProviderId::GROK]);
    }

    #[test]
    fn a_changed_fallback_model_is_the_one_the_next_job_asks_for() {
        let fixture = Fixture::new(chosen_with_fallback());
        assert_eq!(fixture.job()[1], (ProviderId::CODEX, "first".to_owned()));
        fixture.change(|settings| {
            settings.set_fallback_model(&ProviderId::CODEX.into(), Some("second".to_owned()));
        });
        assert_eq!(fixture.job()[1], (ProviderId::CODEX, "second".to_owned()));
    }

    #[test]
    fn a_changed_fallback_order_is_followed_by_the_next_job() {
        let mut start = chosen_with_fallback();
        start.add_fallback(ProviderId::GROK.into(), None).unwrap();
        let fixture = Fixture::new(start);
        let before: Vec<ProviderId> = fixture.job().iter().map(|(id, _)| *id).collect();
        assert_eq!(
            before,
            [ProviderId::CLAUDE, ProviderId::CODEX, ProviderId::GROK]
        );
        fixture.change(|settings| {
            settings.fallback = vec![
                FallbackEntry {
                    item: ProviderId::GROK.into(),
                    model: None,
                },
                FallbackEntry {
                    item: ProviderId::CODEX.into(),
                    model: Some("first".to_owned()),
                },
            ];
        });
        let order: Vec<ProviderId> = fixture.job().iter().map(|(id, _)| *id).collect();
        assert_eq!(
            order,
            [ProviderId::CLAUDE, ProviderId::GROK, ProviderId::CODEX]
        );
    }
}
