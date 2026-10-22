//! Coordinates Herdr snapshot bootstrap, topology subscriptions, and capability readers.

use super::*;
use crate::labels::worker::{LabelWorker, ObservedAgent};
use crate::live;

pub(crate) fn spawn(
    context: SessionSyncContext,
    usage_paths: Option<crate::usage::UsagePaths>,
) -> Result<SessionSyncHandle, String> {
    let (sender, receiver) = channel();
    let worker_sender = sender.clone();
    let republish_pending = Arc::new(AtomicBool::new(false));
    let worker_republish = Arc::clone(&republish_pending);
    let worker = thread::Builder::new()
        .name("herdr-core-session-sync".to_owned())
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
        republish_pending,
        worker: Some(worker),
    })
}

fn run_coordinator(
    context: SessionSyncContext,
    usage_paths: Option<crate::usage::UsagePaths>,
    receiver: Receiver<CoordinatorMessage>,
    sender: Sender<CoordinatorMessage>,
    republish_pending: Arc<AtomicBool>,
) {
    let home_path = usage_paths.as_ref().and_then(|paths| paths.home.clone());
    let mut process_reader = super::process_info::ProcessReader::new(&context);
    let mut lineage_writer = match context.ledger_machine().and_then(|machine| {
        crate::coordination::lineage::Writer::new(
            machine,
            Arc::clone(&context.api_connector),
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
    let mut replica: Option<SessionReplica> = None;
    let mut active_tab_reads: BTreeMap<String, u32> = BTreeMap::new();
    // Kept across reconnects: it is keyed by terminal, and a reconnect
    // neither refills a start's reads nor reports a cold one again.
    let mut launch_settle = super::launch_settle::LaunchSettle::default();
    let mut subscription: Option<ActiveSubscription> = None;
    let mut subscription_generation = 0_u64;
    // Until when a line off the stream is reconciled against the bootstrap
    // snapshot rather than applied strictly. Every connect is a bootstrap,
    // because Herdr's stream has no position to resume from.
    let mut reconcile_until = Instant::now();
    let mut reconnect_at = Instant::now();
    let mut reconnect_delay = RECONNECT_INITIAL_DELAY;
    let mut defer_background_reads = true;
    let mut next_agent_refresh = Instant::now() + AGENT_REFRESH_INTERVAL;
    // How many events the replica had applied when the agent list in
    // flight was asked for. Events that land while it is out may have moved
    // agents its answer does not show, so the list is asked again as soon
    // as `AGENT_REASK_SPACING` after the one answered allows.
    let mut agents_asked_at_events: Option<u64> = None;
    // A refresh asked for while a list was out, owed when it answers.
    let mut agents_refresh_owed = false;
    let mut next_operation_tick = Instant::now() + ASYNC_OPERATION_TICK_INTERVAL;
    let mut catalog_cache = CatalogState {
        wake: Some(RepublishWaker {
            sender: sender.clone(),
            pending: Arc::clone(&republish_pending),
        }),
        ..Default::default()
    };
    // Herdr events received since the last publish, for the stage records of
    // Hide-started operations (PRD instant-pane-topology D-14).
    let mut arrivals: Vec<HerdrArrival> = Vec::new();
    // The runtime's settled worktree removals the catalog was last rebuilt for.
    let mut published_removals = 0;
    let mut purpose_mirror = if context.is_local()
        && let Some(node) = context.node()
    {
        match live::PurposeMirror::new(Arc::clone(&context.api_connector), Arc::clone(node)) {
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
    // Herdr loses a pane's hook tokens when its server is handed off or
    // restarted; this puts back the ones this machine's hook files still hold
    // (issue 799). A device's files are the device's, so only the local
    // coordinator has one.
    let mut hook_token_restorer = hook_home.clone().and_then(|home| {
        match crate::coordination::hook_tokens::Restorer::new(
            home,
            Arc::clone(&context.api_connector),
        ) {
            Ok(restorer) => Some(restorer),
            Err(message) => {
                crate::diagnostic!(json!({
                    "component": "hook_tokens",
                    "kind": "start_failed",
                    "message": message,
                }));
                None
            }
        }
    });
    // The rows read the operator's logins, which are the core's own node's.
    let mut usage_reader = usage_paths
        .zip(context.node().map(Arc::clone))
        .map(|(paths, node)| crate::usage::UsageDriver::new(paths, node));
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
            context
                .node()
                .map(|node| crate::ai::AiReader::new(Arc::clone(node)))
        })
        .flatten();
    if let Some(home) = hook_home.as_deref() {
        publish_ai_settings(&context, home);
    }
    let mut labels = start_label_worker(&context, &sender);
    // The socket reads beside the event stream run on their own worker, so
    // this thread never waits on a fresh connection (`sync_reads.rs`).
    let mut sync_reader = match SyncReader::start(&context, sender.clone()) {
        Ok(reader) => reader,
        Err(message) => {
            publish_failure(&context, SessionFetchError::Unreachable(message));
            return;
        }
    };

    loop {
        if context.runtime.upgrade().is_none() {
            stop_subscription(&mut subscription);
            return;
        }

        if subscription.is_none() && Instant::now() >= reconnect_at {
            let has_projection = replica.is_some();
            let snapshot_started_at = Instant::now();
            if !begin_delivery_pane_read(&context) {
                stop_subscription(&mut subscription);
                return;
            }
            match connect(
                &context,
                &sender,
                &mut subscription_generation,
                has_projection,
            ) {
                Ok(Connected {
                    replica: next_replica,
                    subscription: next_subscription,
                    snapshot_at,
                }) => {
                    if !begin_read_connection(&context, snapshot_started_at) {
                        stop_subscription(&mut subscription);
                        return;
                    }
                    replica = Some(next_replica);
                    subscription = Some(next_subscription);
                    catalog_cache.connection = subscription_generation;
                    catalog_cache.accepted = None;
                    catalog_cache.accepted_stale = false;
                    reconcile_until = snapshot_at + RECONCILE_GRACE;
                    active_tab_reads.clear();
                    if let Err(message) = sync_reader.reset() {
                        crate::diagnostic!(json!({
                            "component": "session_sync",
                            "kind": "sync_read.start_failed",
                            "target": context.log_target(),
                            "message": message,
                        }));
                        stop_subscription(&mut subscription);
                        if !publish_failure(&context, SessionFetchError::Stale(message)) {
                            return;
                        }
                        reconnect_at = Instant::now() + reconnect_delay;
                        reconnect_delay = next_reconnect_delay(reconnect_delay);
                        continue;
                    }
                    defer_background_reads = true;
                    reconnect_delay = RECONNECT_INITIAL_DELAY;
                    next_agent_refresh = Instant::now() + AGENT_REFRESH_INTERVAL;
                    agents_asked_at_events = None;
                    agents_refresh_owed = false;
                    next_operation_tick = Instant::now() + ASYNC_OPERATION_TICK_INTERVAL;
                    if let Some(writer) = lineage_writer.as_mut()
                        && let Some((state, marks)) = lineage_inputs(&context)
                    {
                        writer.reconnected();
                        let current = &replica.as_ref().unwrap().state;
                        writer.observe(
                            &state,
                            &current.agents,
                            &|pane| current.panes.iter().any(|known| known.pane_id == pane),
                            &marks,
                            true,
                            snapshot_started_at,
                        );
                    }
                    if let (Some(restorer), Some(current)) =
                        (hook_token_restorer.as_mut(), replica.as_ref())
                    {
                        restorer.observe(&current.state.agents, true);
                    }
                    if replica
                        .as_ref()
                        .is_some_and(SessionReplica::ready_to_publish)
                        && !publish_replica(
                            &context,
                            replica.as_mut().unwrap(),
                            &mut catalog_cache,
                            &mut purpose_mirror,
                            &mut labels,
                            &mut arrivals,
                        )
                    {
                        stop_subscription(&mut subscription);
                        return;
                    }
                    if let (Some(home), Some(current)) = (hook_home.as_deref(), replica.as_ref()) {
                        sweep_subagent_counters(home, current);
                    }
                }
                Err(error) => {
                    log_sync_failure(&context, "connect.failed", replica.as_ref(), &error);
                    if !publish_failure(&context, error) {
                        return;
                    }
                    reconnect_at = Instant::now() + reconnect_delay;
                    reconnect_delay = next_reconnect_delay(reconnect_delay);
                    defer_background_reads = true;
                }
            }
        }

        if subscription.is_some()
            && let Some(runtime) = context.runtime.upgrade()
        {
            let requested = match runtime.lock() {
                Ok(mut guard) => guard.take_status_refresh_request(),
                Err(_) => false,
            };
            drop(runtime);
            if requested {
                next_agent_refresh = Instant::now();
            }
        }

        if let Some(active) = subscription.as_ref()
            && Instant::now() >= next_agent_refresh
        {
            next_agent_refresh = Instant::now() + AGENT_REFRESH_INTERVAL;
            if sync_reader.ask(active.generation, SyncRead::Agents) {
                agents_asked_at_events = replica.as_ref().map(|current| current.applied_events);
            } else {
                // The list out was asked before this need; its answer asks
                // again.
                agents_refresh_owed = true;
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
                take_label_switch(&context, worker) | worker.tick(Instant::now())
            }) && subscription.is_some()
                && let Some(current) = replica.as_mut()
                && !publish_replica(
                    &context,
                    current,
                    &mut catalog_cache,
                    &mut purpose_mirror,
                    &mut labels,
                    &mut arrivals,
                )
            {
                stop_subscription(&mut subscription);
                return;
            }
            if let Some(active) = subscription.as_ref()
                && let Some(current) = replica.as_mut()
                && let Err(error) = ask_active_tabs(
                    &mut sync_reader,
                    active.generation,
                    current,
                    &mut active_tab_reads,
                )
            {
                log_sync_failure(&context, "active_tab_read.failed", Some(current), &error);
                stop_subscription(&mut subscription);
                if !publish_failure(&context, error) {
                    return;
                }
                reconnect_at = Instant::now() + reconnect_delay;
                reconnect_delay = next_reconnect_delay(reconnect_delay);
            }
            // Herdr sends no event when a pane's cwd settles, so the tick
            // asks the worker to read again every pane whose last answer did
            // not repeat the one before it; a pane the worker could not take
            // now is due again on the next tick.
            if let Some(active) = subscription.as_ref()
                && let Some(current) = replica.as_mut()
            {
                let rereads = current.take_cwd_rereads();
                if !rereads.is_empty() {
                    sync_reader.ask(active.generation, SyncRead::PaneCwds(rereads));
                }
            }
        }

        // With no window attached only label work runs; these readers feed
        // nothing else (PRD labels-in-hided B29). The usage reader also runs
        // while background Memory analysis needs the weekly figure (PRD
        // project-memory-v2 D-10 b).
        let window_attached = window_reads(&context, worktree_reader.as_mut());
        let run_background_reads =
            subscription.is_some() && !defer_background_reads && window_attached;
        let read_usage = subscription.is_some()
            && !defer_background_reads
            && (run_background_reads || read_memory_reads_usage(&context));
        defer_background_reads = false;
        if read_usage && let Some(reader) = usage_reader.as_mut() {
            let Some(activity) = read_usage_activity(&context) else {
                stop_subscription(&mut subscription);
                return;
            };
            if let Some(usage) = reader.read_if_due(activity)
                && !publish_usage(&context, usage)
            {
                stop_subscription(&mut subscription);
                return;
            }
        }
        if run_background_reads {
            if let Some(current) = replica.as_mut()
                && process_reader.poll(&context, current, subscription_generation)
            {
                match current.refresh_published_state() {
                    Ok(true) => {
                        publish_replica(
                            &context,
                            current,
                            &mut catalog_cache,
                            &mut purpose_mirror,
                            &mut labels,
                            &mut arrivals,
                        );
                    }
                    Ok(false) => {}
                    Err(error) => {
                        log_sync_failure(&context, "process.publish_failed", Some(current), &error)
                    }
                }
            }

            if let Some(reader) = ports_reader.as_mut() {
                // `lsof` runs here, outside every lock; only the result is handed
                // in.
                if let Some(ports) = reader.read_if_due()
                    && !publish_ports(&context, ports)
                {
                    stop_subscription(&mut subscription);
                    return;
                }
            }

            if let Some(reader) = worktree_reader.as_mut() {
                let Some(request) = read_worktrees_request(&context) else {
                    stop_subscription(&mut subscription);
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
                            stop_subscription(&mut subscription);
                            return;
                        }
                        Some(changed) => rebuild |= changed,
                    }
                }
                if rebuild
                    && let Some(current) = replica.as_mut()
                    && !publish_replica(
                        &context,
                        current,
                        &mut catalog_cache,
                        &mut purpose_mirror,
                        &mut labels,
                        &mut arrivals,
                    )
                {
                    stop_subscription(&mut subscription);
                    return;
                }
            }

            if let Some(reader) = github_reader.as_mut() {
                let Some(request) = read_github_request(&context) else {
                    stop_subscription(&mut subscription);
                    return;
                };
                if let Some(request) = request
                    && let Some(github) = reader.read_if_due(request)
                    && !publish_github(&context, github)
                {
                    stop_subscription(&mut subscription);
                    return;
                }
            }

            if let Some(reader) = github_poll_reader.as_mut() {
                let Some(request) = read_github_poll_request(&context) else {
                    stop_subscription(&mut subscription);
                    return;
                };
                if let Some(poll) = reader.read_if_due(request)
                    && !ingest_github_poll(&context, poll)
                {
                    stop_subscription(&mut subscription);
                    return;
                }
            }

            if let Some(reader) = disk_reader.as_mut() {
                let Some(request) = read_disk_request(&context) else {
                    stop_subscription(&mut subscription);
                    return;
                };
                if let Some(disk) = reader.read_if_due(request)
                    && !publish_disk_usage(&context, disk)
                {
                    stop_subscription(&mut subscription);
                    return;
                }
            }

            if let Some(reader) = sweep_reader.as_mut() {
                let Some(request) = read_sweep_request(&context) else {
                    stop_subscription(&mut subscription);
                    return;
                };
                if let Some((disk, whole)) = reader.poll(request)
                    && !publish_swept_disk(&context, disk, whole)
                {
                    stop_subscription(&mut subscription);
                    return;
                }
            }

            if let Some(reader) = ai_reader.as_mut() {
                let Some((request, queued_settings, standing_moved)) = read_ai_request(&context)
                else {
                    stop_subscription(&mut subscription);
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
                        stop_subscription(&mut subscription);
                        return;
                    }
                }
                if let Some(background_ai) = reader.read_if_due(request)
                    && !publish_background_ai(&context, background_ai)
                {
                    stop_subscription(&mut subscription);
                    return;
                }
            }
        }

        let timeout = coordinator_wait(
            subscription.is_some(),
            reconnect_at,
            next_agent_refresh,
            next_operation_tick,
        );
        match receiver.recv_timeout(timeout) {
            Ok(CoordinatorMessage::RefreshTopology) => {
                // Discard the old stream generation. The next bootstrap is a
                // read only; it can safely supersede a move's final drawing
                // even when another client has since moved or closed it.
                stop_subscription(&mut subscription);
                reconnect_at = Instant::now();
                defer_background_reads = true;
            }
            Ok(CoordinatorMessage::Stop) => {
                stop_subscription(&mut subscription);
                return;
            }
            Ok(CoordinatorMessage::SubscriptionLine {
                generation,
                line,
                received_at,
            }) => {
                if subscription.as_ref().map(|active| active.generation) != Some(generation) {
                    continue;
                }
                defer_background_reads = true;
                let mode = if received_at <= reconcile_until {
                    ApplyMode::Reconcile
                } else {
                    ApplyMode::Strict
                };
                match parse_subscription_line(&line) {
                    Ok(SubscriptionLine::Event(event)) => {
                        if context.is_local() {
                            let (tab_id, pane_id, layout) = event.subject();
                            if tab_id.is_some() || pane_id.is_some() {
                                if arrivals.len() >= ARRIVAL_BUFFER_LIMIT {
                                    arrivals.remove(0);
                                }
                                arrivals.push(HerdrArrival {
                                    tab_id: tab_id.map(str::to_owned),
                                    pane_id: pane_id.map(str::to_owned),
                                    layout,
                                    received_at,
                                });
                            }
                        }
                        let current = replica
                            .as_mut()
                            .expect("active subscription always has a replica");
                        let applied = current
                            .apply(event, mode)
                            .map_err(|error| ("event.rejected", error));
                        match applied {
                            Ok(outcome) => {
                                // A pane announced with a cwd Herdr has not
                                // confirmed is read back on the worker; the
                                // publish waits for it (`publish_replica`).
                                let awaiting = current.panes_awaiting_cwd();
                                if !awaiting.is_empty() {
                                    sync_reader.ask(generation, SyncRead::PaneCwds(awaiting));
                                }
                                if outcome.refresh_agents {
                                    next_agent_refresh = Instant::now();
                                }
                                if outcome.refresh_worktrees && !request_worktree_refresh(&context)
                                {
                                    stop_subscription(&mut subscription);
                                    return;
                                }
                                if outcome.publish
                                    && !publish_replica(
                                        &context,
                                        current,
                                        &mut catalog_cache,
                                        &mut purpose_mirror,
                                        &mut labels,
                                        &mut arrivals,
                                    )
                                {
                                    stop_subscription(&mut subscription);
                                    return;
                                }
                                if let Err(error) = ask_active_tabs(
                                    &mut sync_reader,
                                    generation,
                                    current,
                                    &mut active_tab_reads,
                                ) {
                                    log_sync_failure(
                                        &context,
                                        "active_tab_read.failed",
                                        Some(current),
                                        &error,
                                    );
                                    stop_subscription(&mut subscription);
                                    if !publish_failure(&context, error) {
                                        return;
                                    }
                                    reconnect_at = Instant::now() + reconnect_delay;
                                    reconnect_delay = next_reconnect_delay(reconnect_delay);
                                }
                            }
                            Err((kind, error)) => {
                                log_sync_failure(&context, kind, Some(current), &error);
                                stop_subscription(&mut subscription);
                                if !publish_failure(&context, error) {
                                    return;
                                }
                                reconnect_at = Instant::now() + reconnect_delay;
                                reconnect_delay = next_reconnect_delay(reconnect_delay);
                            }
                        }
                    }
                    Ok(SubscriptionLine::EventsLost { message }) => {
                        // Herdr answers and has closed only this stream, so
                        // the projection stays on screen until the fresh
                        // subscription and snapshot of the reconnect replace
                        // it; a reconnect that fails is what reaches the
                        // screen. The reconnect delay still bounds a stream
                        // that keeps falling behind.
                        crate::diagnostic!(json!({
                            "component": "session_sync",
                            "kind": "subscription.events_lost",
                            "target": context.log_target(),
                            "applied_events": replica.as_ref().map(|current| current.applied_events),
                            "message": message,
                        }));
                        stop_subscription(&mut subscription);
                        reconnect_at = Instant::now() + reconnect_delay;
                        reconnect_delay = next_reconnect_delay(reconnect_delay);
                    }
                    Ok(SubscriptionLine::Error { code, message }) => {
                        crate::diagnostic!(json!({
                            "component": "session_sync",
                            "kind": "subscription.error",
                            "target": context.log_target(),
                            "code": code,
                            "applied_events": replica.as_ref().map(|current| current.applied_events),
                            "message": message,
                        }));
                        stop_subscription(&mut subscription);
                        let error = SessionFetchError::Stale(format!(
                            "Herdr event stream failed with {code}: {message}"
                        ));
                        if !publish_failure(&context, error) {
                            return;
                        }
                        reconnect_at = Instant::now() + reconnect_delay;
                        reconnect_delay = next_reconnect_delay(reconnect_delay);
                    }
                    Err(error) => {
                        log_sync_failure(
                            &context,
                            "subscription.malformed",
                            replica.as_ref(),
                            &error,
                        );
                        stop_subscription(&mut subscription);
                        if !publish_failure(&context, error) {
                            return;
                        }
                        reconnect_at = Instant::now() + reconnect_delay;
                        reconnect_delay = next_reconnect_delay(reconnect_delay);
                    }
                }
            }
            Ok(CoordinatorMessage::SubscriptionEnded {
                generation,
                message,
            }) => {
                if subscription.as_ref().map(|active| active.generation) != Some(generation) {
                    continue;
                }
                defer_background_reads = true;
                stop_subscription(&mut subscription);
                let applied = replica
                    .as_ref()
                    .map(|current| current.applied_events)
                    .unwrap_or(0);
                let error = SessionFetchError::Stale(format!(
                    "Herdr event stream disconnected after {applied} events: {message}"
                ));
                log_sync_failure(
                    &context,
                    "subscription.disconnected",
                    replica.as_ref(),
                    &error,
                );
                if !publish_failure(&context, error) {
                    return;
                }
                reconnect_at = Instant::now() + reconnect_delay;
                reconnect_delay = next_reconnect_delay(reconnect_delay);
            }
            Ok(CoordinatorMessage::SyncRead { generation, answer }) => {
                sync_reader.answered(generation, &answer);
                if subscription.as_ref().map(|active| active.generation) != Some(generation) {
                    continue;
                }
                match answer {
                    SyncReadAnswer::Agents { started_at, result } => match result {
                        Ok(agents) => {
                            let current = replica
                                .as_mut()
                                .expect("active subscription always has a replica");
                            let moved = agents_asked_at_events
                                .take()
                                .is_some_and(|asked| asked != current.applied_events);
                            if std::mem::take(&mut agents_refresh_owed) || moved {
                                // Spaced from the read just answered, so a
                                // steady event stream is not a standing poll.
                                next_agent_refresh = next_agent_refresh
                                    .min((started_at + AGENT_REASK_SPACING).max(Instant::now()));
                            }
                            let (native_changed, requested) =
                                agent_tick_changes(current, &agents, catalog_cache.as_ref());
                            launch_settle.observe(
                                context.log_target(),
                                &agents,
                                Instant::now(),
                                super::launch_settle::recorded_start,
                                |panes| sync_reader.ask(generation, SyncRead::Launches(panes)),
                            );
                            if let Some(writer) = lineage_writer.as_mut()
                                && let Some((state, marks)) = lineage_inputs(&context)
                            {
                                let panes = &current.state.panes;
                                writer.observe(
                                    &state,
                                    &agents,
                                    &|pane| panes.iter().any(|known| known.pane_id == pane),
                                    &marks,
                                    native_changed,
                                    started_at,
                                );
                            }
                            if let Some(restorer) = hook_token_restorer.as_mut() {
                                restorer.observe(&agents, false);
                            }
                            if requested {
                                current.replace_agents(agents);
                                match current.refresh_published_state() {
                                    Ok(changed)
                                        if (changed || requested)
                                            && !publish_replica(
                                                &context,
                                                current,
                                                &mut catalog_cache,
                                                &mut purpose_mirror,
                                                &mut labels,
                                                &mut arrivals,
                                            ) =>
                                    {
                                        stop_subscription(&mut subscription);
                                        return;
                                    }
                                    Ok(_) => {}
                                    Err(error) => {
                                        log_sync_failure(
                                            &context,
                                            "agent_refresh.invalid_projection",
                                            Some(current),
                                            &error,
                                        );
                                        stop_subscription(&mut subscription);
                                        if !publish_failure(&context, error) {
                                            return;
                                        }
                                        reconnect_at = Instant::now() + reconnect_delay;
                                        reconnect_delay = next_reconnect_delay(reconnect_delay);
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            // An independent query's transport failure says
                            // nothing about the live subscription. Keep its
                            // topology and terminal generation, but stop
                            // presenting the previous telemetry as confirmed.
                            // Protocol/projection failures still rebuild below.
                            if matches!(
                                error,
                                SessionFetchError::SocketMissing(_)
                                    | SessionFetchError::Unreachable(_)
                            ) {
                                agents_asked_at_events = None;
                                let current = replica
                                    .as_mut()
                                    .expect("active subscription always has a replica");
                                if current.invalidate_agents() {
                                    log_sync_failure(
                                        &context,
                                        "agent_refresh.failed",
                                        Some(current),
                                        &error,
                                    );
                                    if !publish_replica(
                                        &context,
                                        current,
                                        &mut catalog_cache,
                                        &mut purpose_mirror,
                                        &mut labels,
                                        &mut arrivals,
                                    ) {
                                        stop_subscription(&mut subscription);
                                        return;
                                    }
                                }
                                continue;
                            }
                            log_sync_failure(
                                &context,
                                "agent_refresh.failed",
                                replica.as_ref(),
                                &error,
                            );
                            stop_subscription(&mut subscription);
                            if !publish_failure(
                                &context,
                                stale_if_projected(replica.as_ref(), error),
                            ) {
                                return;
                            }
                            reconnect_at = Instant::now() + reconnect_delay;
                            reconnect_delay = next_reconnect_delay(reconnect_delay);
                        }
                    },
                    SyncReadAnswer::PaneCwds(result) => {
                        let current = replica
                            .as_mut()
                            .expect("active subscription always has a replica");
                        let confirmed =
                            result.and_then(|cwds| apply_pane_cwds(&context, current, cwds));
                        match confirmed {
                            Ok(changed) => {
                                let awaiting = current.panes_awaiting_cwd();
                                if !awaiting.is_empty() {
                                    // A pane announced while this read was out.
                                    sync_reader.ask(generation, SyncRead::PaneCwds(awaiting));
                                } else if !pane_cwds_answer_publishes(
                                    changed,
                                    &catalog_cache,
                                    current,
                                ) {
                                    // The stand-in already published what
                                    // this answer confirms (PRD
                                    // grid-ahead-of-herdr D-12).
                                } else if !publish_replica(
                                    &context,
                                    current,
                                    &mut catalog_cache,
                                    &mut purpose_mirror,
                                    &mut labels,
                                    &mut arrivals,
                                ) {
                                    stop_subscription(&mut subscription);
                                    return;
                                }
                            }
                            Err(error) => {
                                log_sync_failure(
                                    &context,
                                    "pane_cwd_read.failed",
                                    Some(current),
                                    &error,
                                );
                                stop_subscription(&mut subscription);
                                if !publish_failure(&context, error) {
                                    return;
                                }
                                reconnect_at = Instant::now() + reconnect_delay;
                                reconnect_delay = next_reconnect_delay(reconnect_delay);
                            }
                        }
                    }
                    SyncReadAnswer::Launches(reads) => {
                        launch_settle.answered(context.log_target(), &reads);
                    }
                    SyncReadAnswer::ActiveTabs(result) => {
                        let current = replica
                            .as_mut()
                            .expect("active subscription always has a replica");
                        let settled = result.and_then(|tabs| {
                            settle_active_tabs(&context, current, &mut active_tab_reads, tabs)
                        });
                        match settled {
                            Ok(true) => {
                                if !publish_replica(
                                    &context,
                                    current,
                                    &mut catalog_cache,
                                    &mut purpose_mirror,
                                    &mut labels,
                                    &mut arrivals,
                                ) {
                                    stop_subscription(&mut subscription);
                                    return;
                                }
                            }
                            Ok(false) => {}
                            Err(error) => {
                                log_sync_failure(
                                    &context,
                                    "active_tab_read.failed",
                                    Some(current),
                                    &error,
                                );
                                stop_subscription(&mut subscription);
                                if !publish_failure(&context, error) {
                                    return;
                                }
                                reconnect_at = Instant::now() + reconnect_delay;
                                reconnect_delay = next_reconnect_delay(reconnect_delay);
                            }
                        }
                    }
                }
            }
            Ok(CoordinatorMessage::Republish) => {
                // Cleared before the read, so a wake that lands during the
                // publish asks for the next one rather than being lost.
                republish_pending.store(false, Ordering::Release);
                // Hide draws ahead of Herdr only on this machine's Herdr, so
                // only its replica carries the overlay. The published state
                // already leaves out what still waits for Herdr, as on the
                // event path, so a tab waiting for its layout elsewhere does
                // not hold the drawing back.
                if context.is_local()
                    && subscription.is_some()
                    && let Some(current) = replica.as_mut()
                    && !publish_replica(
                        &context,
                        current,
                        &mut catalog_cache,
                        &mut purpose_mirror,
                        &mut labels,
                        &mut arrivals,
                    )
                {
                    stop_subscription(&mut subscription);
                    return;
                }
            }
            Ok(CoordinatorMessage::Labels) => {
                // A disconnected replica is stale; its labels wait for the
                // reconnect's first publish rather than clearing the error.
                if labels.as_mut().is_some_and(|worker| {
                    let changed = worker.drain(Instant::now(), unix_now_ms())
                        | exchange_pull_requests(&context, worker);
                    hand_memory_receipts(&context, worker);
                    changed
                }) && subscription.is_some()
                    && let Some(current) = replica.as_mut()
                    && !publish_replica(
                        &context,
                        current,
                        &mut catalog_cache,
                        &mut purpose_mirror,
                        &mut labels,
                        &mut arrivals,
                    )
                {
                    stop_subscription(&mut subscription);
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                stop_subscription(&mut subscription);
                return;
            }
        }
    }
}

/// Whether an agent refresh has to republish the projection.
///
/// An `agent.list` identical to the one already held projects to the same
/// sidebar, so recomputing it would rebuild the whole projection and the
/// workspace catalog for a wire that did not move. `ProjectedAgent` is exactly the
/// projection's input - deserializing already drops the fields the projection
/// never reads - so equality here is equality of the projection.
///
/// The catalog is the other reason a tick must publish. It is rebuilt inside
/// `publish_replica` on its own refresh window, and on an idle session this
/// tick is the only thing that calls it, so a skip that ignored the window
/// would freeze every branch and dirty mark in the navigator.
fn agent_tick_changes(
    replica: &SessionReplica,
    agents: &[ProjectedAgent],
    catalog_cache: Option<&CatalogCache>,
) -> (bool, bool) {
    let native_changed = replica.state.agents != agents;
    let publish = native_changed
        || !replica.agents_confirmed
        || catalog_cache.is_none_or(|cache| cache.built_at.elapsed() >= CATALOG_REFRESH_INTERVAL);
    (native_changed, publish)
}

#[cfg(test)]
pub(crate) fn agent_tick_needs_publish(
    replica: &SessionReplica,
    agents: &[ProjectedAgent],
    catalog_cache: Option<&CatalogCache>,
) -> bool {
    agent_tick_changes(replica, agents, catalog_cache).1
}

/// Drops the subagent counts of panes Herdr no longer lists.
///
/// A fresh `session.snapshot` is the one moment the pane set is known to be
/// complete, so it is where the sweep belongs: a pane missing from an event
/// stream may only be one Hide has not heard about yet. Nothing on screen
/// depends on it - a pane with no agent projects no children at all - so this
/// is housekeeping, and a failure is recorded rather than escalated (PRD B31,
/// D-53).
fn sweep_subagent_counters(home: &std::path::Path, replica: &SessionReplica) {
    let live = replica.state.panes.iter().map(|pane| pane.pane_id.as_str());
    match hide_agent_hooks::counters::retain(home, live) {
        Ok(0) => {}
        Ok(dropped) => crate::diagnostic!(json!({
            "component": "agent_hooks",
            "kind": "counters.swept",
            "dropped": dropped,
        })),
        Err(error) => crate::diagnostic!(json!({
            "component": "agent_hooks",
            "kind": "counters.sweep_failed",
            "message": error.to_string(),
        })),
    }
}

/// Whether the worker's cwd answer has to publish: it changed a cwd, the
/// publish before it was held for the pane it answers, or a pane the last
/// publish showed at a sibling's cwd now holds another one in the replica,
/// as when Herdr confirms the cwd the pane was announced with and that is
/// not the sibling's. An answer that confirms what the screen already shows
/// would otherwise repeat that whole publish (PRD grid-ahead-of-herdr D-12:
/// 21 of 21 answers at operator scale changed nothing).
fn pane_cwds_answer_publishes(
    changed: bool,
    catalog_cache: &CatalogState,
    replica: &SessionReplica,
) -> bool {
    changed
        || catalog_cache.held_for_cwd
        || catalog_cache
            .stand_ins
            .iter()
            .any(|(pane_id, shown)| replica.pane_cwd(pane_id) != Some(shown.as_str()))
}

/// Applies the `pane.get` answers the worker read for panes waiting for
/// their cwd: those an event announced with a cwd Herdr had not confirmed,
/// and those an operation tick reads again because the last answer did not
/// repeat the one before it. Returns whether the published projection
/// changed.
fn apply_pane_cwds(
    context: &SessionSyncContext,
    replica: &mut SessionReplica,
    cwds: Vec<(String, Option<String>)>,
) -> Result<bool, SessionFetchError> {
    let mut corrected = false;
    for (pane_id, cwd) in cwds {
        if replica.confirm_pane_cwd(&pane_id, cwd) {
            crate::diagnostic!(json!({
                "component": "session_sync",
                "kind": "pane_cwd.corrected",
                "target": context.log_target(),
                "pane_id": pane_id,
            }));
            corrected = true;
        }
    }
    if !corrected {
        return Ok(false);
    }
    replica.refresh_published_state()
}

/// Asks the worker which tab every workspace still waiting for a
/// replacement active tab now holds. A workspace still waiting after
/// `ACTIVE_TAB_READ_ATTEMPT_LIMIT` reads is a replica that cannot converge,
/// which the caller rebuilds from a snapshot.
fn ask_active_tabs(
    reader: &mut SyncReader,
    generation: u64,
    replica: &SessionReplica,
    attempts: &mut BTreeMap<String, u32>,
) -> Result<(), SessionFetchError> {
    let waiting = replica.workspaces_awaiting_active_tab();
    attempts.retain(|workspace_id, _| waiting.contains(workspace_id));
    if waiting.is_empty() {
        return Ok(());
    }
    if let Some((workspace_id, _)) = attempts
        .iter()
        .find(|(_, attempt)| **attempt >= ACTIVE_TAB_READ_ATTEMPT_LIMIT)
    {
        return Err(SessionFetchError::Stale(format!(
            "workspace {workspace_id} named no active tab the event stream knows in {ACTIVE_TAB_READ_ATTEMPT_LIMIT} reads"
        )));
    }
    let workspace_ids = waiting.to_vec();
    if reader.ask(generation, SyncRead::ActiveTabs(workspace_ids)) {
        for workspace_id in waiting {
            *attempts.entry(workspace_id).or_insert(0) += 1;
        }
    }
    Ok(())
}

/// Settles the replica with the active tabs the worker read. A read that
/// names a tab the event stream has not delivered keeps its workspace
/// waiting for the next ask. Returns whether the published projection
/// changed.
fn settle_active_tabs(
    context: &SessionSyncContext,
    replica: &mut SessionReplica,
    attempts: &mut BTreeMap<String, u32>,
    tabs: Vec<(String, String)>,
) -> Result<bool, SessionFetchError> {
    let mut settled = false;
    for (workspace_id, active_tab_id) in tabs {
        let settled_now = replica.settle_active_tab(&workspace_id, &active_tab_id);
        crate::diagnostic!(json!({
            "component": "session_sync",
            "kind": "active_tab.read",
            "target": context.log_target(),
            "workspace_id": workspace_id,
            "tab_id": active_tab_id,
            "attempt": attempts.get(&workspace_id).copied().unwrap_or(0),
            "settled": settled_now,
        }));
        if settled_now {
            attempts.remove(&workspace_id);
            settled = true;
        }
    }
    if !settled {
        return Ok(false);
    }
    replica.refresh_published_state()
}

/// Gives every pane in `awaiting` that a published layout places a cwd it
/// can be grouped by before Herdr confirms its own: a confirmed pane's in
/// the same tab. A pane in a tab Hide created (`clamped_tabs`) is read as
/// that tab's checkout by the runtime already. Returns the stand-ins it
/// applied, or `None` when some placed pane has neither, which must wait
/// for its read.
fn stand_in_unconfirmed_cwds(
    payload: &mut SessionSnapshotPayload,
    awaiting: &[String],
    clamped_tabs: &HashSet<String>,
) -> Option<Vec<(String, String)>> {
    let mut stand_ins = Vec::new();
    for pane_id in awaiting {
        let Some(layout) = payload
            .layouts
            .iter()
            .find(|layout| layout.panes.iter().any(|pane| &pane.pane_id == pane_id))
        else {
            // No published layout places it, so nothing groups by its cwd.
            continue;
        };
        if clamped_tabs.contains(&layout.tab_id) {
            continue;
        }
        let sibling = layout
            .panes
            .iter()
            .filter(|pane| !awaiting.contains(&pane.pane_id))
            .find_map(|pane| {
                payload
                    .panes
                    .iter()
                    .find(|row| row.pane_id == pane.pane_id)
                    .and_then(|row| row.cwd.clone())
            });
        // A placed pane with no confirmed sibling has to wait for its read.
        stand_ins.push((pane_id.clone(), sibling?));
    }
    for (pane_id, cwd) in &stand_ins {
        if let Some(pane) = payload
            .panes
            .iter_mut()
            .find(|pane| &pane.pane_id == pane_id)
        {
            pane.cwd = Some(cwd.clone());
        }
    }
    Some(stand_ins)
}

fn publish_replica(
    context: &SessionSyncContext,
    replica: &mut SessionReplica,
    catalog_cache: &mut CatalogState,
    purpose_mirror: &mut Option<live::PurposeMirror>,
    labels: &mut Option<LabelWorker>,
    arrivals: &mut Vec<HerdrArrival>,
) -> bool {
    let awaiting = replica.panes_awaiting_cwd();
    let mut payload = replica.project();
    let mut held_state = None;
    let overlay = labels.as_mut().map(|worker| {
        exchange_pull_requests(context, worker);
        hand_memory_receipts(context, worker);
        take_label_switch(context, worker);
        observe_labels(worker, replica);
        worker.overlay()
    });
    // Observe native state from the payload before the overlay is laid on
    // it (the remote path below), so no label adds UI timestamps to what
    // delivery sees; the overlay is only consulted for what each session
    // read says the agent waits for. This is bounded memory work; no
    // delivery I/O or notifier is started here.
    if let Some(runtime) = context.runtime.upgrade()
        && let Ok(mut guard) = runtime.lock()
    {
        if let SessionSyncTarget::Remote { target_id, .. } = &context.target
            && !guard.remote_coordinator_is_current(target_id, &context.api_connector)
        {
            return false;
        }
        // A pane announced with a cwd Herdr has not confirmed must not group
        // its tab under another folder before the worker's read lands. Its
        // layout still publishes now (PRD instant-pane-topology B20): it
        // reads as the folder of a confirmed pane beside it, or as the
        // checkout a tab Hide created was asked for, decided under the lock
        // that reads those tabs. Only a pane with neither holds the publish
        // until the read, which publishes then; delivery and arrivals observe
        // nothing for it (the label worker above has already seen it).
        if !awaiting.is_empty() {
            let clamped_tabs = if context.is_local() {
                guard
                    .created_tab_clamps()
                    .into_iter()
                    .map(|clamp| clamp.tab_id)
                    .collect()
            } else {
                HashSet::new()
            };
            match stand_in_unconfirmed_cwds(&mut payload, &awaiting, &clamped_tabs) {
                Some(stand_ins) => catalog_cache.stand_ins = stand_ins,
                None => {
                    catalog_cache.held_for_cwd = true;
                    if replica.agents_confirmed || catalog_cache.accepted_stale {
                        return true;
                    }
                    let Some(accepted) = catalog_cache.accepted.as_ref() else {
                        return true;
                    };
                    payload = accepted.project();
                    for agent in &mut payload.agents {
                        agent.telemetry_stale = true;
                    }
                    held_state = Some(accepted.clone());
                    catalog_cache.accepted_stale = true;
                }
            }
        } else {
            catalog_cache.stand_ins.clear();
        }
        if held_state.is_none() {
            catalog_cache.held_for_cwd = false;
        }
        if held_state.is_none() && !arrivals.is_empty() {
            guard.observe_herdr_arrivals(&std::mem::take(arrivals));
        }
        match &context.target {
            SessionSyncTarget::Local { socket_path } => {
                let node = guard.node().clone();
                guard.observe_delivery(
                    node.as_str(),
                    &payload,
                    socket_path.to_str(),
                    overlay.as_ref(),
                )
            }
            SessionSyncTarget::Remote { target_id, .. } => {
                guard.observe_delivery(target_id, &payload, None, overlay.as_ref());
                // Sleeping agents Herdr no longer lists are drawn from their
                // records before the session is scoped to the device.
                guard.settle_node_agent_sleep(target_id, &mut payload);
            }
        }
    }
    if held_state.is_none() {
        let mut accepted = replica.published_state.clone();
        // Cache the cwd actually accepted for each pane, including a
        // confirmed sibling's stand-in, before any label overlay is applied.
        let cwds: HashMap<_, _> = payload
            .panes
            .iter()
            .map(|pane| (pane.pane_id.as_str(), &pane.cwd))
            .collect();
        for pane in &mut accepted.panes {
            if let Some(cwd) = cwds.get(pane.pane_id.as_str()) {
                pane.cwd.clone_from(cwd);
            }
        }
        catalog_cache.accepted = Some(accepted);
        catalog_cache.accepted_stale = !replica.agents_confirmed;
    }
    if let SessionSyncTarget::Remote { target_id, .. } = &context.target {
        if let Some(overlay) = &overlay {
            overlay.apply(&mut payload);
        }
        let projection = match held_state.as_ref() {
            Some(state) => SessionReplica::project_remote_state(state, target_id, payload),
            None => replica.project_remote(target_id, payload),
        };
        let (fetched, excluded) = match projection {
            Ok((session, excluded)) => (Ok(session), excluded),
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
        #[cfg(test)]
        coordinator_fence_tests::before_remote_ingest();
        let Some(runtime) = context.runtime.upgrade() else {
            return false;
        };
        let changed = match runtime.lock() {
            Ok(mut guard) => {
                if !guard.remote_coordinator_is_current(target_id, &context.api_connector) {
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
    // never a session to replay; build from the current replica below.
    let mut wanted = workspace::PathIndex::wanted(&node, &registrations, &[], &worktrees);
    wanted.extend(Runtime::session_cwds(&payload));
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
    let previous = catalog_cache
        .as_ref()
        .map(|cache| Arc::clone(&cache.paths))
        .unwrap_or(runtime_paths);
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
    // A focus that differs from Hide's is read back from Herdr, but the
    // update itself is not held for that answer: it is ingested now with
    // the differing focus left alone, and the read runs on its own worker
    // outside the lock.
    let (changed, readback) = match runtime.lock() {
        Ok(mut guard) => {
            if let Some(overlay) = overlay {
                guard.set_label_overlay(overlay);
            }
            guard.ingest_session_awaiting_focus_readback(Ok(payload), Some(precomputed))
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

/// The worker that reads Herdr's focus for a stream move, outside the runtime
/// lock. It owns the runtime's one readback slot from its start to its last
/// answer, and the guard frees the slot on any other way out.
fn spawn_focus_readback(
    context: &SessionSyncContext,
    first: crate::runtime::PendingPaneFocusControl,
) -> Result<(), String> {
    let connector = Arc::clone(&context.api_connector);
    let runtime = context.runtime.clone();
    let notifier = context.notifier.clone();
    thread::Builder::new()
        .name("herdr-core-focus-readback".to_owned())
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
                let answer = live::confirm_pane_focus(connector.as_ref(), &identity.target_id);
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

/// The label worker for this coordinator's Herdr server, built on the
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
                Arc::clone(&context.api_connector),
            ),
            None => Ok(None),
        },
        SessionSyncTarget::Remote { target_id, .. } => services
            .device_worker(
                target_id,
                context.runtime.clone(),
                wake,
                language,
                Arc::clone(&context.api_connector),
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

/// Hands the worker the agents and the complete pane topology the replica
/// publishes. Per publish this is one pass over the agents and the panes.
fn observe_labels(worker: &mut LabelWorker, replica: &SessionReplica) {
    let state = &replica.published_state;
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
            if !guard.remote_coordinator_is_current(target_id, &context.api_connector) {
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
                if !guard.remote_coordinator_is_current(target_id, &context.api_connector) {
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

/// Invalidates the local reader when Herdr reports a worktree topology event.
/// This only changes an in-memory generation while the mutex is held; the git
/// read itself starts later on `WorktreeReader`'s background worker.
fn request_worktree_refresh(context: &SessionSyncContext) -> bool {
    let Some(runtime) = context.runtime.upgrade() else {
        return false;
    };
    match runtime.lock() {
        Ok(mut guard) => guard.refresh_worktrees(),
        Err(_) => return false,
    }
    context.notifier.notify();
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
    subscribed: bool,
    reconnect_at: Instant,
    next_agent_refresh: Instant,
    next_operation_tick: Instant,
) -> Duration {
    let now = Instant::now();
    let deadline = if subscribed {
        next_agent_refresh.min(next_operation_tick)
    } else {
        reconnect_at.min(next_operation_tick)
    };
    deadline.saturating_duration_since(now)
}

fn next_reconnect_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(RECONNECT_MAX_DELAY)
}

fn stale_if_projected(
    replica: Option<&SessionReplica>,
    error: SessionFetchError,
) -> SessionFetchError {
    if replica.is_none() || matches!(error, SessionFetchError::Protocol { .. }) {
        return error;
    }
    SessionFetchError::Stale(error.message().to_owned())
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
                &workspace::paths_here(Runtime::session_cwds(&payload)),
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
                        .chain(Runtime::session_cwds(&payload)),
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
                    api_connector: Arc::new(hide_herdr_client::LocalSocketConnector::new(
                        &socket_path,
                    )),
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
mod focus_readback_order_tests {
    use super::*;
    use crate::fake_herdr::FakeHerdr;
    use crate::model::CoreOptions;
    use std::sync::mpsc::channel;

    /// One tab, panes `p1..` side by side. `focused` is Herdr's own focus.
    fn snapshot(panes: &[&str], focused: &str, zoomed: bool) -> Value {
        let width = 120 / panes.len() as u32;
        let pane_rows = |with_focus: bool| {
            panes
                .iter()
                .enumerate()
                .map(|(index, pane)| {
                    json!({
                        "pane_id": format!("w1:{pane}"),
                        "focused": with_focus && *pane == focused,
                        "rect": {"x": index as u32 * width, "y": 0, "width": width, "height": 60}
                    })
                })
                .collect::<Vec<_>>()
        };
        // Each split divides the area its predecessor left, first pane off.
        let splits = (1..panes.len())
            .map(|index| {
                let x = (index as u32 - 1) * width;
                let remaining = 120 - x;
                json!({
                    "id": format!("split_{index}"),
                    "direction": "right",
                    "ratio": width as f32 / remaining as f32,
                    "rect": {"x": x, "y": 0, "width": remaining, "height": 60}
                })
            })
            .collect::<Vec<_>>();
        json!({
            "version": "0.8.2",
            "protocol": hide_herdr_client::HERDR_PROTOCOL_REVISION,
            "focused_pane_id": format!("w1:{focused}"),
            "workspaces": [{
                "workspace_id": "w1", "label": "fixture", "agent_status": "idle",
                "focused": true, "number": 1, "pane_count": panes.len(), "tab_count": 1,
                "active_tab_id": "w1:t1"
            }],
            "tabs": [{
                "workspace_id": "w1", "tab_id": "w1:t1", "agent_status": "idle",
                "focused": false, "number": 1, "pane_count": panes.len(), "label": "1"
            }],
            "panes": panes.iter().map(|pane| json!({
                "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": format!("w1:{pane}"),
                "terminal_id": "fixture-terminal", "focused": false, "revision": 0,
                "agent_status": "idle", "cwd": "/tmp/fixture"
            })).collect::<Vec<_>>(),
            "layouts": [{
                "workspace_id": "w1", "tab_id": "w1:t1", "zoomed": zoomed,
                "area": {"x": 0, "y": 0, "width": 120, "height": 60},
                "focused_pane_id": format!("w1:{focused}"),
                "panes": pane_rows(true),
                "splits": splits
            }],
            "agents": []
        })
    }

    fn runtime_with(first: &SessionReplica) -> Arc<Mutex<Runtime>> {
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
        runtime.ingest_session(Ok(first.project()));
        Arc::new(Mutex::new(runtime))
    }

    fn context_for(runtime: &Arc<Mutex<Runtime>>, herdr: &FakeHerdr) -> SessionSyncContext {
        SessionSyncContext::local(
            &LiveContext {
                socket_path: herdr.socket_path().to_path_buf(),
                runtime: Arc::downgrade(runtime),
                notifier: ChangeNotifier::noop(),
                api_connector: Arc::new(herdr.connector()),
                node: Arc::new(hide_node::Local::of_process()),
            },
            std::sync::Arc::new(hide_node::Local::of_process()),
        )
    }

    struct HeldPaths {
        started: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        local: hide_node::Local,
    }

    impl crate::node_access::NodeLink for HeldPaths {
        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            if matches!(call, hide_node_link::protocol::Call::PathFacts { .. }) {
                self.started.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(30))
                    .unwrap();
            }
            self.local.call(call, timeout)
        }
    }

    #[test]
    fn confirmed_layout_and_known_checkout_publish_while_path_facts_are_held() {
        let first = SessionReplica::from_snapshot(&snapshot(&["p1", "p2"], "p1", false)).unwrap();
        let runtime = runtime_with(&first);
        let before = runtime
            .lock()
            .unwrap()
            .snapshot()
            .navigator
            .workspaces
            .clone();
        let (started, asked) = channel();
        let (release, held) = channel();
        let herdr = FakeHerdr::start("held-path-topology", |method, _| {
            panic!("unexpected {method}")
        });
        let mut context = context_for(&runtime, &herdr);
        context.node = Some(Arc::new(HeldPaths {
            started,
            release: Mutex::new(held),
            local: hide_node::Local::of_process(),
        }));
        let mut cache = CatalogState::default();
        assert!(publish_replica(
            &context,
            &mut first.clone(),
            &mut cache,
            &mut None,
            &mut None,
            &mut Vec::new()
        ));
        asked.recv_timeout(Duration::from_secs(30)).unwrap();
        let mut changed = SessionReplica::from_snapshot(&snapshot(&["p1"], "p1", true)).unwrap();
        changed.state.panes[0].cwd = Some("/unverified/new-checkout".to_owned());
        assert!(publish_replica(
            &context,
            &mut changed,
            &mut cache,
            &mut None,
            &mut None,
            &mut Vec::new()
        ));
        let (panes, focused, zoomed) = view(&runtime);
        assert_eq!(panes, ["w1:p1"]);
        assert_eq!(focused.as_deref(), Some("w1:p1"));
        assert_eq!(zoomed.as_deref(), Some("w1:p1"));
        let after = runtime
            .lock()
            .unwrap()
            .snapshot()
            .navigator
            .workspaces
            .clone();
        assert_eq!(after[0].path, before[0].path);
        assert_eq!(after[0].checkouts[0].path, before[0].checkouts[0].path);
        assert!(
            asked.try_recv().is_err(),
            "only one read runs while inputs move"
        );
        release.send(()).unwrap();
        // The owner cancels remaining chunks and joins this released read.
        drop(cache);
    }

    /// Herdr's answer to the focus readback: `pane.layout` waits for the test
    /// to say go, after telling it the question has arrived.
    #[test]
    fn path_reader_discards_old_connection_and_starts_only_latest_paths() {
        let first = SessionReplica::from_snapshot(&snapshot(&["p1"], "p1", false)).unwrap();
        let runtime = runtime_with(&first);
        let herdr = FakeHerdr::start("path-reader-fence", |method, _| {
            panic!("unexpected {method}")
        });
        let mut context = context_for(&runtime, &herdr);
        let (started, asked) = channel();
        let (release, held) = channel();
        context.node = Some(Arc::new(HeldPaths {
            started,
            release: Mutex::new(held),
            local: hide_node::Local::of_process(),
        }));
        let mut reader = path_facts::PathReader::new(&context, None);
        let request = |connection, path: &str| path_facts::Request {
            connection,
            paths: BTreeSet::from([path.to_owned()]),
        };
        assert!(reader.poll(request(1, "/fixture/old")).is_none());
        asked.recv_timeout(Duration::from_secs(30)).unwrap();
        for path in ["/fixture/middle", "/fixture/latest"] {
            assert!(reader.poll(request(2, path)).is_none());
        }
        assert!(asked.try_recv().is_err(), "only one active node call");
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            assert!(
                reader.poll(request(2, "/fixture/latest")).is_none(),
                "old facts are fenced"
            );
            if asked.try_recv().is_ok() {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        release.send(()).unwrap();
        let facts = loop {
            if let Some(facts) = reader.poll(request(2, "/fixture/latest")) {
                break facts.unwrap();
            }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        };
        assert!(facts.knows("/fixture/latest"));
        assert!(!facts.knows("/fixture/old"));
        assert!(!facts.knows("/fixture/middle"));
        assert!(
            asked.try_recv().is_err(),
            "no unchanged read follows acceptance"
        );
    }

    struct FailedPaths(&'static str);

    impl crate::node_access::NodeLink for FailedPaths {
        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            _timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            assert!(matches!(
                call,
                hide_node_link::protocol::Call::PathFacts { .. }
            ));
            Err(crate::node_access::LinkError::Unknown(self.0.to_owned()))
        }
    }

    #[test]
    fn unchanged_or_failed_paths_keep_accepted_catalog_without_a_notification() {
        for failure in [
            Some("node disconnected during read"),
            Some("node path facts deadline reached"),
            None,
        ] {
            let mut replica =
                SessionReplica::from_snapshot(&snapshot(&["p1"], "p1", false)).unwrap();
            let runtime = runtime_with(&replica);
            let before = runtime
                .lock()
                .unwrap()
                .snapshot()
                .navigator
                .workspaces
                .clone();
            let herdr = FakeHerdr::start("failed-path-catalog", |method, _| {
                panic!("unexpected {method}")
            });
            let mut context = context_for(&runtime, &herdr);
            if let Some(failure) = failure {
                context.node = Some(Arc::new(FailedPaths(failure)));
            }
            let (core, notifier) = crate::Core::for_runtime_fixture(Arc::clone(&runtime));
            let callbacks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let witness = Arc::clone(&callbacks);
            core.on_change(move || {
                witness.fetch_add(1, Ordering::SeqCst);
            });
            context.notifier = notifier;
            let (sender, ready) = channel();
            let mut cache = CatalogState {
                wake: Some(RepublishWaker {
                    sender,
                    pending: Arc::new(AtomicBool::new(false)),
                }),
                ..Default::default()
            };
            let publish = |replica: &mut SessionReplica, cache: &mut CatalogState| {
                assert!(publish_replica(
                    &context,
                    replica,
                    cache,
                    &mut None,
                    &mut None,
                    &mut Vec::new()
                ));
            };
            publish(&mut replica, &mut cache);
            ready.recv_timeout(Duration::from_secs(30)).unwrap();
            let revision = core.snapshot_delta_payload(0).unwrap().revision;
            callbacks.store(0, Ordering::SeqCst);
            // Consume the actual worker's refused/timed-out answer. The
            // accepted catalog and small selection remain its caller's state.
            publish(&mut replica, &mut cache);
            let mut guard = runtime.lock().unwrap();
            assert_eq!(guard.snapshot().navigator.workspaces, before);
            assert_eq!(guard.snapshot().terminal.pane_id.as_deref(), Some("w1:p1"));
            assert_eq!(guard.snapshot_delta_payload(revision).revision, revision);
            assert_eq!(callbacks.load(Ordering::SeqCst), 0);
        }
    }

    struct HeldChunk {
        started: std::sync::mpsc::Sender<usize>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl crate::node_access::NodeLink for HeldChunk {
        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            _timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            let hide_node_link::protocol::Call::PathFacts { paths } = call else {
                panic!("unexpected node call");
            };
            self.started.send(paths.len()).unwrap();
            if paths.len() == hide_node_link::catalog::PATH_FACTS_LIMIT {
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(30))
                    .unwrap();
            }
            Ok(crate::node_access::LinkAnswer::Parsed(json!(
                hide_node_link::catalog::PathFacts::default()
            )))
        }
    }

    #[test]
    fn ending_catalog_owner_cancels_remaining_chunks_and_joins_the_read() {
        let replica = SessionReplica::from_snapshot(&snapshot(&["p1"], "p1", false)).unwrap();
        let runtime = runtime_with(&replica);
        let herdr = FakeHerdr::start("chunked-path-owner", |method, _| {
            panic!("unexpected {method}")
        });
        let mut context = context_for(&runtime, &herdr);
        let (started, asked) = channel();
        let (release, held) = channel();
        context.node = Some(Arc::new(HeldChunk {
            started,
            release: Mutex::new(held),
        }));
        let mut reader = path_facts::PathReader::new(&context, None);
        let limit = hide_node_link::catalog::PATH_FACTS_LIMIT;
        assert!(
            reader
                .poll(path_facts::Request {
                    connection: 1,
                    paths: (0..limit + 1)
                        .map(|n| format!("/fixture/chunk-{n:05}"))
                        .collect(),
                })
                .is_none()
        );
        assert_eq!(asked.recv_timeout(Duration::from_secs(30)).unwrap(), limit);
        // The first bounded node call may finish; there is no second call
        // after the owner ends, and dropping the reader reclaims its worker.
        drop(runtime);
        release.send(()).unwrap();
        drop(reader);
        assert!(
            asked.try_recv().is_err(),
            "no remaining chunk escaped its owner"
        );
    }

    fn holding_herdr(
        focused: &'static str,
    ) -> (
        FakeHerdr,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (arrived, asked) = channel();
        let (release, held) = channel::<()>();
        let herdr = FakeHerdr::start("focus-readback-order", move |method, _| match method {
            "pane.layout" => {
                arrived.send(()).ok();
                // A dropped sender (the test failed) frees the answer too.
                held.recv().ok();
                json!({"type": "pane_layout", "layout": {
                    "workspace_id": "w1", "tab_id": "w1:t1", "zoomed": true,
                    "area": {"x": 0, "y": 0, "width": 120, "height": 60},
                    "focused_pane_id": format!("w1:{focused}"),
                    "panes": [
                        {"pane_id": "w1:p1", "focused": focused == "p1", "rect": {"x": 0, "y": 0, "width": 60, "height": 60}},
                        {"pane_id": "w1:p2", "focused": focused == "p2", "rect": {"x": 60, "y": 0, "width": 60, "height": 60}}
                    ],
                    "splits": [{"id": "split_1", "direction": "right", "ratio": 0.5, "rect": {"x": 0, "y": 0, "width": 120, "height": 60}}]
                }})
            }
            "workspace.get" => json!({
                "type": "workspace_info",
                "workspace": {"workspace_id": "w1", "number": 1, "label": "fixture", "focused": true, "pane_count": 2, "tab_count": 1, "active_tab_id": "w1:t1", "agent_status": "idle"}
            }),
            other => panic!("unexpected {other}"),
        });
        (herdr, asked, release)
    }

    /// One Herdr event as the subscription delivers it.
    fn event(kind: &str, data: Value) -> ReplicaEvent {
        let line = json!({"event": kind, "data": data}).to_string();
        match parse_subscription_line(&line).expect("event parses") {
            SubscriptionLine::Event(event) => event,
            SubscriptionLine::EventsLost { .. } | SubscriptionLine::Error { .. } => unreachable!(),
        }
    }

    /// Herdr announcing `pane` in `tab` at `cwd`, which it has not
    /// confirmed, then laying the tab out with `panes` side by side.
    fn announce(replica: &mut SessionReplica, tab: &str, pane: &str, cwd: &str, panes: &[&str]) {
        let width = 120 / panes.len() as u32;
        for next in [
            event(
                "pane_created",
                json!({"type": "pane_created", "pane": {
                    "workspace_id": "w1", "tab_id": tab, "pane_id": format!("w1:{pane}"),
                    "terminal_id": "fixture-terminal", "focused": false, "revision": 0,
                    "agent_status": "idle", "cwd": cwd
                }}),
            ),
            event(
                "layout_updated",
                json!({"type": "layout_updated", "layout": {
                    "workspace_id": "w1", "tab_id": tab, "zoomed": false,
                    "area": {"x": 0, "y": 0, "width": 120, "height": 60},
                    "focused_pane_id": format!("w1:{}", panes[0]),
                    "panes": panes.iter().enumerate().map(|(index, pane)| json!({
                        "pane_id": format!("w1:{pane}"), "focused": index == 0,
                        "rect": {"x": index as u32 * width, "y": 0, "width": width, "height": 60}
                    })).collect::<Vec<_>>(),
                    "splits": (1..panes.len()).map(|index| {
                        let x = (index as u32 - 1) * width;
                        json!({
                            "id": format!("split_{index}"), "direction": "right",
                            "ratio": width as f32 / (120 - x) as f32,
                            "rect": {"x": x, "y": 0, "width": 120 - x, "height": 60}
                        })
                    }).collect::<Vec<_>>()
                }}),
            ),
        ] {
            replica
                .apply(next, ApplyMode::Strict)
                .expect("the event applies");
        }
    }

    /// PRD grid-ahead-of-herdr D-12: the worker's cwd answer republishes
    /// only when the screen would change: it changed a cwd, the pane was
    /// shown at a sibling's cwd that is not the one it ends at, or the
    /// publish before it was held for the pane. A pane split beside a
    /// confirmed one publishes with its sibling's cwd standing in; the
    /// answer that confirms that same cwd publishes nothing, while one that
    /// confirms the cwd it was announced with, another folder, publishes so
    /// the pane leaves the sibling's. A pane alone in a new tab holds its
    /// publish, so the same answer publishes once and the screen gets the
    /// tab then.
    #[test]
    fn a_cwd_answer_publishes_only_when_the_screen_would_change() {
        let mut value = snapshot(&["p1"], "p1", false);
        value["agents"] = json!([{
            "pane_id":"w1:p1", "workspace_id":"w1", "tab_id":"w1:t1",
            "terminal_id":"fixture-terminal", "focused":false, "revision":1,
            "name":"fixture-agent", "agent":"codex", "agent_status":"working",
            "state_change_seq":4, "tokens":{}
        }]);
        let first = SessionReplica::from_snapshot(&value).expect("first snapshot");
        let runtime = runtime_with(&first);
        let herdr = FakeHerdr::start("cwd-answer-publish", |method, _| {
            panic!("unexpected {method}")
        });
        let context = context_for(&runtime, &herdr);
        let mut cache = CatalogState::default();
        let publish = |replica: &mut SessionReplica, cache: &mut CatalogState| {
            assert!(publish_replica(
                &context,
                replica,
                cache,
                &mut None,
                &mut None,
                &mut Vec::new()
            ));
        };

        // p2 split beside p1 and announced at p1's own folder: the publish
        // runs with p1's cwd standing in, and the answer confirming that
        // folder changes nothing on the screen.
        let mut replica = first.clone();
        announce(&mut replica, "w1:t1", "p2", "/tmp/fixture", &["p1", "p2"]);
        assert_eq!(replica.panes_awaiting_cwd(), ["w1:p2"]);
        publish(&mut replica, &mut cache);
        assert_eq!(
            view(&runtime).0,
            ["w1:p1", "w1:p2"],
            "published with the stand-in"
        );
        assert!(!cache.held_for_cwd);
        assert_eq!(
            cache.stand_ins,
            [("w1:p2".to_owned(), "/tmp/fixture".to_owned())]
        );
        assert!(!replica.confirm_pane_cwd("w1:p2", Some("/tmp/fixture".to_owned())));
        assert!(
            !pane_cwds_answer_publishes(false, &cache, &replica),
            "the answer confirming the stand-in publishes nothing"
        );
        assert!(
            pane_cwds_answer_publishes(true, &cache, &replica),
            "an answer that moved a cwd publishes"
        );

        // p4 split beside p1 but announced in another folder: the screen
        // shows p1's cwd standing in, so the answer confirming the announced
        // folder, which changes nothing in the replica, still publishes.
        announce(
            &mut replica,
            "w1:t1",
            "p4",
            "/unverified/birth",
            &["p1", "p2", "p4"],
        );
        publish(&mut replica, &mut cache);
        assert_eq!(
            cache.stand_ins,
            [("w1:p4".to_owned(), "/tmp/fixture".to_owned())]
        );
        assert!(!replica.confirm_pane_cwd("w1:p4", Some("/unverified/birth".to_owned())));
        assert!(
            pane_cwds_answer_publishes(false, &cache, &replica),
            "the pane shown at the sibling's folder has to leave it"
        );
        publish(&mut replica, &mut cache);
        assert!(
            cache.stand_ins.is_empty(),
            "nothing stands in once confirmed"
        );
        assert!(!pane_cwds_answer_publishes(false, &cache, &replica));

        // p3 alone in a new tab: nothing stands in, so the publish is held
        // until the worker's answer, which then publishes once.
        replica
            .apply(
                event(
                    "tab_created",
                    json!({"type": "tab_created", "tab": {
                        "workspace_id": "w1", "tab_id": "w1:t2", "agent_status": "idle",
                        "focused": false, "number": 2, "pane_count": 1, "label": "2"
                    }}),
                ),
                ApplyMode::Strict,
            )
            .expect("the tab applies");
        announce(&mut replica, "w1:t2", "p3", "/unverified/birth", &["p3"]);
        assert_eq!(replica.panes_awaiting_cwd(), ["w1:p3"]);
        publish(&mut replica, &mut cache);
        assert!(cache.held_for_cwd, "nothing stands in for p3");
        assert!(
            !view(&runtime).0.contains(&"w1:p3".to_owned()),
            "the held publish showed nothing"
        );
        assert!(
            pane_cwds_answer_publishes(false, &cache, &replica),
            "the answer after a held publish publishes"
        );
        let accepted_panes = view(&runtime).0;
        assert!(replica.invalidate_agents());
        publish(&mut replica, &mut cache);
        assert!(cache.held_for_cwd);
        assert_eq!(
            view(&runtime).0,
            accepted_panes,
            "freshness never leaks held topology"
        );
        let guard = runtime.lock().unwrap();
        let row = guard
            .snapshot()
            .navigator
            .agents
            .iter()
            .find(|row| row.pane_id == "w1:p1")
            .unwrap();
        assert_eq!(
            row.activity, "unknown",
            "cwd cannot hold freshness invalidation"
        );
        drop(guard);
        // Remote projection must use the same accepted topology, too.
        let accepted = cache.accepted.as_ref().unwrap();
        let mut payload = accepted.project();
        for agent in &mut payload.agents {
            agent.telemetry_stale = true;
        }
        let (remote, _) = SessionReplica::project_remote_state(accepted, "mini", payload).unwrap();
        assert!(
            !remote
                .pane_layouts
                .iter()
                .flat_map(|layout| &layout.frames)
                .any(|frame| frame.pane_id.ends_with(":w1:p3"))
        );
        assert_eq!(remote.agents[0].activity, "unknown");
        replica.replace_agents(first.state.agents.clone());
        replica.refresh_published_state().unwrap();
        publish(&mut replica, &mut cache);
        assert!(
            cache.held_for_cwd,
            "an equal list cannot confirm held topology"
        );
        assert_eq!(
            runtime.lock().unwrap().snapshot().navigator.agents[0].activity,
            "unknown"
        );
        // Herdr confirms the cwd the pane was announced with: nothing changed.
        assert!(!replica.confirm_pane_cwd("w1:p3", Some("/unverified/birth".to_owned())));
        assert!(replica.panes_awaiting_cwd().is_empty());
        publish(&mut replica, &mut cache);
        assert!(!cache.held_for_cwd);
        assert!(
            view(&runtime).0.contains(&"w1:p3".to_owned()),
            "the tab reached the screen"
        );
    }

    fn view(runtime: &Arc<Mutex<Runtime>>) -> (Vec<String>, Option<String>, Option<String>) {
        let guard = runtime
            .lock()
            .expect("runtime lock is free during a readback");
        let snapshot = guard.snapshot();
        (
            snapshot
                .pane_layouts
                .iter()
                .flat_map(|layout| layout.pane_ids().into_iter().map(str::to_owned))
                .collect(),
            snapshot.focused.pane_id.clone(),
            snapshot.zoomed.clone(),
        )
    }

    #[test]
    fn a_pane_close_is_ingested_while_herdr_still_holds_the_focus_readback() {
        let first = SessionReplica::from_snapshot(&snapshot(&["p1", "p2", "p3"], "p1", false))
            .expect("first snapshot");
        let runtime = runtime_with(&first);
        // The operator's pane p3 closed, and Herdr's focus moved to p2 by
        // itself, which Hide may follow only after reading it back.
        let mut closed = SessionReplica::from_snapshot(&snapshot(&["p1", "p2"], "p2", true))
            .expect("closed snapshot");
        let (herdr, asked, release) = holding_herdr("p2");
        let context = context_for(&runtime, &herdr);

        let publish = thread::spawn(move || {
            publish_replica(
                &context,
                &mut closed,
                &mut CatalogState::default(),
                &mut None,
                &mut None,
                &mut Vec::new(),
            )
        });
        asked
            .recv_timeout(Duration::from_secs(30))
            .expect("the readback is asked");

        // Herdr has not answered. The close is what the screen needs now.
        let (panes, focused, zoomed) = view(&runtime);
        assert_eq!(
            panes,
            ["w1:p1", "w1:p2"],
            "the close is ingested before Herdr answers the readback"
        );
        assert_eq!(focused.as_deref(), Some("w1:p1"), "focus is not guessed");
        assert_eq!(zoomed, None);

        release.send(()).expect("the readback is answered");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let (_, focused, zoomed) = view(&runtime);
            if focused.as_deref() == Some("w1:p2") {
                assert_eq!(zoomed.as_deref(), Some("w1:p2"));
                break;
            }
            assert!(Instant::now() < deadline, "the confirmed focus is followed");
            thread::yield_now();
        }
        assert!(publish.join().expect("publish does not panic"));
    }

    #[test]
    fn an_unconfirmed_readback_keeps_hides_focus_and_frees_the_slot_for_the_next_update() {
        let first = SessionReplica::from_snapshot(&snapshot(&["p1", "p2"], "p1", false))
            .expect("first snapshot");
        let runtime = runtime_with(&first);
        let mut moved =
            SessionReplica::from_snapshot(&snapshot(&["p1", "p2"], "p2", false)).expect("moved");
        // Herdr's own answer still names p1: the stream's p2 was stale.
        let (herdr, asked, release) = holding_herdr("p1");
        let context = context_for(&runtime, &herdr);

        assert!(publish_replica(
            &context,
            &mut moved,
            &mut CatalogState::default(),
            &mut None,
            &mut None,
            &mut Vec::new()
        ));
        asked
            .recv_timeout(Duration::from_secs(5))
            .expect("the readback is asked");
        release.send(()).expect("the readback is answered");
        // Both reads are answered once the worker has settled the slot.
        herdr.wait_for_requests(2, Duration::from_secs(30));
        let deadline = Instant::now() + Duration::from_secs(30);
        while runtime.lock().unwrap().pane_focus_readback_is_outstanding() {
            assert!(Instant::now() < deadline, "the slot is freed");
            thread::yield_now();
        }
        assert_eq!(view(&runtime).1.as_deref(), Some("w1:p1"));
    }
}

#[cfg(test)]
mod pane_cwd_confirmation_tests {
    use super::*;
    use crate::fake_herdr::FakeHerdr;

    const PROJECT: &str = "/tmp/fixture";

    fn snapshot() -> Value {
        json!({
            "version": "0.9.1",
            "protocol": hide_herdr_client::HERDR_PROTOCOL_REVISION,
            "focused_pane_id": "w1:p1",
            "workspaces": [{
                "workspace_id": "w1", "label": "fixture", "agent_status": "idle",
                "focused": true, "number": 1, "pane_count": 1, "tab_count": 1,
                "active_tab_id": "w1:t1"
            }],
            "tabs": [{
                "workspace_id": "w1", "tab_id": "w1:t1", "agent_status": "idle",
                "focused": false, "number": 1, "pane_count": 1, "label": "1"
            }],
            "panes": [pane("w1:t1", "w1:p1", PROJECT)],
            "layouts": [layout("w1:t1", "w1:p1")],
            "agents": []
        })
    }

    fn pane(tab_id: &str, pane_id: &str, cwd: &str) -> Value {
        json!({
            "workspace_id": "w1", "tab_id": tab_id, "pane_id": pane_id,
            "terminal_id": "fixture-terminal", "focused": false, "revision": 0,
            "agent_status": "idle", "cwd": cwd
        })
    }

    fn layout(tab_id: &str, pane_id: &str) -> Value {
        json!({
            "workspace_id": "w1", "tab_id": tab_id, "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 120, "height": 60},
            "focused_pane_id": pane_id,
            "panes": [{"pane_id": pane_id, "focused": true,
                "rect": {"x": 0, "y": 0, "width": 120, "height": 60}}],
            "splits": []
        })
    }

    fn event(kind: &str, data: Value) -> ReplicaEvent {
        let line = json!({"event": kind, "data": data}).to_string();
        match parse_subscription_line(&line).expect("event parses") {
            SubscriptionLine::Event(event) => event,
            SubscriptionLine::EventsLost { .. } | SubscriptionLine::Error { .. } => unreachable!(),
        }
    }

    fn context_for(herdr: &FakeHerdr) -> SessionSyncContext {
        SessionSyncContext::local(
            &LiveContext {
                socket_path: herdr.socket_path().to_path_buf(),
                runtime: Weak::new(),
                notifier: ChangeNotifier::noop(),
                api_connector: Arc::new(herdr.connector()),
                node: Arc::new(hide_node::Local::of_process()),
            },
            std::sync::Arc::new(hide_node::Local::of_process()),
        )
    }

    /// A Herdr whose `pane.get` answers `cwd` for the pane it is asked about.
    fn herdr_reading(cwd: &'static str) -> FakeHerdr {
        FakeHerdr::start(
            "pane-cwd-confirmation",
            move |method, params| match method {
                "pane.get" => {
                    let pane_id = params["pane_id"].as_str().expect("pane.get names a pane");
                    json!({"type": "pane_info", "pane": {
                        "pane_id": pane_id, "terminal_id": "fixture-terminal",
                        "workspace_id": "w1", "tab_id": "w1:t2", "focused": false,
                        "agent_status": "idle", "revision": 1, "cwd": cwd
                    }})
                }
                other => panic!("unexpected {other}"),
            },
        )
    }

    /// A Herdr whose `pane.get` answers each of `cwds` in turn, then the
    /// last one for good.
    fn herdr_answering(cwds: &'static [&'static str]) -> FakeHerdr {
        let mut answers = cwds.iter();
        let mut last = cwds[0];
        FakeHerdr::start("pane-cwd-moving", move |method, params| match method {
            "pane.get" => {
                last = answers.next().copied().unwrap_or(last);
                let pane_id = params["pane_id"].as_str().expect("pane.get names a pane");
                json!({"type": "pane_info", "pane": {
                    "pane_id": pane_id, "terminal_id": "fixture-terminal",
                    "workspace_id": "w1", "tab_id": "w1:t2", "focused": false,
                    "agent_status": "idle", "revision": 1, "cwd": last
                }})
            }
            other => panic!("unexpected {other}"),
        })
    }

    /// One operation tick of the coordinator, as far as pane cwds go: the
    /// panes it asks the worker to read again, read and applied.
    fn tick(context: &SessionSyncContext, replica: &mut SessionReplica) {
        let rereads = replica.take_cwd_rereads();
        read_and_apply(context, replica, rereads).expect("cwd read succeeds");
    }

    /// Reads back every pane waiting for its cwd and applies the answers, as
    /// the sync-read worker and the coordinator do together.
    fn confirm_pane_cwds(
        context: &SessionSyncContext,
        replica: &mut SessionReplica,
    ) -> Result<bool, SessionFetchError> {
        let awaiting = replica.panes_awaiting_cwd();
        read_and_apply(context, replica, awaiting)
    }

    /// What the sync-read worker reads for `panes`, applied as the
    /// coordinator applies its answer.
    fn read_and_apply(
        context: &SessionSyncContext,
        replica: &mut SessionReplica,
        panes: Vec<String>,
    ) -> Result<bool, SessionFetchError> {
        let cwds = panes
            .into_iter()
            .map(|pane_id| fetch_pane_cwd(context, &pane_id).map(|cwd| (pane_id, cwd)))
            .collect::<Result<Vec<_>, _>>()?;
        apply_pane_cwds(context, replica, cwds)
    }

    /// Applies one event the way the coordinator's event path does.
    fn apply(context: &SessionSyncContext, replica: &mut SessionReplica, next: ReplicaEvent) {
        replica
            .apply(next, ApplyMode::Strict)
            .expect("event applies");
        confirm_pane_cwds(context, replica).expect("cwd read succeeds");
    }

    fn create_tab(context: &SessionSyncContext, replica: &mut SessionReplica, born_in: &str) {
        let tab = json!({
            "workspace_id": "w1", "tab_id": "w1:t2", "agent_status": "idle",
            "focused": false, "number": 2, "pane_count": 1, "label": "2"
        });
        apply(
            context,
            replica,
            event("tab_created", json!({"type": "tab_created", "tab": tab})),
        );
        apply(
            context,
            replica,
            event(
                "pane_created",
                json!({"type": "pane_created", "pane": pane("w1:t2", "w1:p2", born_in)}),
            ),
        );
        apply(
            context,
            replica,
            event(
                "layout_updated",
                json!({"type": "layout_updated", "layout": layout("w1:t2", "w1:p2")}),
            ),
        );
    }

    fn projected_cwd(replica: &SessionReplica, pane_id: &str) -> Option<String> {
        replica
            .project()
            .panes
            .into_iter()
            .find(|pane| pane.pane_id == pane_id)
            .expect("the created pane is published")
            .cwd
    }

    #[test]
    fn a_pane_announced_with_another_cwd_publishes_the_cwd_herdr_reads_back() {
        let herdr = herdr_reading(PROJECT);
        let context = context_for(&herdr);
        let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");

        // Herdr 0.9.1 announced a tab created in the project with `/tmp`.
        create_tab(&context, &mut replica, "/tmp");

        assert_eq!(projected_cwd(&replica, "w1:p2").as_deref(), Some(PROJECT));
        assert_eq!(
            herdr.calls(),
            [("pane.get".to_owned(), json!({"pane_id": "w1:p2"}))],
            "later events read nothing; only a tick reads the pane again"
        );
    }

    /// Herdr 0.9.1 can answer a new pane's cwd with a folder above the one
    /// it was created in while the machine is loaded (seen on CI as the
    /// fixture's parent, issue 699), and it sends no event when the answer
    /// settles, so the pane waits until two answers in a row agree.
    #[test]
    fn a_pane_cwd_settles_when_two_answers_in_a_row_agree() {
        const ROOT: &str = "/tmp/hde-root";
        const FIXTURE: &str = "/tmp/hde-root/fixture";
        // (cwd the event carries, what each read answers, cwd published, reads)
        let rows: [(&str, &'static [&'static str], &str, usize); 4] = [
            ("/tmp", &[ROOT, FIXTURE, FIXTURE], FIXTURE, 3),
            (FIXTURE, &[FIXTURE], FIXTURE, 1),
            ("/tmp", &[FIXTURE, FIXTURE], FIXTURE, 2),
            // Never settles: the eighth answer is taken and nothing reads again.
            (
                "/tmp",
                &[ROOT, FIXTURE, ROOT, FIXTURE, ROOT, FIXTURE, ROOT, FIXTURE],
                FIXTURE,
                8,
            ),
        ];
        for (born_in, answers, settled, reads) in rows {
            let herdr = herdr_answering(answers);
            let context = context_for(&herdr);
            let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");

            create_tab(&context, &mut replica, born_in);
            tick(&context, &mut replica);
            assert_eq!(
                herdr.methods().len(),
                1,
                "a tick that follows the event's read at once does not read again"
            );
            for _ in 0..20 {
                tick(&context, &mut replica);
            }

            assert_eq!(
                projected_cwd(&replica, "w1:p2").as_deref(),
                Some(settled),
                "announced {born_in}, then read {answers:?}"
            );
            assert_eq!(
                herdr.methods().len(),
                reads,
                "announced {born_in}, then read {answers:?}"
            );
        }
    }

    #[test]
    fn a_pane_the_first_snapshot_caught_being_created_publishes_the_cwd_herdr_reads_back() {
        let herdr = herdr_reading(PROJECT);
        let context = context_for(&herdr);
        // The snapshot was read while Herdr created w1:p2, so it holds the
        // creation cwd, and the creation event follows in the reconcile window.
        let mut first = snapshot();
        first["workspaces"][0]["tab_count"] = json!(2);
        first["workspaces"][0]["pane_count"] = json!(2);
        first["tabs"].as_array_mut().unwrap().push(json!({
            "workspace_id": "w1", "tab_id": "w1:t2", "agent_status": "idle",
            "focused": false, "number": 2, "pane_count": 1, "label": "2"
        }));
        first["panes"]
            .as_array_mut()
            .unwrap()
            .push(pane("w1:t2", "w1:p2", "/tmp"));
        first["layouts"]
            .as_array_mut()
            .unwrap()
            .push(layout("w1:t2", "w1:p2"));
        let mut replica = SessionReplica::from_snapshot(&first).expect("snapshot");

        replica
            .apply(
                event(
                    "pane_created",
                    json!({"type": "pane_created", "pane": pane("w1:t2", "w1:p2", "/tmp")}),
                ),
                ApplyMode::Reconcile,
            )
            .expect("a creation the snapshot holds is reconciled");
        confirm_pane_cwds(&context, &mut replica).expect("cwd read succeeds");

        assert_eq!(projected_cwd(&replica, "w1:p2").as_deref(), Some(PROJECT));
        assert_eq!(
            herdr.calls(),
            [("pane.get".to_owned(), json!({"pane_id": "w1:p2"}))]
        );
    }

    #[test]
    fn a_pane_closed_before_its_read_keeps_the_cwd_its_event_carried() {
        let herdr = FakeHerdr::start_with_errors("pane-cwd-gone", |method, _| match method {
            "pane.get" => Err(("pane_not_found".to_owned(), "no such pane".to_owned())),
            other => panic!("unexpected {other}"),
        });
        let context = context_for(&herdr);
        let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");

        create_tab(&context, &mut replica, PROJECT);

        assert_eq!(projected_cwd(&replica, "w1:p2").as_deref(), Some(PROJECT));
        assert_eq!(herdr.methods(), ["pane.get"]);
    }

    #[test]
    fn panes_past_the_cap_keep_their_event_cwd_and_stay_in_the_replica() {
        let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
        // Nothing confirms them: the replica alone holds the waits.
        for index in 0..65 {
            let pane_id = format!("w1:x{index}");
            replica
                .apply(
                    event(
                        "pane_created",
                        json!({"type": "pane_created", "pane": pane("w1:t1", &pane_id, "/tmp")}),
                    ),
                    ApplyMode::Strict,
                )
                .expect("pane applies");
        }
        assert_eq!(replica.panes_awaiting_cwd().len(), 64);
        assert!(
            replica
                .state
                .panes
                .iter()
                .any(|pane| pane.pane_id == "w1:x64"),
            "the pane past the cap is kept, only its read is not"
        );
    }
}

#[cfg(test)]
mod cwd_stand_in_tests {
    use super::*;

    /// Tab `w1:t1` holds `p1` in the checkout and `p2`, just split from it,
    /// announced at Herdr's wrong birth cwd; tab `w1:t2` holds only `p3`.
    fn payload() -> SessionSnapshotPayload {
        let pane = |tab: &str, pane: &str, cwd: &str| {
            json!({
                "workspace_id": "w1", "tab_id": tab, "pane_id": pane,
                "terminal_id": "fixture-terminal", "focused": false, "revision": 0,
                "agent_status": "idle", "cwd": cwd
            })
        };
        let layout = |tab: &str, panes: &[&str]| {
            json!({
                "workspace_id": "w1", "tab_id": tab, "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 120, "height": 60},
                "focused_pane_id": panes[0],
                "panes": panes.iter().enumerate().map(|(index, pane)| json!({
                    "pane_id": pane, "focused": index == 0,
                    "rect": {"x": index as u32 * 60, "y": 0, "width": 60, "height": 60}
                })).collect::<Vec<_>>(),
                "splits": if panes.len() > 1 {
                    vec![json!({"id": "split_1", "direction": "right", "ratio": 0.5,
                                "rect": {"x": 0, "y": 0, "width": 120, "height": 60}})]
                } else {
                    Vec::new()
                }
            })
        };
        let snapshot = json!({
            "version": "0.8.2",
            "protocol": hide_herdr_client::HERDR_PROTOCOL_REVISION,
            "focused_pane_id": "w1:p1",
            "workspaces": [{
                "workspace_id": "w1", "label": "fixture", "agent_status": "idle",
                "focused": true, "number": 1, "pane_count": 3, "tab_count": 2,
                "active_tab_id": "w1:t1"
            }],
            "tabs": [
                {"workspace_id": "w1", "tab_id": "w1:t1", "agent_status": "idle",
                 "focused": true, "number": 1, "pane_count": 2, "label": "1"},
                {"workspace_id": "w1", "tab_id": "w1:t2", "agent_status": "idle",
                 "focused": false, "number": 2, "pane_count": 1, "label": "2"}
            ],
            "panes": [
                pane("w1:t1", "w1:p1", "/work/repo"),
                pane("w1:t1", "w1:p2", "/tmp"),
                pane("w1:t2", "w1:p3", "/tmp")
            ],
            "layouts": [layout("w1:t1", &["w1:p1", "w1:p2"]), layout("w1:t2", &["w1:p3"])],
            "agents": []
        });
        SessionReplica::from_snapshot(&snapshot)
            .expect("fixture snapshot")
            .project()
    }

    fn cwd(payload: &SessionSnapshotPayload, pane_id: &str) -> Option<String> {
        payload
            .panes
            .iter()
            .find(|pane| pane.pane_id == pane_id)
            .and_then(|pane| pane.cwd.clone())
    }

    /// B20: a split's pane publishes with its layout before Herdr confirms
    /// its cwd, read as the folder of the pane it was split from.
    #[test]
    fn a_new_pane_beside_a_confirmed_one_publishes_in_that_folder() {
        let mut payload = payload();
        assert_eq!(
            stand_in_unconfirmed_cwds(&mut payload, &["w1:p2".to_owned()], &HashSet::new()),
            Some(vec![("w1:p2".to_owned(), "/work/repo".to_owned())])
        );
        assert_eq!(cwd(&payload, "w1:p2").as_deref(), Some("/work/repo"));
    }

    /// A lone unconfirmed pane in a tab Hide did not create waits for its
    /// read; one in a tab Hide created publishes, read as that tab's checkout.
    #[test]
    fn a_lone_unconfirmed_pane_waits_unless_hide_created_its_tab() {
        let mut payload = payload();
        let awaiting = ["w1:p3".to_owned()];
        assert_eq!(
            stand_in_unconfirmed_cwds(&mut payload, &awaiting, &HashSet::new()),
            None
        );
        assert_eq!(
            stand_in_unconfirmed_cwds(
                &mut payload,
                &awaiting,
                &HashSet::from(["w1:t2".to_owned()])
            ),
            Some(Vec::new()),
            "a clamped tab's pane needs no stand-in"
        );
        assert_eq!(cwd(&payload, "w1:p3").as_deref(), Some("/tmp"));
    }
}

#[cfg(test)]
mod pending_core_tests {
    use super::*;
    use crate::fake_herdr::FakeHerdr;

    /// PRD core-host-node-move amendment 3: a core waiting for its move's
    /// link asks GitHub nothing; the read is asked once the hold is released.
    #[test]
    fn a_pending_core_reads_nothing_from_github_until_its_link_commits() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, ..) = crate::runtime::delivery::tests::fixture(root.path());
        let herdr = FakeHerdr::start("pending-github", |method, _| panic!("unexpected {method}"));
        let context = SessionSyncContext::local(
            &LiveContext {
                socket_path: herdr.socket_path().to_path_buf(),
                runtime: Arc::downgrade(&runtime),
                notifier: ChangeNotifier::noop(),
                api_connector: Arc::new(herdr.connector()),
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
                api_connector: Arc::new(hide_herdr_client::LocalSocketConnector::new(&socket_path)),
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
#[path = "coordinator_fence_tests.rs"]
mod coordinator_fence_tests;
