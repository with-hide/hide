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
}

pub struct Core {
    _delivery: Option<crate::delivery::worker::Worker>,
    _terminal_reports: Option<crate::terminal_reports::ReportPump>,
    _changes: Option<crate::changes::ChangesPump>,
    _kit: Option<crate::kit::KitPump>,
    _session_sync: Option<crate::session_sync::SessionSyncHandle>,
    _links: Option<crate::links::worker::LinkWorker>,
    _usage_ledger: Option<crate::usage_ledger::worker::Worker>,
    labels: Option<Arc<crate::labels::LabelServices>>,
    factory: Option<crate::factory::FactoryHost>,
    own_node: Arc<dyn crate::node_access::NodeLink>,
    runtime: Arc<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    owner_thread: ThreadId,
}

impl Drop for Core {
    fn drop(&mut self) {
        // First: the Factory engine sends through delivery and ends the
        // verify runs and judgments it started (rule 14).
        if let Some(mut factory) = self.factory.take() {
            factory.shutdown();
        }
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
        self._terminal_reports.take();
        self._changes.take();
        self._session_sync.take();
        // Taken under the lock, joined outside it: a coordinator's last act is
        // to lock the runtime, so a join under the lock never returns.
        let remote_syncs = { lock_recover(&self.runtime).take_remote_syncs() };
        drop(remote_syncs);
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
            _kit: None,
            _session_sync: None,
            _links: None,
            _usage_ledger: None,
            _delivery: None,
            labels: None,
            factory: None,
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
        // The Software Factory engine; it opens its store only when a Factory
        // exists or is asked for.
        let factory = std::path::Path::new(&options.app_state_path)
            .parent()
            .filter(|directory| !directory.as_os_str().is_empty())
            .and_then(|state_dir| {
                crate::factory::FactoryHost::start(
                    state_dir,
                    environment_home.clone(),
                    Arc::downgrade(&runtime),
                    notifier.clone(),
                    lock_recover(&runtime).effects(),
                )
                .map_err(|message| {
                    crate::diagnostic!(
                        serde_json::json!({"component":"factory","kind":"host.start_failed","message":message})
                    );
                })
                .ok()
            });
        // The screens reach the engine through the same bounded queue.
        if let Some(factory) = &factory {
            lock_recover(&runtime).set_factory_screen_port(factory.screen_port());
        }
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
                lock_recover(&runtime).set_local_kit_unavailable(&format!(
                    "Hide could not start its installer: {error}"
                ));
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
            _kit: kit,
            own_node,
            _session_sync: session_sync,
            _links: links,
            _usage_ledger: usage_ledger,
            labels,
            factory,
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

    /// A `hide factory` command from a pane or checkout capability: the
    /// caller is checked as delivery checks it, and the engine decides the
    /// role from its own record of workers (D-33).
    pub fn prepare_factory(
        &self,
        device: &str,
        caller: &str,
        expected: &crate::workspace_control::Context,
        hint: Option<&str>,
        command: hide_factory::Command,
    ) -> Result<crate::factory::PreparedFactory, String> {
        if !check_owner_thread(self, "factory.prepare") {
            return Err("factory_unavailable".into());
        }
        let runtime = lock_recover(&self.runtime);
        let factory = self.factory.as_ref().ok_or("factory_unavailable")?;
        let caller = runtime.factory_caller(device, caller, expected, hint)?;
        drop(runtime);
        Ok(factory.prepare(caller, command))
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

    /// Prepare one read against an already-open Factory, with pane-only
    /// authority and a deadline supplied by the authenticated server route.
    #[allow(clippy::too_many_arguments)] // each identity fact is supplied by the authenticated pane boundary
    pub fn prepare_factory_question_guard(
        &self,
        device: &str,
        caller: &str,
        expected: &crate::workspace_control::Context,
        session: &str,
        agent_runtime: &str,
        terminal_id: &str,
        deadline: std::time::Instant,
    ) -> Result<crate::factory::PreparedQuestionGuard, String> {
        if !check_owner_thread(self, "factory.question_guard")
            || std::time::Instant::now() >= deadline
        {
            return Err("factory_guard_expired".into());
        }
        let factory = self.factory.as_ref().ok_or("factory_guard_unavailable")?;
        let runtime = self.runtime.try_lock().map_err(|_| "factory_guard_busy")?;
        let caller = runtime.factory_question_caller(
            device,
            caller,
            expected,
            session,
            agent_runtime,
            terminal_id,
        )?;
        drop(runtime);
        Ok(factory.prepare_question_guard(
            caller,
            std::sync::Arc::downgrade(&self.runtime),
            deadline,
        ))
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
