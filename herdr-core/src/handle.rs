//! The safe handle hided drives the runtime through: create it, dispatch
//! events into it, read snapshot deltas out of it, and be told when it
//! changed. Every call belongs to the thread that created the handle; hided
//! keeps that thread and forwards its Axum workers' requests over a channel.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, ThreadId};

use crate::model::CoreOptions;
use crate::runtime::{Runtime, validate_options};
use crate::{environment, live};

type Callback = Arc<dyn Fn() + Send + Sync>;

/// Thread-safe handle that fires the registered change callback. Cloned into
/// live worker threads so PTY and session-sync output can wake the daemon's
/// snapshot reader.
///
/// Announcements coalesce. The reader answers one by taking the whole delta,
/// so every change between an announcement and the read that answers it is
/// already carried by that read; announcing each one separately bought the
/// reader one wake and one turn waiting on the runtime mutex per PTY chunk.
#[derive(Clone)]
pub struct ChangeNotifier {
    registration: Arc<Mutex<Option<Callback>>>,
    /// True from an announcement until the read that answers it begins.
    announced: Arc<AtomicBool>,
}

impl ChangeNotifier {
    pub(crate) fn new() -> Self {
        Self {
            registration: Arc::new(Mutex::new(None)),
            announced: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn notify(&self) {
        let registration = lock_recover(&self.registration).clone();
        // Latching with nobody listening would swallow the first real
        // announcement, so an unregistered notifier stays silent and unlatched.
        let Some(registration) = registration else {
            return;
        };
        if self.announced.swap(true, Ordering::AcqRel) {
            return;
        }
        // The callback runs on whichever worker announced; a panic in it must
        // not take a PTY or session-sync thread down with it.
        let _ = catch_unwind(AssertUnwindSafe(|| registration()));
    }

    /// Called before the reader takes the runtime lock, never after.
    ///
    /// Clearing first means a change that lands while the delta is being taken
    /// announces itself again; the reader may then run once for nothing, which
    /// costs a read. Clearing afterwards would read that announcement as
    /// already delivered and leave the change on screen-invisible state until
    /// something else happened to notify.
    fn clear_announcement(&self) {
        self.announced.store(false, Ordering::Release);
    }

    fn set_callback(&self, registration: Option<Callback>) {
        *lock_recover(&self.registration) = registration;
    }

    #[cfg(test)]
    pub(crate) fn noop() -> Self {
        Self::new()
    }

    /// A notifier that runs `callback` on every announcement, as though
    /// each were read at once: for a test that waits for a worker's change
    /// rather than for time.
    #[cfg(test)]
    pub(crate) fn announcing(callback: impl Fn() + Send + Sync + 'static) -> Self {
        let notifier = Self::new();
        let announced = Arc::clone(&notifier.announced);
        notifier.set_callback(Some(Arc::new(move || {
            announced.store(false, Ordering::Release);
            callback();
        })));
        notifier
    }
}

pub struct Core {
    _delivery: Option<crate::delivery::worker::Worker>,
    _terminal_reports: Option<crate::terminal_reports::ReportPump>,
    _changes: Option<crate::changes::ChangesPump>,
    _fetch: Option<crate::fetch::FetchPump>,
    _kit: Option<crate::kit::KitPump>,
    _session_sync: Option<crate::session_sync::SessionSyncHandle>,
    _links: Option<crate::links::worker::LinkWorker>,
    _suggestions: Option<crate::suggestions::worker::SuggestionWorker>,
    _usage_ledger: Option<crate::usage_ledger::worker::Worker>,
    labels: Option<Arc<crate::labels::LabelServices>>,
    own_node: Arc<dyn crate::node_access::NodeLink>,
    runtime: Arc<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    owner_thread: ThreadId,
}

impl Drop for Core {
    fn drop(&mut self) {
        self._delivery.take();
        // Reserve cancellation before any coordinator shutdown can wait: a
        // completing attachment must not enqueue input during destruction.
        let attachment_worker = { lock_recover(&self.runtime).take_attachment_worker() };
        // First, so the provider child ends with the daemon even while a
        // coordinator below waits out a slow device read: the running
        // request is cancelled and anything a worker hands in later is
        // answered as stopped (labels-in-hided B22).
        if let Some(labels) = self.labels.take() {
            labels.analyzer.shutdown();
        }
        // A kit step running on this machine's node ends the child it waits
        // on, so the kit worker below is joined without waiting it out.
        if let Some(kit) = self._kit.as_ref() {
            kit.stop();
        }
        self.own_node.close("core stopping");
        self._kit.take();
        self._usage_ledger.take();
        self._links.take();
        self._suggestions.take();
        self._terminal_reports.take();
        self._changes.take();
        self._fetch.take();
        self._session_sync.take();
        // Taken under the lock, joined outside it: a coordinator's last act is
        // to lock the runtime, so a join under the lock never returns.
        let remote_syncs = { lock_recover(&self.runtime).take_remote_syncs() };
        drop(remote_syncs);
        // After the coordinators, so no timer dials a device again: each
        // device's link ends with the core, and with it what the link owns
        // here, a WSL distribution's `wsl.exe` and the helper inside it
        // (PRD wsl-device B27). Closed outside the lock, which a link's end
        // takes to record it.
        let device_links = { lock_recover(&self.runtime).ready_device_channels() };
        for (_, link) in device_links {
            link.close("core stopping");
        }
        // A dial still running has its `ssh` killed and reaped, and a closed
        // link's is reaped after its grace; waited for outside the lock, up
        // to five seconds, since a tied `ssh` ends with this process
        // regardless.
        let attempts = { lock_recover(&self.runtime).take_device_attempts() };
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for attempt in attempts {
            attempt.wait_ended(until.saturating_duration_since(std::time::Instant::now()));
        }
        if let Some(worker) = attachment_worker {
            let _ = worker.join();
        }
        // A Send in flight ends with the core; its comments stay unsent.
        let feedback_workers = { lock_recover(&self.runtime).take_view_feedback_workers() };
        for worker in feedback_workers {
            if worker.join().is_err() {
                crate::diagnostic!(
                    serde_json::json!({"component":"view_feedback", "kind":"worker.join_failed"})
                );
            }
        }
        // A layout or View tab changed just before quitting is on disk before
        // the process goes (B19).
        let views_worker = { lock_recover(&self.runtime).take_workspace_views_save_worker() };
        if let Some(worker) = views_worker
            && worker.join().is_err()
        {
            crate::diagnostic!(
                serde_json::json!({"component":"workspace_views", "kind":"save.join_failed"})
            );
        }
        let worker = { lock_recover(&self.runtime).take_state_save_worker() };
        if let Some(worker) = worker
            && worker.join().is_err()
        {
            crate::diagnostic!(
                serde_json::json!({"component":"ui_state", "kind":"save.join_failed"})
            );
        }
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn check_owner_thread(core: &Core, operation: &str) -> bool {
    if thread::current().id() == core.owner_thread {
        return true;
    }
    lock_recover(&core.runtime).set_error(
        "core.wrong_thread",
        format!("{operation} must run on the thread that created herdr-core"),
        false,
    );
    false
}

fn notify_change(core: &Core) {
    core.notifier.notify();
}

/// `create`, `dispatch_bytes`, `snapshot_delta`, `on_change`, and `Drop`
/// belong to the thread that created the value. Axum workers talk to it
/// through an owner-thread channel in `hided`.
impl Core {
    #[cfg(test)]
    pub(crate) fn for_runtime_fixture(runtime: Arc<Mutex<Runtime>>) -> (Self, ChangeNotifier) {
        let notifier = ChangeNotifier::new();
        let own_node = lock_recover(&runtime).own_node();
        let core = Self {
            own_node,
            _terminal_reports: None,
            _changes: None,
            _fetch: None,
            _kit: None,
            _session_sync: None,
            _links: None,
            _suggestions: None,
            _usage_ledger: None,
            _delivery: None,
            labels: None,
            runtime,
            notifier: notifier.clone(),
            owner_thread: thread::current().id(),
        };
        (core, notifier)
    }

    /// `own_node` answers for the machine this core runs on (PRD
    /// core-host-node D-21); the core reaches that machine only through it.
    /// `own_herdr` is that node's connection to the Herdr server at
    /// `options.herdr_socket_path`, given exactly when a socket is.
    /// `devices` opens the transport to each registered device.
    /// `terminals` reaches every node's terminal service, and `reports` is
    /// what those services answer (PRD core-host-node-terminal D-05): the
    /// core decides which panes are attached and reads their state, while
    /// keys and output pass between the screen and the node beside it.
    pub fn create(
        options: CoreOptions,
        own_node: std::sync::Arc<dyn crate::node_access::NodeLink>,
        own_herdr: Option<std::sync::Arc<dyn hide_herdr_client::ApiConnector>>,
        devices: std::sync::Arc<dyn crate::remote::DeviceConnector>,
        terminals: std::sync::Arc<dyn hide_node_link::terminal::TerminalRoutes>,
        reports: crate::terminal_reports::TerminalReports,
    ) -> Option<Box<Self>> {
        Self::create_with_product_analytics(
            options, own_node, own_herdr, devices, terminals, reports, None,
        )
    }

    /// The daemon's optional content-free statistics transport, installed
    /// before any background router starts. Its owner must outlive this core.
    pub fn create_with_product_analytics(
        options: CoreOptions,
        own_node: Arc<dyn crate::node_access::NodeLink>,
        own_herdr: Option<Arc<dyn hide_herdr_client::ApiConnector>>,
        devices: Arc<dyn crate::remote::DeviceConnector>,
        terminals: Arc<dyn hide_node_link::terminal::TerminalRoutes>,
        reports: crate::terminal_reports::TerminalReports,
        analytics: Option<Arc<dyn crate::product_analytics::Sink>>,
    ) -> Option<Box<Self>> {
        if validate_options(&options).is_err()
            || options.herdr_socket_path.is_some() != own_herdr.is_some()
        {
            return None;
        }
        let environment = match options.home.as_deref() {
            Some(home) => environment::read_and_validate().with_home(home.into()),
            None => environment::read_and_validate(),
        };
        let environment_home = environment.home_path.clone();
        let usage_paths = crate::usage::UsagePaths {
            home: environment.home_path.clone(),
            claude_cwd: std::path::Path::new(&options.app_state_path)
                .parent()
                .filter(|directory| !directory.as_os_str().is_empty())
                .map(std::path::Path::to_path_buf),
            codex_home: environment.codex_home.clone(),
        };
        #[cfg(not(test))]
        if let Err(error) =
            crate::diagnostics::install(std::path::Path::new(&options.app_state_path))
        {
            std::eprintln!(
                "{}",
                serde_json::json!({"kind": "diagnostics.open_failed", "message": error.to_string()})
            );
        }
        // Before the runtime exists, so the read is never made under its lock.
        let system_language = crate::runtime::output_language::read_system_language();
        let runtime = Arc::new(Mutex::new(Runtime::new(
            options.clone(),
            environment,
            Arc::clone(&own_node),
            devices,
        )));
        if let Some(sink) = analytics {
            lock_recover(&runtime).install_product_analytics(sink);
        }
        lock_recover(&runtime).set_system_language(system_language);
        let notifier = ChangeNotifier::new();
        lock_recover(&runtime).install_worker_context(Arc::downgrade(&runtime), notifier.clone());
        lock_recover(&runtime).set_guardrails_own_file(options.account_state_folder);
        lock_recover(&runtime).publish_own_guardrail_policy();
        lock_recover(&runtime).set_terminals(terminals);
        // Started before anything can attach, so no report waits on it.
        let terminal_reports = match crate::terminal_reports::ReportPump::spawn(
            reports,
            Arc::downgrade(&runtime),
            notifier.clone(),
        ) {
            Ok(pump) => Some(pump),
            Err(error) => {
                lock_recover(&runtime).set_error(
                    "terminal.reports_unavailable",
                    error.to_string(),
                    true,
                );
                None
            }
        };
        let delivery_path = hide_kit::layout::delivery_ledger(
            std::path::Path::new(&options.app_state_path)
                .parent()
                .unwrap_or(std::path::Path::new(".")),
        );
        let delivery = match crate::delivery::worker::Worker::spawn(
            Arc::downgrade(&runtime),
            notifier.clone(),
            delivery_path,
        ) {
            Ok((worker, client)) => {
                lock_recover(&runtime).install_delivery_client(client);
                Some(worker)
            }
            Err(code) => {
                crate::diagnostic!(
                    serde_json::json!({"component":"delivery","kind":"worker.start_failed","code":code})
                );
                None
            }
        };
        // Before any coordinator starts, so the sidebar draws the last run's
        // pull request state from its first frame instead of after a `gh` pass.
        if let Some(directory) = std::path::Path::new(&options.app_state_path)
            .parent()
            .filter(|directory| !directory.as_os_str().is_empty())
        {
            // Read before the lock is taken: the file is the disk's, not the
            // runtime's.
            let store =
                crate::github_store::GithubStore::new(hide_kit::layout::github_snapshot(directory));
            let restored = store.restore();
            lock_recover(&runtime).install_github_store(store, restored);
            let composers = crate::composer_store::ComposerStore::new(
                hide_kit::layout::composer_marks(directory),
            );
            let marks = composers.restore(crate::links::now_ms());
            lock_recover(&runtime).install_composer_store(composers, marks);
            let checks = crate::composer_checks::ComposerChecks::new(
                hide_kit::layout::composer_checks(directory),
            );
            let restored = checks.restore();
            lock_recover(&runtime).install_composer_checks(checks, restored);
            let missions =
                crate::missions::MissionStore::new(hide_kit::layout::missions(directory));
            let restored = missions.restore();
            let events = crate::mission_events::MissionEventStore::new(
                hide_kit::layout::mission_events(directory),
            );
            let logs = match &restored {
                crate::missions::Restored::Read(book) => {
                    events.sweep(book.missions.iter().map(|mission| mission.id.as_str()));
                    events.restore(book.missions.iter().map(|mission| mission.id.as_str()))
                }
                _ => std::collections::HashMap::new(),
            };
            let mut guard = lock_recover(&runtime);
            guard.install_mission_store(missions, restored);
            guard.install_mission_events(events, logs);
            drop(guard);
        }
        // Before any coordinator starts, because each builds its label worker
        // on these, and before the kit runs, because the plugin state the
        // store imports once is what the kit's retirement deletes.
        // Taken before the match: a guard in its scrutinee would be held
        // through the arms, which lock the runtime again.
        let (usage_gate, ai_usage) = {
            let runtime = lock_recover(&runtime);
            (runtime.usage_gate(), runtime.ai_usage())
        };
        let labels = match crate::labels::LabelServices::start(
            std::path::Path::new(&options.app_state_path)
                .parent()
                .filter(|directory| !directory.as_os_str().is_empty()),
            environment_home.clone(),
            Arc::downgrade(&runtime),
            &options.node_id,
            Arc::clone(&own_node),
            usage_gate,
            ai_usage,
        ) {
            Ok(services) => {
                let services = Arc::new(services);
                lock_recover(&runtime).install_label_services(Arc::clone(&services));
                Some(services)
            }
            Err(message) => {
                crate::diagnostic!(
                    serde_json::json!({"component":"labels","kind":"services.start_failed","message":message})
                );
                None
            }
        };
        // Before any coordinator, so the first catalog and GitHub answers
        // reach it.
        let links = match crate::runtime::links::spawn_worker(&runtime, notifier.clone()) {
            Ok(worker) => {
                if let Some(labels) = labels.as_ref()
                    && let Ok(mut client) = labels.ledger.lock()
                {
                    *client = Some(worker.client());
                }
                Some(worker)
            }
            Err(message) => {
                crate::diagnostic!(
                    serde_json::json!({"component":"links","kind":"worker.start_failed","message":message})
                );
                None
            }
        };
        let suggestions = match crate::runtime::suggestions::spawn_worker(
            &runtime,
            notifier.clone(),
        ) {
            Ok(worker) => Some(worker),
            Err(message) => {
                crate::diagnostic!(
                    serde_json::json!({"component":"suggestions","kind":"worker.start_failed","message":message})
                );
                None
            }
        };
        let session_sync = if let (Some(socket_path), Some(own_herdr)) =
            (options.herdr_socket_path.as_deref(), own_herdr)
        {
            live::install(
                &runtime,
                notifier.clone(),
                Arc::clone(&own_node),
                own_herdr,
                socket_path,
                usage_paths,
            )
        } else {
            None
        };
        if lock_recover(&runtime).connect_registered_devices() {
            notifier.notify();
        }
        // History reads through each checkout's own host, so it runs whether
        // or not this machine has a Herdr session.
        let changes =
            match crate::changes::ChangesPump::spawn(Arc::downgrade(&runtime), notifier.clone()) {
                Ok(pump) => Some(pump),
                Err(error) => {
                    lock_recover(&runtime).set_error(
                        "changes.reader_unavailable",
                        error.to_string(),
                        true,
                    );
                    None
                }
            };
        // The background fetch asks each repository's own node, so it runs
        // whether or not this machine has a Herdr session.
        let fetch = match crate::fetch::FetchPump::spawn(Arc::downgrade(&runtime), notifier.clone())
        {
            Ok(pump) => Some(pump),
            Err(error) => {
                lock_recover(&runtime).set_error("fetch.pump_unavailable", error.to_string(), true);
                None
            }
        };
        // This machine's install kit runs on its own node whether or not
        // Herdr answers; only retiring the old labels plugin needs it (PRD
        // labels-in-hided D-12). Without a socket from the embedder the node
        // uses the one Herdr would for its home.
        lock_recover(&runtime).queue_local_kit_launch();
        let kit = match crate::kit::KitPump::spawn(
            Arc::downgrade(&runtime),
            notifier.clone(),
            Arc::clone(&own_node),
            options.herdr_socket_path.clone(),
            options.node_id.clone(),
        ) {
            Ok(pump) => Some(pump),
            Err(error) => {
                lock_recover(&runtime).set_local_kit_unavailable(
                    crate::model::KitUnavailable::InstallerUnavailable,
                    &format!("Hide could not start its installer: {error}"),
                );
                None
            }
        };
        let usage_ledger =
            match crate::runtime::usage_ledger::spawn_worker(&runtime, notifier.clone()) {
                Ok(worker) => Some(worker),
                Err(code) => {
                    crate::diagnostic!(
                        serde_json::json!({"component":"usage_ledger", "kind":code})
                    );
                    let value = crate::usage::consumption::Publication::prepare(
                        crate::usage_ledger::worker::Report {
                            projection: None,
                            projects: Default::default(),
                            progress: Vec::new(),
                            failure: Some(code),
                            price_failure: None,
                        },
                        crate::links::now_ms(),
                    );
                    lock_recover(&runtime).ingest_ledger(value);
                    None
                }
            };
        Some(Box::new(Core {
            _delivery: delivery,
            _terminal_reports: terminal_reports,
            _changes: changes,
            _fetch: fetch,
            _kit: kit,
            own_node,
            _session_sync: session_sync,
            _links: links,
            _suggestions: suggestions,
            _usage_ledger: usage_ledger,
            labels,
            runtime,
            notifier,
            owner_thread: thread::current().id(),
        }))
    }

    pub fn dispatch_bytes(&self, bytes: &[u8]) -> bool {
        if !check_owner_thread(self, "dispatch") {
            notify_change(self);
            return false;
        }
        let (changed, retired_syncs) = {
            let mut runtime = lock_recover(&self.runtime);
            let changed = runtime.dispatch_json(bytes);
            (changed, runtime.take_retired_remote_syncs())
        };
        drop(retired_syncs);
        if changed {
            notify_change(self);
        }
        changed
    }

    pub fn prepare_delivery(
        &self,
        device: &str,
        caller: &str,
        expected: &crate::workspace_control::Context,
        hint: Option<&str>,
        command: crate::delivery::Command,
    ) -> Result<crate::delivery::worker::Prepared, String> {
        if !check_owner_thread(self, "delivery.prepare") {
            return Err("delivery_unavailable".into());
        }
        lock_recover(&self.runtime).prepare_delivery(device, caller, expected, hint, command)
    }

    /// Runs a Conductor's `hide mission` command (PRD
    /// software-factory-conductor D-23): one change under the lock, saved
    /// by the mission store's own thread.
    pub fn mission_command(
        &self,
        device: &str,
        caller: &str,
        expected: &crate::workspace_control::Context,
        command: crate::missions::MissionCommand,
    ) -> Result<serde_json::Value, String> {
        if !check_owner_thread(self, "mission.command") {
            return Err("mission_unavailable".into());
        }
        let (answer, changed) =
            lock_recover(&self.runtime).mission_command(device, caller, expected, command)?;
        if changed {
            notify_change(self);
        }
        Ok(answer)
    }

    /// Runs `hide agent sleep` (PRD mission-worker-sleep D-20): the end it
    /// starts is the runtime's, so it is asked under the lock.
    pub fn agent_sleep(
        &self,
        device: &str,
        caller: &str,
        expected: &crate::workspace_control::Context,
        id: &str,
        actor: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        if !check_owner_thread(self, "agent.sleep") {
            return Err("agent_unavailable".into());
        }
        let (answer, changed) =
            lock_recover(&self.runtime).agent_sleep_command(device, caller, expected, id, actor)?;
        if changed {
            notify_change(self);
        }
        Ok(answer)
    }

    /// Runs a `hide plan` command (PRD hide-plan D-27): one change under the
    /// lock, saved by the link worker.
    pub fn plan_command(
        &self,
        device: &str,
        caller: &str,
        expected: &crate::workspace_control::Context,
        command: crate::plans::PlanCommand,
    ) -> Result<serde_json::Value, String> {
        if !check_owner_thread(self, "plan.command") {
            return Err("plan_unavailable".into());
        }
        let (answer, changed) =
            lock_recover(&self.runtime).plan_command(device, caller, expected, command)?;
        if changed {
            notify_change(self);
        }
        Ok(answer)
    }

    /// The plan and mission summary the prompt hook keeps for `caller`'s
    /// pane (PRD hide-plan D-30); none for a caller in no pane.
    pub fn plan_context(
        &self,
        device: &str,
        caller: &str,
    ) -> Option<hide_agent_hooks::plan_record::PlanContext> {
        if !check_owner_thread(self, "plan.context") {
            return None;
        }
        lock_recover(&self.runtime).plan_context(device, caller)
    }

    /// `hide agent catalog`: the agents, models and efforts of `machine`, or
    /// of the caller's own `device` (PRD software-factory-conductor D-35).
    pub fn agent_catalog(
        &self,
        device: &str,
        machine: Option<&str>,
        timed_out: bool,
    ) -> Result<serde_json::Value, String> {
        if !check_owner_thread(self, "agent.catalog") {
            return Err("catalog_unavailable".into());
        }
        lock_recover(&self.runtime).agent_catalog(device, machine, timed_out)
    }

    /// `hide device` for a caller on `device`: a registration or removal
    /// changes the snapshot the shell draws, so it announces like a dispatch.
    pub fn device_command(
        &self,
        device: &str,
        command: crate::device_command::DeviceCommand,
    ) -> Result<serde_json::Value, String> {
        if !check_owner_thread(self, "device.command") {
            return Err("device_unavailable".into());
        }
        let (answer, retired_syncs) = {
            let mut runtime = lock_recover(&self.runtime);
            let answer = runtime.device_command(device, command);
            (answer, runtime.take_retired_remote_syncs())
        };
        drop(retired_syncs);
        let (value, changed) = answer?;
        if changed {
            notify_change(self);
        }
        Ok(value)
    }

    /// `hide <path>` for a caller on `device`: brings the project holding
    /// `path` in front, or starts its registration, under `request_id`,
    /// whose receipt [`Self::project_open_receipt`] reads; `unheld` says
    /// whether a folder no registered checkout holds may be registered.
    pub fn project_open(
        &self,
        device: &str,
        path: &str,
        request_id: &str,
        unheld: crate::project_open::Unheld,
    ) -> Result<(), String> {
        if !check_owner_thread(self, "project.open") {
            return Err("project_open_unavailable".into());
        }
        let (changed, retired_syncs) = {
            let mut runtime = lock_recover(&self.runtime);
            let changed = runtime.project_open(device, path, request_id, unheld);
            (changed, runtime.take_retired_remote_syncs())
        };
        drop(retired_syncs);
        if changed? {
            notify_change(self);
        }
        Ok(())
    }

    /// Where the `hide <path>` request `request_id` stands.
    pub fn project_open_receipt(
        &self,
        request_id: &str,
    ) -> Option<crate::project_open::ProjectOpenReceipt> {
        lock_recover(&self.runtime).project_open_receipt(request_id)
    }

    /// `hide ps` for a caller on `device` in `pane`: the rows are taken under
    /// the lock, and joined with the ledger and serialized after it.
    pub fn ps(&self, device: &str, pane: &str, all: bool) -> Result<serde_json::Value, String> {
        if !check_owner_thread(self, "agent.ps") {
            return Err("ps_unavailable".into());
        }
        let rows = lock_recover(&self.runtime).ps_rows(device, pane)?;
        serde_json::to_value(rows.answer(all)).map_err(|_| "ps_unavailable".into())
    }

    /// Checks a `hide memory` caller and takes what its command needs; the
    /// command then runs on the daemon's blocking task, off this lock.
    pub fn prepare_memory(
        &self,
        device: &str,
        caller: &str,
        expected: &crate::workspace_control::Context,
        command: crate::memory_command::MemoryCommand,
    ) -> Result<crate::memory_command::PreparedMemory, String> {
        if !check_owner_thread(self, "memory.prepare") {
            return Err("memory_unavailable".into());
        }
        lock_recover(&self.runtime).prepare_memory(device, caller, expected, command)
    }

    #[allow(clippy::too_many_arguments)] // identity facts and deadline cross the authenticated boundary intact
    pub fn prepare_direct_observation(
        &self,
        device: &str,
        caller: &str,
        expected: &crate::workspace_control::Context,
        terminal: &str,
        observation: hide_agent_hooks::observation::DirectObservation,
        start: bool,
        deadline: std::time::Instant,
    ) -> Result<crate::direct_observation::PreparedObservation, String> {
        if !check_owner_thread(self, "observation.prepare") || std::time::Instant::now() >= deadline
        {
            return Err("observation_expired".into());
        }
        let runtime = self.runtime.try_lock().map_err(|_| "observation_busy")?;
        let mut prepared = runtime.prepare_direct_observation(
            device,
            caller,
            expected,
            terminal,
            observation,
            start,
            deadline,
        )?;
        prepared.runtime = std::sync::Arc::downgrade(&self.runtime);
        Ok(prepared)
    }

    pub fn prepare_delivery_human(&self) -> Result<crate::delivery::worker::PreparedHuman, String> {
        if !check_owner_thread(self, "delivery.human.prepare") {
            return Err("delivery_unavailable".into());
        }
        lock_recover(&self.runtime).prepare_delivery_human()
    }

    /// The move's link committed this core: what it held while pending
    /// runs now (`effects.rs`). Returns whether this released the hold.
    pub fn release_effects(&self) -> bool {
        if !check_owner_thread(self, "effects.release") {
            return false;
        }
        let released = {
            let mut runtime = lock_recover(&self.runtime);
            let released = runtime.release_effects();
            if released {
                // A held core wrote no policy at its start.
                runtime.publish_own_guardrail_policy();
            }
            released
        };
        if released {
            notify_change(self);
        }
        released
    }

    /// What moves this machine's Herdr server onto the Herdr this app ships
    /// (PRD herdr-takeover); a daemon outside a desktop package installs one
    /// that finds nothing (`NoTakeover`), so nothing is offered and a saved
    /// Herdr step goes on. Looks at the server at once.
    pub fn set_herdr_takeover(&self, takeover: Arc<dyn crate::herdr_takeover::HerdrTakeover>) {
        if check_owner_thread(self, "set_herdr_takeover") {
            let mut runtime = lock_recover(&self.runtime);
            runtime.install_herdr_takeover(
                takeover,
                Arc::downgrade(&self.runtime),
                self.notifier.clone(),
            );
        }
    }

    /// The checkout roots the daemon verified, by the identity each was
    /// opened with, which the core's own file work is checked against.
    pub fn set_file_roots(&self, roots: crate::files::FileRoots) {
        if check_owner_thread(self, "set_file_roots")
            && lock_recover(&self.runtime).set_file_roots(roots)
        {
            notify_change(self);
        }
    }

    /// The link to the node `device_id` names, for the daemon's own
    /// requests that answer outside the snapshot (the Explorer's listing and
    /// watch, the links query). Asking may start a device's helper, which
    /// the snapshot announces.
    pub fn node_link(
        &self,
        device_id: &str,
    ) -> Result<std::sync::Arc<dyn crate::node_access::NodeLink>, String> {
        if !check_owner_thread(self, "node_link") {
            return Err("the core was called off its owner thread".to_owned());
        }
        let result = lock_recover(&self.runtime).node_link(device_id);
        // Only a refusal can have started a helper connection.
        if result.is_err() {
            notify_change(self);
        }
        result
    }

    /// Why a link from `node` would be refused now, if it would.
    pub fn inbound_refusal(&self, node: &str) -> Option<String> {
        if !check_owner_thread(self, "inbound_refusal") {
            return Some("core_unavailable".to_owned());
        }
        lock_recover(&self.runtime)
            .inbound_refusal(node)
            .map(str::to_owned)
    }

    /// What a move of the core to `device` is made from (PRD
    /// core-host-node-move B4), with the device's label records, which memory
    /// holds (bar their marks), in the form `labels.json` stores them.
    pub fn move_source(
        &self,
        device: &str,
    ) -> Result<(crate::MoveSource, serde_json::Value), crate::MoveSourceRefusal> {
        if !check_owner_thread(self, "move_source") {
            return Err(crate::MoveSourceRefusal::NotReady);
        }
        let source = lock_recover(&self.runtime).move_source(device)?;
        // A core whose label services never started holds no records.
        let labels = match &self.labels {
            Some(labels) => labels
                .store
                .export(&crate::labels::device_target(device))
                .map_err(crate::MoveSourceRefusal::Unstorable)?,
            None => serde_json::Value::Object(serde_json::Map::new()),
        };
        Ok((source, labels))
    }

    /// What a move of the core back to the node that dialed it is made
    /// from (PRD core-host-node-move B6), with that node's label records,
    /// which memory holds (bar their marks), in the form `labels.json` stores them.
    pub fn release_source(
        &self,
        node: &str,
    ) -> Result<(crate::ReleaseSource, serde_json::Value), crate::MoveSourceRefusal> {
        if !check_owner_thread(self, "release_source") {
            return Err(crate::MoveSourceRefusal::NotReady);
        }
        let source = lock_recover(&self.runtime).release_source(node)?;
        let labels = match &self.labels {
            Some(labels) => labels
                .store
                .export(&crate::labels::device_target(node))
                .map_err(crate::MoveSourceRefusal::Unstorable)?,
            None => serde_json::Value::Object(serde_json::Map::new()),
        };
        Ok((source, labels))
    }

    /// The nodes that dialed this core and are connected now (PRD
    /// core-host-node-remote-core D-18).
    pub fn linked_nodes(&self) -> Vec<String> {
        if !check_owner_thread(self, "linked_nodes") {
            return Vec::new();
        }
        lock_recover(&self.runtime).linked_nodes()
    }

    /// Takes the link a node that dialed this core brought (PRD
    /// core-host-node-remote-core D-04, D-10), registering the node on its
    /// first link; the refusal is the reason the node is told.
    pub fn accept_inbound_node(
        &self,
        node: &str,
        label: &str,
        arrived: crate::remote::Arrived,
    ) -> Result<(), String> {
        if !check_owner_thread(self, "accept_inbound_node") {
            return Err("core_unavailable".to_owned());
        }
        let (accepted, retired_syncs) = {
            let mut runtime = lock_recover(&self.runtime);
            let accepted = runtime.accept_inbound_node(node, label, arrived);
            (accepted, runtime.take_retired_remote_syncs())
        };
        drop(retired_syncs);
        if accepted.is_ok() {
            notify_change(self);
        }
        accepted
    }

    /// The Herdr API connection the core holds for a connected SSH device;
    /// `None` when the device is not connected. The daemon reads and writes
    /// a device pane through it without joining the attach set.
    pub fn remote_herdr_api(
        &self,
        device_id: &str,
    ) -> Option<std::sync::Arc<dyn hide_herdr_client::ApiConnector>> {
        if !check_owner_thread(self, "remote_herdr_api") {
            return None;
        }
        lock_recover(&self.runtime).remote_herdr_api(device_id)
    }

    /// The gate a writer that bypasses a node's terminal (a phone's reply)
    /// asks at the write: it refuses a pane whose agent is ending, asleep or
    /// waking. Taken once and kept; the runtime keeps it current.
    pub fn input_gate(&self) -> std::sync::Arc<crate::input_gate::PaneInputGate> {
        lock_recover(&self.runtime).input_gate()
    }

    /// Daemon-only read of consented, connected SSH routes. Opening a route
    /// happens after this call has released the runtime lock.
    pub fn workspace_remote_routes(&self) -> Vec<crate::WorkspaceRemoteRoute> {
        if !check_owner_thread(self, "workspace_remote_routes") {
            return Vec::new();
        }
        lock_recover(&self.runtime).workspace_remote_routes()
    }

    pub fn browser_route_source(
        &self,
        device_id: &str,
        checkout_path: &str,
        view_id: &str,
        load: u64,
    ) -> Option<crate::workspace_control::BrowserRouteSource> {
        if !check_owner_thread(self, "browser_route_source") {
            return None;
        }
        lock_recover(&self.runtime).browser_route_source(device_id, checkout_path, view_id, load)
    }

    /// Daemon-only pane query. The daemon validates the process that asked;
    /// the core then resolves current pane membership at the point of use.
    /// Only owned result data leaves the runtime lock.
    /// The registered Projects and the caller checkout's own, for a
    /// `hide links` read the daemon then answers off this thread.
    pub fn links_scope(
        &self,
        context: &crate::workspace_control::Context,
    ) -> Option<crate::links::query::Scope> {
        if !check_owner_thread(self, "links_scope") {
            return None;
        }
        lock_recover(&self.runtime).links_scope(context)
    }

    /// What a hook's Memory read for a pane in `context` is answered from,
    /// which the daemon then answers off this thread
    /// (`crate::memory_hook::answer`), or why there is none.
    pub fn memory_scope(
        &self,
        context: &crate::workspace_control::Context,
    ) -> Result<crate::memory_hook::Scope, &'static str> {
        if !check_owner_thread(self, "memory_scope") {
            return Err("core_unavailable");
        }
        lock_recover(&self.runtime).memory_scope(context)
    }

    pub fn workspace_control_query(
        &self,
        device_id: &str,
        pane_id: &str,
        query: crate::workspace_control::Query,
    ) -> Result<crate::workspace_control::QueryResult, crate::workspace_control::Refusal> {
        if !check_owner_thread(self, "workspace_control_query") {
            return Err(crate::workspace_control::Refusal {
                reason: "core_unavailable",
                next_action: "Reconnect Hide and retry",
            });
        }
        lock_recover(&self.runtime).workspace_control_query(device_id, pane_id, query)
    }

    /// A pane-scoped View action runs as one core transition. The daemon
    /// attests the caller, and the core rechecks current membership.
    pub fn workspace_control_prepare_action(
        &self,
        device_id: &str,
        pane_id: &str,
        expected: &crate::workspace_control::Context,
        request_id: &str,
        action: &crate::workspace_control::Action,
    ) -> Result<crate::workspace_control::ActionPreparation, crate::workspace_control::Refusal>
    {
        if !check_owner_thread(self, "workspace_control_prepare_action") {
            return Err(crate::workspace_control::Refusal {
                reason: "core_unavailable",
                next_action: "Reconnect Hide and retry",
            });
        }
        lock_recover(&self.runtime)
            .workspace_control_prepare_action(device_id, pane_id, expected, request_id, action)
    }

    pub fn workspace_control_action(
        &self,
        device_id: &str,
        pane_id: &str,
        expected: &crate::workspace_control::Context,
        request_id: &str,
        action: crate::workspace_control::Action,
        material: Result<
            Option<crate::workspace_control::ActionMaterial>,
            crate::workspace_control::Refusal,
        >,
    ) -> Result<crate::workspace_control::ActionResult, crate::workspace_control::Refusal> {
        if !check_owner_thread(self, "workspace_control_action") {
            return Err(crate::workspace_control::Refusal {
                reason: "core_unavailable",
                next_action: "Reconnect Hide and retry",
            });
        }
        let result = lock_recover(&self.runtime)
            .workspace_control_action(device_id, pane_id, expected, request_id, action, material);
        if result.as_ref().is_ok_and(|result| result.changed) {
            notify_change(self);
        }
        result
    }

    pub fn snapshot_delta(&self, have_revision: u64) -> Vec<u8> {
        self.snapshot_delta_payload(have_revision)
            .and_then(|payload| {
                crate::runtime::serialize_snapshot_delta(&payload)
                    .map_err(|error| error.to_string())
            })
            .unwrap_or_default()
    }

    /// Typed ownership crosses the core owner boundary; transports serialize
    /// it after this call, without occupying dispatch or holding Runtime.
    pub fn snapshot_delta_payload(
        &self,
        have_revision: u64,
    ) -> Result<crate::model::SnapshotDeltaPayload, String> {
        if !check_owner_thread(self, "snapshot") {
            notify_change(self);
            return Err("snapshot requested off the core owner thread".to_owned());
        }
        self.notifier.clear_announcement();
        Ok({
            let mut runtime = lock_recover(&self.runtime);
            runtime.snapshot_delta_payload(have_revision)
        })
    }

    pub fn on_change<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        if !check_owner_thread(self, "on_change") {
            notify_change(self);
            return;
        }
        self.notifier.set_callback(Some(Arc::new(callback)));
    }

    pub fn clear_on_change(&self) {
        if !check_owner_thread(self, "on_change") {
            notify_change(self);
            return;
        }
        self.notifier.set_callback(None);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    use serde_json::json;

    /// A hang guard, not a speed bound: the device's alias lets `ssh` wait
    /// ten minutes for the greeting and an attempt's sign-in is bounded at
    /// 90 s, so within this guard only the core ends the dial
    /// (with-hide/hide-private#75).
    const ONLY_THE_CORE_ENDS_THE_DIAL: Duration = Duration::from_secs(30);

    /// The listener that stands for the device: its one accept runs on a
    /// thread this value owns, which its drop wakes and joins, so a setup
    /// that fails before the core dials leaves no thread or socket behind.
    struct SilentListener {
        address: std::net::SocketAddr,
        accepted: std::sync::mpsc::Receiver<std::io::Result<TcpStream>>,
        accepting: Option<std::thread::JoinHandle<()>>,
    }

    impl SilentListener {
        fn bind() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let (sender, accepted) = std::sync::mpsc::channel();
            let accepting = std::thread::spawn(move || {
                let _ = sender.send(listener.accept().map(|(peer, _)| peer));
            });
            Self {
                address,
                accepted,
                accepting: Some(accepting),
            }
        }

        /// The connection the core's attempt made, which then waits for the
        /// greeting.
        fn dialed(&self) -> TcpStream {
            self.accepted
                .recv_timeout(ONLY_THE_CORE_ENDS_THE_DIAL)
                .expect("the core never dialed the device")
                .expect("the listener accepts the dial")
        }
    }

    impl Drop for SilentListener {
        fn drop(&mut self) {
            let Some(accepting) = self.accepting.take() else {
                return;
            };
            // An accept still waiting takes this connection and returns.
            if !accepting.is_finished() {
                let _ = TcpStream::connect(self.address);
            }
            if accepting.join().is_err() && !std::thread::panicking() {
                panic!("the listener's accept thread panicked");
            }
        }
    }

    /// A core whose one device is an alias for a listener that takes the
    /// TCP connection and never sends the SSH greeting, and that connection
    /// as the device holds it once the core's attempt dialed it.
    struct SilentDevice {
        core: Option<Box<super::Core>>,
        peer: TcpStream,
        _listener: SilentListener,
        _home: tempfile::TempDir,
        _state: tempfile::TempDir,
    }

    impl SilentDevice {
        fn dialed() -> Self {
            let home = tempfile::tempdir().unwrap();
            let state = tempfile::tempdir().unwrap();
            let listener = SilentListener::bind();
            let ssh = home.path().join(".ssh");
            std::fs::create_dir_all(&ssh).unwrap();
            // `-F` and `HOME` do not move what `ssh` reads from the account's
            // own home, so every key and host file is named here: the dial
            // reads none of the account's (`/` reads on Windows too).
            let private = |name: &str| ssh.join(name).to_string_lossy().replace('\\', "/");
            std::fs::write(
                ssh.join("config"),
                format!(
                    "Host silent\n  HostName 127.0.0.1\n  Port {}\n  User fixture\n  IdentityFile none\n  IdentitiesOnly yes\n  IdentityAgent none\n  UserKnownHostsFile \"{}\"\n  GlobalKnownHostsFile \"{}\"\n  ConnectTimeout 600\n",
                    listener.address.port(),
                    private("known_hosts"),
                    private("known_hosts_global"),
                ),
            )
            .unwrap();
            let core = super::Core::create(
                crate::CoreOptions {
                    schema_version: crate::SCHEMA_VERSION,
                    home: Some(home.path().display().to_string()),
                    node_id: crate::node::test_node(),
                    herdr_socket_path: None,
                    herdr_bin_path: None,
                    app_state_path: state.path().join("core-state.json").display().to_string(),
                    host_helper_root: None,
                    host_cli_dir: None,
                    workspace_views_path: None,
                    shortcut_import_path: None,
                    local_issues_path: None,
                    effects_held: false,
                    machine_name: None,
                    account_state_folder: false,
                },
                std::sync::Arc::new(hide_node::Local::new(Some(home.path().to_path_buf()))),
                None,
                crate::node::test_devices(),
                crate::node::test_terminals().0,
                crate::node::test_terminals().1,
            )
            .expect("a core starts");
            assert!(core.dispatch_bytes(&event(
                "register_device",
                json!({"id": "silent", "label": "Silent", "ssh_alias": "silent", "host_consent": true}),
            )));
            // The accept is the barrier: the attempt's `ssh` is connected and
            // waits for the greeting.
            let peer = listener.dialed();
            Self {
                core: Some(core),
                peer,
                _listener: listener,
                _home: home,
                _state: state,
            }
        }

        /// The device sees its connection closed: the dial's `ssh` ended.
        /// One still waiting for the greeting would hold it open.
        fn assert_dial_ended(&mut self) {
            self.peer
                .set_read_timeout(Some(ONLY_THE_CORE_ENDS_THE_DIAL))
                .unwrap();
            let mut bytes = [0_u8; 256];
            loop {
                match self.peer.read(&mut bytes) {
                    Ok(0) => return,
                    // The client's own greeting, sent before it waits.
                    Ok(_) => continue,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        ) =>
                    {
                        return;
                    }
                    Err(error) => panic!("the dial still holds the device's connection: {error}"),
                }
            }
        }
    }

    fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema_version": crate::SCHEMA_VERSION,
            "kind": kind,
            "payload": payload,
        }))
        .unwrap()
    }

    #[test]
    fn a_stopping_core_ends_a_dial_to_a_device_that_never_greets() {
        let mut device = SilentDevice::dialed();
        let started = Instant::now();
        drop(device.core.take());
        let stopped = started.elapsed();
        assert!(
            stopped < ONLY_THE_CORE_ENDS_THE_DIAL,
            "the core's stop waited {stopped:?} for the device's dial"
        );
        device.assert_dial_ended();
    }

    #[test]
    fn removing_a_device_ends_its_dial_that_waits_for_a_greeting() {
        let mut device = SilentDevice::dialed();
        let core = device.core.as_ref().unwrap();
        assert!(core.dispatch_bytes(&event("remove_device", json!({"device_id": "silent"}))));
        device.assert_dial_ended();
    }
}
