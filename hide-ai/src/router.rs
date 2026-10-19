use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::log::{AiLogEvent, AiLogSink};
use crate::schema;
use crate::usage_limit::{PastLimit, UsageLimits};
use crate::{
    AiBackend, AiError, AiRequest, AiResponse, AiResult, AiUsageSink, Availability, CancelToken,
    ModelCatalog, ProcessMeasurement, ProviderId,
};

/// Retry and selection policy. The defaults carry the retired label plugin's
/// proven constants forward: exponential backoff capped at four attempts for a
/// refusal that may clear on its own, two for one settled by the input.
/// A request whose completion is unknown is never attempted again.
#[derive(Clone, Debug)]
pub struct RouterConfig {
    /// The providers a request may be asked of, in the order they are tried:
    /// the operator's choice first, then the fallback list. A provider that is
    /// registered but not listed here is never asked a request (D-16); it is
    /// only inspected, for the Settings screen.
    pub priority: Vec<ProviderId>,
    /// Use Hide AI. Off, `execute` returns [`AiError::Disabled`] without
    /// asking any provider anything (B33).
    pub enabled: bool,
    /// One request may spend at most its own deadline times this on every
    /// provider it is moved through, never more than `overall_deadline_cap`
    /// and never less than its own deadline (B43).
    pub overall_deadline_factor: u32,
    pub overall_deadline_cap: Duration,
    pub availability_ttl: Duration,
    pub backoff_base: Duration,
    pub max_transient_attempts: u8,
    pub max_invalid_output_attempts: u8,
    /// Cooldown applied when a provider reports a usage limit without a
    /// reset time.
    pub default_cooldown: Duration,
    /// Budget caps (resident-process practice, rule 4). Crossing one is a
    /// reported `OverBudget` failure, never a larger number. The values are
    /// measured constants, not settings: a background feature peaked at 20
    /// requests a minute and a healthy codex tree at one or two descendants
    /// (2026-09-17).
    ///
    /// - `max_in_flight`: requests running at once across the router.
    /// - `max_per_minute`: requests started in any 60-second window.
    /// - `max_app_server_descendants`: children under the codex app-server
    ///   after a turn.
    /// - `max_app_server_rss_bytes`: resident size of the app-server tree.
    pub max_in_flight: usize,
    pub max_per_minute: usize,
    pub max_app_server_descendants: usize,
    pub max_app_server_rss_bytes: u64,
    /// How many consecutive over-budget restarts of the app-server are allowed
    /// before the request fails with `ProviderUnavailable(app_server_restart_cap)`.
    /// A request that completes under the cap resets the count.
    pub max_consecutive_restarts: u32,
    /// The operator's usage limit and the readings it is judged by. A router
    /// that has one skips an agent that is at or past the limit, as it skips
    /// any agent that cannot answer, so the request moves to the next one of
    /// the priority; only requests Hide starts on its own are given one, and
    /// work an operator waits on is not (PRD hide-ai-usage-limit D-03). A
    /// request that names its own agent is never judged by it.
    pub usage_gate: Option<Arc<UsageLimits>>,
    /// Where the usage of every attempt that reached a provider is told. A
    /// router without one counts nothing: the Settings probe never submits a
    /// model turn, and a test router has no ledger.
    pub usage_sink: Option<Arc<dyn AiUsageSink>>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            priority: vec![crate::PROVIDERS[0]],
            enabled: true,
            overall_deadline_factor: 2,
            overall_deadline_cap: Duration::from_secs(90),
            availability_ttl: Duration::from_secs(30),
            backoff_base: Duration::from_secs(5),
            max_transient_attempts: 4,
            max_invalid_output_attempts: 2,
            default_cooldown: Duration::from_secs(600),
            max_in_flight: 1,
            max_per_minute: 30,
            max_app_server_descendants: 4,
            max_app_server_rss_bytes: 1024 * 1024 * 1024,
            max_consecutive_restarts: 3,
            usage_gate: None,
            usage_sink: None,
        }
    }
}

/// Which provider answers now, and why the selected one is not.
///
/// The router already names the answering provider in each `AiResult`; what
/// a consumer could not ask before is the standing question, "who answers
/// right now and why is it not my choice". This is that answer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderState {
    /// The provider the configured priority puts first: the user's choice.
    pub selected: ProviderId,
    /// The provider a request made now would run on, or `None` when every
    /// provider is degraded. That state is reachable, so it is reported
    /// rather than filled in with a provider that cannot answer.
    pub active: Option<ProviderId>,
    /// Why `selected` stepped aside; absent when it did not.
    pub degraded: Option<Degraded>,
}

/// Why one provider is not answering, and until when when that is known.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Degraded {
    pub provider: ProviderId,
    pub reason: Availability,
    /// What remains of an account-wide cooldown. `None` when the reason has
    /// no end the provider told us about, such as a missing login.
    pub until: Option<Duration>,
}

/// One registered provider as the Settings screen needs it: whether the
/// operator can choose it, and why not when they cannot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderStatus {
    pub provider: ProviderId,
    /// What the provider answered, or the cooldown it is parked in.
    pub availability: Availability,
    /// The agent can be chosen: its CLI is installed, it is logged in, it has
    /// a backend and the call it makes is read-only (D-15, B35). A provider
    /// parked in a usage cooldown is still selectable: it was logged in and
    /// will answer again.
    pub selectable: bool,
    /// What remains of an account-wide usage cooldown (B41).
    pub parked_for: Option<Duration>,
}

/// What selection decided about one provider.
///
/// `Parked` exists so a provider under an account-wide cooldown is never
/// asked anything: the cooldown already answers, and probing a provider that
/// cannot answer for hours is the poke that sticky failover exists to stop.
#[derive(Clone, Debug)]
enum Selection {
    Ready,
    Parked(Duration),
    Blocked(Availability),
    /// Signed in and able, but at or past the operator's usage limit: not
    /// asked, and not a refusal of its own, since the agent would answer.
    PastLimit(PastLimit),
}

impl Selection {
    fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// The availability a caller is told about, including the cooldown that
    /// was never a provider answer.
    fn availability(&self) -> Availability {
        match self {
            Self::Ready => Availability::Ready,
            Self::Parked(remaining) => Availability::Unavailable {
                reason: format!("usage_limited;retry_after_s={}", remaining.as_secs()),
            },
            Self::Blocked(state) => state.clone(),
            Self::PastLimit(past) => Availability::Unavailable {
                reason: past_limit_reason(past),
            },
        }
    }

    /// Short machine-readable reason for a log line.
    fn detail(&self) -> String {
        match self {
            Self::Ready => "ready".to_owned(),
            Self::Parked(remaining) => {
                format!("usage_limited;retry_after_s={}", remaining.as_secs())
            }
            Self::Blocked(
                state @ (Availability::Unavailable { reason }
                | Availability::Unsupported { reason }),
            ) => format!("{}:{reason}", state.class()),
            Self::Blocked(state) => state.class().to_owned(),
            Self::PastLimit(past) => past_limit_reason(past),
        }
    }
}

fn past_limit_reason(past: &PastLimit) -> String {
    format!(
        "usage_limit;used={};limit={}",
        past.used_percent, past.limit_percent
    )
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DedupKey {
    feature_id: std::borrow::Cow<'static, str>,
    subject_id: String,
    input_hash: u64,
    pick: Option<crate::AiPick>,
}

struct InFlight {
    done: Mutex<Option<Result<AiResult, AiError>>>,
    changed: Condvar,
}

/// Owns one in-flight entry for its leader. Settling it wakes every joiner
/// and frees the key; dropping it unsettled, which only a panic does, settles
/// it with an explicit error so no joiner waits on a leader that is gone.
struct FlightGuard<'a> {
    router: &'a AiRouter,
    key: DedupKey,
    flight: Arc<InFlight>,
    settled: bool,
    /// Set once the router admits this leader against the concurrency cap, so
    /// its slot is released on every exit path including a panic.
    admitted: bool,
}

impl FlightGuard<'_> {
    fn settle(&mut self, result: Result<AiResult, AiError>) {
        self.settled = true;
        {
            let mut done = self.flight.done.lock().unwrap_or_else(|e| e.into_inner());
            *done = Some(result);
        }
        self.flight.changed.notify_all();
        self.router.lock().in_flight.remove(&self.key);
    }
}

impl Drop for FlightGuard<'_> {
    fn drop(&mut self) {
        if !self.settled {
            self.settle(Err(AiError::Internal("leader_panicked".to_owned())));
        }
        if self.admitted {
            self.router.release();
        }
    }
}

#[derive(Default)]
struct Rollup {
    day: u64,
    counts: HashMap<(ProviderId, &'static str), u64>,
}

/// What a backend measured of its own process during a turn, and whether it
/// is one that can be measured at all.
#[derive(Clone, Copy)]
struct Measured {
    measurement: ProcessMeasurement,
    measurable: bool,
}

impl Measured {
    fn none() -> Self {
        Self {
            measurement: ProcessMeasurement::Unavailable,
            measurable: false,
        }
    }
}

struct State {
    in_flight: HashMap<DedupKey, Arc<InFlight>>,
    /// Distinct intents running at once, for the concurrency cap. Joiners of an
    /// existing intent do not count; only a leader increments it.
    in_flight_count: usize,
    /// Start times of recently admitted requests, for the per-minute cap.
    recent_starts: VecDeque<Instant>,
    /// Consecutive over-budget restarts of the app-server, per provider. A
    /// request that completes under the cap clears it.
    restart_streak: HashMap<ProviderId, u32>,
    cooldown_until: HashMap<ProviderId, Instant>,
    availability: HashMap<ProviderId, (Availability, Instant)>,
    /// The selected provider a degradation event has already been logged
    /// for, so entering and leaving failover are each announced once rather
    /// than on every request.
    announced_degraded: Option<ProviderId>,
    /// The reason each provider's model list was last unavailable, so a read
    /// that fails the same way again is not logged again.
    models_unavailable: HashMap<ProviderId, String>,
    /// The provider that answered the most recent request.
    last_answered: Option<ProviderId>,
    rollup: Rollup,
}

/// Selects a provider, suppresses duplicate intents, applies the retry and
/// fallback policy, and re-validates every answer against the feature schema.
pub struct AiRouter {
    backends: Vec<Arc<dyn AiBackend>>,
    config: RouterConfig,
    sink: Arc<dyn AiLogSink>,
    sleep: Box<dyn Fn(Duration) + Send + Sync>,
    /// The time every park, cache window and per-minute window is measured
    /// against: the system's monotonic clock unless a test gives another.
    now: Box<dyn Fn() -> Instant + Send + Sync>,
    state: Mutex<State>,
}

impl AiRouter {
    #[allow(clippy::disallowed_methods)] // a production wait, not test code
    pub fn new(
        backends: Vec<Arc<dyn AiBackend>>,
        config: RouterConfig,
        sink: Arc<dyn AiLogSink>,
    ) -> Self {
        Self::with_sleep(backends, config, sink, Box::new(std::thread::sleep))
    }

    /// Tests inject a sleeper so backoff is observable without waiting it out.
    pub fn with_sleep(
        backends: Vec<Arc<dyn AiBackend>>,
        config: RouterConfig,
        sink: Arc<dyn AiLogSink>,
        sleep: Box<dyn Fn(Duration) + Send + Sync>,
    ) -> Self {
        Self {
            backends,
            config,
            sink,
            sleep,
            now: Box::new(Instant::now),
            state: Mutex::new(State {
                in_flight: HashMap::new(),
                in_flight_count: 0,
                recent_starts: VecDeque::new(),
                restart_streak: HashMap::new(),
                cooldown_until: HashMap::new(),
                availability: HashMap::new(),
                announced_degraded: None,
                models_unavailable: HashMap::new(),
                last_answered: None,
                rollup: Rollup {
                    day: utc_day(),
                    counts: HashMap::new(),
                },
            }),
        }
    }

    /// Tests inject a clock so a park or a cache window can end without
    /// waiting it out.
    #[cfg(test)]
    fn with_clock(mut self, now: Box<dyn Fn() -> Instant + Send + Sync>) -> Self {
        self.now = now;
        self
    }

    /// Current availability of every registered provider, refreshed when the
    /// cached answer is older than the configured window. A provider parked
    /// by an account-wide cooldown reports that park instead of being asked.
    pub fn availability(&self) -> Vec<(ProviderId, Availability)> {
        self.select_among(self.inspected_backends())
            .into_iter()
            .map(|(backend, selection)| (backend.id(), selection.availability()))
            .collect()
    }

    /// Which provider answers a request made now, and why the selected one
    /// stepped aside. `None` when the router has no registered provider,
    /// which is a wiring mistake rather than a runtime state.
    ///
    /// This is a query: it never logs and never runs a request. It may
    /// refresh a stale availability answer, which is what makes the return
    /// to the selected provider visible once its cooldown has expired.
    pub fn provider_state(&self) -> Option<ProviderState> {
        let selection = self.select_among(self.candidates());
        let (first, chosen) = selection.first()?;
        let selected = first.id();
        let degraded = degraded_of(selected, chosen);
        // Who a request would run on is judged with the usage limit, who is
        // degraded is not: an agent past the limit is signed in and able.
        let active = active_provider(&self.apply_usage_limit(selection.clone()));
        Some(ProviderState {
            selected,
            active,
            degraded,
        })
    }

    /// What each registered provider offers, in the configured priority
    /// order. This asks the provider rather than the router's cache, because
    /// a model list is not part of a selection decision and has no window of
    /// its own; the caller decides how often to ask.
    pub fn models(&self) -> Vec<(ProviderId, ModelCatalog)> {
        let catalogs: Vec<(ProviderId, ModelCatalog)> = self
            .inspected_backends()
            .into_iter()
            .map(|backend| (backend.id(), backend.models()))
            .collect();
        self.log_unreadable_catalogs(&catalogs);
        catalogs
    }

    /// The screen shows only that a list is unavailable and a code for it
    /// (design rule 13); what the operator or a maintainer needs to find the
    /// cause is here. Written when a provider's reason first appears or
    /// changes, not on every read, which the open tab repeats every few
    /// seconds, and again if the list comes back and fails later.
    fn log_unreadable_catalogs(&self, catalogs: &[(ProviderId, ModelCatalog)]) {
        let mut announced = Vec::new();
        {
            let mut state = self.lock();
            for (provider, catalog) in catalogs {
                match catalog.unknown_reason() {
                    Some(reason) => {
                        if state.models_unavailable.get(provider).map(String::as_str)
                            != Some(reason)
                        {
                            state
                                .models_unavailable
                                .insert(*provider, reason.to_owned());
                            announced.push((*provider, reason.to_owned()));
                        }
                    }
                    None => {
                        state.models_unavailable.remove(provider);
                    }
                }
            }
        }
        for (provider, reason) in announced {
            let mut event = AiLogEvent::new("ai.models.unavailable");
            event.provider = Some(provider);
            event.detail = Some(format!("reason={reason}"));
            self.sink.log(event);
        }
    }

    /// Every registered provider's standing for the Settings screen: which can
    /// be chosen, and what remains of a usage cooldown. Asks like
    /// [`Self::availability`].
    pub fn statuses(&self) -> Vec<ProviderStatus> {
        self.select_among(self.inspected_backends())
            .into_iter()
            .map(|(backend, selection)| ProviderStatus {
                provider: backend.id(),
                selectable: matches!(selection, Selection::Ready | Selection::Parked(_)),
                parked_for: match selection {
                    Selection::Parked(remaining) => Some(remaining),
                    _ => None,
                },
                availability: selection.availability(),
            })
            .collect()
    }

    /// The provider that answered the most recent request, which is what
    /// "Using Codex" is said of while the chosen provider cannot answer (B41).
    pub fn last_answered(&self) -> Option<ProviderId> {
        self.lock().last_answered
    }

    /// Forces the next selection to ask each provider again, for a caller
    /// that knows something changed (a login completed).
    pub fn invalidate_availability(&self) {
        self.lock().availability.clear();
    }

    /// The process measurement the named provider took during its last turn,
    /// so a verification command can report the app-server's descendant count.
    /// A provider that is not registered, or owns no resident process, answers
    /// `Unavailable`.
    pub fn process_measurement(&self, provider: ProviderId) -> ProcessMeasurement {
        self.backends
            .iter()
            .find(|backend| backend.id() == provider)
            .map_or(ProcessMeasurement::Unavailable, |backend| {
                backend.last_measurement()
            })
    }

    /// Runs the request on the selected provider and returns the validated
    /// answer with the provider that produced it. Every failure is typed;
    /// see [`AiError`] for which ones the router retries or moves.
    pub fn execute(&self, request: &AiRequest, cancel: &CancelToken) -> Result<AiResult, AiError> {
        if !self.config.enabled {
            return Err(AiError::Disabled);
        }
        let key = DedupKey {
            feature_id: request.feature_id.clone(),
            subject_id: request.subject_id.clone(),
            input_hash: input_hash(request),
            pick: request.pick.clone(),
        };
        let (flight, leader) = {
            let mut state = self.lock();
            match state.in_flight.get(&key) {
                Some(existing) => (Arc::clone(existing), false),
                None => {
                    let flight = Arc::new(InFlight {
                        done: Mutex::new(None),
                        changed: Condvar::new(),
                    });
                    state.in_flight.insert(key.clone(), Arc::clone(&flight));
                    (flight, true)
                }
            }
        };
        if !leader {
            self.log(request, |event| {
                event.event = "ai.request.joined".into();
            });
            return self.join(&flight, cancel);
        }
        let mut guard = FlightGuard {
            router: self,
            key,
            flight,
            settled: false,
            admitted: false,
        };
        // The concurrency and per-minute caps are checked before the request
        // is submitted; crossing one is a refusal, safe to retry, and it does
        // not move to another provider (the cap is the account's, not the
        // provider's). It is not counted against a specific provider.
        if let Err(over) = self.admit() {
            self.log(request, |event| {
                event.event = "ai.budget.exceeded".into();
                event.outcome_class = Some(over.class().into());
                event.detail = Some(over.to_string());
            });
            guard.settle(Err(over.clone()));
            return Err(over);
        }
        guard.admitted = true;
        let result = self.execute_selected(request, cancel);
        guard.settle(result.clone());
        result
    }

    /// Admits one leader against the concurrency and per-minute caps, counting
    /// it atomically so a race cannot slip a second past the limit.
    fn admit(&self) -> Result<(), AiError> {
        let mut state = self.lock();
        if state.in_flight_count >= self.config.max_in_flight {
            return Err(AiError::OverBudget {
                cap: "in_flight".into(),
                measured: state.in_flight_count as u64 + 1,
            });
        }
        let now = (self.now)();
        let window = Duration::from_secs(60);
        while state
            .recent_starts
            .front()
            .is_some_and(|start| now.duration_since(*start) >= window)
        {
            state.recent_starts.pop_front();
        }
        if state.recent_starts.len() >= self.config.max_per_minute {
            return Err(AiError::OverBudget {
                cap: "per_minute".into(),
                measured: state.recent_starts.len() as u64 + 1,
            });
        }
        state.in_flight_count += 1;
        state.recent_starts.push_back(now);
        Ok(())
    }

    fn release(&self) {
        let mut state = self.lock();
        state.in_flight_count = state.in_flight_count.saturating_sub(1);
    }

    /// Emits the daily rollup for the day that just ended, if any. Called on
    /// every completed request and available to a caller at shutdown.
    pub fn flush_rollup(&self) {
        let today = utc_day();
        let previous = {
            let mut state = self.lock();
            if state.rollup.day == today && !state.rollup.counts.is_empty() {
                // The day is still open; nothing to close.
                return;
            }
            std::mem::replace(
                &mut state.rollup,
                Rollup {
                    day: today,
                    counts: HashMap::new(),
                },
            )
        };
        if previous.counts.is_empty() {
            return;
        }
        let mut parts: Vec<String> = previous
            .counts
            .iter()
            .map(|((provider, class), count)| format!("{provider}.{class}={count}"))
            .collect();
        parts.sort();
        let mut event = AiLogEvent::new("ai.daily_rollup");
        event.detail = Some(format!("day={};{}", previous.day, parts.join(";")));
        self.sink.log(event);
    }

    fn join(&self, flight: &InFlight, cancel: &CancelToken) -> Result<AiResult, AiError> {
        let mut done = flight.done.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(result) = done.as_ref() {
                return result.clone();
            }
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled);
            }
            let (guard, _) = flight
                .changed
                .wait_timeout(done, Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner());
            done = guard;
        }
    }

    fn execute_selected(
        &self,
        request: &AiRequest,
        cancel: &CancelToken,
    ) -> Result<AiResult, AiError> {
        let started = (self.now)();
        let limit = started + self.overall_deadline(request);
        // Availability is read once per request, and a parked provider is
        // not read at all. Only the providers the operator listed are asked,
        // or the one agent the caller picked.
        if let Some(pick) = &request.pick
            && let Some(effort) = pick.effort.as_deref()
            && !pick.provider.efforts().contains(&effort)
        {
            let error =
                AiError::Unsupported(format!("effort_not_declared:{}:{effort}", pick.provider));
            self.finish(
                request,
                Some(pick.provider),
                &Err(error.clone()),
                started,
                0,
                Measured::none(),
            );
            return Err(error);
        }
        let candidates = match &request.pick {
            Some(pick) => self
                .backends
                .iter()
                .filter(|backend| backend.id() == pick.provider)
                .cloned()
                .collect(),
            None => self.candidates(),
        };
        let mut selection = self.select_among(candidates);
        if request.pick.is_none() {
            selection = self.apply_usage_limit(selection);
        }
        self.announce_degradation(request, &selection);
        self.log_usage_limit(request, &selection);
        let connected: Vec<&Arc<dyn AiBackend>> = selection
            .iter()
            .filter(|(_, state)| state.is_ready())
            .map(|(backend, _)| backend)
            .collect();
        if connected.is_empty() {
            // A parked provider is the more useful answer than "no provider":
            // it says the account is waiting, which is what a caller decides
            // its own retry on, and it names the wait.
            let parked = selection.iter().find_map(|(backend, state)| match state {
                Selection::Parked(remaining) => Some((backend.id(), *remaining)),
                _ => None,
            });
            // Nothing is wrong with an agent that is only past the limit, so a
            // sign-in or an outage elsewhere in the order is the more useful
            // thing to say; with none, the wait is the earliest reset.
            let past = selection.iter().find_map(|(_, state)| match state {
                Selection::PastLimit(past) => Some(*past),
                _ => None,
            });
            let blocked = selection
                .iter()
                .any(|(_, state)| matches!(state, Selection::Blocked(_)));
            let (provider, error) = match (parked, past) {
                (Some((provider, remaining)), _) => (
                    Some(provider),
                    AiError::UsageLimited {
                        retry_after: Some(remaining),
                    },
                ),
                (None, Some(past)) if !blocked => (
                    Some(past.provider),
                    AiError::UsageLimited {
                        retry_after: self.earliest_reset(&selection),
                    },
                ),
                _ => (
                    None,
                    AiError::NoProvider(
                        selection
                            .iter()
                            .map(|(backend, state)| (backend.id(), state.availability()))
                            .collect(),
                    ),
                ),
            };
            self.finish(
                request,
                provider,
                &Err(error.clone()),
                started,
                0,
                Measured::none(),
            );
            return Err(error);
        }

        let mut last_error = None;
        let mut previous: Option<ProviderId> = None;
        for backend in connected {
            let provider = backend.id();
            // A refusal that took time leaves the next provider what remains
            // of the request's overall deadline, and nothing once it is gone
            // (B43): the request then ends with the refusal that was seen.
            if previous.is_some() && limit.saturating_duration_since((self.now)()).is_zero() {
                break;
            }
            if let Some(from) = previous {
                self.log(request, |event| {
                    event.event = "ai.fallback".into();
                    event.provider = Some(provider);
                    event.detail = Some(format!("from={from};to={provider}"));
                });
            }
            let outcome = self.execute_on(backend.as_ref(), request, cancel, limit);
            // The measurement the backend took during the turn goes on this
            // request's finished line, whichever way it ended.
            let measurement = Measured {
                measurement: backend.last_measurement(),
                measurable: backend.measurable(),
            };
            match outcome {
                Ok((value, attempt)) => {
                    self.lock().last_answered = Some(provider);
                    let result = AiResult { provider, value };
                    self.finish(
                        request,
                        Some(provider),
                        &Ok(result.clone()),
                        started,
                        attempt,
                        measurement,
                    );
                    return Ok(result);
                }
                Err((error, attempt)) => {
                    self.finish(
                        request,
                        Some(provider),
                        &Err(error.clone()),
                        started,
                        attempt,
                        measurement,
                    );
                    // Only a refusal moves on: the provider never took the
                    // request, so another provider repeats nothing.
                    let eligible = matches!(
                        error,
                        AiError::ProviderUnavailable(_)
                            | AiError::NotAuthenticated
                            | AiError::UsageLimited { .. }
                            | AiError::Unsupported(_)
                    );
                    if !eligible || cancel.is_cancelled() {
                        return Err(error);
                    }
                    previous = Some(provider);
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or(AiError::NoProvider(Vec::new())))
    }

    /// Runs the retry policy on one provider. The returned attempt number is
    /// the last one made.
    fn execute_on(
        &self,
        backend: &dyn AiBackend,
        request: &AiRequest,
        cancel: &CancelToken,
        limit: Instant,
    ) -> Result<(Value, u8), (AiError, u8)> {
        let provider = backend.id();
        let mut attempt: u8 = 0;
        loop {
            attempt = attempt.saturating_add(1);
            if cancel.is_cancelled() {
                return Err((AiError::Cancelled, attempt));
            }
            // Each attempt may take what is left of the request's overall
            // deadline, no more than its own deadline.
            let remaining = limit.saturating_duration_since((self.now)());
            let bounded;
            let attempt_request = if remaining < request.deadline {
                bounded = AiRequest {
                    deadline: remaining,
                    ..request.clone()
                };
                &bounded
            } else {
                request
            };
            let result = backend.execute(attempt_request, cancel);
            // Before the schema judges the answer: a rejected answer was paid
            // for all the same.
            self.tell_usage(request, provider, &result);
            let result = result.and_then(|response| {
                schema::validate(&request.output_schema, &response.value)?;
                Ok(response)
            });
            match result {
                Ok(response) => {
                    self.log(request, |event| {
                        event.event = "ai.attempt".into();
                        event.provider = Some(provider);
                        event.outcome_class = Some("ok".into());
                        event.attempt = Some(attempt);
                        event.effort = request.effort(provider).map(str::to_owned);
                        event.output_tokens = response.usage.output_tokens;
                        event.thinking_tokens =
                            response.usage.thinking_tokens.filter(|tokens| *tokens > 0);
                    });
                    // A turn that answered still fails if the process it ran in
                    // crossed the cap: crossing a cap is a reported failure,
                    // not a larger number (resident-process practice, rule 4).
                    if let Some(error) = self.enforce_process_budget(backend, request) {
                        return Err((error, attempt));
                    }
                    return Ok((response.value, attempt));
                }
                Err(error) => {
                    self.log(request, |event| {
                        event.event = "ai.attempt".into();
                        event.provider = Some(provider);
                        event.outcome_class = Some(error.class().into());
                        event.attempt = Some(attempt);
                        event.effort = request.effort(provider).map(str::to_owned);
                    });
                    let attempts = match &error {
                        AiError::Transient(_) => self.config.max_transient_attempts,
                        AiError::InvalidOutput(_) | AiError::Rejected(_) => {
                            self.config.max_invalid_output_attempts
                        }
                        // Submitted, fate unknown: a second attempt could be
                        // a second completion.
                        AiError::Timeout | AiError::CompletionUnknown(_) => 0,
                        AiError::UsageLimited { retry_after } => {
                            let wait = retry_after.unwrap_or(self.config.default_cooldown);
                            self.lock()
                                .cooldown_until
                                .insert(provider, (self.now)() + wait);
                            0
                        }
                        AiError::NotAuthenticated => {
                            self.lock()
                                .availability
                                .insert(provider, (Availability::NeedsLogin, (self.now)()));
                            0
                        }
                        AiError::ProviderUnavailable(reason) => {
                            self.lock().availability.insert(
                                provider,
                                (
                                    Availability::Unavailable {
                                        reason: reason.clone(),
                                    },
                                    (self.now)(),
                                ),
                            );
                            0
                        }
                        AiError::Cancelled
                        | AiError::Unsupported(_)
                        | AiError::OverBudget { .. }
                        | AiError::NoProvider(_)
                        | AiError::Disabled
                        | AiError::Internal(_) => 0,
                    };
                    if attempt >= attempts {
                        return Err((error, attempt));
                    }
                    let backoff = self.config.backoff_base * 4u32.pow(u32::from(attempt - 1));
                    // A retry that would start after the overall deadline is
                    // not started: the request ends with the failure in hand.
                    if (self.now)() + backoff >= limit {
                        return Err((error, attempt));
                    }
                    (self.sleep)(backoff);
                }
            }
        }
    }

    /// Decides every provider's standing in priority order without running
    /// anything. A provider under a live cooldown is skipped rather than
    /// probed: that skip is what makes a failover sticky, because the reason
    /// it stepped aside outlives the request that discovered it.
    fn select_among(
        &self,
        backends: Vec<Arc<dyn AiBackend>>,
    ) -> Vec<(Arc<dyn AiBackend>, Selection)> {
        backends
            .into_iter()
            .map(|backend| {
                let selection = match self.cooldown_remaining(backend.id()) {
                    Some(remaining) => Selection::Parked(remaining),
                    None => match self.cached_availability(backend.as_ref()) {
                        state if state.is_ready() => Selection::Ready,
                        state => Selection::Blocked(state),
                    },
                };
                (backend, selection)
            })
            .collect()
    }

    /// Logs the two edges of a failover.
    ///
    /// `ai.fallback` only exists when one request tries two providers. Once
    /// the failover is sticky the parked provider is never tried, so without
    /// these events the log would show the provider silently change and
    /// silently change back.
    fn announce_degradation(
        &self,
        request: &AiRequest,
        selection: &[(Arc<dyn AiBackend>, Selection)],
    ) {
        let Some((backend, chosen)) = selection.first() else {
            return;
        };
        let selected = backend.id();
        let announced = self.lock().announced_degraded;
        match (announced, chosen.is_ready()) {
            (None, false) => {
                self.lock().announced_degraded = Some(selected);
                let active = active_provider(selection);
                let reason = chosen.detail();
                self.log(request, |event| {
                    event.event = "ai.provider.degraded".into();
                    event.provider = active;
                    event.outcome_class = Some(chosen.availability().class().into());
                    event.detail = Some(format!(
                        "selected={selected};reason={reason};active={}",
                        active.map_or("none".to_owned(), |provider| provider.to_string())
                    ));
                });
            }
            (Some(_), true) => {
                self.lock().announced_degraded = None;
                self.log(request, |event| {
                    event.event = "ai.provider.recovered".into();
                    event.provider = Some(selected);
                    event.outcome_class = Some("ready".into());
                    event.detail = Some(format!("selected={selected}"));
                });
            }
            _ => {}
        }
    }

    /// Tells the sink what one backend attempt cost. An answer carries its
    /// usage. A failure counts as an unknown call when the backend had been
    /// handed the request, so a model turn may have run and the error carries
    /// no usage: `Timeout`, `CompletionUnknown`, `Cancelled` and
    /// `InvalidOutput`. A backend raises none of them before it submits the
    /// prompt (a spawn or pipe failure is `ProviderUnavailable`, a lost
    /// session before `turn/start` too). Every other failure is a refusal
    /// before any model turn and cost nothing. A cancel that never reached a
    /// backend, and a caller joined onto another's flight, return before this
    /// is called, and a schema failure of an answer comes after it, so none
    /// of them is told twice or at all.
    fn tell_usage(
        &self,
        request: &AiRequest,
        provider: ProviderId,
        result: &Result<AiResponse, AiError>,
    ) {
        let Some(sink) = &self.config.usage_sink else {
            return;
        };
        match result {
            Ok(response) => sink.attempt(&request.feature_id, provider, &response.usage.models),
            Err(
                AiError::Timeout
                | AiError::CompletionUnknown(_)
                | AiError::Cancelled
                | AiError::InvalidOutput(_),
            ) => sink.attempt(&request.feature_id, provider, &[]),
            Err(_) => {}
        }
    }

    /// Turns every agent that is able to answer but at or past the operator's
    /// usage limit into one that is skipped. Without a gate, or for an agent
    /// with no reading, nothing changes (PRD hide-ai-usage-limit D-02, D-07).
    fn apply_usage_limit(
        &self,
        selection: Vec<(Arc<dyn AiBackend>, Selection)>,
    ) -> Vec<(Arc<dyn AiBackend>, Selection)> {
        let Some(gate) = &self.config.usage_gate else {
            return selection;
        };
        let now = unix_ms();
        selection
            .into_iter()
            .map(|(backend, state)| match state {
                Selection::Ready => match gate.past_limit(backend.id(), now) {
                    Some(past) => (backend, Selection::PastLimit(past)),
                    None => (backend, Selection::Ready),
                },
                other => (backend, other),
            })
            .collect()
    }

    /// Logs each agent the usage limit kept this request from asking: the ones
    /// ahead of the agent that answers, or every one when none can (D-09).
    fn log_usage_limit(&self, request: &AiRequest, selection: &[(Arc<dyn AiBackend>, Selection)]) {
        let answering = active_provider(selection);
        for (_, state) in selection {
            if let Selection::PastLimit(past) = state {
                self.log(request, |event| {
                    event.event = "ai.usage_limit".into();
                    event.provider = Some(past.provider);
                    event.detail = Some(format!(
                        "reason=usage_limit;used={};limit={};action={}",
                        past.used_percent,
                        past.limit_percent,
                        answering.map_or("paused".to_owned(), |to| format!("fallback;to={to}"))
                    ));
                });
            } else if state.is_ready() {
                break;
            }
        }
    }

    /// How long until the soonest window that holds an agent past the limit
    /// resets, when any of them said.
    fn earliest_reset(&self, selection: &[(Arc<dyn AiBackend>, Selection)]) -> Option<Duration> {
        let now = unix_ms();
        selection
            .iter()
            .filter_map(|(_, state)| match state {
                Selection::PastLimit(past) => past.resets_at_unix_ms,
                _ => None,
            })
            .min()
            .map(|reset| Duration::from_millis(reset.saturating_sub(now)))
    }

    /// The providers a request may be asked of, in the order they are tried:
    /// the configured priority, each once, and only those that are
    /// registered. A registered provider outside the priority is never in
    /// this list, however many are installed (D-16).
    fn candidates(&self) -> Vec<Arc<dyn AiBackend>> {
        let mut ordered: Vec<Arc<dyn AiBackend>> = Vec::new();
        for provider in &self.config.priority {
            if ordered.iter().any(|b| b.id() == *provider) {
                continue;
            }
            if let Some(backend) = self.backends.iter().find(|b| b.id() == *provider) {
                ordered.push(Arc::clone(backend));
            }
        }
        ordered
    }

    /// Every registered provider: the candidates in their order, then the
    /// rest. This is what Settings asks about, so an agent that is not in use
    /// can still be offered, or shown with the reason it cannot be; it is
    /// never what a request is run on.
    fn inspected_backends(&self) -> Vec<Arc<dyn AiBackend>> {
        let mut ordered = self.candidates();
        for backend in &self.backends {
            if !ordered.iter().any(|b| b.id() == backend.id()) {
                ordered.push(Arc::clone(backend));
            }
        }
        ordered
    }

    /// What one request may spend across every provider it is moved through:
    /// its own deadline times the configured factor, capped, and never less
    /// than its own deadline.
    fn overall_deadline(&self, request: &AiRequest) -> Duration {
        request.deadline.max(
            request
                .deadline
                .saturating_mul(self.config.overall_deadline_factor)
                .min(self.config.overall_deadline_cap),
        )
    }

    fn cached_availability(&self, backend: &dyn AiBackend) -> Availability {
        let provider = backend.id();
        if let Some((state, checked)) = self.lock().availability.get(&provider)
            && (self.now)().saturating_duration_since(*checked) < self.config.availability_ttl
        {
            return state.clone();
        }
        let state = backend.availability();
        self.lock()
            .availability
            .insert(provider, (state.clone(), (self.now)()));
        state
    }

    fn cooldown_remaining(&self, provider: ProviderId) -> Option<Duration> {
        let mut state = self.lock();
        let until = *state.cooldown_until.get(&provider)?;
        let now = (self.now)();
        if until <= now {
            state.cooldown_until.remove(&provider);
            // The park is over. Drop the cached answer with it so the next
            // question is asked of the provider once, rather than answered
            // from a cache filled while it could not answer at all.
            state.availability.remove(&provider);
            return None;
        }
        Some(until - now)
    }

    fn finish(
        &self,
        request: &AiRequest,
        provider: Option<ProviderId>,
        result: &Result<AiResult, AiError>,
        started: Instant,
        attempt: u8,
        measured: Measured,
    ) {
        let Measured {
            measurement,
            measurable,
        } = measured;
        let class = match result {
            Ok(_) => "ok",
            Err(error) => error.class(),
        };
        self.log(request, |event| {
            event.event = "ai.request.finished".into();
            event.provider = provider;
            event.outcome_class = Some(class.into());
            event.duration_ms =
                Some((self.now)().saturating_duration_since(started).as_millis() as u64);
            event.attempt = Some(attempt);
            let mut detail = Vec::new();
            if let Err(error) = result {
                detail.push(error.to_string());
            }
            match measurement {
                ProcessMeasurement::Available {
                    app_server_pid,
                    descendants,
                    rss_bytes,
                } => {
                    event.app_server_pid = Some(app_server_pid);
                    event.descendants = Some(descendants);
                    event.rss_bytes = Some(rss_bytes);
                }
                // A resident-process provider whose process could not be
                // measured says so rather than logging a zero that reads as a
                // healthy tree.
                ProcessMeasurement::Unavailable if measurable => {
                    detail.push("measurement=unavailable".to_owned());
                }
                ProcessMeasurement::Unavailable => {}
            }
            if !detail.is_empty() {
                event.detail = Some(detail.join(";"));
            }
        });
        if let Some(provider) = provider {
            let mut state = self.lock();
            *state.rollup.counts.entry((provider, class)).or_insert(0) += 1;
        }
        self.flush_rollup();
    }

    /// Enforces the codex app-server's descendant and resident-size caps after
    /// a turn. Under the cap resets the consecutive-restart count; over it logs
    /// `ai.app_server.over_budget`, restarts the app-server, and fails the
    /// request - with `ProviderUnavailable(app_server_restart_cap)` once the
    /// restarts stop clearing. A provider that cannot be measured is not
    /// capped.
    fn enforce_process_budget(
        &self,
        backend: &dyn AiBackend,
        request: &AiRequest,
    ) -> Option<AiError> {
        let provider = backend.id();
        let ProcessMeasurement::Available {
            app_server_pid,
            descendants,
            rss_bytes,
        } = backend.last_measurement()
        else {
            return None;
        };
        let over = if descendants > self.config.max_app_server_descendants {
            Some(("app_server_descendants", descendants as u64))
        } else if rss_bytes > self.config.max_app_server_rss_bytes {
            Some(("app_server_rss", rss_bytes))
        } else {
            None
        };
        let Some((cap, measured)) = over else {
            // A completed request under the cap returns the restart count.
            self.lock().restart_streak.remove(&provider);
            return None;
        };
        let streak = {
            let mut state = self.lock();
            let entry = state.restart_streak.entry(provider).or_insert(0);
            *entry += 1;
            *entry
        };
        self.log(request, |event| {
            event.event = "ai.app_server.over_budget".into();
            event.provider = Some(provider);
            event.outcome_class = Some("over_budget".into());
            event.app_server_pid = Some(app_server_pid);
            event.descendants = Some(descendants);
            event.rss_bytes = Some(rss_bytes);
            event.detail = Some(format!(
                "cap={cap};measured={measured};restart_streak={streak}"
            ));
        });
        backend.restart();
        if streak >= self.config.max_consecutive_restarts {
            Some(AiError::ProviderUnavailable(
                "app_server_restart_cap".to_owned(),
            ))
        } else {
            Some(AiError::OverBudget {
                cap: cap.into(),
                measured,
            })
        }
    }

    fn log(&self, request: &AiRequest, fill: impl FnOnce(&mut AiLogEvent)) {
        let mut event = AiLogEvent::new("ai.event");
        event.request_id = Some(request.request_id.0.clone());
        event.feature_id = Some(request.feature_id.clone());
        event.input_chars = Some(request.input.chars().count());
        event.schema_version = Some(request.schema_version.clone());
        fill(&mut event);
        self.sink.log(event);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The highest-priority provider that can answer now, if any.
fn active_provider(selection: &[(Arc<dyn AiBackend>, Selection)]) -> Option<ProviderId> {
    selection
        .iter()
        .find(|(_, state)| state.is_ready())
        .map(|(backend, _)| backend.id())
}

fn degraded_of(selected: ProviderId, chosen: &Selection) -> Option<Degraded> {
    match chosen {
        Selection::Ready => None,
        Selection::Parked(remaining) => Some(Degraded {
            provider: selected,
            reason: chosen.availability(),
            until: Some(*remaining),
        }),
        Selection::Blocked(reason) => Some(Degraded {
            provider: selected,
            reason: reason.clone(),
            until: None,
        }),
        // The limit is judged where a request is placed, never on this
        // selection: an agent past it is signed in and able, not degraded.
        Selection::PastLimit(_) => None,
    }
}

fn input_hash(request: &AiRequest) -> u64 {
    let mut hasher = DefaultHasher::new();
    request.system.hash(&mut hasher);
    // Asked in another language, the same input is another question: a
    // request made after the language changed never joins one from before.
    request.output_language.hash(&mut hasher);
    request.input.hash(&mut hasher);
    request.output_schema.to_string().hash(&mut hasher);
    hasher.finish()
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn utc_day() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AiResponse, RequestId};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Scripted provider: answers from a queue of outcomes and counts both
    /// the requests and the availability probes it was asked for.
    struct Scripted {
        id: ProviderId,
        availability: Mutex<Availability>,
        probes: AtomicUsize,
        outcomes: Mutex<VecDeque<Result<Value, AiError>>>,
        calls: AtomicUsize,
        /// Why `models()` cannot list, while it cannot.
        models_unreadable: Mutex<Option<String>>,
    }

    impl Scripted {
        fn new(id: ProviderId, outcomes: Vec<Result<Value, AiError>>) -> Arc<Self> {
            Arc::new(Self {
                id,
                availability: Mutex::new(Availability::Ready),
                probes: AtomicUsize::new(0),
                outcomes: Mutex::new(outcomes.into()),
                calls: AtomicUsize::new(0),
                models_unreadable: Mutex::new(None),
            })
        }

        fn with_availability(id: ProviderId, availability: Availability) -> Arc<Self> {
            Arc::new(Self {
                id,
                availability: Mutex::new(availability),
                probes: AtomicUsize::new(0),
                outcomes: Mutex::new(VecDeque::new()),
                calls: AtomicUsize::new(0),
                models_unreadable: Mutex::new(None),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn probes(&self) -> usize {
            self.probes.load(Ordering::SeqCst)
        }

        fn set_models_unreadable(&self, reason: Option<&str>) {
            *self.models_unreadable.lock().unwrap() = reason.map(str::to_owned);
        }

        fn set_availability(&self, state: Availability) {
            *self.availability.lock().unwrap() = state;
        }

        fn queue(&self, outcomes: Vec<Result<Value, AiError>>) {
            self.outcomes.lock().unwrap().extend(outcomes);
        }
    }

    impl AiBackend for Scripted {
        fn id(&self) -> ProviderId {
            self.id
        }
        /// Stands in for codex's resident app-server, the one backend that
        /// owns a process the router measures.
        fn measurable(&self) -> bool {
            self.id == ProviderId::CODEX
        }
        fn availability(&self) -> Availability {
            self.probes.fetch_add(1, Ordering::SeqCst);
            self.availability.lock().unwrap().clone()
        }
        fn models(&self) -> ModelCatalog {
            match self.models_unreadable.lock().unwrap().clone() {
                Some(reason) => ModelCatalog::Unknown { reason },
                None => ModelCatalog::Offered(vec![format!("{}-model", self.id)]),
            }
        }
        fn execute(&self, _: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let next = self
                .outcomes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(json!({"summary": "late"})));
            next.map(|value| AiResponse {
                value,
                usage: crate::AiUsage::default(),
            })
        }
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<AiLogEvent>>, Condvar);

    impl AiLogSink for Recorder {
        fn log(&self, event: AiLogEvent) {
            self.0.lock().unwrap().push(event);
            self.1.notify_all();
        }
    }

    impl Recorder {
        /// Blocks until `count` events named `name` were logged. The limit
        /// only ends a wait on an event that never comes; it is not measured.
        fn wait_for_events(&self, name: &str, count: usize) {
            let logged = self.0.lock().unwrap();
            let (_events, timeout) = self
                .1
                .wait_timeout_while(logged, Duration::from_secs(30), |events| {
                    events.iter().filter(|e| e.event == name).count() < count
                })
                .unwrap();
            assert!(!timeout.timed_out(), "{count} x {name} was never logged");
        }

        fn events(&self, name: &str) -> Vec<AiLogEvent> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.event == name)
                .cloned()
                .collect()
        }
    }

    /// A flag one thread raises and another waits for: what a test holds a
    /// request on, so it decides when a call is still running instead of a delay.
    #[derive(Default)]
    struct Gate(Mutex<bool>, Condvar);

    impl Gate {
        fn open(&self) {
            *self.0.lock().unwrap() = true;
            self.1.notify_all();
        }

        /// The limit only ends a wait on a gate nobody opens; it is not measured.
        fn wait(&self) {
            let open = self.0.lock().unwrap();
            let (_open, timeout) = self
                .1
                .wait_timeout_while(open, Duration::from_secs(30), |open| !*open)
                .unwrap();
            assert!(!timeout.timed_out(), "the gate was never opened");
        }
    }

    /// A provider that answers `shared` only once the test opens `release`,
    /// keeping the system prompt of each call it was given.
    #[derive(Default)]
    struct Held {
        started: Gate,
        release: Gate,
        calls: AtomicUsize,
        prompts: Mutex<Vec<String>>,
    }

    impl AiBackend for Held {
        fn id(&self) -> ProviderId {
            ProviderId::CODEX
        }
        fn availability(&self) -> Availability {
            Availability::Ready
        }
        fn models(&self) -> ModelCatalog {
            ModelCatalog::Offered(Vec::new())
        }
        fn execute(&self, request: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
            self.prompts
                .lock()
                .unwrap()
                .push(request.system_prompt().into_owned());
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.open();
            self.release.wait();
            Ok(AiResponse {
                value: json!({"summary": "shared"}),
                usage: crate::AiUsage::default(),
            })
        }
    }

    fn schema() -> Value {
        json!({"type": "object", "required": ["summary"], "properties": {"summary": {"type": "string"}}})
    }

    fn request(subject: &str, input: &str) -> AiRequest {
        AiRequest {
            feature_id: "test_feature".into(),
            request_id: RequestId(format!("r-{subject}")),
            subject_id: subject.to_owned(),
            system: "sys".to_owned(),
            input: input.to_owned(),
            output_schema: schema(),
            deadline: Duration::from_secs(5),
            schema_version: "test.v1".into(),
            pick: None,
            output_language: None,
            answer: crate::AnswerMode::default(),
            provider_efforts: Vec::new(),
        }
    }

    /// A feature's effort applies to its own provider only, and a caller's
    /// pick replaces it (label-judgment-fixes B17).
    #[test]
    fn a_features_effort_is_asked_of_its_provider_alone_and_a_pick_wins() {
        let mut asked = request("p", "x");
        asked.provider_efforts = vec![(ProviderId::CODEX, "medium".to_owned())];
        assert_eq!(asked.effort(ProviderId::CODEX), Some("medium"));
        assert_eq!(asked.effort(ProviderId::CLAUDE), None);
        asked.pick = Some(crate::AiPick {
            provider: ProviderId::CODEX,
            model: None,
            effort: Some("high".to_owned()),
        });
        assert_eq!(asked.effort(ProviderId::CODEX), Some("high"));
        asked.pick.as_mut().unwrap().effort = None;
        assert_eq!(asked.effort(ProviderId::CODEX), None);
    }

    /// A clock the test moves by hand, starting where the system's was when
    /// it was made.
    #[derive(Clone)]
    struct ManualClock(Arc<Mutex<Instant>>);

    impl ManualClock {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(Instant::now())))
        }

        fn advance(&self, by: Duration) {
            *self.0.lock().unwrap() += by;
        }

        fn source(&self) -> Box<dyn Fn() -> Instant + Send + Sync> {
            let clock = Arc::clone(&self.0);
            Box::new(move || *clock.lock().unwrap())
        }
    }

    /// The priority is the order the backends are given in: the first is the
    /// operator's choice and the rest are its fallback list.
    fn make_router(backends: Vec<Arc<dyn AiBackend>>, sink: Arc<Recorder>) -> AiRouter {
        let config = RouterConfig {
            priority: backends.iter().map(|backend| backend.id()).collect(),
            backoff_base: Duration::from_millis(1),
            ..RouterConfig::default()
        };
        AiRouter::with_sleep(backends, config, sink, Box::new(|_| {}))
    }

    fn ok(summary: &str) -> Result<Value, AiError> {
        Ok(json!({"summary": summary}))
    }

    #[test]
    fn a_picked_provider_is_the_only_one_asked_and_an_undeclared_effort_is_refused() {
        let sink = Arc::new(Recorder::default());
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let codex = Scripted::new(ProviderId::CODEX, vec![ok("codex")]);
        let router = make_router(vec![claude.clone(), codex.clone()], sink.clone());
        let picked = AiRequest {
            pick: Some(crate::AiPick {
                provider: ProviderId::CODEX,
                model: Some("gpt-5.5".into()),
                effort: Some("high".into()),
            }),
            ..request("p1", "x")
        };
        let result = router.execute(&picked, &CancelToken::new()).unwrap();
        assert_eq!(result.provider, ProviderId::CODEX);
        assert_eq!(claude.calls(), 0);

        let refused = AiRequest {
            pick: Some(crate::AiPick {
                provider: ProviderId::CODEX,
                model: None,
                effort: Some("max".into()),
            }),
            ..request("p2", "x")
        };
        assert!(matches!(
            router.execute(&refused, &CancelToken::new()),
            Err(AiError::Unsupported(_))
        ));
        assert_eq!(codex.calls(), 1);
    }

    #[test]
    fn a_single_connected_provider_is_used_and_its_answer_validated() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(ProviderId::CODEX, vec![ok("a")]);
        let router = make_router(vec![codex.clone()], sink.clone());
        let result = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        assert_eq!(result.value["summary"], "a");
        assert_eq!(result.provider, ProviderId::CODEX);
        let finished = sink.events("ai.request.finished");
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].provider, Some(ProviderId::CODEX));
        assert_eq!(finished[0].outcome_class.as_deref(), Some("ok"));
        assert_eq!(finished[0].request_id.as_deref(), Some("r-p1"));
        // A measurable backend whose process cannot be measured records that
        // on the line rather than a zero (B9); a real codex on macOS carries
        // the descendant fields instead.
        assert_eq!(
            finished[0].detail.as_deref(),
            Some("measurement=unavailable")
        );
        assert!(finished[0].descendants.is_none());
    }

    /// A provider that answers with a fixed thinking-token count.
    struct Thinks(Option<u64>);

    impl AiBackend for Thinks {
        fn id(&self) -> ProviderId {
            ProviderId::CLAUDE
        }
        fn availability(&self) -> Availability {
            Availability::Ready
        }
        fn models(&self) -> ModelCatalog {
            ModelCatalog::Offered(Vec::new())
        }
        fn execute(&self, _: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
            Ok(AiResponse {
                value: json!({"summary": "a"}),
                usage: crate::AiUsage {
                    output_tokens: Some(460),
                    thinking_tokens: self.0,
                    ..crate::AiUsage::default()
                },
            })
        }
    }

    /// A turn that thought although thinking was off shows it on the attempt
    /// line next to its output tokens; a turn that did not think, or whose
    /// provider reports none, adds no field.
    #[test]
    fn thinking_tokens_reach_the_attempt_line_only_when_there_were_some() {
        for (reported, logged) in [(Some(384), Some(384)), (Some(0), None), (None, None)] {
            let sink = Arc::new(Recorder::default());
            let router = make_router(vec![Arc::new(Thinks(reported))], sink.clone());
            router
                .execute(&request("p1", "x"), &CancelToken::new())
                .unwrap();
            let attempts = sink.events("ai.attempt");
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].output_tokens, Some(460));
            assert_eq!(attempts[0].thinking_tokens, logged, "{reported:?}");
        }
    }

    #[test]
    fn a_schema_mismatch_is_invalid_output_after_two_attempts_and_never_falls_back() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(
            ProviderId::CODEX,
            vec![
                Ok(json!({"other": 1})),
                Ok(json!({"summary": 5})),
                ok("never"),
            ],
        );
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let router = make_router(vec![codex.clone(), claude.clone()], sink);
        let error = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap_err();
        assert!(matches!(error, AiError::InvalidOutput(_)), "{error:?}");
        assert_eq!(codex.calls(), 2);
        assert_eq!(claude.calls(), 0);
    }

    /// A submitted request whose fate is unknown is spent: one call, no
    /// second attempt on the same provider, no attempt on another.
    #[test]
    fn a_completion_unknown_outcome_is_final_and_never_runs_elsewhere() {
        for error in [
            AiError::Timeout,
            AiError::CompletionUnknown("app_server_exited:signal".to_owned()),
        ] {
            let sink = Arc::new(Recorder::default());
            let codex = Scripted::new(ProviderId::CODEX, vec![Err(error.clone()), ok("again")]);
            let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
            let router = make_router(vec![codex.clone(), claude.clone()], sink.clone());
            assert_eq!(
                router.execute(&request("p1", "x"), &CancelToken::new()),
                Err(error.clone()),
                "{error:?}"
            );
            assert_eq!(codex.calls(), 1, "{error:?}");
            assert_eq!(claude.calls(), 0, "{error:?}");
            assert!(sink.events("ai.fallback").is_empty());
            let finished = sink.events("ai.request.finished");
            assert_eq!(finished.len(), 1);
            assert_eq!(finished[0].outcome_class.as_deref(), Some(error.class()));
            // The provider is not marked unavailable: the next intent is new.
            assert_eq!(
                router
                    .execute(&request("p2", "y"), &CancelToken::new())
                    .unwrap()
                    .value["summary"],
                "again"
            );
        }
    }

    /// A refusal that may clear is retried on the same provider only.
    #[test]
    fn a_transient_refusal_retries_the_same_provider_and_never_runs_elsewhere() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(
            ProviderId::CODEX,
            vec![
                Err(AiError::Transient("rpc_error:thread/start:-1".to_owned())),
                Err(AiError::Transient(
                    "control_timeout:thread/start".to_owned(),
                )),
                ok("third"),
            ],
        );
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let router = make_router(vec![codex.clone(), claude.clone()], sink.clone());
        let result = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        assert_eq!(result.value["summary"], "third");
        assert_eq!(codex.calls(), 3);
        assert_eq!(claude.calls(), 0);
        assert!(sink.events("ai.fallback").is_empty());

        let exhausted = Scripted::new(
            ProviderId::CODEX,
            vec![Err(AiError::Transient("busy".to_owned())); 4],
        );
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let router = make_router(vec![exhausted.clone(), claude.clone()], sink);
        assert!(matches!(
            router.execute(&request("p2", "x"), &CancelToken::new()),
            Err(AiError::Transient(_))
        ));
        assert_eq!(exhausted.calls(), 4);
        assert_eq!(claude.calls(), 0);
    }

    /// Whoever answers is named in the result, so a fallback can never be
    /// mistaken for the first provider's answer.
    #[test]
    fn a_result_names_the_provider_that_answered_after_a_fallback() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::with_availability(ProviderId::CODEX, Availability::NeedsLogin);
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let router = make_router(vec![codex.clone(), claude.clone()], sink.clone());
        let result = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        assert_eq!(result.provider, ProviderId::CLAUDE);
        assert_eq!(result.value["summary"], "claude");
        let finished = sink.events("ai.request.finished");
        assert_eq!(finished[0].provider, Some(ProviderId::CLAUDE));
    }

    /// A leader that panics must not strand its joiners or its key.
    #[test]
    fn a_panicking_leader_settles_every_joiner_with_an_error_and_frees_the_key() {
        struct Panicking {
            started: Arc<(Mutex<bool>, Condvar)>,
            release: Arc<(Mutex<bool>, Condvar)>,
            calls: AtomicUsize,
        }
        impl AiBackend for Panicking {
            fn id(&self) -> ProviderId {
                ProviderId::CODEX
            }
            fn availability(&self) -> Availability {
                Availability::Ready
            }
            fn models(&self) -> ModelCatalog {
                ModelCatalog::Offered(Vec::new())
            }
            fn execute(&self, _: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
                let calls = self.calls.fetch_add(1, Ordering::SeqCst);
                if calls > 0 {
                    return Ok(AiResponse {
                        value: json!({"summary": "fresh"}),
                        usage: crate::AiUsage::default(),
                    });
                }
                signal(&self.started);
                wait_for(&self.release);
                panic!("provider bug");
            }
        }
        fn signal(pair: &(Mutex<bool>, Condvar)) {
            *pair.0.lock().unwrap() = true;
            pair.1.notify_all();
        }
        fn wait_for(pair: &(Mutex<bool>, Condvar)) {
            let mut flag = pair.0.lock().unwrap();
            while !*flag {
                flag = pair.1.wait(flag).unwrap();
            }
        }
        let backend = Arc::new(Panicking {
            started: Arc::new((Mutex::new(false), Condvar::new())),
            release: Arc::new((Mutex::new(false), Condvar::new())),
            calls: AtomicUsize::new(0),
        });
        let sink = Arc::new(Recorder::default());
        let router = Arc::new(make_router(vec![backend.clone()], sink.clone()));
        let leader = {
            let router = Arc::clone(&router);
            std::thread::spawn(move || router.execute(&request("same", "x"), &CancelToken::new()))
        };
        wait_for(&backend.started);
        let joiner = {
            let router = Arc::clone(&router);
            std::thread::spawn(move || router.execute(&request("same", "x"), &CancelToken::new()))
        };
        // The joiner logs that it joined before it waits on the entry; only
        // then let the leader blow up.
        sink.wait_for_events("ai.request.joined", 1);
        signal(&backend.release);
        assert!(leader.join().is_err(), "the leader's panic propagates");
        assert_eq!(
            joiner.join().unwrap(),
            Err(AiError::Internal("leader_panicked".to_owned()))
        );
        assert!(router.lock().in_flight.is_empty());
        // The key is free: the same intent starts a new call.
        assert_eq!(
            router
                .execute(&request("same", "x"), &CancelToken::new())
                .unwrap()
                .value["summary"],
            "fresh"
        );
    }

    #[test]
    fn provider_unavailable_falls_back_in_priority_order_with_a_logged_event() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(
            ProviderId::CODEX,
            vec![Err(AiError::ProviderUnavailable("gone".to_owned()))],
        );
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let router = make_router(vec![codex.clone(), claude.clone()], sink.clone());
        let result = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        assert_eq!(result.value["summary"], "claude");
        assert_eq!(result.provider, ProviderId::CLAUDE);
        let fallback = sink.events("ai.fallback");
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].detail.as_deref(), Some("from=codex;to=claude"));
        // The failed provider's state is remembered, so the next request
        // goes straight to the fallback without a second failure.
        let result = router
            .execute(&request("p2", "y"), &CancelToken::new())
            .unwrap();
        assert_eq!(result.value["summary"], "late");
        assert_eq!(codex.calls(), 1);
    }

    fn gated_router(
        backends: Vec<Arc<dyn AiBackend>>,
        sink: Arc<Recorder>,
        gate: &Arc<UsageLimits>,
    ) -> AiRouter {
        let config = RouterConfig {
            priority: backends.iter().map(|backend| backend.id()).collect(),
            backoff_base: Duration::from_millis(1),
            usage_gate: Some(Arc::clone(gate)),
            ..RouterConfig::default()
        };
        AiRouter::with_sleep(backends, config, sink, Box::new(|_| {}))
    }

    fn week(used: f64) -> crate::UsageReading {
        crate::UsageReading {
            used_percent: used,
            resets_at_unix_ms: None,
        }
    }

    fn gate(
        limit: crate::UsageLimit,
        rows: &[(ProviderId, crate::UsageReading)],
    ) -> Arc<UsageLimits> {
        let gate = Arc::new(UsageLimits::new());
        gate.set(limit, &rows.iter().copied().collect());
        gate
    }

    /// B4, D-02: Runs on at 86% of a week with an 80% limit cannot answer a
    /// background request, so the next agent of the same order does, and the
    /// log says which agent the limit moved the request off.
    #[test]
    fn an_agent_past_the_usage_limit_is_skipped_for_the_next_one_of_the_order() {
        let sink = Arc::new(Recorder::default());
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let codex = Scripted::new(ProviderId::CODEX, vec![ok("codex")]);
        let limits = gate(
            crate::UsageLimit::Percent(80),
            &[
                (ProviderId::CLAUDE, week(86.0)),
                (ProviderId::CODEX, week(31.0)),
            ],
        );
        let router = gated_router(vec![claude.clone(), codex.clone()], sink.clone(), &limits);
        let result = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        assert_eq!(result.provider, ProviderId::CODEX);
        assert_eq!(claude.calls(), 0, "the agent past the limit is not asked");
        let moved = sink.events("ai.usage_limit");
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].provider, Some(ProviderId::CLAUDE));
        assert_eq!(
            moved[0].detail.as_deref(),
            Some("reason=usage_limit;used=86;limit=80;action=fallback;to=codex")
        );
        // Settings still offers the agent as signed in: past the limit is not
        // a sign-in or a selection problem.
        let status = router
            .statuses()
            .into_iter()
            .find(|status| status.provider == ProviderId::CLAUDE)
            .unwrap();
        assert!(status.selectable && status.availability.is_ready());
    }

    /// B9: the same 86% under a 90% limit is no reason to move.
    #[test]
    fn an_agent_under_the_usage_limit_answers_and_nothing_is_logged() {
        let sink = Arc::new(Recorder::default());
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let codex = Scripted::new(ProviderId::CODEX, vec![ok("codex")]);
        let limits = gate(
            crate::UsageLimit::Percent(90),
            &[(ProviderId::CLAUDE, week(86.0))],
        );
        let router = gated_router(vec![claude.clone(), codex], sink.clone(), &limits);
        let result = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        assert_eq!(result.provider, ProviderId::CLAUDE);
        assert!(sink.events("ai.usage_limit").is_empty());
    }

    /// D-06: with every agent past the limit nothing is asked; the answer is
    /// a wait that ends at the soonest reset, never a provider call.
    #[test]
    fn every_agent_past_the_usage_limit_asks_nobody_and_waits_for_the_soonest_reset() {
        let sink = Arc::new(Recorder::default());
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let codex = Scripted::new(ProviderId::CODEX, vec![ok("codex")]);
        let soon = unix_ms() + 3_600_000;
        let limits = gate(
            crate::UsageLimit::Percent(90),
            &[
                (
                    ProviderId::CLAUDE,
                    crate::UsageReading {
                        used_percent: 93.0,
                        resets_at_unix_ms: Some(soon + 86_400_000),
                    },
                ),
                (
                    ProviderId::CODEX,
                    crate::UsageReading {
                        used_percent: 96.0,
                        resets_at_unix_ms: Some(soon),
                    },
                ),
            ],
        );
        let router = gated_router(vec![claude.clone(), codex.clone()], sink.clone(), &limits);
        match router.execute(&request("p1", "x"), &CancelToken::new()) {
            Err(AiError::UsageLimited {
                retry_after: Some(wait),
            }) => assert!(
                wait <= Duration::from_secs(3_600) && wait > Duration::from_secs(3_500),
                "{wait:?}"
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!((claude.calls(), codex.calls()), (0, 0));
        let paused: Vec<_> = sink
            .events("ai.usage_limit")
            .into_iter()
            .map(|event| event.detail.unwrap())
            .collect();
        assert_eq!(paused.len(), 2);
        assert!(
            paused
                .iter()
                .all(|detail| detail.ends_with("action=paused"))
        );
    }

    /// B7, D-07: No limit stops nothing, and an agent with no figure is never
    /// held back, whatever the limit.
    #[test]
    fn no_limit_and_no_figure_never_move_a_request() {
        let rows = [(ProviderId::CLAUDE, week(99.0))];
        for (limit, ask_claude) in [
            (crate::UsageLimit::None, true),
            (crate::UsageLimit::Percent(80), false),
        ] {
            let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
            let codex = Scripted::new(ProviderId::CODEX, vec![ok("codex")]);
            let router = gated_router(
                vec![claude.clone(), codex.clone()],
                Arc::new(Recorder::default()),
                &gate(limit, &rows),
            );
            router
                .execute(&request("p1", "x"), &CancelToken::new())
                .unwrap();
            assert_eq!(claude.calls() == 1, ask_claude, "{limit:?}");
        }
        // Codex has no reading under an 80% limit: it answers when Claude is
        // the one past it and Codex is the only choice.
        let codex = Scripted::new(ProviderId::CODEX, vec![ok("codex")]);
        let router = gated_router(
            vec![codex.clone()],
            Arc::new(Recorder::default()),
            &gate(crate::UsageLimit::Percent(80), &rows),
        );
        router
            .execute(&request("p2", "y"), &CancelToken::new())
            .unwrap();
        assert_eq!(codex.calls(), 1);
    }

    /// B2: the limit is read at each request, so raising it on the shared cell
    /// takes effect on the next request of the router already running.
    #[test]
    fn a_changed_limit_applies_to_the_next_request_of_a_running_router() {
        let sink = Arc::new(Recorder::default());
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude"), ok("claude")]);
        let codex = Scripted::new(ProviderId::CODEX, vec![ok("codex"), ok("codex")]);
        let rows: std::collections::BTreeMap<_, _> = [(ProviderId::CLAUDE, week(86.0))].into();
        let limits = Arc::new(UsageLimits::new());
        limits.set(crate::UsageLimit::Percent(80), &rows);
        let router = gated_router(vec![claude.clone(), codex.clone()], sink, &limits);
        let first = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        assert_eq!(first.provider, ProviderId::CODEX);
        limits.set(crate::UsageLimit::Percent(95), &rows);
        let second = router
            .execute(&request("p2", "y"), &CancelToken::new())
            .unwrap();
        assert_eq!(second.provider, ProviderId::CLAUDE);
    }

    /// D-03: work an operator waits on is asked of a router with no gate, and
    /// a request that names its own agent is not judged by the limit either.
    #[test]
    fn a_router_without_a_gate_and_a_picked_agent_ignore_the_usage_limit() {
        let rows = [(ProviderId::CLAUDE, week(99.0))];
        let waited_on = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        make_router(vec![waited_on.clone()], Arc::new(Recorder::default()))
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        assert_eq!(waited_on.calls(), 1);

        let picked = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let router = gated_router(
            vec![picked.clone()],
            Arc::new(Recorder::default()),
            &gate(crate::UsageLimit::Percent(80), &rows),
        );
        let request = AiRequest {
            pick: Some(crate::AiPick {
                provider: ProviderId::CLAUDE,
                model: None,
                effort: None,
            }),
            ..request("p2", "y")
        };
        router.execute(&request, &CancelToken::new()).unwrap();
        assert_eq!(picked.calls(), 1);
    }

    /// A sign-in problem elsewhere in the order is what the caller can act
    /// on, so it is named instead of a wait for a reset.
    #[test]
    fn a_signed_out_agent_beside_one_past_the_limit_is_reported_as_no_provider() {
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let codex = Scripted::with_availability(ProviderId::CODEX, Availability::NeedsLogin);
        let router = gated_router(
            vec![claude.clone(), codex],
            Arc::new(Recorder::default()),
            &gate(
                crate::UsageLimit::Percent(80),
                &[(ProviderId::CLAUDE, week(90.0))],
            ),
        );
        assert!(matches!(
            router.execute(&request("p1", "x"), &CancelToken::new()),
            Err(AiError::NoProvider(_))
        ));
        assert_eq!(claude.calls(), 0);
    }

    /// The standing question: an agent past the limit is not degraded (it is
    /// signed in), but a request made now would not run on it.
    #[test]
    fn provider_state_names_the_agent_a_request_would_run_on_under_the_limit() {
        let claude = Scripted::new(ProviderId::CLAUDE, vec![]);
        let codex = Scripted::new(ProviderId::CODEX, vec![]);
        let router = gated_router(
            vec![claude, codex],
            Arc::new(Recorder::default()),
            &gate(
                crate::UsageLimit::Percent(80),
                &[(ProviderId::CLAUDE, week(86.0))],
            ),
        );
        let state = router.provider_state().unwrap();
        assert_eq!(state.selected, ProviderId::CLAUDE);
        assert_eq!(state.active, Some(ProviderId::CODEX));
        assert!(state.degraded.is_none());
    }

    #[test]
    fn an_unsupported_provider_is_reported_and_never_selected() {
        let sink = Arc::new(Recorder::default());
        let claude = Scripted::with_availability(
            ProviderId::CLAUDE,
            Availability::Unsupported {
                reason: "gap".to_owned(),
            },
        );
        let codex = Scripted::with_availability(ProviderId::CODEX, Availability::NeedsLogin);
        let router = make_router(vec![codex.clone(), claude.clone()], sink);
        let error = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap_err();
        match error {
            AiError::NoProvider(states) => {
                assert_eq!(states[0], (ProviderId::CODEX, Availability::NeedsLogin));
                assert_eq!(states[1].0, ProviderId::CLAUDE);
                assert_eq!(states[1].1.class(), "unsupported");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(claude.calls(), 0);
    }

    #[test]
    fn a_usage_limit_parks_the_provider_account_wide_until_it_resets() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(
            ProviderId::CODEX,
            vec![
                Err(AiError::UsageLimited {
                    retry_after: Some(Duration::from_millis(200)),
                }),
                ok("after"),
            ],
        );
        let clock = ManualClock::new();
        let router = make_router(vec![codex.clone()], sink).with_clock(clock.source());
        let first = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap_err();
        assert!(matches!(first, AiError::UsageLimited { .. }));
        // Another feature and subject on the same account is parked too,
        // without spending a call.
        let mut other = request("p2", "y");
        other.feature_id = "other_feature".into();
        let second = router.execute(&other, &CancelToken::new()).unwrap_err();
        assert!(matches!(
            second,
            AiError::UsageLimited {
                retry_after: Some(_)
            }
        ));
        assert_eq!(codex.calls(), 1);
        // The park lasts the reset window the provider named, and no longer.
        clock.advance(Duration::from_millis(199));
        assert!(matches!(
            router.execute(&other, &CancelToken::new()),
            Err(AiError::UsageLimited { .. })
        ));
        assert_eq!(codex.calls(), 1);
        clock.advance(Duration::from_millis(1));
        assert_eq!(
            router
                .execute(&request("p3", "z"), &CancelToken::new())
                .unwrap()
                .value["summary"],
            "after"
        );
    }

    #[test]
    fn not_authenticated_is_not_retried_until_availability_changes() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(
            ProviderId::CODEX,
            vec![Err(AiError::NotAuthenticated), ok("logged in")],
        );
        let router = make_router(vec![codex.clone()], sink);
        assert_eq!(
            router.execute(&request("p1", "x"), &CancelToken::new()),
            Err(AiError::NotAuthenticated)
        );
        let parked = router
            .execute(&request("p2", "y"), &CancelToken::new())
            .unwrap_err();
        assert!(matches!(parked, AiError::NoProvider(_)), "{parked:?}");
        assert_eq!(codex.calls(), 1);
        router.invalidate_availability();
        assert_eq!(
            router
                .execute(&request("p3", "z"), &CancelToken::new())
                .unwrap()
                .value["summary"],
            "logged in"
        );
    }

    /// Every remaining provider checks sign-in, so a login failure can be
    /// re-read once its ordinary availability window passes.
    #[test]
    fn a_sign_in_failure_is_rechecked_after_the_availability_window() {
        for provider in crate::PROVIDERS {
            let agent = Scripted::new(
                *provider,
                vec![Err(AiError::NotAuthenticated), ok("signed in")],
            );
            let clock = ManualClock::new();
            let router = make_router(vec![agent.clone()], Arc::new(Recorder::default()))
                .with_clock(clock.source());
            assert_eq!(
                router.execute(&request("p1", "x"), &CancelToken::new()),
                Err(AiError::NotAuthenticated)
            );
            clock.advance(RouterConfig::default().availability_ttl + Duration::from_secs(1));
            assert_eq!(
                router
                    .execute(&request("p2", "y"), &CancelToken::new())
                    .unwrap()
                    .value["summary"],
                "signed in"
            );
            assert_eq!(agent.calls(), 2, "{provider}");
        }
    }

    /// The screen shows a code only; the cause of a list that could not be
    /// read is in the log, once per change rather than on every read the open
    /// tab repeats (B68).
    #[test]
    fn a_model_list_that_could_not_be_read_is_logged_once_with_its_reason() {
        let sink = Arc::new(Recorder::default());
        let grok = Scripted::new(ProviderId::GROK, vec![]);
        let router = make_router(vec![grok.clone()], sink.clone());

        router.models();
        assert!(sink.events("ai.models.unavailable").is_empty());

        grok.set_models_unreadable(Some("grok_models_unreadable"));
        router.models();
        router.models();
        let logged = sink.events("ai.models.unavailable");
        assert_eq!(logged.len(), 1, "a repeated failure is not logged again");
        assert_eq!(logged[0].provider, Some(ProviderId::GROK));
        assert_eq!(
            logged[0].detail.as_deref(),
            Some("reason=grok_models_unreadable")
        );

        grok.set_models_unreadable(Some("grok_not_authenticated"));
        router.models();
        assert_eq!(
            sink.events("ai.models.unavailable").len(),
            2,
            "a new cause is"
        );

        grok.set_models_unreadable(None);
        router.models();
        grok.set_models_unreadable(Some("grok_not_authenticated"));
        router.models();
        assert_eq!(
            sink.events("ai.models.unavailable").len(),
            3,
            "a failure after the list came back is a new one"
        );
    }

    #[test]
    fn concurrent_duplicates_share_one_provider_call() {
        let sink = Arc::new(Recorder::default());
        let held = Arc::new(Held::default());
        let router = Arc::new(make_router(vec![held.clone()], sink.clone()));
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let router = Arc::clone(&router);
                std::thread::spawn(move || {
                    router
                        .execute(&request("same", "same input"), &CancelToken::new())
                        .unwrap()
                })
            })
            .collect();
        // The one call stays running until both duplicates have joined it.
        sink.wait_for_events("ai.request.joined", 2);
        held.release.open();
        for handle in handles {
            assert_eq!(handle.join().unwrap().value["summary"], "shared");
        }
        assert_eq!(held.calls.load(Ordering::SeqCst), 1);
        // A different input for the same subject is a new intent.
        router
            .execute(&request("same", "other input"), &CancelToken::new())
            .unwrap();
        assert_eq!(held.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_output_language_reaches_the_provider_prompt_and_is_part_of_the_intent() {
        let sink = Arc::new(Recorder::default());
        let held = Arc::new(Held::default());
        let router = Arc::new(make_router(vec![held.clone()], sink.clone()));
        let asked = |language| AiRequest {
            output_language: language,
            answer: crate::AnswerMode::default(),
            ..request("same", "same input")
        };
        let korean = {
            let router = Arc::clone(&router);
            let request = asked(Some(crate::OutputLanguage::Korean));
            std::thread::spawn(move || router.execute(&request, &CancelToken::new()).unwrap())
        };
        held.started.wait();
        // The same input asked in English while the Korean one runs is
        // another intent: it is not handed the Korean answer, and the one
        // call the router allows at a time refuses it.
        assert!(matches!(
            router.execute(&asked(Some(crate::OutputLanguage::English)), &CancelToken::new()),
            Err(AiError::OverBudget { cap, .. }) if cap == "in_flight"
        ));
        assert!(sink.events("ai.request.joined").is_empty());
        held.release.open();
        korean.join().unwrap();
        for language in [Some(crate::OutputLanguage::English), None] {
            router
                .execute(&asked(language), &CancelToken::new())
                .unwrap();
        }
        let mut prompts = held.prompts.lock().unwrap().clone();
        prompts.sort();
        let rule = |name: &str| {
            format!(
                "sys\n\nWrite every text a person reads in {name}, whatever language the input is in; keep code, commands, paths, identifiers, names and quoted text exactly as they are."
            )
        };
        assert_eq!(
            prompts,
            vec!["sys".to_owned(), rule("English"), rule("Korean")]
        );
    }

    #[test]
    fn a_request_without_its_output_language_is_refused_on_the_wire() {
        let mut wire = serde_json::to_value(AiRequest {
            output_language: Some(crate::OutputLanguage::SimplifiedChinese),
            answer: crate::AnswerMode::default(),
            ..request("s", "x")
        })
        .unwrap();
        assert_eq!(wire["output_language"], "zh-CN");
        wire.as_object_mut().unwrap().remove("output_language");
        assert!(serde_json::from_value::<AiRequest>(wire).is_err());
    }

    /// The availability cache is deliberately switched off here, so what
    /// keeps a parked provider from being asked is the cooldown itself and
    /// not a cache window that happens to be open.
    fn make_sticky_router(backends: Vec<Arc<dyn AiBackend>>, sink: Arc<Recorder>) -> AiRouter {
        let config = RouterConfig {
            priority: backends.iter().map(|backend| backend.id()).collect(),
            availability_ttl: Duration::ZERO,
            backoff_base: Duration::from_millis(1),
            ..RouterConfig::default()
        };
        AiRouter::with_sleep(backends, config, sink, Box::new(|_| {}))
    }

    /// A consumer has to be able to ask the standing question rather than
    /// infer it from the last result it happened to see.
    #[test]
    fn provider_state_names_the_active_provider_and_why_the_selected_one_stepped_aside() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::with_availability(ProviderId::CODEX, Availability::Ready);
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let router = make_sticky_router(vec![codex.clone(), claude.clone()], sink);
        assert_eq!(
            router.provider_state().unwrap(),
            ProviderState {
                selected: ProviderId::CODEX,
                active: Some(ProviderId::CODEX),
                degraded: None,
            }
        );
        codex.set_availability(Availability::NeedsLogin);
        assert_eq!(
            router.provider_state().unwrap(),
            ProviderState {
                selected: ProviderId::CODEX,
                active: Some(ProviderId::CLAUDE),
                degraded: Some(Degraded {
                    provider: ProviderId::CODEX,
                    reason: Availability::NeedsLogin,
                    until: None,
                }),
            }
        );
        // A query answers; it does not run the request.
        assert_eq!(claude.calls(), 0);
    }

    /// A router with nothing registered has no selected provider, and says so
    /// instead of naming one.
    #[test]
    fn a_router_without_providers_has_no_provider_state() {
        let router = make_router(Vec::new(), Arc::new(Recorder::default()));
        assert_eq!(router.provider_state(), None);
    }

    /// Sticky means the reason outlives the request that found it: the parked
    /// provider is not asked for an answer, and is not even asked whether it
    /// is logged in, until its cooldown expires.
    #[test]
    fn a_usage_limit_is_sticky_and_stops_asking_the_parked_provider_anything() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(
            ProviderId::CODEX,
            vec![Err(AiError::UsageLimited {
                retry_after: Some(Duration::from_secs(600)),
            })],
        );
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let router = make_sticky_router(vec![codex.clone(), claude.clone()], sink.clone());
        assert_eq!(
            router
                .execute(&request("p1", "x"), &CancelToken::new())
                .unwrap()
                .provider,
            ProviderId::CLAUDE
        );
        let probes_after_park = codex.probes();
        for subject in ["p2", "p3", "p4"] {
            assert_eq!(
                router
                    .execute(&request(subject, subject), &CancelToken::new())
                    .unwrap()
                    .provider,
                ProviderId::CLAUDE
            );
        }
        // One request spent on the parked provider, and nothing since.
        assert_eq!(codex.calls(), 1);
        assert_eq!(codex.probes(), probes_after_park, "the park was re-probed");

        let state = router.provider_state().unwrap();
        assert_eq!(state.selected, ProviderId::CODEX);
        assert_eq!(state.active, Some(ProviderId::CLAUDE));
        let degraded = state.degraded.expect("the reason stays queryable");
        assert_eq!(degraded.provider, ProviderId::CODEX);
        assert!(degraded.until.expect("a cooldown ends") > Duration::from_secs(500));
        assert_eq!(degraded.reason.class(), "unavailable");
        assert_eq!(codex.probes(), probes_after_park);
    }

    /// When the reason ends the router asks the selected provider once and
    /// goes back to it; nobody has to tell it the limit reset.
    #[test]
    fn an_expired_cooldown_re_reads_availability_once_and_returns_to_the_selected_provider() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(
            ProviderId::CODEX,
            vec![Err(AiError::UsageLimited {
                retry_after: Some(Duration::from_millis(200)),
            })],
        );
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude")]);
        let clock = ManualClock::new();
        let router = make_sticky_router(vec![codex.clone(), claude.clone()], sink.clone())
            .with_clock(clock.source());
        router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        let parked_probes = codex.probes();
        clock.advance(Duration::from_millis(200));
        codex.queue(vec![ok("codex is back")]);

        let state = router.provider_state().unwrap();
        assert_eq!(state.active, Some(ProviderId::CODEX));
        assert_eq!(state.degraded, None);
        assert_eq!(
            codex.probes(),
            parked_probes + 1,
            "the return asks the provider exactly once"
        );
        assert_eq!(
            router
                .execute(&request("p2", "y"), &CancelToken::new())
                .unwrap()
                .value["summary"],
            "codex is back"
        );
    }

    /// Both providers down is reachable now that both can answer, so it is a
    /// reported state: no provider is claimed to be active, and the caller is
    /// told the wait it can act on rather than that nothing is installed.
    #[test]
    fn every_provider_degraded_is_reported_and_claims_no_active_provider() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::with_availability(ProviderId::CODEX, Availability::NeedsLogin);
        let claude = Scripted::new(
            ProviderId::CLAUDE,
            vec![Err(AiError::UsageLimited {
                retry_after: Some(Duration::from_secs(600)),
            })],
        );
        let router = make_sticky_router(vec![codex.clone(), claude.clone()], sink.clone());
        assert!(matches!(
            router.execute(&request("p1", "x"), &CancelToken::new()),
            Err(AiError::UsageLimited { .. })
        ));

        let state = router.provider_state().unwrap();
        assert_eq!(state.selected, ProviderId::CODEX);
        assert_eq!(state.active, None, "no provider can answer");
        assert_eq!(
            state.degraded,
            Some(Degraded {
                provider: ProviderId::CODEX,
                reason: Availability::NeedsLogin,
                until: None,
            })
        );
        // Every provider's own reason is still readable next to it.
        let availability = router.availability();
        assert_eq!(availability[0].1, Availability::NeedsLogin);
        match &availability[1].1 {
            Availability::Unavailable { reason } => {
                assert!(reason.starts_with("usage_limited"), "{reason}");
            }
            other => panic!("{other:?}"),
        }

        // The parked wait is the answer, not "no provider": a caller decides
        // its own retry on it.
        match router.execute(&request("p2", "y"), &CancelToken::new()) {
            Err(AiError::UsageLimited { retry_after }) => {
                assert!(retry_after.expect("the wait is named") > Duration::from_secs(500));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(claude.calls(), 1, "the parked provider was asked again");
    }

    /// `ai.fallback` is per-request and a sticky failover has no second
    /// provider to try, so entering and leaving get their own events.
    #[test]
    fn entering_and_leaving_failover_each_leave_their_own_event() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(
            ProviderId::CODEX,
            vec![Err(AiError::ProviderUnavailable("gone".to_owned()))],
        );
        let claude = Scripted::new(ProviderId::CLAUDE, vec![ok("claude"), ok("claude again")]);
        // The availability cache is the mechanism here: an outage discovered
        // during a request is what keeps the next one off that provider.
        let router = make_router(vec![codex.clone(), claude.clone()], sink.clone());

        // The first request discovers the outage mid-flight, so it is the
        // per-request fallback line that records it.
        router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        assert_eq!(sink.events("ai.fallback").len(), 1);
        assert!(sink.events("ai.provider.degraded").is_empty());

        // The next request never tries codex, so without this event the log
        // would show the provider change with no reason.
        router
            .execute(&request("p2", "y"), &CancelToken::new())
            .unwrap();
        let degraded = sink.events("ai.provider.degraded");
        assert_eq!(degraded.len(), 1);
        assert_eq!(degraded[0].provider, Some(ProviderId::CLAUDE));
        let detail = degraded[0].detail.clone().expect("a reason");
        assert!(detail.contains("selected=codex"), "{detail}");
        assert!(detail.contains("active=claude"), "{detail}");
        assert!(detail.contains("gone"), "{detail}");
        assert_eq!(sink.events("ai.fallback").len(), 1, "no second fallback");

        // Announced once, not once per request.
        router
            .execute(&request("p3", "z"), &CancelToken::new())
            .unwrap();
        assert_eq!(sink.events("ai.provider.degraded").len(), 1);
        assert!(sink.events("ai.provider.recovered").is_empty());

        codex.queue(vec![ok("codex is back")]);
        router.invalidate_availability();
        assert_eq!(
            router
                .execute(&request("p4", "w"), &CancelToken::new())
                .unwrap()
                .value["summary"],
            "codex is back"
        );
        let recovered = sink.events("ai.provider.recovered");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].provider, Some(ProviderId::CODEX));
        assert_eq!(recovered[0].outcome_class.as_deref(), Some("ready"));
    }

    #[test]
    fn log_events_carry_identifiers_and_classes_but_no_content() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(ProviderId::CODEX, vec![ok("secret summary text")]);
        let router = make_router(vec![codex], sink.clone());
        let mut req = request("p1", "the user's private transcript");
        req.system = "confidential prompt".to_owned();
        router.execute(&req, &CancelToken::new()).unwrap();
        let all = sink.0.lock().unwrap();
        assert!(!all.is_empty());
        for event in all.iter() {
            let rendered = format!("{event:?}");
            assert!(!rendered.contains("private transcript"), "{rendered}");
            assert!(!rendered.contains("confidential"), "{rendered}");
            assert!(!rendered.contains("secret summary"), "{rendered}");
            assert_eq!(event.feature_id.as_deref(), Some("test_feature"));
            assert_eq!(event.schema_version.as_deref(), Some("test.v1"));
            assert_eq!(event.input_chars, Some(req.input.chars().count()));
        }
    }

    /// A backend that reports a fixed process measurement and counts the
    /// restarts the router asks of it, for the process-budget tests.
    struct Measuring {
        descendants: Mutex<usize>,
        restarts: AtomicUsize,
    }

    impl Measuring {
        fn new(descendants: usize) -> Arc<Self> {
            Arc::new(Self {
                descendants: Mutex::new(descendants),
                restarts: AtomicUsize::new(0),
            })
        }
    }

    impl AiBackend for Measuring {
        fn id(&self) -> ProviderId {
            ProviderId::CODEX
        }
        fn availability(&self) -> Availability {
            Availability::Ready
        }
        fn models(&self) -> ModelCatalog {
            ModelCatalog::Offered(Vec::new())
        }
        fn execute(&self, _: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
            Ok(AiResponse {
                value: json!({"summary": "ok"}),
                usage: crate::AiUsage::default(),
            })
        }
        fn last_measurement(&self) -> ProcessMeasurement {
            ProcessMeasurement::Available {
                app_server_pid: 42,
                descendants: *self.descendants.lock().unwrap(),
                rss_bytes: 0,
            }
        }
        fn restart(&self) {
            self.restarts.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// A second concurrent intent is refused by the concurrency cap without
    /// touching a provider, and the first request is untouched.
    #[test]
    fn a_second_concurrent_request_is_over_budget_in_flight() {
        let sink = Arc::new(Recorder::default());
        let held = Arc::new(Held::default());
        let router = Arc::new(make_router(vec![held.clone()], sink.clone()));
        let leader = {
            let router = Arc::clone(&router);
            std::thread::spawn(move || router.execute(&request("a", "x"), &CancelToken::new()))
        };
        held.started.wait();
        let rejected = router
            .execute(&request("b", "y"), &CancelToken::new())
            .unwrap_err();
        match rejected {
            AiError::OverBudget { cap, .. } => assert_eq!(cap, "in_flight"),
            other => panic!("{other:?}"),
        }
        held.release.open();
        assert_eq!(leader.join().unwrap().unwrap().value["summary"], "shared");
        assert_eq!(
            held.calls.load(Ordering::SeqCst),
            1,
            "the rejected request never reached a provider"
        );
        assert_eq!(sink.events("ai.budget.exceeded").len(), 1);
    }

    /// The per-minute cap refuses the request that crosses it, and only that
    /// one; the ones under the cap all run.
    #[test]
    fn requests_past_the_per_minute_cap_are_over_budget() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(ProviderId::CODEX, vec![ok("a"), ok("b")]);
        let config = RouterConfig {
            priority: vec![ProviderId::CODEX],
            max_per_minute: 2,
            backoff_base: Duration::from_millis(1),
            ..RouterConfig::default()
        };
        let router = AiRouter::with_sleep(vec![codex.clone()], config, sink, Box::new(|_| {}));
        assert!(
            router
                .execute(&request("a", "1"), &CancelToken::new())
                .is_ok()
        );
        assert!(
            router
                .execute(&request("b", "2"), &CancelToken::new())
                .is_ok()
        );
        match router
            .execute(&request("c", "3"), &CancelToken::new())
            .unwrap_err()
        {
            AiError::OverBudget { cap, .. } => assert_eq!(cap, "per_minute"),
            other => panic!("{other:?}"),
        }
        assert_eq!(codex.calls(), 2);
    }

    /// Crossing the descendant cap restarts the app-server and fails the
    /// request; three consecutive restarts escalate to the restart cap, and a
    /// request that completes under the cap clears the streak.
    #[test]
    fn the_process_cap_restarts_then_caps_consecutive_restarts() {
        let sink = Arc::new(Recorder::default());
        let over = Measuring::new(RouterConfig::default().max_app_server_descendants + 1);
        let router = make_router(vec![over.clone()], sink);
        // First two crossings are over-budget restarts.
        for _ in 0..2 {
            match router
                .execute(
                    &request("s", &format!("{}", rand_input())),
                    &CancelToken::new(),
                )
                .unwrap_err()
            {
                AiError::OverBudget { cap, .. } => assert_eq!(cap, "app_server_descendants"),
                other => panic!("{other:?}"),
            }
        }
        // The third consecutive restart hits the cap.
        match router
            .execute(
                &request("s", &format!("{}", rand_input())),
                &CancelToken::new(),
            )
            .unwrap_err()
        {
            AiError::ProviderUnavailable(reason) => assert_eq!(reason, "app_server_restart_cap"),
            other => panic!("{other:?}"),
        }
        assert_eq!(over.restarts.load(Ordering::SeqCst), 3);

        // Under the cap now: the request succeeds and the streak resets, so a
        // later crossing is an over-budget restart again rather than the cap.
        *over.descendants.lock().unwrap() = 0;
        assert!(
            router
                .execute(&request("s", "clear"), &CancelToken::new())
                .is_ok()
        );
        *over.descendants.lock().unwrap() = RouterConfig::default().max_app_server_descendants + 1;
        match router
            .execute(&request("s", "again"), &CancelToken::new())
            .unwrap_err()
        {
            AiError::OverBudget { cap, .. } => assert_eq!(cap, "app_server_descendants"),
            other => panic!("{other:?}"),
        }
    }

    /// The provider the operator did not list is registered, so Settings can
    /// still show it, and it is never asked a request (D-16, B40).
    #[test]
    fn a_provider_outside_the_priority_is_inspected_but_never_asked_a_request() {
        let sink = Arc::new(Recorder::default());
        let chosen = Scripted::new(
            ProviderId::CLAUDE,
            vec![Err(AiError::ProviderUnavailable("down".to_owned()))],
        );
        let unlisted = Scripted::new(ProviderId::CODEX, vec![ok("never asked")]);
        let config = RouterConfig {
            priority: vec![ProviderId::CLAUDE],
            backoff_base: Duration::from_millis(1),
            ..RouterConfig::default()
        };
        let router = AiRouter::with_sleep(
            vec![chosen.clone(), unlisted.clone()],
            config,
            sink,
            Box::new(|_| {}),
        );
        let error = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap_err();
        assert_eq!(error, AiError::ProviderUnavailable("down".to_owned()));
        assert_eq!(
            unlisted.calls(),
            0,
            "an installed, signed-in agent outside the list is not used"
        );
        let inspected: Vec<_> = router
            .availability()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            inspected,
            vec![ProviderId::CLAUDE, ProviderId::CODEX],
            "Settings still sees every registered provider"
        );
        assert_eq!(router.models().len(), 2);
        assert_eq!(
            router.provider_state().unwrap().selected,
            ProviderId::CLAUDE
        );
    }

    #[test]
    fn the_fallback_list_is_tried_in_its_order_once_each() {
        let sink = Arc::new(Recorder::default());
        let first = Scripted::new(ProviderId::CLAUDE, vec![Err(AiError::NotAuthenticated)]);
        let second = Scripted::new(
            ProviderId::PI,
            vec![Err(AiError::UsageLimited { retry_after: None })],
        );
        let third = Scripted::new(ProviderId::CODEX, vec![ok("third")]);
        let config = RouterConfig {
            // The chosen agent listed again behind the others is still one hop.
            priority: vec![
                ProviderId::CLAUDE,
                ProviderId::PI,
                ProviderId::CODEX,
                ProviderId::CLAUDE,
            ],
            backoff_base: Duration::from_millis(1),
            ..RouterConfig::default()
        };
        let router = AiRouter::with_sleep(
            vec![first.clone(), second.clone(), third.clone()],
            config,
            sink,
            Box::new(|_| {}),
        );
        let result = router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        assert_eq!(result.provider, ProviderId::CODEX);
        assert_eq!((first.calls(), second.calls(), third.calls()), (1, 1, 1));
        assert_eq!(router.last_answered(), Some(ProviderId::CODEX));
    }

    /// B33: Use Hide AI off means no model is asked anything, not even for
    /// availability.
    #[test]
    fn a_disabled_router_asks_no_provider_anything() {
        let sink = Arc::new(Recorder::default());
        let codex = Scripted::new(ProviderId::CODEX, vec![ok("a")]);
        let config = RouterConfig {
            priority: vec![ProviderId::CODEX],
            enabled: false,
            ..RouterConfig::default()
        };
        let router = AiRouter::with_sleep(vec![codex.clone()], config, sink, Box::new(|_| {}));
        assert_eq!(
            router
                .execute(&request("p1", "x"), &CancelToken::new())
                .unwrap_err(),
            AiError::Disabled
        );
        assert_eq!((codex.calls(), codex.probes()), (0, 0));
    }

    #[test]
    fn a_provider_that_cannot_be_chosen_says_why_and_a_parked_one_still_can() {
        let sink = Arc::new(Recorder::default());
        let ready = Scripted::new(ProviderId::CLAUDE, vec![]);
        let parked = Scripted::new(
            ProviderId::CODEX,
            vec![Err(AiError::UsageLimited {
                retry_after: Some(Duration::from_secs(3600)),
            })],
        );
        let unsupported = Scripted::with_availability(
            ProviderId::OPENCODE,
            Availability::Unsupported {
                reason: "cannot_guarantee_read_only".to_owned(),
            },
        );
        let signed_out = Scripted::with_availability(ProviderId::GROK, Availability::NeedsLogin);
        let router = AiRouter::with_sleep(
            vec![parked.clone(), ready, unsupported, signed_out],
            RouterConfig {
                priority: vec![ProviderId::CODEX, ProviderId::CLAUDE],
                backoff_base: Duration::from_millis(1),
                ..RouterConfig::default()
            },
            sink,
            Box::new(|_| {}),
        );
        router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        let statuses = router.statuses();
        let by = |id| {
            statuses
                .iter()
                .find(|status| status.provider == id)
                .unwrap()
        };
        assert!(by(ProviderId::CLAUDE).selectable);
        assert!(
            by(ProviderId::CODEX).selectable,
            "out of usage is not a reason to hide it"
        );
        assert!(by(ProviderId::CODEX).parked_for.is_some());
        assert!(!by(ProviderId::OPENCODE).selectable);
        assert_eq!(
            by(ProviderId::OPENCODE).availability,
            Availability::Unsupported {
                reason: "cannot_guarantee_read_only".to_owned()
            }
        );
        assert!(!by(ProviderId::GROK).selectable);
        assert_eq!(router.last_answered(), Some(ProviderId::CLAUDE));
        let state = router.provider_state().unwrap();
        assert_eq!(state.selected, ProviderId::CODEX);
        assert_eq!(state.active, Some(ProviderId::CLAUDE));
        assert!(state.degraded.unwrap().until.is_some());
    }

    /// A provider that takes the time it takes and then refuses, on the
    /// router's own clock.
    struct Spending {
        id: ProviderId,
        clock: ManualClock,
        spend: Duration,
        refusal: AiError,
        deadlines: Mutex<Vec<Duration>>,
    }

    impl Spending {
        fn new(id: ProviderId, clock: &ManualClock, spend: Duration) -> Arc<Self> {
            Arc::new(Self {
                id,
                clock: clock.clone(),
                spend,
                refusal: AiError::ProviderUnavailable(format!("{id}_down")),
                deadlines: Mutex::new(Vec::new()),
            })
        }

        fn deadlines(&self) -> Vec<Duration> {
            self.deadlines.lock().unwrap().clone()
        }
    }

    impl AiBackend for Spending {
        fn id(&self) -> ProviderId {
            self.id
        }
        fn availability(&self) -> Availability {
            Availability::Ready
        }
        fn models(&self) -> ModelCatalog {
            ModelCatalog::Offered(Vec::new())
        }
        fn execute(&self, request: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
            self.deadlines.lock().unwrap().push(request.deadline);
            self.clock.advance(self.spend);
            Err(self.refusal.clone())
        }
    }

    /// B43: three providers that each refuse slowly cannot hold one request
    /// past its overall deadline; the last one is never asked, and the request
    /// ends with the refusal that was seen.
    #[test]
    fn a_request_moved_through_slow_refusals_ends_inside_its_overall_deadline() {
        let clock = ManualClock::new();
        let first = Spending::new(ProviderId::CLAUDE, &clock, Duration::from_secs(45));
        let second = Spending::new(ProviderId::CODEX, &clock, Duration::from_secs(20));
        let third = Spending::new(ProviderId::PI, &clock, Duration::from_secs(1));
        let router = make_router(
            vec![first.clone(), second.clone(), third.clone()],
            Arc::new(Recorder::default()),
        )
        .with_clock(clock.source());
        // 30 s per attempt, so 60 s overall.
        let mut ask = request("p1", "x");
        ask.deadline = Duration::from_secs(30);
        let error = router.execute(&ask, &CancelToken::new()).unwrap_err();
        assert_eq!(error, AiError::ProviderUnavailable("codex_down".to_owned()));
        assert_eq!(first.deadlines(), vec![Duration::from_secs(30)]);
        assert_eq!(
            second.deadlines(),
            vec![Duration::from_secs(15)],
            "the second provider gets what remains of the 60 seconds"
        );
        assert!(third.deadlines().is_empty(), "nothing is left for a third");
    }

    #[test]
    fn the_overall_deadline_is_twice_the_request_capped_and_never_below_the_request() {
        let router = make_router(Vec::new(), Arc::new(Recorder::default()));
        let with = |seconds| {
            let mut ask = request("p", "x");
            ask.deadline = Duration::from_secs(seconds);
            router.overall_deadline(&ask)
        };
        assert_eq!(with(30), Duration::from_secs(60));
        assert_eq!(with(60), Duration::from_secs(90));
        assert_eq!(
            with(120),
            Duration::from_secs(120),
            "a request that asked for more than the cap keeps its own deadline"
        );
    }

    /// Remembers every attempt the router told it, as (feature, provider,
    /// models told).
    #[derive(Debug, Default)]
    struct Told(Mutex<Vec<(String, ProviderId, Vec<crate::ModelTokens>)>>);

    impl AiUsageSink for Told {
        fn attempt(&self, feature_id: &str, provider: ProviderId, models: &[crate::ModelTokens]) {
            self.0
                .lock()
                .unwrap()
                .push((feature_id.to_owned(), provider, models.to_vec()));
        }
    }

    impl Told {
        fn all(&self) -> Vec<(String, ProviderId, Vec<crate::ModelTokens>)> {
            self.0.lock().unwrap().clone()
        }
    }

    /// A provider that answers from a queue of whole responses, usage included.
    struct Metered {
        id: ProviderId,
        outcomes: Mutex<VecDeque<Result<AiResponse, AiError>>>,
    }

    impl Metered {
        fn new(id: ProviderId, outcomes: Vec<Result<AiResponse, AiError>>) -> Arc<Self> {
            Arc::new(Self {
                id,
                outcomes: Mutex::new(outcomes.into()),
            })
        }
    }

    impl AiBackend for Metered {
        fn id(&self) -> ProviderId {
            self.id
        }
        fn availability(&self) -> Availability {
            Availability::Ready
        }
        fn models(&self) -> ModelCatalog {
            ModelCatalog::Offered(Vec::new())
        }
        fn execute(&self, _: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
            self.outcomes
                .lock()
                .unwrap()
                .pop_front()
                .expect("a scripted outcome")
        }
    }

    fn tokens(model: &str, input: u64) -> crate::ModelTokens {
        crate::ModelTokens {
            model: model.to_owned(),
            input,
            output: 1,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: 0,
            reasoning: None,
            lower_bound: false,
        }
    }

    fn answered(summary: Value, models: Vec<crate::ModelTokens>) -> Result<AiResponse, AiError> {
        Ok(AiResponse {
            value: summary,
            usage: crate::AiUsage {
                models,
                ..crate::AiUsage::default()
            },
        })
    }

    fn counting_router(backends: Vec<Arc<dyn AiBackend>>, told: &Arc<Told>) -> AiRouter {
        let config = RouterConfig {
            priority: backends.iter().map(|backend| backend.id()).collect(),
            backoff_base: Duration::from_millis(1),
            usage_sink: Some(Arc::clone(told) as Arc<dyn AiUsageSink>),
            ..RouterConfig::default()
        };
        AiRouter::with_sleep(
            backends,
            config,
            Arc::new(Recorder::default()),
            Box::new(|_| {}),
        )
    }

    #[test]
    fn every_answer_is_told_with_its_usage_and_a_rejected_answer_is_told_each_time() {
        let told = Arc::new(Told::default());
        let claude = Metered::new(
            ProviderId::CLAUDE,
            vec![
                answered(json!({"other": 1}), vec![tokens("m", 10)]),
                answered(json!({"summary": 5}), vec![tokens("m", 20)]),
            ],
        );
        let router = counting_router(vec![claude], &told);
        assert!(matches!(
            router.execute(&request("p1", "x"), &CancelToken::new()),
            Err(AiError::InvalidOutput(_))
        ));
        let attempts: Vec<_> = told
            .all()
            .into_iter()
            .map(|a| (a.2[0].input, a.0))
            .collect();
        assert_eq!(
            attempts,
            vec![
                (10, "test_feature".to_owned()),
                (20, "test_feature".to_owned())
            ]
        );
    }

    #[test]
    fn a_retry_and_a_fallback_are_each_their_own_attempt() {
        let told = Arc::new(Told::default());
        let claude = Metered::new(
            ProviderId::CLAUDE,
            vec![
                Err(AiError::Transient("busy".to_owned())),
                answered(json!({"summary": "ok"}), vec![tokens("m", 7)]),
            ],
        );
        let codex = Metered::new(
            ProviderId::CODEX,
            vec![Err(AiError::ProviderUnavailable("gone".to_owned()))],
        );
        let router = counting_router(vec![codex, claude], &told);
        router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        // The refusals never took the request; only the answer is counted.
        let all = told.all();
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].1, all[0].2[0].input), (ProviderId::CLAUDE, 7));
    }

    #[test]
    fn a_submitted_attempt_whose_end_is_unknown_is_an_unknown_call() {
        for failure in [
            AiError::Timeout,
            AiError::CompletionUnknown("lost".to_owned()),
        ] {
            let told = Arc::new(Told::default());
            let router = counting_router(
                vec![Metered::new(ProviderId::CLAUDE, vec![Err(failure.clone())])],
                &told,
            );
            let _ = router.execute(&request("p1", "x"), &CancelToken::new());
            let all = told.all();
            assert_eq!(all.len(), 1, "{failure:?}");
            assert!(all[0].2.is_empty(), "{failure:?}");
        }
    }

    #[test]
    fn a_provider_whose_usage_is_not_read_is_an_unknown_call_and_a_refusal_is_not_counted() {
        let told = Arc::new(Told::default());
        let router = counting_router(
            vec![Metered::new(
                ProviderId::GROK,
                vec![
                    answered(json!({"summary": "a"}), Vec::new()),
                    Err(AiError::NotAuthenticated),
                ],
            )],
            &told,
        );
        router
            .execute(&request("p1", "x"), &CancelToken::new())
            .unwrap();
        let _ = router.execute(&request("p2", "y"), &CancelToken::new());
        let all = told.all();
        assert_eq!(all.len(), 1, "{all:?}");
        assert!(all[0].2.is_empty());
    }

    #[test]
    fn a_request_the_provider_refused_itself_did_no_work_and_is_not_told() {
        let told = Arc::new(Told::default());
        let router = counting_router(
            vec![Metered::new(
                ProviderId::CLAUDE,
                vec![
                    // Retried like an invalid output, and neither is counted.
                    Err(AiError::Rejected("claude_api_400".to_owned())),
                    Err(AiError::Rejected("claude_api_400".to_owned())),
                ],
            )],
            &told,
        );
        assert!(matches!(
            router.execute(&request("p1", "x"), &CancelToken::new()),
            Err(AiError::Rejected(_))
        ));
        assert!(told.all().is_empty());
    }

    #[test]
    fn a_backend_failure_after_submission_is_an_unknown_call_for_each_attempt() {
        let told = Arc::new(Told::default());
        let router = counting_router(
            vec![Metered::new(
                ProviderId::CLAUDE,
                vec![
                    // The input is deterministic, so the router asks twice:
                    // two dispatches, two unknown calls.
                    Err(AiError::InvalidOutput("no_structured_output".to_owned())),
                    Err(AiError::InvalidOutput("no_structured_output".to_owned())),
                    Err(AiError::Cancelled),
                ],
            )],
            &told,
        );
        assert!(matches!(
            router.execute(&request("p1", "x"), &CancelToken::new()),
            Err(AiError::InvalidOutput(_))
        ));
        assert!(matches!(
            router.execute(&request("p2", "y"), &CancelToken::new()),
            Err(AiError::Cancelled)
        ));
        let all = told.all();
        assert_eq!(all.len(), 3, "{all:?}");
        assert!(all.iter().all(|attempt| attempt.2.is_empty()), "{all:?}");
    }

    #[test]
    fn a_cancel_before_any_backend_was_asked_does_no_work_and_is_not_told() {
        let told = Arc::new(Told::default());
        // No scripted outcome: asking the backend would panic.
        let router = counting_router(vec![Metered::new(ProviderId::CLAUDE, Vec::new())], &told);
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            router.execute(&request("p1", "x"), &cancel),
            Err(AiError::Cancelled)
        ));
        assert!(told.all().is_empty());
    }

    #[test]
    fn a_joined_caller_that_gives_up_adds_nothing_to_the_one_execution() {
        let told = Arc::new(Told::default());
        let sink = Arc::new(Recorder::default());
        let held = Arc::new(Held::default());
        let config = RouterConfig {
            priority: vec![held.id()],
            usage_sink: Some(Arc::clone(&told) as Arc<dyn AiUsageSink>),
            ..RouterConfig::default()
        };
        let router = Arc::new(AiRouter::with_sleep(
            vec![held.clone()],
            config,
            sink.clone(),
            Box::new(|_| {}),
        ));
        let leader = {
            let router = Arc::clone(&router);
            std::thread::spawn(move || {
                router.execute(&request("same", "same input"), &CancelToken::new())
            })
        };
        held.started.wait();
        let joiner_cancel = CancelToken::new();
        let joiner = {
            let router = Arc::clone(&router);
            let cancel = joiner_cancel.clone();
            std::thread::spawn(move || router.execute(&request("same", "same input"), &cancel))
        };
        sink.wait_for_events("ai.request.joined", 1);
        joiner_cancel.cancel();
        assert!(matches!(joiner.join().unwrap(), Err(AiError::Cancelled)));
        held.release.open();
        leader.join().unwrap().unwrap();
        assert_eq!(told.all().len(), 1);
    }

    #[test]
    fn callers_that_join_one_execution_are_told_once() {
        let told = Arc::new(Told::default());
        let sink = Arc::new(Recorder::default());
        let held = Arc::new(Held::default());
        let config = RouterConfig {
            priority: vec![held.id()],
            usage_sink: Some(Arc::clone(&told) as Arc<dyn AiUsageSink>),
            ..RouterConfig::default()
        };
        let router = Arc::new(AiRouter::with_sleep(
            vec![held.clone()],
            config,
            sink.clone(),
            Box::new(|_| {}),
        ));
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let router = Arc::clone(&router);
                std::thread::spawn(move || {
                    router
                        .execute(&request("same", "same input"), &CancelToken::new())
                        .unwrap()
                })
            })
            .collect();
        sink.wait_for_events("ai.request.joined", 2);
        held.release.open();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(told.all().len(), 1);
    }

    fn rand_input() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    }
}
