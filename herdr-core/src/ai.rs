//! What the background AI providers can do right now, read for the Settings
//! screen.
//!
//! Answering costs child processes: `codex app-server` is started and asked
//! for the account and its model list, and `claude auth status` is run. So
//! this reader is shaped like the project panel's readers rather than like a
//! projection: the session-sync coordinator drives it, `read_if_due` decides
//! whether anything runs, the work itself happens on
//! [`BackgroundRead`](crate::reader::BackgroundRead)'s worker thread, and the
//! runtime mutex is never held across any of it.
//!
//! Every provider backend here is a [`NodeBackend`]: the router stays in
//! the core and each backend call is answered by the core's own node, which
//! runs the provider processes with its logins (`hide_host::ai`).
//!
//! It also reads nothing at all while nobody is looking. `observing` is false
//! unless the Background AI group is on screen, and an unobserved request
//! answers from no provider, so an idle Hide starts no provider process.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hide_ai::{
    AiBackend, AiError, AiLogEvent, AiLogSink, AiRequest as ProviderRequest, AiResponse, AiRouter,
    AiSettings, AiUsageSink, Availability, CancelToken, ModelCatalog, ProcessMeasurement,
    ProviderId, ProviderStatus, UsageLimits,
};
use hide_node_link::ai::{BackendSpec, Logged};
use hide_node_link::protocol::Call;

use crate::node_access::{LinkError, NodeLink, call_as, call_as_with_progress};

use crate::model::{BackgroundAiProviderSnapshot, BackgroundAiSnapshot};
use crate::reader::BackgroundRead;

/// How stale a provider answer may be while the group is on screen.
///
/// A login completes outside Hide, so the screen has to notice on its own;
/// thirty seconds is soon enough for someone who has just logged in and rare
/// enough that the child processes are not a background load. It matches the
/// router's own availability window, so the two never disagree for long.
const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
/// The least time between one read finishing and a changed request starting
/// the next, so switching the model twice does not start two probes.
const SPACING: Duration = Duration::from_secs(2);
/// How often the agents that are switched on are asked whether they are
/// signed in while nobody has chosen one and the screen is not open (D-27).
/// A login takes minutes and the answer costs one status call per agent, so
/// the idle cadence is slow; opening Settings asks at once.
const SELECTING_INTERVAL: Duration = Duration::from_secs(300);

/// What to ask the providers, and the freshness key for asking again.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AiRequest {
    /// False unless the Background AI group is on screen. An unobserved
    /// request starts no provider process; see the module comment.
    pub observing: bool,
    /// The model each provider is configured with. Codex reports a model the
    /// account does not offer as an availability state, so a changed choice
    /// is a different question and is asked at once rather than waiting out
    /// the interval.
    pub models: BTreeMap<ProviderId, String>,
    /// The agents to ask whether they are signed in while the screen is not
    /// open: those switched on in this Mac's kit and installed, and only
    /// while nobody has chosen one and Hide AI is on, so the first signed-in
    /// agent can be chosen by itself (D-18, D-27, B45, B47). Empty otherwise.
    /// Nothing but availability is asked of them: no model list.
    pub selecting: BTreeSet<ProviderId>,
    /// The agents whose program this Mac's install kit found, `None` while
    /// the kit has not answered. The kit owns "is it installed": an agent Hide
    /// AI cannot use yet is listed as not installed from this, not from a
    /// program table of its own.
    pub cli_found: Option<BTreeSet<ProviderId>>,
}

impl AiRequest {
    /// Whether anything is asked at all.
    fn asks(&self) -> bool {
        self.observing || !self.selecting.is_empty()
    }

    /// The agents the router is built with: every registered one while the
    /// screen is open, only the ones being selected from otherwise.
    fn only(&self) -> Option<&BTreeSet<ProviderId>> {
        (!self.observing).then_some(&self.selecting)
    }
}

/// The router one screen reuses, and what it was built for. Rebuilding is
/// what a changed model costs, because a backend is constructed with one.
type CachedRouter = Arc<
    Mutex<
        Option<(
            BTreeMap<ProviderId, String>,
            Option<BTreeSet<ProviderId>>,
            Arc<AiRouter>,
        )>,
    >,
>;

pub struct AiReader {
    inner: BackgroundRead<AiRequest, BackgroundAiSnapshot>,
}

impl AiReader {
    /// A reader whose providers run on `node`, the core's own machine.
    pub fn new(node: Arc<dyn NodeLink>) -> Self {
        // One router outlives the reads that use it, so a Codex app-server
        // child is started once for a screen rather than once per read. It is
        // rebuilt only when the configured models change, because the model
        // is what the backend was constructed with.
        let cached: CachedRouter = Arc::new(Mutex::new(None));
        Self {
            inner: BackgroundRead::new(REFRESH_INTERVAL, SPACING, move |request: &AiRequest| {
                let mut guard = cached.lock().unwrap_or_else(|error| error.into_inner());
                if !request.asks() {
                    // Drop the router with the screen: its Codex child is a
                    // process, and nothing is reading its answers.
                    *guard = None;
                    return BackgroundAiSnapshot::unread();
                }
                let only = request.only().cloned();
                let router = match guard.as_ref() {
                    Some((models, built_for, router))
                        if *models == request.models && *built_for == only =>
                    {
                        Arc::clone(router)
                    }
                    _ => {
                        let router = Arc::new(AiRouter::new(
                            backends(&node, &request.models, only.as_ref()),
                            hide_ai::RouterConfig::default(),
                            Arc::new(DiagnosticLogSink),
                        ));
                        *guard = Some((request.models.clone(), only, Arc::clone(&router)));
                        router
                    }
                };
                drop(guard);
                read(
                    &router,
                    &request.models,
                    request.observing,
                    request.cli_found.as_ref(),
                )
            }),
        }
    }

    pub fn read_if_due(&mut self, request: AiRequest) -> Option<BackgroundAiSnapshot> {
        self.inner.set_interval(if request.observing {
            REFRESH_INTERVAL
        } else {
            SELECTING_INTERVAL
        });
        self.inner.poll(request)
    }
}

/// Writes the provider boundary's events to the core's diagnostic log, so a
/// fallback, an over-budget restart or a swept `CODEX_HOME` is visible from
/// outside the process. `emit` only queues, so logging from a provider thread
/// never waits on the file.
struct DiagnosticLogSink;

impl AiLogSink for DiagnosticLogSink {
    fn log(&self, event: AiLogEvent) {
        crate::diagnostic!(diagnostic_record(&event));
    }
}

/// The event as a diagnostic record: its name under the log's `kind` key,
/// `component` `ai`, the other fields as they serialize.
fn diagnostic_record(event: &AiLogEvent) -> serde_json::Value {
    let mut record = serde_json::to_value(event)
        .unwrap_or_else(|error| serde_json::json!({ "serialize_error": error.to_string() }));
    if let Some(fields) = record.as_object_mut() {
        fields.remove("event");
        fields.insert("kind".to_owned(), event.event.as_ref().into());
        fields.insert("component".to_owned(), "ai".into());
    }
    record
}

fn backends(
    node: &Arc<dyn NodeLink>,
    models: &BTreeMap<ProviderId, String>,
    only: Option<&BTreeSet<ProviderId>>,
) -> Vec<Arc<dyn AiBackend>> {
    hide_ai::PROVIDERS
        .iter()
        .filter(|provider| only.is_none_or(|only| only.contains(*provider)))
        .map(|provider| {
            let model = models
                .get(provider)
                .cloned()
                .unwrap_or_else(|| hide_ai::settings::default_model(*provider).to_owned());
            Arc::new(NodeBackend::new(Arc::clone(node), *provider, model)) as Arc<dyn AiBackend>
        })
        .collect()
}

/// The agents Hide AI as stored in `home` asks, in its order, with the
/// model each is asked for (PRD core-host-node-move D-04): what the machine
/// taking the core must answer for. `Err` says why Hide AI asks none.
pub fn hide_ai_asks(home: &std::path::Path) -> Result<Vec<(String, String)>, String> {
    asks_of(
        hide_ai::settings::load_choice(home)
            .map_err(|error| format!("the Hide AI settings could not be read: {error}"))?,
    )
}

/// [`hide_ai_asks`] of the settings [`stored_hide_ai_settings`] read on
/// another machine: on a move back, the core's, which come back with it and
/// which the machine taking the core back must answer for.
pub fn hide_ai_asks_of_stored(stored: &serde_json::Value) -> Result<Vec<(String, String)>, String> {
    asks_of(Some(serde_json::from_value(stored.clone()).map_err(
        |error| format!("the Hide AI settings could not be read: {error}"),
    )?))
}

fn asks_of(
    settings: Option<hide_ai::settings::AiSettings>,
) -> Result<Vec<(String, String)>, String> {
    let settings = settings
        .filter(|settings| settings.chosen)
        .ok_or("Hide AI is not set up: no agent is chosen")?;
    if !settings.enabled {
        return Err("Hide AI is off".to_owned());
    }
    let models = settings.models_by_provider();
    Ok(settings
        .router_config()
        .priority
        .into_iter()
        .map(|provider| {
            let model = models
                .get(&provider)
                .cloned()
                .unwrap_or_else(|| hide_ai::settings::default_model(provider).to_owned());
            (provider.as_str().to_owned(), model)
        })
        .collect())
}

/// Whether every agent in `asks` answers `Ready` on this machine with this
/// account's logins. `Err` names each that does not and why.
pub fn hide_ai_ready_here(asks: &[(String, String)]) -> Result<(), String> {
    let missing: Vec<String> = asks
        .iter()
        .filter_map(|(id, model)| {
            let Some(provider) = ProviderId::from_id(id) else {
                return Some(format!("{id}: unknown to this build"));
            };
            let backend = hide_ai::build_backend(provider, model, Arc::new(DiagnosticLogSink));
            match backend.availability() {
                Availability::Ready => None,
                other => Some(format!("{id}: {}", other.class())),
            }
        })
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing.join(", "))
    }
}

/// The Hide AI settings stored in `home`, read as this build reads them,
/// so two machines' files compare by what they choose rather than by their
/// bytes; `None` when none are stored.
pub fn stored_hide_ai_settings(
    home: &std::path::Path,
) -> Result<Option<serde_json::Value>, String> {
    hide_ai_settings_at(&hide_ai_settings_path(home))
}

/// Where Hide AI keeps its settings under `home`.
pub fn hide_ai_settings_path(home: &std::path::Path) -> std::path::PathBuf {
    hide_ai::settings::settings_path(home)
}

/// [`stored_hide_ai_settings`] of the settings file at `path`.
pub(crate) fn hide_ai_settings_at(
    path: &std::path::Path,
) -> Result<Option<serde_json::Value>, String> {
    hide_ai::settings::load_choice_at(path)
        .map_err(|error| format!("the Hide AI settings could not be read: {error}"))?
        .map(|settings| serde_json::to_value(settings).map_err(|error| error.to_string()))
        .transpose()
}

/// Numbers each backend the core builds; its node keeps one real backend
/// per number.
static NEXT_BACKEND: AtomicU64 = AtomicU64::new(1);
/// How long a node may take to say whether a provider can answer: the
/// Codex probe starts its app-server first.
const NODE_PROBE_TIMEOUT: Duration = Duration::from_secs(120);
/// What a request may take past its own deadline before the core stops
/// waiting for its node.
const NODE_EXECUTE_MARGIN: Duration = Duration::from_secs(30);
const NODE_RELEASE_TIMEOUT: Duration = Duration::from_secs(10);

/// A provider backend answered by a node. The router in the core picks,
/// retries and budgets; the node runs the provider. Dropping it releases the
/// node's backend, which ends its resident process.
struct NodeBackend {
    node: Arc<dyn NodeLink>,
    spec: BackendSpec,
}

impl NodeBackend {
    fn new(node: Arc<dyn NodeLink>, provider: ProviderId, model: String) -> Self {
        Self {
            node,
            spec: BackendSpec {
                instance: NEXT_BACKEND.fetch_add(1, Ordering::Relaxed),
                provider,
                model,
            },
        }
    }

    /// One call whose answer carries the backend's log events, which go to
    /// the diagnostic log here.
    fn logged<T: serde::de::DeserializeOwned>(&self, call: Call) -> Result<T, LinkError> {
        let answer: Logged<T> = call_as(self.node.as_ref(), call, NODE_PROBE_TIMEOUT)?;
        write_log(answer.log);
        Ok(answer.value)
    }
}

fn write_log(events: Vec<AiLogEvent>) {
    for event in events {
        DiagnosticLogSink.log(event);
    }
}

impl AiBackend for NodeBackend {
    fn id(&self) -> ProviderId {
        self.spec.provider
    }

    fn availability(&self) -> Availability {
        self.logged(Call::AiAvailability {
            backend: self.spec.clone(),
        })
        .unwrap_or_else(|error| Availability::Unavailable {
            reason: format!("The provider's machine could not be asked: {error}"),
        })
    }

    fn models(&self) -> ModelCatalog {
        self.logged(Call::AiModels {
            backend: self.spec.clone(),
        })
        .unwrap_or_else(|error| ModelCatalog::Unknown {
            reason: format!("The provider's machine could not be asked: {error}"),
        })
    }

    fn execute(
        &self,
        request: &ProviderRequest,
        cancel: &CancelToken,
    ) -> Result<AiResponse, AiError> {
        let call = Call::AiExecute {
            backend: self.spec.clone(),
            request: request.clone(),
        };
        let answer = call_as_with_progress::<Logged<Result<AiResponse, AiError>>, serde_json::Value>(
            self.node.as_ref(),
            call,
            request.deadline + NODE_EXECUTE_MARGIN,
            |_| !cancel.is_cancelled(),
        );
        match answer {
            Ok(answer) => {
                write_log(answer.log);
                answer.value
            }
            // Nothing reached the provider.
            Err(error @ (LinkError::NotConnected(_) | LinkError::Busy | LinkError::Refused(_))) => {
                Err(AiError::ProviderUnavailable(error.to_string()))
            }
            Err(LinkError::Unknown(reason)) => Err(AiError::CompletionUnknown(reason)),
        }
    }

    fn last_measurement(&self) -> ProcessMeasurement {
        call_as(
            self.node.as_ref(),
            Call::AiMeasurement {
                backend: self.spec.clone(),
            },
            NODE_RELEASE_TIMEOUT,
        )
        .unwrap_or(ProcessMeasurement::Unavailable)
    }

    fn restart(&self) {
        let restarted = call_as::<()>(
            self.node.as_ref(),
            Call::AiRestart {
                backend: self.spec.clone(),
            },
            NODE_RELEASE_TIMEOUT,
        );
        if let Err(error) = restarted {
            crate::diagnostic!(serde_json::json!({
                "component": "ai", "kind": "backend.restart_failed",
                "provider": self.spec.provider.as_str(), "error": error.to_string(),
            }));
        }
    }
}

impl Drop for NodeBackend {
    fn drop(&mut self) {
        let released = call_as::<()>(
            self.node.as_ref(),
            Call::AiRelease {
                instance: self.spec.instance,
            },
            NODE_RELEASE_TIMEOUT,
        );
        if let Err(error) = released {
            crate::diagnostic!(serde_json::json!({
                "component": "ai", "kind": "backend.release_failed",
                "provider": self.spec.provider.as_str(), "error": error.to_string(),
            }));
        }
    }
}

/// Builds the same provider boundary for Project Memory that the Settings
/// probe describes. The feature owns its prompt and parsing; this retains the
/// shared provider selection, process, retry, cancellation, and budget caps.
pub(crate) fn memory_router(
    node: &Arc<dyn NodeLink>,
    settings: &AiSettings,
    gate: &Arc<UsageLimits>,
    usage: &Arc<dyn AiUsageSink>,
) -> AiRouter {
    let mut config = memory_router_config(settings);
    config.usage_gate = Some(Arc::clone(gate));
    config.usage_sink = Some(Arc::clone(usage));
    AiRouter::new(
        backends(node, &settings.models_by_provider(), None),
        config,
        Arc::new(DiagnosticLogSink),
    )
}

/// The router one `hide memory add` asks, whose every try ends before the
/// caller stops waiting (PRD project-memory-v2 B8).
pub(crate) fn memory_add_router(
    node: &Arc<dyn NodeLink>,
    settings: &AiSettings,
    usage: &Arc<dyn AiUsageSink>,
) -> AiRouter {
    let mut config = memory_router_config(settings);
    config.overall_deadline_cap = crate::memory_command::ADD_PROVIDER_DEADLINE;
    config.usage_sink = Some(Arc::clone(usage));
    AiRouter::new(
        backends(node, &settings.models_by_provider(), None),
        config,
        Arc::new(DiagnosticLogSink),
    )
}

/// The Software Factory's own router (PRD software-factory D-13, D-44): the
/// operator's provider choice with one request in flight, separate from the
/// label and Project Memory routers so its queue never takes their budget.
/// Like the Start dialog's worktree name it is not judged by the usage limit
/// (PRD hide-ai-usage-limit D-03): an operator or a Factory is waiting on it.
pub(crate) fn factory_router(
    node: &Arc<dyn NodeLink>,
    settings: &AiSettings,
    usage: &Arc<dyn AiUsageSink>,
) -> AiRouter {
    waited_router(node, settings, usage)
}

/// A router for work the operator waits on, such as the Start dialog's
/// worktree name: the provider order with no usage limit, so it answers
/// however much of the week is spent (PRD hide-ai-usage-limit D-03, B6).
pub(crate) fn waited_router(
    node: &Arc<dyn NodeLink>,
    settings: &AiSettings,
    usage: &Arc<dyn AiUsageSink>,
) -> AiRouter {
    let mut config = settings.router_config();
    config.usage_sink = Some(Arc::clone(usage));
    AiRouter::new(
        backends(node, &settings.models_by_provider(), None),
        config,
        Arc::new(DiagnosticLogSink),
    )
}

/// The same provider boundary for agent labels (PRD labels-in-hided D-05):
/// the operator's provider and model choice, and the router's own retry,
/// cooldown and budget rules, judged by the usage limit.
pub(crate) fn labels_router(
    node: &Arc<dyn NodeLink>,
    settings: &AiSettings,
    gate: &Arc<UsageLimits>,
    usage: &Arc<dyn AiUsageSink>,
) -> AiRouter {
    let mut config = settings.router_config();
    config.usage_gate = Some(Arc::clone(gate));
    config.usage_sink = Some(Arc::clone(usage));
    AiRouter::new(
        backends(node, &settings.models_by_provider(), None),
        config,
        Arc::new(DiagnosticLogSink),
    )
}

fn memory_router_config(settings: &AiSettings) -> hide_ai::RouterConfig {
    settings.router_config()
}

fn read(
    router: &AiRouter,
    models: &BTreeMap<ProviderId, String>,
    with_catalogs: bool,
    cli_found: Option<&BTreeSet<ProviderId>>,
) -> BackgroundAiSnapshot {
    let statuses = router.statuses();
    let catalogs = if with_catalogs {
        router.models()
    } else {
        Vec::new()
    };
    let providers = hide_ai::PROVIDERS
        .iter()
        .map(|provider| {
            let status = statuses.iter().find(|status| status.provider == *provider);
            let catalog = catalogs
                .iter()
                .find(|(id, _)| id == provider)
                .map(|(_, catalog)| catalog);
            project(*provider, status, catalog, models, with_catalogs, cli_found)
        })
        .collect();
    BackgroundAiSnapshot {
        providers,
        ..BackgroundAiSnapshot::default()
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

/// One provider row. Every reason is a code the shell turns into words; the
/// short headline is the only English the core writes, kept for the rows
/// that still show it.
fn project(
    provider: ProviderId,
    status: Option<&ProviderStatus>,
    catalog: Option<&ModelCatalog>,
    models: &BTreeMap<ProviderId, String>,
    catalogs_asked: bool,
    cli_found: Option<&BTreeSet<ProviderId>>,
) -> BackgroundAiProviderSnapshot {
    let mut row = BackgroundAiProviderSnapshot::unread(provider);
    row.model = models
        .get(&provider)
        .cloned()
        .unwrap_or_else(|| provider.default_model().to_owned());
    if let Some(status) = status {
        let (state, headline, message) = match (&status.availability, status.parked_for) {
            (_, Some(_)) => ("usage_limited", "Out of usage", None),
            (Availability::Ready, None) => ("ready", "Signed in", None),
            (Availability::NeedsLogin, None) => ("needs_login", "Sign in required", None),
            (Availability::NotInstalled, None) => ("not_installed", "Not installed", None),
            (Availability::Unavailable { reason }, None) => {
                ("unavailable", "Cannot answer", Some(reason.clone()))
            }
            (Availability::Unsupported { reason }, None) => {
                ("unsupported", "Not supported", Some(reason.clone()))
            }
        };
        row.state = state.to_owned();
        row.headline = headline.to_owned();
        row.message = message;
        row.installed = !matches!(status.availability, Availability::NotInstalled);
        // An agent Hide AI cannot use yet has no backend that looks for its
        // program; the install kit says whether it is on this machine.
        if state == "unsupported" && cli_found.is_some_and(|found| !found.contains(&provider)) {
            row.state = "not_installed".to_owned();
            row.headline = "Not installed".to_owned();
            row.message = None;
            row.installed = false;
        }
        row.selectable = status.selectable;
        row.retry_at_ms = status
            .parked_for
            .map(|remaining| unix_now_ms().saturating_add(remaining.as_millis() as u64));
    }
    match catalog {
        Some(catalog) => {
            row.models = catalog.offered().to_vec();
            row.models_fixed = catalog.is_fixed();
            row.models_unavailable_reason = catalog.unknown_reason().map(str::to_owned);
        }
        None => {
            row.models_unavailable_reason = Some(
                if catalogs_asked {
                    "not_asked"
                } else {
                    "not_observed"
                }
                .to_owned(),
            );
        }
    }
    row
}

/// What the last label analysis found about the agent Hide AI runs on: it
/// could not answer, why, until when it said, and which listed agent a request
/// made now would run on (B41, B42). Written on the analyzer's thread after
/// each job and read by the runtime on its own thread, so it is one value
/// behind a mutex and never a reader of any provider.
#[derive(Default)]
pub(crate) struct AiStanding {
    record: Mutex<Option<Standing>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Standing {
    /// Runs on.
    pub(crate) selected: ProviderId,
    /// `usage_limited`, `needs_login`, `not_installed`, `unavailable` or
    /// `unsupported`.
    pub(crate) reason: &'static str,
    pub(crate) retry_at_ms: Option<u64>,
    /// The agent a request made now would run on, when one can answer.
    pub(crate) using: Option<ProviderId>,
}

impl AiStanding {
    /// Records how `router` stands now. Asking is a query: it never runs a
    /// request, and it refreshes a stale availability answer at most.
    pub(crate) fn observe(&self, router: &AiRouter) {
        let next = router.provider_state().and_then(|state| {
            let degraded = state.degraded?;
            Some(Standing {
                selected: degraded.provider,
                reason: refusal_class(&degraded.reason),
                retry_at_ms: degraded.until.map(|until| {
                    // Seconds are what an account reports; a changing
                    // millisecond would republish the snapshot for nothing.
                    unix_now_ms().saturating_add(until.as_secs().saturating_mul(1_000)) / 1_000
                        * 1_000
                }),
                using: state.active.filter(|active| *active != degraded.provider),
            })
        });
        *self
            .record
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = next;
    }

    pub(crate) fn current(&self) -> Option<Standing> {
        self.record
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

/// The class of a refusal the router reported for the selected agent. A
/// usage-limit park is the one `Unavailable` that has its own class, because
/// it is the one the shell says an end time for.
fn refusal_class(reason: &Availability) -> &'static str {
    match reason {
        Availability::Unavailable { reason } if reason.starts_with("usage_limited") => {
            "usage_limited"
        }
        other => other.class(),
    }
}

/// The `worktree_name` feature: a short English branch slug for a worktree
/// started from an issue. The Start dialog fills a deterministic name from the
/// issue's number and title first and replaces it with this answer only while
/// the operator has not edited the field, so a slow or absent provider costs
/// nothing but the better name.
const WORKTREE_NAME_SYSTEM: &str = "You name git branches for a coding task. \
Read the issue title and body and answer with a short English kebab-case slug of two to five words \
that says what the change does, like `sigterm-handler` or `overview-issue-board`. \
Use only lowercase ASCII letters, digits and hyphens. Do not include an issue number, a prefix such as feat/ or fix/, or quotes.";
/// The slug's own ceiling; the whole branch name is checked again by Git.
const WORKTREE_SLUG_LIMIT: usize = 48;
/// The body the model reads, in characters; the title carries most of it.
const WORKTREE_NAME_BODY_LIMIT: usize = 2_000;

/// A branch-safe slug: lowercase ASCII words joined by single hyphens.
pub(crate) fn branch_slug(text: &str) -> String {
    let mut slug = String::new();
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug = slug.trim_matches('-').to_owned();
    if slug.len() > WORKTREE_SLUG_LIMIT {
        slug.truncate(WORKTREE_SLUG_LIMIT);
        if let Some(cut) = slug.rfind('-') {
            slug.truncate(cut);
        }
    }
    slug.trim_matches('-').to_owned()
}

/// Asks the operator's background AI for a worktree name. Runs on a worker
/// thread; the router starts and stops its own provider process.
pub(crate) fn suggest_worktree_name(
    node: &Arc<dyn NodeLink>,
    settings: &AiSettings,
    usage: &Arc<dyn AiUsageSink>,
    subject: &str,
    prefix: &str,
    title: &str,
    body: &str,
) -> Result<String, String> {
    let body: String = body.chars().take(WORKTREE_NAME_BODY_LIMIT).collect();
    let request = hide_ai::AiRequest {
        feature_id: "worktree_name".into(),
        request_id: hide_ai::RequestId(format!("worktree-name-{subject}")),
        subject_id: subject.to_owned(),
        system: WORKTREE_NAME_SYSTEM.to_owned(),
        input: serde_json::json!({ "title": title, "body": body }).to_string(),
        output_schema: serde_json::json!({
            "type": "object",
            "properties": { "slug": { "type": "string", "maxLength": WORKTREE_SLUG_LIMIT } },
            "required": ["slug"],
            "additionalProperties": false,
        }),
        deadline: Duration::from_secs(30),
        schema_version: "1".into(),
        pick: None,
        // A Git name is lowercase ASCII whatever the operator reads (D-07).
        output_language: None,
        answer: hide_ai::AnswerMode::PlainJson,
        provider_efforts: Vec::new(),
    };
    let router = waited_router(node, settings, usage);
    let answer = router
        .execute(&request, &hide_ai::CancelToken::new())
        .map_err(|error| format!("{error:?}"))?;
    let slug = answer
        .value
        .get("slug")
        .and_then(serde_json::Value::as_str)
        .map(branch_slug)
        .filter(|slug| !slug.is_empty())
        .ok_or_else(|| "the answer had no usable name".to_owned())?;
    Ok(format!("{prefix}{slug}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine the tests run on, as the core reaches it.
    fn node() -> Arc<dyn NodeLink> {
        Arc::new(hide_node::Local::of_process())
    }

    fn models() -> BTreeMap<ProviderId, String> {
        AiSettings::default().models_by_provider()
    }

    #[test]
    fn a_slug_is_lowercase_ascii_words_within_the_limit() {
        assert_eq!(
            branch_slug("Graceful SIGTERM handler!"),
            "graceful-sigterm-handler"
        );
        assert_eq!(branch_slug("  --Overview v4 보드--  "), "overview-v4");
        assert_eq!(branch_slug("한국어만"), "");
        let long = branch_slug(&"word ".repeat(30));
        assert!(long.len() <= WORKTREE_SLUG_LIMIT && !long.ends_with('-'));
    }

    #[test]
    fn an_ai_event_reaches_the_diagnostic_log_as_a_kind_record_without_absent_fields() {
        let mut event = AiLogEvent::new("ai.codex_home.swept");
        event.provider = Some(ProviderId::CODEX);
        event.detail = Some("removed=2;bytes=10".to_owned());
        assert_eq!(
            diagnostic_record(&event),
            serde_json::json!({
                "kind": "ai.codex_home.swept",
                "component": "ai",
                "provider": "codex",
                "detail": "removed=2;bytes=10",
            })
        );
    }

    #[test]
    fn project_memory_uses_the_existing_selected_provider_and_fallback_policy() {
        let settings = AiSettings {
            provider: ProviderId::CLAUDE,
            ..AiSettings::default()
        };
        let router = memory_router(
            &node(),
            &settings,
            &Arc::new(UsageLimits::new()),
            &(crate::usage_ledger::ai_usage::AiUsageQueue::new() as Arc<dyn AiUsageSink>),
        );
        assert_eq!(
            router.provider_state().unwrap().selected,
            ProviderId::CLAUDE
        );
        assert_eq!(
            memory_router_config(&settings).priority,
            vec![ProviderId::CLAUDE]
        );
    }

    fn status(availability: Availability, selectable: bool) -> ProviderStatus {
        ProviderStatus {
            provider: ProviderId::CODEX,
            availability,
            selectable,
            parked_for: None,
        }
    }

    #[test]
    fn an_unobserved_request_asks_no_provider_anything() {
        let mut reader = AiReader::new(node());
        let request = AiRequest {
            observing: false,
            models: models(),
            selecting: BTreeSet::new(),
            cli_found: None,
        };
        // The first read starts the worker; once it has ended, the next read
        // hands its answer back.
        let answer = reader.read_if_due(request.clone()).or_else(|| {
            reader.inner.join_pending();
            reader.read_if_due(request)
        });
        let answer = answer.expect("the reader answers an unobserved request");
        assert_eq!(
            answer,
            BackgroundAiSnapshot::unread(),
            "nothing was asked, so every provider is unread"
        );
        assert!(
            answer
                .providers
                .iter()
                .all(|provider| provider.state == "unread"),
            "an unobserved read reports unread rather than a guess"
        );
    }

    #[test]
    fn a_request_selecting_among_agents_asks_only_those_agents() {
        let only: BTreeSet<ProviderId> = [ProviderId::PI].into();
        let request = AiRequest {
            observing: false,
            models: models(),
            selecting: only.clone(),
            cli_found: None,
        };
        assert!(request.asks());
        assert_eq!(request.only(), Some(&only));
        let watching = AiRequest {
            observing: true,
            ..request
        };
        assert_eq!(watching.only(), None, "an open tab asks every agent");
        assert!(!AiRequest::default().asks());
    }

    #[test]
    fn a_provider_that_was_not_asked_is_unread_rather_than_unavailable() {
        let row = project(ProviderId::CODEX, None, None, &models(), true, None);
        assert_eq!(row.state, "unread");
        assert!(row.models.is_empty());
        assert!(!row.selectable);
        assert_eq!(row.agent, "codex");
        assert!(row.models_unavailable_reason.is_some());
    }

    #[test]
    fn each_availability_state_carries_its_code_and_whether_it_can_be_chosen() {
        let models = models();
        let ready = project(
            ProviderId::CODEX,
            Some(&status(Availability::Ready, true)),
            None,
            &models,
            true,
            None,
        );
        assert_eq!((ready.state.as_str(), ready.selectable), ("ready", true));
        assert_eq!(ready.message, None);
        assert!(ready.installed);

        let needs_login = project(
            ProviderId::CLAUDE,
            Some(&status(Availability::NeedsLogin, false)),
            None,
            &models,
            true,
            None,
        );
        assert_eq!(needs_login.state, "needs_login");
        assert!(needs_login.installed && !needs_login.selectable);

        let missing = project(
            ProviderId::PI,
            Some(&status(Availability::NotInstalled, false)),
            None,
            &models,
            true,
            None,
        );
        assert_eq!(missing.state, "not_installed");
        assert!(!missing.installed);

        let unsupported = project(
            ProviderId::OPENCODE,
            Some(&status(
                Availability::Unsupported {
                    reason: "cannot_guarantee_read_only".to_owned(),
                },
                false,
            )),
            None,
            &models,
            true,
            None,
        );
        assert_eq!(unsupported.state, "unsupported");
        assert_eq!(
            unsupported.message.as_deref(),
            Some("cannot_guarantee_read_only"),
            "the provider layer's own reason code reaches the screen"
        );
        assert!(unsupported.installed && !unsupported.selectable);
    }

    /// The install kit owns "is the program on this Mac": an agent Hide AI
    /// cannot use yet is listed as not installed from what the kit found, and
    /// a kit that has not answered claims nothing.
    #[test]
    fn an_unsupported_agent_is_not_installed_when_the_kit_did_not_find_it() {
        let unsupported = status(
            Availability::Unsupported {
                reason: "cannot_guarantee_read_only".to_owned(),
            },
            false,
        );
        let row = |found: Option<&BTreeSet<ProviderId>>| {
            project(
                ProviderId::OPENCODE,
                Some(&unsupported),
                None,
                &models(),
                true,
                found,
            )
        };
        let found: BTreeSet<ProviderId> = [ProviderId::OPENCODE].into();
        let missing: BTreeSet<ProviderId> = [ProviderId::CODEX].into();

        let on_this_mac = row(Some(&found));
        assert_eq!(on_this_mac.state, "unsupported");
        assert!(on_this_mac.installed);

        let absent = row(Some(&missing));
        assert_eq!(absent.state, "not_installed");
        assert_eq!(absent.message, None);
        assert!(!absent.installed);

        assert_eq!(
            row(None).state,
            "unsupported",
            "no kit answer yet is not a claim that the program is missing"
        );
    }

    #[test]
    fn every_current_provider_reports_that_it_checks_sign_in() {
        let ready = status(Availability::Ready, true);
        for provider in hide_ai::PROVIDERS {
            let row = project(*provider, Some(&ready), None, &models(), true, None);
            assert!(row.login_checked, "{provider}");
            assert!(
                BackgroundAiProviderSnapshot::unread(*provider).login_checked,
                "{provider}"
            );
        }
    }

    #[test]
    fn an_agent_in_a_usage_limit_is_still_selectable_and_says_when_it_ends() {
        let mut parked = status(
            Availability::Unavailable {
                reason: "usage_limited;retry_after_s=60".to_owned(),
            },
            true,
        );
        parked.parked_for = Some(Duration::from_secs(60));
        let row = project(
            ProviderId::CLAUDE,
            Some(&parked),
            None,
            &models(),
            true,
            None,
        );
        assert_eq!(row.state, "usage_limited");
        assert!(row.selectable);
        let at = row.retry_at_ms.expect("the end of the limit is known");
        assert!(at >= unix_now_ms() + 59_000, "{at}");
    }

    #[test]
    fn an_unknown_model_list_is_reported_as_unknown_and_a_fixed_one_as_fixed() {
        let ready = status(Availability::Ready, true);
        let row = project(
            ProviderId::CODEX,
            Some(&ready),
            Some(&ModelCatalog::Unknown {
                reason: "codex_not_installed".to_owned(),
            }),
            &models(),
            true,
            None,
        );
        assert!(row.models.is_empty());
        assert_eq!(
            row.models_unavailable_reason.as_deref(),
            Some("codex_not_installed")
        );

        let offered = project(
            ProviderId::CLAUDE,
            Some(&ready),
            Some(&ModelCatalog::Offered(vec!["haiku".to_owned()])),
            &models(),
            true,
            None,
        );
        assert_eq!(offered.models, vec!["haiku".to_owned()]);
        assert_eq!(offered.models_unavailable_reason, None);
        assert!(!offered.models_fixed);

        // A compatible older node can still return a fixed catalog.
        let fixed = project(
            ProviderId::GROK,
            Some(&ready),
            Some(&ModelCatalog::Fixed(vec!["pro".to_owned()])),
            &models(),
            true,
            None,
        );
        assert!(fixed.models_fixed && fixed.models == vec!["pro".to_owned()]);
        assert!(
            fixed.cli_default,
            "a new agent can be asked for its CLI default"
        );
        assert!(
            !offered.cli_default,
            "Claude Code keeps its measured default"
        );
    }

    #[test]
    fn a_row_reports_the_model_the_provider_is_configured_with() {
        let mut models = models();
        models.insert(ProviderId::CLAUDE, "opus".to_owned());
        assert_eq!(
            project(ProviderId::CLAUDE, None, None, &models, true, None).model,
            "opus"
        );
        assert_eq!(
            project(ProviderId::CLAUDE, None, None, &BTreeMap::new(), true, None).model,
            hide_ai::settings::default_model(ProviderId::CLAUDE),
            "a provider with no configured model reports the backend's own default"
        );
    }

    #[test]
    fn a_usage_limit_park_is_the_one_unavailable_that_has_its_own_refusal_class() {
        assert_eq!(
            refusal_class(&Availability::Unavailable {
                reason: "usage_limited;retry_after_s=5".to_owned()
            }),
            "usage_limited"
        );
        assert_eq!(
            refusal_class(&Availability::Unavailable {
                reason: "model_not_offered:x".to_owned()
            }),
            "unavailable"
        );
        assert_eq!(refusal_class(&Availability::NeedsLogin), "needs_login");
    }
}
