//! Coordinates the session's publishes and the capability readers: the
//! engine wakes this thread once its session changed, and the session is
//! read whole on each wake (`pulled.rs`) and published through
//! [`publish_session`].

use super::*;
use crate::labels::worker::{LabelWorker, ObservedAgent};
use crate::live;

pub(crate) fn spawn(
    context: SessionSyncContext,
    usage_paths: Option<crate::usage::UsagePaths>,
) -> Result<SessionSyncHandle, String> {
    let (sender, receiver) = channel();
    let worker_sender = sender.clone();
    let republish = Arc::new(RepublishFlags::default());
    let worker_republish = Arc::clone(&republish);
    let worker = thread::Builder::new()
        .name("hide-core-session-sync".to_owned())
        .spawn(move || {
            run_coordinator(
                context,
                usage_paths,
                receiver,
                worker_sender,
                worker_republish,
            )
        })
        .map_err(|error| format!("session sync worker could not be started: {error}"))?;
    Ok(SessionSyncHandle {
        sender,
        republish,
        worker: Some(worker),
    })
}

fn run_coordinator(
    context: SessionSyncContext,
    usage_paths: Option<crate::usage::UsagePaths>,
    receiver: Receiver<CoordinatorMessage>,
    sender: Sender<CoordinatorMessage>,
    republish: Arc<RepublishFlags>,
) {
    let home_path = usage_paths.as_ref().and_then(|paths| paths.home.clone());
    let mut lineage_writer = match context.ledger_machine().and_then(|machine| {
        crate::coordination::lineage::Writer::new(
            machine,
            Arc::clone(&context.engine),
            context.runtime.clone(),
        )
    }) {
        Ok(writer) => Some(writer),
        Err(message) => {
            crate::diagnostic!(
                json!({"component":"lineage","kind":"start_failed","message":message})
            );
            None
        }
    };
    // Each session read that follows a failure starts a new connection, the
    // generation the catalog keeps its accepted state per.
    let mut connection_generation = 0_u64;
    let mut reconnect_at = Instant::now();
    let mut reconnect_delay = RECONNECT_INITIAL_DELAY;
    let mut defer_background_reads = true;
    let mut next_operation_tick = Instant::now() + ASYNC_OPERATION_TICK_INTERVAL;
    let mut catalog_cache = CatalogState {
        wake: Some(RepublishWaker::asking(
            sender.clone(),
            Arc::clone(&republish),
        )),
        ..Default::default()
    };
    // The runtime's settled worktree removals the catalog was last rebuilt for.
    let mut published_removals = 0;
    let mut purpose_mirror = if context.is_local()
        && let Some(node) = context.node()
    {
        match crate::worktree_control::PurposeMirror::new(
            Arc::clone(&context.engine),
            Arc::clone(node),
        ) {
            Ok(mirror) => Some(mirror),
            Err(message) => {
                crate::diagnostic!(json!({
                    "component": "checkout_purpose",
                    "kind": "mirror.start_failed",
                    "message": message,
                }));
                None
            }
        }
    } else {
        None
    };
    // Kept for the AI settings and the counter sweep below, which read this
    // machine's files themselves.
    let hook_home = context.is_local().then(|| home_path.clone()).flatten();
    // The rows read the operator's logins, which are the core's own node's.
    let mut usage_reader = usage_paths
        .zip(context.node().map(Arc::clone))
        .map(|(paths, node)| crate::usage::UsageDriver::new(paths, node));
    // The agents a device's panes run are the device's own: its coordinator
    // asks for their versions, which the doorbell reads a composer by. This
    // machine's come with its kit.
    let mut device_agents_reader = match &context.target {
        SessionSyncTarget::Remote { target_id, .. } => Some(
            crate::kit::DeviceDiagnosisReader::new(context.runtime.clone(), target_id.clone()),
        ),
        SessionSyncTarget::Local { .. } => None,
    };
    // A listening port is its machine's: the local coordinator looks at this
    // machine's, and a node that dials this core is asked for its own.
    let mut ports_reader = context
        .machine()
        .map(|node| crate::ports::PortsReader::new(Arc::clone(node)));
    // The three project-panel readers describe this machine's repositories:
    // its worktrees, its `gh` login's view of their pull requests, and one
    // checkout's size on this disk. All three run their subprocess on a worker
    // thread, so a slow `gh` or `du` costs no coordinator latency.
    let mut worktree_reader = context
        .node()
        .map(|node| crate::worktrees::WorktreeReader::new(Arc::clone(node)));
    // `gh` runs here with the operator's login, for this machine's projects
    // and, by repository name, for those of a node that dials this core.
    let mut github_reader = context
        .login()
        .map(|node| crate::github::GithubReader::new(Arc::clone(node)));
    // The advisory poll asks GitHub each minute whether a list behind a pull
    // request mark changed, and only says so (`github_poll`); this machine's
    // projects only, through the same login.
    let mut github_poll_reader = context
        .login()
        .filter(|_| matches!(context.target, SessionSyncTarget::Local { .. }))
        .map(|node| crate::github_poll::GithubPollReader::new(Arc::clone(node)));
    // Sizes are measured on the machine the folders are on: this one's by
    // its own node, a node that dials this core through its link.
    let mut disk_reader = context
        .machine()
        .map(|node| crate::disk::DiskReader::new(Arc::clone(node)));
    // The Projects tab's sweep measures one project at a time on its own
    // reader, so the lane card's measurement never waits behind it.
    let mut sweep_reader = context
        .machine()
        .map(|node| crate::disk::DiskReader::new(Arc::clone(node)));
    // The provider probe starts a `codex app-server` child and runs
    // `claude auth status`, so it is a reader like the three above and it
    // reads nothing at all while the Background AI group is off screen.
    let mut ai_reader = context
        .is_local()
        .then(|| {
            context.node().map(|node| {
                crate::ai::AiReader::new(crate::ai::AiMachines {
                    own: Arc::clone(node),
                    devices: crate::runtime::device_links(context.runtime.clone()),
                })
            })
        })
        .flatten();
    if let Some(home) = hook_home.as_deref() {
        publish_ai_settings(&context, home);
    }
    let mut labels = start_label_worker(&context, &sender);
    // The engine wakes this thread once its session changed, and the session
    // is read whole on each wake and published as it is (`pulled.rs`).
    {
        let waker = RepublishWaker::engine(sender.clone(), Arc::clone(&republish));
        context.engine.changes(Arc::new(move || waker.wake()));
    }
    let mut pulled: Option<super::pulled::PulledSession> = None;

    loop {
        if context.runtime.upgrade().is_none() {
            return;
        }

        if pulled.is_none() && Instant::now() >= reconnect_at {
            let snapshot_started_at = Instant::now();
            if !begin_delivery_pane_read(&context) {
                return;
            }
            match super::pulled::PulledSession::read(&context) {
                Ok(session) => {
                    if !begin_read_connection(&context, snapshot_started_at) {
                        return;
                    }
                    connection_generation = connection_generation.saturating_add(1);
                    catalog_cache.connection = connection_generation;
                    defer_background_reads = true;
                    reconnect_delay = RECONNECT_INITIAL_DELAY;
                    next_operation_tick = Instant::now() + ASYNC_OPERATION_TICK_INTERVAL;
                    if !publish_session(
                        &context,
                        &session,
                        &mut catalog_cache,
                        &mut purpose_mirror,
                        &mut labels,
                    ) {
                        return;
                    }
                    if let Some(writer) = lineage_writer.as_mut() {
                        writer.reconnected();
                        observe_pulled(
                            writer,
                            &context,
                            session.published_state(),
                            true,
                            snapshot_started_at,
                        );
                    }
                    pulled = Some(session);
                }
                Err(error) => {
                    log_sync_failure(&context, "pulled_read.failed", &error);
                    if !publish_failure(&context, error) {
                        return;
                    }
                    reconnect_at = Instant::now() + reconnect_delay;
                    reconnect_delay = next_reconnect_delay(reconnect_delay);
                    defer_background_reads = true;
                }
            }
        }

        if Instant::now() >= next_operation_tick {
            next_operation_tick = Instant::now() + ASYNC_OPERATION_TICK_INTERVAL;
            let now_unix_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
                .unwrap_or(0);
            if let Some(runtime) = context.runtime.upgrade() {
                let changed = match runtime.lock() {
                    Ok(mut guard) => {
                        let changed = guard.tick_async_operations(now_unix_ms);
                        // Each machine decides its own agents' sleep: this
                        // core's, or a node's that dials it.
                        changed
                            | match &context.target {
                                SessionSyncTarget::Local { .. } => {
                                    guard.tick_agent_sleep(now_unix_ms)
                                }
                                SessionSyncTarget::Remote { target_id, .. } => {
                                    guard.tick_node_agent_sleep(target_id, now_unix_ms)
                                }
                            }
                    }
                    Err(_) => false,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            }
            if labels.as_mut().is_some_and(|worker| {
                take_label_switch(&context, worker)
                    | take_session_rereads(&context, worker)
                    | worker.tick(Instant::now())
            }) && let Some(current) = pulled.as_ref()
                && !publish_session(
                    &context,
                    current,
                    &mut catalog_cache,
                    &mut purpose_mirror,
                    &mut labels,
                )
            {
                return;
            }
        }

        // With no window attached only label work runs; these readers feed
        // nothing else (PRD labels-in-hided B29). The usage reader also runs
        // while background Memory analysis needs the weekly figure (PRD
        // project-memory-v2 D-10 b).
        let window_attached = window_reads(&context, worktree_reader.as_mut());
        let connected = pulled.is_some();
        let run_background_reads = connected && !defer_background_reads && window_attached;
        let read_usage = connected
            && !defer_background_reads
            && (run_background_reads || read_memory_reads_usage(&context));
        let read_device_agents = connected && !defer_background_reads;
        defer_background_reads = false;
        if read_usage && let Some(reader) = usage_reader.as_mut() {
            let Some(activity) = read_usage_activity(&context) else {
                return;
            };
            if let Some(usage) = reader.read_if_due(activity)
                && !publish_usage(&context, usage)
            {
                return;
            }
        }
        // Letters are delivered with no window attached, so the versions the
        // doorbell reads a composer by are read then too.
        if read_device_agents
            && let Some(reader) = device_agents_reader.as_mut()
            && let Some(Some(diagnosis)) = reader.read_if_due()
            && !publish_device_agents(&context, diagnosis)
        {
            return;
        }
        if run_background_reads {
            if let Some(reader) = ports_reader.as_mut() {
                // `lsof` runs here, outside every lock; only the result is handed
                // in.
                if let Some(ports) = reader.read_if_due()
                    && !publish_ports(&context, ports)
                {
                    return;
                }
            }

            if let Some(reader) = worktree_reader.as_mut() {
                let Some(request) = read_worktrees_request(&context) else {
                    return;
                };
                // A settled removal already took its worktree out of the
                // catalog, so the rows built on it are rebuilt on this wake
                // rather than when the next read lands.
                let mut rebuild = request.removals != published_removals;
                published_removals = request.removals;
                if let Some(answer) = reader.read_if_due(request) {
                    // New worktree facts change which rows exist and what they
                    // say, so the catalog is rebuilt rather than only stored.
                    match publish_worktrees(&context, answer) {
                        None => {
                            return;
                        }
                        Some(changed) => rebuild |= changed,
                    }
                }
                if rebuild
                    && let Some(current) = pulled.as_ref()
                    && !publish_session(
                        &context,
                        current,
                        &mut catalog_cache,
                        &mut purpose_mirror,
                        &mut labels,
                    )
                {
                    return;
                }
            }

            if let Some(reader) = github_reader.as_mut() {
                let Some(request) = read_github_request(&context) else {
                    return;
                };
                if let Some(request) = request
                    && let Some(github) = reader.read_if_due(request)
                    && !publish_github(&context, github)
                {
                    return;
                }
            }

            if let Some(reader) = github_poll_reader.as_mut() {
                let Some(request) = read_github_poll_request(&context) else {
                    return;
                };
                if let Some(poll) = reader.read_if_due(request)
                    && !ingest_github_poll(&context, poll)
                {
                    return;
                }
            }

            if let Some(reader) = disk_reader.as_mut() {
                let Some(request) = read_disk_request(&context) else {
                    return;
                };
                if let Some(disk) = reader.read_if_due(request)
                    && !publish_disk_usage(&context, disk)
                {
                    return;
                }
            }

            if let Some(reader) = sweep_reader.as_mut() {
                let Some(request) = read_sweep_request(&context) else {
                    return;
                };
                if let Some((disk, whole)) = reader.poll(request)
                    && !publish_swept_disk(&context, disk, whole)
                {
                    return;
                }
            }

            if let Some(reader) = ai_reader.as_mut() {
                let Some((request, queued_settings, standing_moved)) = read_ai_request(&context)
                else {
                    return;
                };
                if standing_moved {
                    context.notifier.notify();
                }
                // The settings write is file I/O, so it happens here rather than
                // under the runtime mutex that took the operator's choice.
                if let Some(settings) = queued_settings {
                    // The choice has already been taken out of the runtime, so a
                    // home this process could not resolve must not swallow it in
                    // silence; it is the same failure as a refused write and it
                    // reaches the same line on the group.
                    let saved = match hook_home.as_deref() {
                        Some(home) => save_ai_settings(&context, home, &settings),
                        None => {
                            crate::diagnostic!(serde_json::json!({
                                "component": "ai_settings",
                                "kind": "settings.write_skipped",
                                "message": "no home directory on this session",
                            }));
                            report_ai_settings_failure(
                                &context,
                                "The choice could not be saved (no home directory); \
                             it applies to this session only"
                                    .to_string(),
                            )
                        }
                    };
                    if !saved {
                        return;
                    }
                }
                if let Some(background_ai) = reader.read_if_due(request)
                    && !publish_background_ai(&context, background_ai)
                {
                    return;
                }
            }
        }

        // A start the published read has pending can end with nothing the
        // engine reports, so the read itself says when to read again.
        let settles_at = pulled
            .as_ref()
            .and_then(|session| session.published_state().settles_at);
        let timeout = coordinator_wait(
            pulled.is_some(),
            reconnect_at,
            next_operation_tick,
            settles_at,
            Instant::now(),
        );
        // A settle that comes due is published whatever the read shows.
        let (message, settle) = match receiver.recv_timeout(timeout) {
            Err(RecvTimeoutError::Timeout) if settle_due(settles_at, Instant::now()) => {
                (Ok(CoordinatorMessage::Republish), true)
            }
            message => (message, false),
        };
        match message {
            Ok(CoordinatorMessage::RefreshTopology) => {
                // Discard the old stream generation. The next bootstrap is a
                // read only; it can safely supersede a move's final drawing
                // even when another client has since moved or closed it.
                pulled = None;
                reconnect_at = Instant::now();
                defer_background_reads = true;
            }
            Ok(CoordinatorMessage::Stop) => {
                return;
            }
            Ok(CoordinatorMessage::Republish) => {
                let asked = republish.take() || settle;
                // The engine woke this thread because its session changed, or
                // the core asked: the session is read again, whole, and
                // published.
                if pulled.is_none() {
                    // Not connected yet; the connect reads it.
                    continue;
                }
                let started_at = Instant::now();
                match super::pulled::PulledSession::read(&context) {
                    // A burst of the engine's changes that the session does
                    // not show, or that a publish already read, publishes
                    // nothing more: a publish rebuilds every pane and agent
                    // under the runtime lock.
                    Ok(session)
                        if !publishes(
                            asked,
                            pulled.as_ref().map(|previous| previous.published_state()),
                            session.published_state(),
                        ) => {}
                    Ok(session) => {
                        let moved = pulled.as_ref().is_none_or(|previous| {
                            previous.published_state().agents != session.published_state().agents
                        });
                        let current = pulled.insert(session);
                        if !publish_session(
                            &context,
                            current,
                            &mut catalog_cache,
                            &mut purpose_mirror,
                            &mut labels,
                        ) {
                            return;
                        }
                        if let Some(writer) = lineage_writer.as_mut() {
                            observe_pulled(
                                writer,
                                &context,
                                current.published_state(),
                                moved,
                                started_at,
                            );
                        }
                    }
                    Err(error) => {
                        log_sync_failure(&context, "pulled_read.failed", &error);
                        pulled = None;
                        if !publish_failure(&context, error) {
                            return;
                        }
                        reconnect_at = Instant::now() + reconnect_delay;
                        reconnect_delay = next_reconnect_delay(reconnect_delay);
                    }
                }
            }
            Ok(CoordinatorMessage::Labels) => {
                // A disconnected session is stale; its labels wait for the
                // reconnect's first publish rather than clearing the error.
                if labels.as_mut().is_some_and(|worker| {
                    let changed = worker.drain(Instant::now(), unix_now_ms())
                        | exchange_pull_requests(&context, worker);
                    hand_memory_receipts(&context, worker);
                    changed
                }) && let Some(current) = pulled.as_ref()
                    && !publish_session(
                        &context,
                        current,
                        &mut catalog_cache,
                        &mut purpose_mirror,
                        &mut labels,
                    )
                {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return;
            }
        }
    }
}

/// Whether a session read is published: one a wake of the core's asked for
/// always is, one the engine's wake brought only when it differs from the
/// last one published.
fn publishes(asked: bool, previous: Option<&ProjectionState>, read: &ProjectionState) -> bool {
    asked || previous != Some(read)
}

fn log_sync_failure(context: &SessionSyncContext, kind: &str, error: &SessionFetchError) {
    crate::diagnostic!(json!({
        "component": "session_sync",
        "kind": kind,
        "target": context.log_target(),
        "state": error.state(),
        "message": error.message(),
    }));
}

fn publish_session(
    context: &SessionSyncContext,
    session: &super::pulled::PulledSession,
    catalog_cache: &mut CatalogState,
    purpose_mirror: &mut Option<crate::worktree_control::PurposeMirror>,
    labels: &mut Option<LabelWorker>,
) -> bool {
    let mut payload = session.project();
    let resting = session.published_state().resting_payloads();
    let not_resumed = &session.published_state().not_resumed;
    let overlay = labels.as_mut().map(|worker| {
        exchange_pull_requests(context, worker);
        hand_memory_receipts(context, worker);
        take_label_switch(context, worker);
        observe_labels(worker, session.published_state());
        worker.overlay()
    });
    // A pass observes its panes for delivery in the same step under the lock
    // that lays their turns (`observe_device_session` here, the ingest for
    // this machine), so a tool or turn-end ask never meets an observation
    // whose turn is not laid yet (with-hide/hide-private#560). That step is
    // bounded memory work; no delivery I/O or notifier is started in it.
    if let SessionSyncTarget::Remote { target_id, .. } = &context.target {
        let Some(runtime) = context.runtime.upgrade() else {
            return false;
        };
        let hook_tokens = match runtime.lock() {
            Ok(mut guard) => {
                if !guard.remote_coordinator_is_current(target_id, &context.engine) {
                    return false;
                }
                let tokens =
                    guard.observe_device_session(target_id, &mut payload, overlay.as_ref());
                guard.end_unresumed(target_id, None, &payload, not_resumed);
                tokens
            }
            Err(_) => return false,
        };
        drop(runtime);
        let projection = session.published_state().project_remote(target_id, payload);
        let (fetched, excluded) = match projection {
            Ok((mut session, excluded)) => {
                session.pane_hook_tokens = hook_tokens
                    .into_iter()
                    .map(|(pane, tokens)| {
                        (crate::address::scoped_pane_id(target_id, &pane), tokens)
                    })
                    .collect();
                (Ok(session), excluded)
            }
            Err(error) => (Err(error), Vec::new()),
        };
        for exclusion in excluded {
            crate::diagnostic!(json!({
                "component": "remote_session",
                "kind": "agent.excluded",
                "target": target_id,
                "pane_id": exclusion.pane_id,
                "source_index": exclusion.source_index,
                "message": exclusion.reason,
            }));
        }
        let Some(runtime) = context.runtime.upgrade() else {
            return false;
        };
        let changed = match runtime.lock() {
            Ok(mut guard) => {
                if !guard.remote_coordinator_is_current(target_id, &context.engine) {
                    return false;
                }
                guard.ingest_remote_session(target_id, fetched)
            }
            Err(_) => return false,
        };
        drop(runtime);
        if changed {
            context.notifier.notify();
        }
        return true;
    }

    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let (
        node,
        registrations,
        worktrees,
        unconfirmed_created_purposes,
        created_tab_clamps,
        runtime_paths,
        accepted_tabs,
    ) = match runtime.lock() {
        Ok(mut guard) => {
            guard.begin_catalog_coordinator();
            (
                guard.node().clone(),
                guard.snapshot().ui_state.workspace_registrations.clone(),
                guard.worktree_catalog(),
                guard.unconfirmed_created_purpose_values(),
                guard.created_tab_clamps(),
                guard.catalog_paths(),
                guard
                    .snapshot()
                    .navigator
                    .workspaces
                    .iter()
                    .flat_map(|workspace| &workspace.checkouts)
                    .flat_map(|checkout| {
                        checkout
                            .tabs
                            .iter()
                            .filter_map(|tab| Some((tab.id.clone()?, checkout.path.clone())))
                    })
                    .collect::<HashMap<_, _>>(),
            )
        }
        Err(_) => return false,
    };
    drop(runtime);

    // The reader owns the only blocking work. Its answer describes facts,
    // never a session to replay; build from the current session below.
    let mut wanted = workspace::PathIndex::wanted(&node, &registrations, &[], &worktrees);
    wanted.extend(Runtime::session_paths(&payload));
    wanted.extend(created_tab_clamps.iter().map(|clamp| clamp.path.clone()));
    let reader = catalog_cache
        .reader
        .get_or_insert_with(|| path_facts::PathReader::new(context, catalog_cache.wake.clone()));
    let answer = reader.poll(path_facts::Request {
        connection: catalog_cache.connection,
        paths: wanted.clone(),
    });
    let mut built_at = catalog_cache
        .as_ref()
        .map(|cache| cache.built_at)
        .unwrap_or_else(Instant::now);
    let previous = paths_to_build_from(
        catalog_cache.as_ref().map(|cache| Arc::clone(&cache.paths)),
        runtime_paths,
        &node,
        &registrations,
    );
    let paths = match answer {
        Some(Ok(paths)) => {
            built_at = Instant::now();
            if *previous == paths {
                previous
            } else {
                Arc::new(paths)
            }
        }
        Some(Err(error)) => {
            crate::diagnostic!(json!({
                "component": "session_sync",
                "kind": "catalog.path_facts_failed",
                "target": context.log_target(),
                "message": error,
            }));
            previous
        }
        None => previous,
    };

    // The catalog is built here, outside the lock, from the cwd the runtime
    // will read for a created tab, not the one it was born with.
    // Only a tab this session carries has a cwd to clamp; a record whose tab
    // has not arrived costs a publish nothing.
    let created_tab_clamps = created_tab_clamps
        .into_iter()
        .filter(|clamp| {
            payload
                .layouts
                .iter()
                .any(|layout| layout.tab_id == clamp.tab_id)
        })
        .collect::<Vec<_>>();
    let clamped;
    let birth_free = if created_tab_clamps.is_empty() {
        &payload
    } else {
        let mut copy = payload.clone();
        Runtime::clamp_created_tab_cwds(&mut copy, &created_tab_clamps, &paths);
        clamped = copy;
        &clamped
    };
    let mut spaces = Runtime::session_spaces(birth_free, &paths);
    // An unanswered cwd has no authority to invent a folder project. Known
    // tabs retain their accepted checkout in the runtime's reconcile.
    for space in &mut spaces {
        space.cwds.retain(|cwd| paths.knows(cwd));
        for layout in &birth_free.layouts {
            if layout.workspace_id == space.id
                && let Some(path) = accepted_tabs.get(&layout.tab_id)
                && layout.panes.iter().any(|pane| {
                    Runtime::pane_cwd(birth_free, &pane.pane_id)
                        .is_some_and(|cwd| !paths.knows(&cwd))
                })
                && !space.cwds.contains(path)
            {
                space.cwds.push(path.clone());
            }
        }
    }
    let cache_is_fresh = catalog_cache.as_ref().is_some_and(|cache| {
        Arc::ptr_eq(&cache.paths, &paths)
            && cache.registrations == registrations
            && cache.spaces == spaces
            && cache.worktrees == worktrees
    });
    if !cache_is_fresh {
        let workspaces =
            workspace::build_catalog(&node, &registrations, &spaces, &worktrees, &paths);
        catalog_cache.cache = Some(CatalogCache {
            registrations: registrations.clone(),
            spaces,
            worktrees,
            workspaces,
            paths,
            built_at,
            purposes_synced: None,
        });
    } else if let Some(cache) = catalog_cache.as_mut() {
        cache.built_at = built_at;
    }
    let cache = catalog_cache
        .as_mut()
        .expect("catalog cache is filled on a miss");
    if let Some(mirror) = purpose_mirror.as_mut()
        && cache.purposes_synced.as_ref() != Some(&unconfirmed_created_purposes)
    {
        mirror.sync(
            &cache.spaces,
            &cache.workspaces,
            &unconfirmed_created_purposes,
        );
        cache.purposes_synced = Some(unconfirmed_created_purposes);
    }
    let precomputed = PrecomputedCatalog {
        registrations,
        worktrees: cache.worktrees.clone(),
        workspaces: cache.workspaces.clone(),
        paths: Arc::clone(&cache.paths),
    };

    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    // The unsequenced stream can still contain a layout from an older focus.
    // A focus that differs from Hide's is read back from the engine, but the
    // update itself is not held for that answer: it is ingested now with
    // the differing focus left alone, and the read runs on its own worker
    // outside the lock.
    let host_scope = match &context.target {
        SessionSyncTarget::Local { socket_path } => socket_path.to_str(),
        SessionSyncTarget::Remote { .. } => None,
    };
    let (changed, readback) = match runtime.lock() {
        Ok(mut guard) => {
            // A restore's resting agents become sleeping records first, so
            // this ingest draws their rows as sleeping.
            let kept = guard.adopt_resting(resting, overlay.as_ref());
            // So is an agent a restore did not resume ended, with the engine
            // still naming it until an agent shows in its pane.
            let device = guard.node().to_string();
            guard.end_unresumed(&device, host_scope, &payload, not_resumed);
            // The ingest observes this payload for delivery and lays the
            // turns its observations wait on, with this pass's overlay.
            let (changed, readback) =
                guard.ingest_observed_session(payload, Some(precomputed), host_scope, overlay);
            (changed || kept, readback)
        }
        Err(_) => return false,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
    if let Some(identity) = readback
        && let Err(message) = spawn_focus_readback(context, identity.clone())
    {
        crate::diagnostic!(json!({
            "component": "pane_focus",
            "kind": "pane.focus.stream_readback_not_started",
            "pane_id": identity.target_id,
            "message": message,
        }));
        if let Some(runtime) = context.runtime.upgrade()
            && let Ok(mut guard) = runtime.lock()
        {
            guard.abandon_pane_focus_readback();
        }
    }
    true
}

/// The path answers a catalog is built from while the node's answer to this
/// round's paths is pending: the coordinator's last ones, unless they cannot
/// place a registration of this machine that the runtime's can. A creation
/// installs its own catalog and paths (`ingest_workspace_creation`); building
/// from older answers would draw the new project at its bare path, without
/// the tabs of the repository around it, until the node answered (issue 270).
fn paths_to_build_from(
    cached: Option<Arc<workspace::PathIndex>>,
    runtime: Arc<workspace::PathIndex>,
    node: &crate::node::NodeId,
    registrations: &[crate::model::WorkspaceRegistration],
) -> Arc<workspace::PathIndex> {
    let answers = |paths: &workspace::PathIndex| {
        registrations
            .iter()
            .filter(|registration| *node == registration.device_id)
            .all(|registration| paths.knows(&registration.path))
    };
    match cached {
        Some(cached) if answers(&cached) || !answers(&runtime) => cached,
        _ => runtime,
    }
}

/// The worker that reads the engine's focus for a stream move, outside the runtime
/// lock. It owns the runtime's one readback slot from its start to its last
/// answer, and the guard frees the slot on any other way out.
fn spawn_focus_readback(
    context: &SessionSyncContext,
    first: crate::runtime::PendingPaneFocusControl,
) -> Result<(), String> {
    let engine = Arc::clone(&context.engine);
    let runtime = context.runtime.clone();
    let notifier = context.notifier.clone();
    thread::Builder::new()
        .name("hide-core-focus-readback".to_owned())
        .spawn(move || {
            struct Slot(Weak<Mutex<Runtime>>, bool);
            impl Drop for Slot {
                fn drop(&mut self) {
                    if self.1
                        && let Some(runtime) = self.0.upgrade()
                        && let Ok(mut guard) = runtime.lock()
                    {
                        guard.abandon_pane_focus_readback();
                    }
                }
            }
            let mut slot = Slot(runtime.clone(), true);
            let mut next = Some(first);
            while let Some(identity) = next.take() {
                let answer = live::confirm_pane_focus(engine.as_ref(), &identity.target_id);
                let Some(runtime) = runtime.upgrade() else {
                    return;
                };
                let (changed, following) = match runtime.lock() {
                    Ok(mut guard) => guard.finish_pane_focus_readback(identity, answer),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    notifier.notify();
                }
                slot.1 = following.is_some();
                next = following;
            }
        })
        .map(|_| ())
        .map_err(|error| format!("focus readback worker could not be started: {error}"))
}

/// The label worker for this coordinator's engine, built on the
/// core's shared label services. `None` when the core has none (tests) or
/// the worker could not start, which is logged: the rows then show
/// provider names.
fn start_label_worker(
    context: &SessionSyncContext,
    sender: &Sender<CoordinatorMessage>,
) -> Option<LabelWorker> {
    let (services, language) = {
        let runtime = context.runtime.upgrade()?;
        let guard = runtime.lock().ok()?;
        (guard.label_services()?, guard.output_language())
    };
    let wake_sender = sender.clone();
    let wake: crate::labels::worker::Wake = Arc::new(move || {
        let _ = wake_sender.send(CoordinatorMessage::Labels);
    });
    let worker = match &context.target {
        SessionSyncTarget::Local { socket_path } => match context.node() {
            Some(node) => services.local_worker(
                socket_path,
                Arc::clone(node),
                wake,
                language,
                Arc::clone(&context.engine),
            ),
            None => Ok(None),
        },
        SessionSyncTarget::Remote { target_id, .. } => services
            .device_worker(
                target_id,
                context.runtime.clone(),
                wake,
                language,
                Arc::clone(&context.engine),
            )
            .map(Some),
    };
    worker.unwrap_or_else(|message| {
        crate::diagnostic!(json!({
            "component": "labels",
            "kind": "worker.start_failed",
            "target": context.log_target(),
            "message": message,
        }));
        None
    })
}

/// Hands the runtime the pull requests this Mac's worker sighted that GitHub's
/// answer did not hold, and the worker the runtime's pull request creation
/// times, under one brief lock. Only this Mac's projects have their pull
/// requests read (a device's rows link none), so a device's worker is handed
/// no times and keeps no sightings. Returns whether a session's pull requests
/// changed.
fn exchange_pull_requests(context: &SessionSyncContext, worker: &mut LabelWorker) -> bool {
    if !context.is_local() {
        return false;
    }
    let sighted = worker.take_sighted();
    let now = (!sighted.is_empty()).then(|| (Instant::now(), unix_now_ms()));
    let times = context.runtime.upgrade().and_then(|runtime| {
        runtime.lock().ok().map(|mut guard| {
            if let Some(now) = now {
                guard.read_sighted_pull_requests(&sighted, now);
            }
            guard.pull_request_times()
        })
    });
    times.is_some_and(|times| worker.set_pull_request_times(times))
}

/// Hands the runtime the Memory receipts this target's worker found in its
/// sessions and the sessions its live panes hold, under one brief lock taken
/// only when there is something; the runtime checks and records receipts off
/// the lock (PRD project-memory-v2 D-16), for this machine and for a node
/// that dials in (PRD core-host-node-move B14).
fn hand_memory_receipts(context: &SessionSyncContext, worker: &mut LabelWorker) {
    let Some(handoff) = worker.take_memory_handoff() else {
        return;
    };
    let device = match &context.target {
        SessionSyncTarget::Local { .. } => None,
        SessionSyncTarget::Remote { target_id, .. } => Some(target_id.as_str()),
    };
    if let Some(runtime) = context.runtime.upgrade()
        && let Ok(mut guard) = runtime.lock()
    {
        let device = device.map_or_else(|| guard.node().as_str().to_owned(), str::to_owned);
        guard.take_memory_handoff(&device, handoff);
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

/// Hands the worker the operator's agent-summary switch and the core's
/// output language, under one brief lock, on every tick and publish: a
/// turned switch reaches the screen and the running request within one
/// tick, and a changed language the next request. Returns whether the shown
/// labels changed.
fn take_label_switch(context: &SessionSyncContext, worker: &mut LabelWorker) -> bool {
    let taken = context.runtime.upgrade().and_then(|runtime| {
        runtime
            .lock()
            .ok()
            .map(|guard| (guard.labels_asked(), guard.output_language()))
    });
    taken.is_some_and(|(on, language)| worker.follow_core(on, language, Instant::now()))
}

/// Hands this machine's worker the panes whose sessions typed input waits
/// to see recorded (`Runtime::take_session_rereads`), under one brief lock.
/// A device's worker reads on its own machine and is handed none.
fn take_session_rereads(context: &SessionSyncContext, worker: &mut LabelWorker) -> bool {
    if !context.is_local() {
        return false;
    }
    let panes = context
        .runtime
        .upgrade()
        .and_then(|runtime| {
            runtime
                .lock()
                .ok()
                .map(|mut guard| guard.take_session_rereads())
        })
        .unwrap_or_default();
    !panes.is_empty() && worker.read_again(&panes)
}

/// Hands the worker the agents and the complete pane topology the session
/// publishes. Per publish this is one pass over the agents and the panes.
fn observe_labels(worker: &mut LabelWorker, state: &ProjectionState) {
    let agents: Vec<ObservedAgent> = state
        .agents
        .iter()
        .map(|agent| ObservedAgent {
            pane_id: agent.pane_id.clone(),
            agent: agent.agent.clone(),
            status: agent.agent_status.clone(),
            reference: agent
                .agent_session
                .as_ref()
                .map(|session| (session.kind.clone(), session.value.clone())),
            cwd: agent.cwd.clone(),
            state_change_seq: agent.state_change_seq,
        })
        .collect();
    let live: HashSet<String> = state
        .panes
        .iter()
        .map(|pane| pane.pane_id.clone())
        .collect();
    let now_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0);
    worker.observe(&agents, Some(&live), Instant::now(), now_unix_ms);
}

/// Starts the read axis's view of this connection before its first
/// projection: the server reached may be the one lost or a restored one, and
/// only the panes' marks tell (`agent_state::ReadConnection`).
fn begin_read_connection(context: &SessionSyncContext, snapshot_started_at: Instant) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let Ok(mut guard) = runtime.lock() else {
        return false;
    };
    guard.begin_read_connection(context.read_target());
    let changed =
        guard.release_pane_move_publication_barriers(context.read_target(), snapshot_started_at);
    drop(guard);
    if changed {
        context.notifier.notify();
    }
    true
}

/// What the lineage writer reads from the runtime in one lock: the delivery
/// ledger and the panes that should carry this connection's mark.
/// Hands a pulled engine's agents to the lineage writer, as the event path
/// hands it each agent list the engine answers: the writer settles the
/// registrations Hide owes an agent once its session shows (a fork, a spawn
/// given no prompt) and writes each registration's lineage tokens on its
/// pane. Without it no fork on the core's own engine is ever registered.
/// `moved` says the agents differ from the last session read.
fn observe_pulled(
    writer: &mut crate::coordination::lineage::Writer,
    context: &SessionSyncContext,
    state: &ProjectionState,
    moved: bool,
    started_at: Instant,
) {
    let Some((ledger, marks)) = lineage_inputs(context) else {
        return;
    };
    writer.observe(
        &ledger,
        &state.agents,
        &|pane| state.panes.iter().any(|known| known.pane_id == pane),
        &marks,
        moved,
        started_at,
    );
}

fn lineage_inputs(
    context: &SessionSyncContext,
) -> Option<(
    Arc<crate::delivery::ledger::Ledger>,
    BTreeMap<String, String>,
)> {
    let runtime = context.runtime.upgrade()?;
    let guard = runtime.lock().ok()?;
    let ledger = guard.delivery_state().ok()?;
    Some((ledger, guard.wanted_server_marks(context.read_target())))
}

/// Forgets the host's published panes before a fresh snapshot is asked for,
/// and notes the ledger position that snapshot is complete for.
fn begin_delivery_pane_read(context: &SessionSyncContext) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let Ok(mut guard) = runtime.lock() else {
        return false;
    };
    let device = match &context.target {
        SessionSyncTarget::Local { .. } => guard.node().to_string(),
        SessionSyncTarget::Remote { target_id, .. } => {
            if !guard.remote_coordinator_is_current(target_id, &context.engine) {
                return false;
            }
            target_id.clone()
        }
    };
    guard.begin_delivery_pane_read(&device);
    true
}

fn publish_failure(context: &SessionSyncContext, error: SessionFetchError) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => match &context.target {
            SessionSyncTarget::Local { .. } => guard.ingest_session_with_catalog(Err(error), None),
            SessionSyncTarget::Remote { target_id, .. } => {
                if !guard.remote_coordinator_is_current(target_id, &context.engine) {
                    return false;
                }
                guard.ingest_remote_session(target_id, Err(error))
            }
        },
        Err(_) => return false,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
    true
}

fn publish_usage(
    context: &SessionSyncContext,
    usage: Vec<crate::usage::model::AgentUsage>,
) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => guard.ingest_usage(usage),
        Err(_) => return false,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
    true
}

fn read_usage_activity(context: &SessionSyncContext) -> Option<crate::usage::UsageActivity> {
    let runtime = context.runtime.upgrade()?;
    runtime.lock().ok().map(|guard| guard.usage_activity())
}

/// Whether the weekly usage read runs with no window attached: Memory analysis
/// wants it, or the usage limit needs a fresh figure.
fn read_memory_reads_usage(context: &SessionSyncContext) -> bool {
    context
        .runtime
        .upgrade()
        .and_then(|runtime| {
            runtime
                .lock()
                .ok()
                .map(|guard| guard.reads_usage_headless())
        })
        .unwrap_or(false)
}

/// Whether a window draws the snapshot; a gone runtime reads as attached,
/// and the loop's own check ends the coordinator.
/// Whether a renderer is attached, so the readers only a window uses run on
/// this wake. With none, the worktree reader's Git watch is ended too
/// (`WorktreeReader::rest`): nothing takes its reports while the window is
/// closed, and the next read after a renderer is back starts it again.
fn window_reads(
    context: &SessionSyncContext,
    worktree_reader: Option<&mut crate::worktrees::WorktreeReader>,
) -> bool {
    let attached = read_ui_attached(context);
    if !attached && let Some(reader) = worktree_reader {
        reader.rest();
    }
    attached
}

fn read_ui_attached(context: &SessionSyncContext) -> bool {
    context
        .runtime
        .upgrade()
        .and_then(|runtime| runtime.lock().ok().map(|guard| guard.ui_attached()))
        .unwrap_or(true)
}

/// Reads what the worktree reader needs, holding the runtime mutex only for
/// the read itself. `None` means the runtime is gone.
fn read_worktrees_request(
    context: &SessionSyncContext,
) -> Option<crate::worktrees::WorktreeRequest> {
    let runtime = context.runtime.upgrade()?;
    let request = runtime.lock().ok()?.worktrees_request();
    drop(runtime);
    Some(request)
}

/// The GitHub read due now; `Some(None)` while the core waits for its move's
/// link, which reads nothing from GitHub until then.
fn read_github_request(
    context: &SessionSyncContext,
) -> Option<Option<crate::github::GithubRequest>> {
    let runtime = context.runtime.upgrade()?;
    let request = {
        let mut guard = runtime.lock().ok()?;
        if guard.effects_held() {
            return Some(None);
        }
        let now = Instant::now();
        match &context.target {
            SessionSyncTarget::Local { .. } => {
                guard.reread_pending_checks(now);
                guard.reread_stale_github(now);
                guard.ask_polled_changes(now);
                guard.github_request()
            }
            SessionSyncTarget::Remote { target_id, .. } => {
                guard.device_github_request(target_id, now)
            }
        }
    };
    drop(runtime);
    Some(Some(request))
}

/// What the advisory poll reads now: this machine's projects with open pull
/// requests, built from the last answers under the lock and read after it is
/// released. Empty while the core waits for its move's link.
fn read_github_poll_request(
    context: &SessionSyncContext,
) -> Option<crate::github_poll::PollRequest> {
    let runtime = context.runtime.upgrade()?;
    let request = {
        let mut guard = runtime.lock().ok()?;
        if guard.effects_held() {
            crate::github_poll::PollRequest::default()
        } else {
            guard.github_poll_request()
        }
    };
    drop(runtime);
    Some(request)
}

/// Hands the projects the poll saw change to the runtime, which asks the full
/// read for them by the rule a look follows. False when the core is gone.
fn ingest_github_poll(context: &SessionSyncContext, poll: crate::github_poll::PollAnswer) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let Ok(mut guard) = runtime.lock() else {
        return false;
    };
    guard.note_polled_changes(poll.changed);
    let held = guard.note_poll_held(poll.held);
    drop(guard);
    drop(runtime);
    if held {
        context.notifier.notify();
    }
    true
}

fn read_disk_request(context: &SessionSyncContext) -> Option<crate::disk::DiskRequest> {
    let runtime = context.runtime.upgrade()?;
    let request = match &context.target {
        SessionSyncTarget::Local { .. } => runtime.lock().ok()?.disk_request(),
        SessionSyncTarget::Remote { target_id, .. } => {
            runtime.lock().ok()?.device_disk_request(target_id)
        }
    };
    drop(runtime);
    Some(request)
}

fn read_sweep_request(context: &SessionSyncContext) -> Option<crate::disk::DiskRequest> {
    let runtime = context.runtime.upgrade()?;
    let request = runtime.lock().ok()?.disk_sweep_request(sweep_node(context));
    drop(runtime);
    Some(request)
}

/// The machine a sweep is keyed by: `None` for this one.
fn sweep_node(context: &SessionSyncContext) -> Option<&str> {
    match &context.target {
        SessionSyncTarget::Local { .. } => None,
        SessionSyncTarget::Remote { target_id, .. } => Some(target_id),
    }
}

fn publish_swept_disk(
    context: &SessionSyncContext,
    disk: Vec<crate::model::DiskUsageSnapshot>,
    whole: bool,
) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => guard.ingest_swept_disk(sweep_node(context), disk, whole),
        Err(_) => return false,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
    true
}

/// What the provider probe should ask, and any choice waiting to be written.
///
/// Both come out of one lock acquisition rather than two. The coordinator
/// takes this lock once per wake for each reader it drives, and a settings
/// write is rare enough that it does not deserve a wake of its own.
fn read_ai_request(
    context: &SessionSyncContext,
) -> Option<(crate::ai::AiRequest, Option<hide_ai::AiSettings>, bool)> {
    let runtime = context.runtime.upgrade()?;
    let read = {
        let mut guard = runtime.lock().ok()?;
        let standing_moved = guard.refresh_ai_standing();
        (
            guard.ai_request(),
            guard.take_ai_settings_save(),
            standing_moved,
        )
    };
    drop(runtime);
    Some(read)
}

/// Reads the operator's saved background AI choice once at startup.
///
/// A file that is not there means nobody has chosen, so the defaults stand
/// and nothing is reported. A file that exists and cannot be read is stated
/// once, here, and the defaults are used with that reason attached rather
/// than in silence.
fn publish_ai_settings(context: &SessionSyncContext, home: &std::path::Path) {
    let (settings, reason) = match hide_ai::settings::load(home) {
        Ok(settings) => (settings, None),
        Err(error) => {
            crate::diagnostic!(serde_json::json!({
                "component": "ai_settings",
                "kind": "settings.unreadable",
                "message": error.to_string(),
            }));
            (
                hide_ai::AiSettings::default(),
                Some(format!(
                    "The saved choice could not be read ({error}); the defaults are in use"
                )),
            )
        }
    };
    let Some(runtime) = context.runtime.upgrade() else {
        return;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => guard.ingest_ai_settings(settings, reason),
        Err(_) => return,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
}

/// Writes a choice the runtime queued. `false` means the runtime is gone.
fn save_ai_settings(
    context: &SessionSyncContext,
    home: &std::path::Path,
    settings: &hide_ai::AiSettings,
) -> bool {
    let Err(error) = hide_ai::settings::save(home, settings) else {
        // The file now holds the operator's choice, so the group says
        // "chosen" from this write on, not only after the next launch reads it.
        let Some(runtime) = context.runtime.upgrade() else {
            return false;
        };
        let changed = match runtime.lock() {
            Ok(mut guard) => guard.ingest_ai_settings(settings.clone(), None),
            Err(_) => return false,
        };
        drop(runtime);
        if changed {
            context.notifier.notify();
        }
        return true;
    };
    crate::diagnostic!(serde_json::json!({
        "component": "ai_settings",
        "kind": "settings.write_failed",
        "message": error.to_string(),
    }));
    report_ai_settings_failure(
        context,
        format!("The choice could not be saved ({error}); it applies to this session only"),
    )
}

/// Puts the reason a choice was not written on the group that took it, and
/// reports whether the coordinator can carry on. A choice the runtime has
/// already handed over is gone either way; what this decides is whether the
/// operator is told.
fn report_ai_settings_failure(context: &SessionSyncContext, message: String) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => guard.report_ai_settings_failure(message),
        Err(_) => return false,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
    true
}

/// Stores a worktree catalog. `None` means the runtime is gone; `Some(true)`
/// means the rows changed and the navigator catalog must be rebuilt.
fn publish_worktrees(
    context: &SessionSyncContext,
    answer: crate::worktrees::WorktreeAnswer,
) -> Option<bool> {
    let runtime = context.runtime.upgrade()?;
    let mut guard = runtime.lock().ok()?;
    let current = answer.observations_current && guard.worktrees_request() == answer.request;
    let changed = guard.ingest_worktrees_answer(answer.catalog, answer.removals, current);
    drop(guard);
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
    Some(changed)
}

fn publish_github(context: &SessionSyncContext, answer: crate::github::GithubAnswer) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => match &context.target {
            SessionSyncTarget::Local { .. } => {
                let current = guard.github_request() == answer.request;
                guard.ingest_github_answer(answer.snapshot, current)
            }
            SessionSyncTarget::Remote { target_id, .. } => {
                guard.ingest_device_github(target_id, &answer.request, answer.snapshot)
            }
        },
        Err(_) => return false,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
    true
}

fn publish_disk_usage(
    context: &SessionSyncContext,
    disk: Vec<crate::model::DiskUsageSnapshot>,
) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => match &context.target {
            SessionSyncTarget::Local { .. } => guard.ingest_disk_usage(disk),
            SessionSyncTarget::Remote { target_id, .. } => {
                guard.ingest_device_disk_usage(target_id, disk)
            }
        },
        Err(_) => return false,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
    true
}

fn publish_background_ai(
    context: &SessionSyncContext,
    background_ai: crate::model::BackgroundAiSnapshot,
) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => guard.ingest_background_ai(background_ai),
        Err(_) => return false,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
    true
}

/// Keeps what a device's hook diagnosis said, for the agent versions its panes
/// run. Nothing on the snapshot changes with it, so nothing is announced; the
/// doorbell's next pass reads it. False when the runtime has ended.
fn publish_device_agents(
    context: &SessionSyncContext,
    diagnosis: hide_agent_hooks::Diagnosis,
) -> bool {
    let SessionSyncTarget::Remote { target_id, .. } = &context.target else {
        return true;
    };
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    match runtime.lock() {
        Ok(mut guard) => {
            guard.ingest_device_hook_diagnosis(target_id, diagnosis);
            true
        }
        Err(_) => false,
    }
}

fn publish_ports(
    context: &SessionSyncContext,
    ports: crate::model::ListeningPortsSnapshot,
) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    let changed = match (runtime.lock(), &context.target) {
        (Ok(mut guard), SessionSyncTarget::Local { .. }) => guard.ingest_listening_ports(ports),
        (Ok(mut guard), SessionSyncTarget::Remote { target_id, .. }) => {
            guard.ingest_device_listening_ports(target_id, ports)
        }
        (Err(_), _) => return false,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
    true
}

fn coordinator_wait(
    connected: bool,
    reconnect_at: Instant,
    next_operation_tick: Instant,
    settles_at: Option<Instant>,
    now: Instant,
) -> Duration {
    let deadline = if connected {
        // The engine wakes this mailbox on changes and nothing reconnects
        // while connected, so the reconnect clock can stay in the past
        // without making the coordinator spin; a start the published read
        // has pending is read again when it can settle.
        settles_at.map_or(next_operation_tick, |at| at.min(next_operation_tick))
    } else {
        reconnect_at.min(next_operation_tick)
    };
    deadline.saturating_duration_since(now)
}

/// Whether the moment the published read named for a pending start to settle
/// (`ProjectionState::settles_at`) has come, so the session is read and
/// published again.
fn settle_due(settles_at: Option<Instant>, now: Instant) -> bool {
    settles_at.is_some_and(|at| now >= at)
}

fn next_reconnect_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(RECONNECT_MAX_DELAY)
}

#[cfg(test)]
mod republish_tests {
    use super::*;
    use crate::engine::fake::session;

    /// Wakes before the coordinator takes the first are one message, and only
    /// a wake of the core's asks for a publish the session may not show.
    #[test]
    fn engine_wakes_ask_for_no_publish_and_a_core_wake_does_once() {
        let (sender, receiver) = channel();
        let flags = Arc::new(RepublishFlags::default());
        let engine = RepublishWaker::engine(sender.clone(), Arc::clone(&flags));
        let core = RepublishWaker::asking(sender, Arc::clone(&flags));
        engine.wake();
        engine.wake();
        assert_eq!(receiver.try_iter().count(), 1);
        assert!(!flags.take());
        engine.wake();
        core.wake();
        assert_eq!(receiver.try_iter().count(), 1);
        assert!(flags.take());
        assert!(!flags.take());
    }

    /// An engine wake whose read equals the last publish (a title whose
    /// spinner alone turned) publishes nothing; a changed read, the first
    /// read and a wake the core asked for do.
    #[test]
    fn only_a_read_that_moved_or_one_the_core_asked_for_is_published() {
        let one = session(json!({"panes": [{"pane_id": "w1:p1", "terminal_title": "Fix"}]}));
        let two = session(json!({"panes": [{"pane_id": "w1:p1", "terminal_title": "Fix it"}]}));
        assert!(!publishes(false, Some(&one), &one));
        assert!(publishes(false, Some(&one), &two));
        assert!(publishes(false, None, &one));
        assert!(publishes(true, Some(&one), &one));
    }
}

#[cfg(test)]
mod coordinator_wait_tests {
    use super::*;

    #[test]
    fn a_connected_coordinator_waits_for_its_tick_after_the_reconnect_clock_expires() {
        let now = Instant::now();
        let expired = now - Duration::from_secs(1);
        assert_eq!(
            coordinator_wait(true, expired, now + Duration::from_millis(250), None, now),
            Duration::from_millis(250),
        );
        assert_eq!(
            coordinator_wait(true, expired, now, None, now),
            Duration::ZERO
        );
    }

    /// A start the published read has pending is read again when it can
    /// settle, sooner than the tick, and then counts as due; a later moment
    /// leaves the tick in charge.
    #[test]
    fn a_connected_coordinator_reads_again_when_the_published_read_can_settle() {
        let now = Instant::now();
        let expired = now - Duration::from_secs(1);
        let tick = now + Duration::from_millis(250);
        let soon = now + Duration::from_millis(40);
        assert_eq!(
            coordinator_wait(true, expired, tick, Some(soon), now),
            Duration::from_millis(40)
        );
        assert_eq!(
            coordinator_wait(true, expired, tick, Some(now + Duration::from_secs(9)), now),
            Duration::from_millis(250)
        );
        assert!(!settle_due(Some(soon), now));
        assert!(settle_due(Some(soon), soon));
        assert!(!settle_due(None, soon));
    }

    #[test]
    fn a_disconnected_coordinator_waits_for_reconnect_or_the_operation_tick() {
        let now = Instant::now();
        for (reconnect_ms, expected_ms) in [(0, 0), (100, 100), (5000, 250)] {
            assert_eq!(
                coordinator_wait(
                    false,
                    now + Duration::from_millis(reconnect_ms),
                    now + Duration::from_millis(250),
                    None,
                    now,
                ),
                Duration::from_millis(expected_ms),
            );
        }
    }
}

#[cfg(test)]
mod worktree_observer_tests {
    use super::*;
    use crate::handle::Core;
    use crate::model::{
        CoreOptions, ProjectWorktreesSnapshot, WorktreeCatalogSnapshot, WorktreeSnapshot,
    };
    use crate::worktrees::WorktreeAnswer;
    use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};

    struct ObserverFixture {
        core: Core,
        runtime: Arc<Mutex<Runtime>>,
        context: SessionSyncContext,
        observations: Receiver<bool>,
        _directory: tempfile::TempDir,
    }

    impl ObserverFixture {
        fn new() -> Self {
            let directory = tempfile::tempdir_in(workspace::temp_base_outside_any_repository())
                .expect("isolated observer directory");
            let root = directory.path().canonicalize().expect("fixture path");
            let path = hide_platform::path::to_wire_lossy(&root);
            let options: CoreOptions = serde_json::from_value(json!({
                "schema_version": crate::model::SCHEMA_VERSION,
                "node_id": "test-node",
                "app_state_path": ""
            }))
            .expect("workerless core options");
            let mut runtime = Runtime::new(
                options,
                crate::environment::EnvironmentReport {
                    statuses: Vec::new(),
                    home_path: None,
                    codex_home: None,
                },
                std::sync::Arc::new(hide_node::Local::of_process()),
                crate::node::test_devices(),
            );
            let catalog = WorktreeCatalogSnapshot {
                projects: vec![ProjectWorktreesSnapshot {
                    root_path: path.clone(),
                    worktrees: vec![WorktreeSnapshot {
                        path: path.clone(),
                        branch: Some("feature/preflight".into()),
                        ignored_repositories: vec!["target/original".into()],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
            };
            let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(json!({
                "agents": [],
                "focused_pane_id": "w1:p1",
                "focused_workspace_id": "w1",
                "workspaces": [{"workspace_id": "w1", "active_tab_id": "w1:t1"}],
                "tabs": [{"workspace_id": "w1", "tab_id": "w1:t1"}],
                "panes": [{"pane_id": "w1:p1", "cwd": path}],
                "layouts": [{
                    "workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                    "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                    "focused_pane_id": "w1:p1",
                    "panes": [{"pane_id": "w1:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                    "splits": []
                }]
            }))
            .expect("focused project input");
            let spaces = Runtime::session_spaces(
                &payload,
                &workspace::paths_here(Runtime::session_paths(&payload)),
            );
            let precomputed = PrecomputedCatalog {
                registrations: Vec::new(),
                worktrees: runtime.worktree_catalog(),
                workspaces: workspace::build_catalog(
                    &crate::node::test_node(),
                    &[],
                    &spaces,
                    &catalog,
                    &workspace::catalog_paths_here(&[], &spaces, &catalog),
                ),
                paths: std::sync::Arc::new(workspace::paths_here(
                    spaces
                        .iter()
                        .flat_map(|space| space.cwds.clone())
                        .chain(Runtime::session_paths(&payload)),
                )),
            };
            runtime.ingest_session_with_catalog(Ok(payload), Some(precomputed));
            // A populated baseline preserves the existing initial stale-answer policy.
            runtime.ingest_worktrees(catalog, 0);
            let runtime = Arc::new(Mutex::new(runtime));
            let (core, notifier) = Core::for_runtime_fixture(Arc::clone(&runtime));
            let socket_path = root.join("unused.sock");
            let context = SessionSyncContext::local(
                &LiveContext {
                    socket_path: socket_path.clone(),
                    runtime: Arc::downgrade(&runtime),
                    notifier,
                    engine: crate::engine::fake::FakeEngine::absent().engine(),
                    node: Arc::new(hide_node::Local::of_process()),
                },
                std::sync::Arc::new(hide_node::Local::of_process()),
            );
            let baseline: Value =
                serde_json::from_slice(&core.snapshot_delta(0)).expect("initial observer snapshot");
            assert_eq!(
                baseline["rest"]["git_worktrees"]["worktrees"][0]["branch"],
                "feature/preflight"
            );
            let (sender, observations) = sync_channel(8);
            let weak_runtime = Arc::downgrade(&runtime);
            core.on_change(move || {
                let unlocked = weak_runtime
                    .upgrade()
                    .is_some_and(|runtime| runtime.try_lock().is_ok());
                // notify catches callback panics, so the caller asserts this witness.
                let _ = sender.try_send(unlocked);
            });
            Self {
                core,
                runtime,
                context,
                observations,
                _directory: directory,
            }
        }

        fn catalog(&self) -> WorktreeCatalogSnapshot {
            self.runtime.lock().expect("runtime").worktree_catalog()
        }

        fn answer(&self, catalog: WorktreeCatalogSnapshot) -> WorktreeAnswer {
            let request = self.runtime.lock().expect("runtime").worktrees_request();
            WorktreeAnswer {
                removals: request.removals,
                request,
                catalog,
                observations_current: true,
            }
        }

        fn snapshot(&self) -> Value {
            serde_json::from_slice(&self.core.snapshot_delta(0)).expect("observer snapshot")
        }

        fn expect_wake_after_unlock(&self) {
            assert!(
                self.observations.try_recv().expect("observer was notified"),
                "the observer must run after releasing the runtime lock"
            );
            self.expect_silence();
        }

        fn expect_silence(&self) {
            assert_eq!(self.observations.try_recv(), Err(TryRecvError::Empty));
        }
    }

    #[test]
    fn changed_worktree_facts_wake_the_observer_and_coalesce_until_its_snapshot() {
        let fixture = ObserverFixture::new();
        let mut catalog = fixture.catalog();
        catalog.projects[0].worktrees[0].lock_reason = Some("Review $(literal)".into());
        let _ = publish_worktrees(&fixture.context, fixture.answer(catalog));
        fixture.expect_wake_after_unlock();

        let mut catalog = fixture.catalog();
        catalog.projects[0].worktrees[0].ignored_repositories =
            vec!["target/review".into(), "vendor/next".into()];
        let _ = publish_worktrees(&fixture.context, fixture.answer(catalog));
        fixture.expect_silence();
        let snapshot = fixture.snapshot();
        let row = &snapshot["rest"]["git_worktrees"]["worktrees"][0];
        assert_eq!(row["lock_reason"], "Review $(literal)");
        assert_eq!(
            row["ignored_repositories"],
            json!(["target/review", "vendor/next"])
        );

        let mut catalog = fixture.catalog();
        catalog.projects[0].worktrees[0].lock_reason = None;
        let _ = publish_worktrees(&fixture.context, fixture.answer(catalog));
        fixture.expect_wake_after_unlock();
        let snapshot = fixture.snapshot();
        assert!(snapshot["rest"]["git_worktrees"]["worktrees"][0]["lock_reason"].is_null());
    }

    #[test]
    fn worktree_loading_completion_wakes_the_observer_with_unchanged_facts() {
        let fixture = ObserverFixture::new();
        let catalog = fixture.catalog();
        fixture.runtime.lock().expect("runtime").refresh_worktrees();
        let before = fixture.snapshot();
        assert_eq!(before["rest"]["git_worktrees_loading"], true);

        let _ = publish_worktrees(&fixture.context, fixture.answer(catalog));
        fixture.expect_wake_after_unlock();
        let after = fixture.snapshot();
        assert_eq!(after["rest"]["git_worktrees_loading"], false);
        assert_eq!(
            after["rest"]["git_worktrees"],
            before["rest"]["git_worktrees"]
        );
    }

    #[test]
    fn unchanged_and_rejected_worktree_answers_leave_the_observer_silent() {
        let fixture = ObserverFixture::new();
        let catalog = fixture.catalog();
        let original_request = fixture.answer(catalog.clone()).request;
        let before = fixture.snapshot();
        let _ = publish_worktrees(&fixture.context, fixture.answer(catalog));
        fixture.expect_silence();
        let after = fixture.snapshot();
        assert_eq!(
            after["rest"]["git_worktrees"],
            before["rest"]["git_worktrees"]
        );
        assert_eq!(after["rest"]["git_worktrees_loading"], false);

        fixture.runtime.lock().expect("runtime").refresh_worktrees();
        assert_eq!(fixture.snapshot()["rest"]["git_worktrees_loading"], true);
        let mut rejected = fixture.catalog();
        rejected.projects[0].worktrees[0].lock_reason = Some("Unaccepted facts".into());
        let mut stale_request = fixture.answer(rejected.clone());
        stale_request.request = original_request;
        let _ = publish_worktrees(&fixture.context, stale_request);
        fixture.expect_silence();
        let after = fixture.snapshot();
        assert_eq!(
            after["rest"]["git_worktrees"],
            before["rest"]["git_worktrees"]
        );
        assert_eq!(after["rest"]["git_worktrees_loading"], true);

        let mut stale_observation = fixture.answer(rejected);
        stale_observation.observations_current = false;
        let _ = publish_worktrees(&fixture.context, stale_observation);
        fixture.expect_silence();
        let after = fixture.snapshot();
        assert_eq!(
            after["rest"]["git_worktrees"],
            before["rest"]["git_worktrees"]
        );
        assert_eq!(after["rest"]["git_worktrees_loading"], true);
    }
}

#[cfg(test)]
mod pending_core_tests {
    use super::*;
    use crate::engine::fake::FakeEngine;

    /// PRD core-host-node-move amendment 3: a core waiting for its move's
    /// link asks GitHub nothing; the read is asked once the hold is released.
    #[test]
    fn a_pending_core_reads_nothing_from_github_until_its_link_commits() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, ..) = crate::runtime::delivery::tests::fixture(root.path());
        let fake = FakeEngine::start(|method, _| panic!("unexpected {method}"));
        let context = SessionSyncContext::local(
            &LiveContext {
                socket_path: fake.scope(),
                runtime: Arc::downgrade(&runtime),
                notifier: ChangeNotifier::noop(),
                engine: fake.engine(),
                node: Arc::new(hide_node::Local::of_process()),
            },
            Arc::new(hide_node::Local::of_process()),
        );
        runtime.lock().unwrap().hold_effects();
        assert!(matches!(read_github_request(&context), Some(None)));
        assert!(runtime.lock().unwrap().release_effects());
        assert!(matches!(read_github_request(&context), Some(Some(_))));
    }
}

#[cfg(test)]
mod window_reads_tests {
    use super::*;
    use crate::model::CoreOptions;
    use hide_node_link::cleanup::RepositoryDirs;
    use hide_node_link::protocol::Call;
    use hide_node_link::worktrees::GitWatchReport;
    use std::sync::mpsc::{Sender, channel};

    /// A node whose every project is its own repository and whose Git watch
    /// says it has started and is watching, then reports a heartbeat each
    /// time the test ticks until the watch is stopped, and then says it ended.
    struct WatchingNode {
        started: Mutex<Sender<()>>,
        tick: Mutex<std::sync::mpsc::Receiver<()>>,
        ended: Mutex<Sender<()>>,
    }

    impl crate::node_access::NodeLink for WatchingNode {
        fn call(
            &self,
            call: Call,
            timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            self.call_with_progress(call, timeout, &mut |_| true)
        }

        fn call_with_progress(
            &self,
            call: Call,
            _timeout: Duration,
            progress: &mut dyn FnMut(Value) -> bool,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            match call {
                Call::Repository { path } => Ok(json!(RepositoryDirs {
                    root: path.clone(),
                    git_dir: format!("{path}/.git"),
                    common_dir: format!("{path}/.git"),
                })
                .into()),
                Call::GitWatch { .. } => {
                    let _ = self.started.lock().unwrap().send(());
                    let mut report = serde_json::to_value(GitWatchReport::Watching {
                        unwatched: Vec::new(),
                    })
                    .unwrap();
                    while progress(report) {
                        if self.tick.lock().unwrap().recv().is_err() {
                            break;
                        }
                        report = serde_json::to_value(GitWatchReport::Quiet).unwrap();
                    }
                    let _ = self.ended.lock().unwrap().send(());
                    Ok(Value::Null.into())
                }
                _ => Err(crate::node_access::LinkError::Unknown(
                    "not part of this fixture".to_owned(),
                )),
            }
        }
    }

    /// The last renderer leaving, dispatched as the daemon sends it, ends the
    /// node's Git watch on the coordinator's next wake; the first renderer
    /// back starts it again on the next read.
    #[test]
    fn the_git_watch_ends_while_no_renderer_is_attached_and_starts_when_one_is() {
        let options: CoreOptions = serde_json::from_value(json!({
            "schema_version": crate::model::SCHEMA_VERSION,
            "node_id": "test-node",
            "app_state_path": ""
        }))
        .expect("workerless core options");
        let runtime = Arc::new(Mutex::new(Runtime::new(
            options,
            crate::environment::EnvironmentReport {
                statuses: Vec::new(),
                home_path: None,
                codex_home: None,
            },
            Arc::new(hide_node::Local::of_process()),
            crate::node::test_devices(),
        )));
        let socket_path = std::env::temp_dir().join("hide-window-reads-unused.sock");
        let context = SessionSyncContext::local(
            &LiveContext {
                socket_path: socket_path.clone(),
                runtime: Arc::downgrade(&runtime),
                notifier: ChangeNotifier::noop(),
                engine: crate::engine::fake::FakeEngine::absent().engine(),
                node: Arc::new(hide_node::Local::of_process()),
            },
            Arc::new(hide_node::Local::of_process()),
        );
        let (ended, watch_ended) = channel();
        let (heartbeat, tick) = channel();
        let (started, watch_started) = channel();
        let node = Arc::new(WatchingNode {
            started: Mutex::new(started),
            tick: Mutex::new(tick),
            ended: Mutex::new(ended),
        });
        let mut reader = crate::worktrees::WorktreeReader::new(node.clone());
        // Dropped before the reader, so a watch still running at the end
        // returns and the reader's end does not wait for it.
        let heartbeat = heartbeat;
        let request = crate::worktrees::WorktreeRequest {
            projects: vec![crate::worktrees::WorktreeProjectRequest {
                root_path: std::path::PathBuf::from("/a"),
                bases: Default::default(),
                base_override: None,
                remote_default: None,
                generation: 0,
            }],
            generation: 0,
            removals: 0,
        };
        let attached = |attached: bool| {
            let event = serde_json::to_vec(&json!({
                "schema_version": 2, "kind": "ui_attached", "payload": {"attached": attached}
            }))
            .unwrap();
            runtime.lock().unwrap().dispatch_json(&event);
        };
        assert!(window_reads(&context, Some(&mut reader)));
        let _ = reader.read_if_due(request.clone());
        // The watch call runs on a thread of its own.
        watch_started
            .recv_timeout(Duration::from_secs(5))
            .expect("the first read starts the watch");

        attached(false);
        assert!(!window_reads(&context, Some(&mut reader)));
        heartbeat.send(()).unwrap();
        watch_ended
            .recv_timeout(Duration::from_secs(5))
            .expect("the node's watch ended once no renderer was attached");

        attached(true);
        assert!(window_reads(&context, Some(&mut reader)));
        let _ = reader.read_if_due(request);
        watch_started
            .recv_timeout(Duration::from_secs(5))
            .expect("the watch starts again on the first read with a renderer back");
    }
}

#[cfg(test)]
mod catalog_path_tests {
    use super::*;
    use crate::node::test_node;

    #[test]
    fn a_pending_answer_builds_from_the_paths_that_place_every_registration() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("fixture");
        let registered = repo.join("registered");
        std::fs::create_dir_all(&registered).unwrap();
        let repo = repo.to_string_lossy().into_owned();
        let registered = registered.to_string_lossy().into_owned();
        let node = test_node();
        let registrations =
            vec![workspace::registration(&registered, "registered", node.as_str()).unwrap()];
        let before = Arc::new(workspace::paths_here([repo.clone()]));
        let created = Arc::new(workspace::paths_here([repo.clone(), registered.clone()]));

        // The creation installed answers for its project; the coordinator's last ones predate it.
        let chosen = paths_to_build_from(
            Some(Arc::clone(&before)),
            Arc::clone(&created),
            &node,
            &registrations,
        );
        assert!(Arc::ptr_eq(&chosen, &created));
        // Answers that already place every registration stay the coordinator's own.
        let chosen = paths_to_build_from(
            Some(Arc::clone(&created)),
            Arc::clone(&before),
            &node,
            &registrations,
        );
        assert!(Arc::ptr_eq(&chosen, &created));
        // When neither places it, nothing newer exists: the last answers stand.
        let chosen = paths_to_build_from(
            Some(Arc::clone(&before)),
            Arc::new(workspace::paths_here([repo])),
            &node,
            &registrations,
        );
        assert!(Arc::ptr_eq(&chosen, &before));
    }
}
