//! Registry-driven limits. All I/O runs on bounded background reads through
//! the core's own node; the coordinator only polls and publishes transitions.
mod limits;
pub mod model;
pub mod prefs;
mod retry;

use crate::node_access::{NodeLink, call_as_with_progress};
use crate::reader::BackgroundRead;
use hide_agent_adapter::usage::LimitSource;
pub mod consumption;
use hide_ai::CancelToken;
use hide_node_link::protocol::Call;
use hide_node_link::usage::{FailureDisposition, LimitFailure, LimitFetch};
use model::*;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const INITIAL_DELAY: Duration = Duration::from_secs(1);
const REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);
const POPOVER_REFRESH_AGE: Duration = Duration::from_secs(60);
const STALE_LIMIT_MS: u64 = 15 * 60 * 1000;
const READ_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const RETRY_BACKOFF: [Duration; 3] = [
    Duration::from_secs(300),
    Duration::from_secs(600),
    Duration::from_secs(900),
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct UsageActivity {
    /// The agents' own figures are wanted now: a window shows them, or
    /// Memory or the usage limit reads the weekly figure.
    pub window_visible: bool,
    /// A window shows the usage, which alone makes a profile account's read
    /// (B23): Memory and the limit ask for the default account only.
    pub shown: bool,
    pub popover_open_generation: u64,
    pub hidden: Vec<String>,
    pub required: Vec<String>,
    /// This machine's profiles of agents whose limits Hide reads, in the
    /// list's order (PRD agent-profiles B23): each account is read once,
    /// for the first profile in it.
    pub profiles: Vec<UsageProfile>,
}

/// One of this machine's profiles, as a usage read sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UsageProfile {
    /// The agent's engine kind, which keys the usage rows.
    pub agent: String,
    pub id: hide_agent_adapter::ProfileId,
    pub account: hide_agent_adapter::AccountKey,
}
#[derive(Clone, Debug)]
pub(crate) struct UsagePaths {
    pub home: Option<PathBuf>,
    pub claude_cwd: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
}
#[derive(Clone, Debug)]
struct Success {
    value: LimitReading,
    observed_at_ms: u64,
}
struct Answer {
    checked_at_ms: u64,
    result: Result<Success, Failure>,
    fallback: Option<Success>,
}
#[derive(Clone, Debug)]
struct Failure {
    cause: Cause,
    kind: String,
    status: Option<u16>,
    retry_after: Option<Duration>,
}
impl Failure {
    fn from_node(f: LimitFailure) -> Self {
        let cause = match f.disposition {
            FailureDisposition::Authentication => Cause::SignedOut,
            FailureDisposition::Schema => Cause::FormatChanged,
            FailureDisposition::NotInstalled => Cause::NotInstalled,
            FailureDisposition::Offline => {
                if f.status == Some(429) {
                    Cause::RateLimited
                } else if f.kind == "timeout" {
                    Cause::TimedOut
                } else {
                    Cause::Offline
                }
            }
        };
        Self {
            cause,
            kind: f.kind,
            status: f.status,
            retry_after: f.retry_after.as_deref().and_then(retry::parse_retry_after),
        }
    }
    fn parse(cause: Cause) -> Self {
        Self {
            cause,
            kind: "response_format".into(),
            status: None,
            retry_after: None,
        }
    }
    fn transient(&self) -> bool {
        matches!(
            self.cause,
            Cause::Offline | Cause::TimedOut | Cause::RateLimited
        )
    }
}
struct SourceRead {
    agent: &'static str,
    /// The profile whose account is read; `None` for the agent's default.
    profile: Option<hide_agent_adapter::ProfileId>,
    source: LimitSource,
    worker: BackgroundRead<u64, Answer>,
    attempt: u64,
    pending: bool,
    last_attempt: Option<Instant>,
    checked_at_ms: Option<u64>,
    success: Option<Success>,
    fallback: Option<Success>,
    failure: Option<Failure>,
    retry_at: Option<Instant>,
    backoff_index: usize,
}
impl SourceRead {
    fn new(
        agent: &'static str,
        source: LimitSource,
        root: Option<PathBuf>,
        profile: Option<hide_agent_adapter::ProfileId>,
        node: Arc<dyn NodeLink>,
        cancel: CancelToken,
    ) -> Self {
        let read_profile = profile.clone();
        let worker = BackgroundRead::on_change(Duration::ZERO, move |_: &u64| {
            let checked_at_ms = unix_ms();
            let fetch = match &root {
                Some(root) => call_as_with_progress::<LimitFetch, Value>(
                    node.as_ref(),
                    Call::UsageLimitFetch {
                        source,
                        root: root.to_string_lossy().into_owned(),
                        profile: read_profile.clone(),
                    },
                    READ_TIMEOUT,
                    |_| !cancel.is_cancelled(),
                )
                .map_err(|_| Failure {
                    cause: Cause::Offline,
                    kind: "usage_node_unavailable".into(),
                    status: None,
                    retry_after: None,
                }),
                None => Err(Failure::parse(Cause::FormatChanged)),
            };
            match fetch {
                Err(failure) => Answer {
                    checked_at_ms,
                    result: Err(failure),
                    fallback: None,
                },
                Ok(fetch) => {
                    let result = fetch
                        .result
                        .map_err(Failure::from_node)
                        .and_then(|body| {
                            limits::parse(source, &body, checked_at_ms / 1000)
                                .map_err(Failure::parse)
                        })
                        .map(|value| Success {
                            value,
                            observed_at_ms: checked_at_ms,
                        });
                    // A schema failure immediately invalidates even an old success.
                    let fallback = if result
                        .as_ref()
                        .is_err_and(|f| f.cause != Cause::FormatChanged && f.cause != Cause::Capped)
                    {
                        fetch.fallback.and_then(|read| {
                            limits::parse(source, &read.body, checked_at_ms / 1000)
                                .ok()
                                .map(|value| Success {
                                    value,
                                    observed_at_ms: read.source_at_unix_ms,
                                })
                        })
                    } else {
                        None
                    };
                    Answer {
                        checked_at_ms,
                        result,
                        fallback,
                    }
                }
            }
        });
        Self {
            agent,
            profile,
            source,
            worker,
            attempt: 0,
            pending: false,
            last_attempt: None,
            checked_at_ms: None,
            success: None,
            fallback: None,
            failure: None,
            retry_at: None,
            backoff_index: 0,
        }
    }
    fn due(&self, now: Instant, active: bool, opened: bool) -> bool {
        !self.pending
            && self.retry_at.is_none_or(|retry| now >= retry)
            && self.last_attempt.is_none_or(|last| {
                (active && now.duration_since(last) >= REFRESH_INTERVAL)
                    || (opened && now.duration_since(last) > POPOVER_REFRESH_AGE)
            })
    }
    fn collect(&mut self, now: Instant) {
        if self.attempt == 0 {
            return;
        }
        let Some(answer) = self.worker.poll(self.attempt) else {
            return;
        };
        self.pending = false;
        self.checked_at_ms = Some(answer.checked_at_ms);
        self.last_attempt = Some(now);
        match answer.result {
            Ok(success) => {
                self.success = Some(success);
                self.failure = None;
                self.fallback = None;
                self.retry_at = None;
                self.backoff_index = 0;
            }
            Err(failure) => {
                crate::diagnostic!(
                    json!({"component":"usage", "agent":self.agent,"source":self.source,"kind":failure.kind,"status":failure.status})
                );
                self.retry_at = (failure.status == Some(429)).then(|| {
                    let delay = failure.retry_after.unwrap_or_else(|| {
                        let delay = RETRY_BACKOFF[self.backoff_index];
                        self.backoff_index = (self.backoff_index + 1).min(RETRY_BACKOFF.len() - 1);
                        delay
                    });
                    // A hostile Retry-After cannot overflow Instant.
                    now.checked_add(delay).unwrap_or(now + RETRY_BACKOFF[2])
                });
                self.failure = Some(failure);
                self.fallback = answer.fallback;
            }
        }
    }
    fn poll(&mut self, now: Instant, enabled: bool, active: bool, opened: bool) {
        if enabled && self.due(now, active, opened) {
            self.attempt += 1;
            self.pending = true;
            self.last_attempt = Some(now);
        }
        self.collect(now);
    }
    fn project(&self, now_ms: u64) -> Cell<LimitReading> {
        let reading = |success: &Success, freshness| {
            let mut value = success.value.clone();
            value.expire(now_ms / 1000);
            Cell::Reading {
                value,
                freshness,
                observed_at_ms: success.observed_at_ms,
            }
        };
        match &self.failure {
            None => self
                .success
                .as_ref()
                .map_or(Cell::Loading, |s| reading(s, Freshness::Fresh)),
            Some(failure) => {
                if failure.transient()
                    && let Some(success) = &self.success
                    && now_ms.saturating_sub(success.observed_at_ms) <= STALE_LIMIT_MS
                {
                    return reading(success, Freshness::Stale);
                }
                if let Some(fallback) = &self.fallback {
                    return reading(fallback, Freshness::Fallback);
                }
                Cell::Unavailable {
                    cause: failure.cause,
                    last_checked_at_ms: self.checked_at_ms,
                }
            }
        }
    }
}

pub struct UsageDriver {
    started_at: Instant,
    last_seen_popover_generation: u64,
    reads: Vec<SourceRead>,
    /// One read per source and profile account (B23), by the agent, the
    /// source and the account; at most one per profile the machine keeps.
    accounts: std::collections::BTreeMap<
        (&'static str, LimitSource, hide_agent_adapter::AccountKey),
        SourceRead,
    >,
    paths: UsagePaths,
    node: Arc<dyn NodeLink>,
    published: Vec<AgentUsage>,
    cancel: CancelToken,
}
impl UsageDriver {
    pub(crate) fn new(paths: UsagePaths, node: Arc<dyn NodeLink>) -> Self {
        let cancel = CancelToken::new();
        let reads = hide_agent_adapter::ADAPTERS
            .iter()
            .flat_map(|row| {
                row.usage
                    .limit_sources()
                    .map(|source| {
                        SourceRead::new(
                            row.engine_kind,
                            source,
                            paths.root(source),
                            None,
                            Arc::clone(&node),
                            cancel.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        Self {
            started_at: Instant::now(),
            last_seen_popover_generation: 0,
            reads,
            accounts: std::collections::BTreeMap::new(),
            paths,
            node,
            published: AgentUsage::initial_rows(),
            cancel,
        }
    }

    /// Keeps one read per account the machine's profiles run in: a new
    /// account's read starts with the next due poll, and one no profile
    /// names any longer goes (its worker ends with its read). Which profile
    /// each row shows the account's reading for is laid at publish
    /// ([`Self::profile_rows`]).
    fn follow_profiles(&mut self, profiles: &[UsageProfile]) {
        let mut wanted: std::collections::BTreeMap<_, &UsageProfile> =
            std::collections::BTreeMap::new();
        for profile in profiles
            .iter()
            .filter(|profile| !profile.account.is_default())
        {
            let Some(row) = hide_agent_adapter::ADAPTERS
                .iter()
                .find(|row| row.engine_kind == profile.agent)
            else {
                continue;
            };
            for source in row.usage.limit_sources() {
                // agy keeps no account apart.
                if source == LimitSource::AntigravityUsage {
                    continue;
                }
                wanted
                    .entry((row.engine_kind, source, profile.account.clone()))
                    .or_insert(profile);
            }
        }
        self.accounts.retain(|key, read| {
            wanted
                .get(key)
                .is_some_and(|profile| read.profile.as_ref() == Some(&profile.id))
        });
        for (key, profile) in wanted {
            if self.accounts.contains_key(&key) {
                continue;
            }
            let read = SourceRead::new(
                key.0,
                key.1,
                self.paths.root(key.1),
                Some(profile.id.clone()),
                Arc::clone(&self.node),
                self.cancel.clone(),
            );
            self.accounts.insert(key, read);
        }
    }

    /// Each profile's row of its agent's usage: its account's reading, for
    /// the first profile in that account; shared for one in an account a
    /// profile before it, or the agent's default, already shows (B23).
    fn profile_rows(
        &self,
        profiles: &[UsageProfile],
        agent: &str,
        now_ms: u64,
    ) -> Vec<ProfileUsage> {
        let mut seen = std::collections::BTreeSet::new();
        profiles
            .iter()
            .filter(|profile| profile.agent == agent)
            .map(|profile| {
                let first = !profile.account.is_default() && seen.insert(&profile.account);
                let mut limits: Option<Cell<LimitReading>> = None;
                if first {
                    for (_, read) in self
                        .accounts
                        .iter()
                        .filter(|((kind, _, key), _)| *kind == agent && *key == profile.account)
                    {
                        limits = Some(merge(limits.unwrap_or(Cell::Loading), read.project(now_ms)));
                    }
                }
                ProfileUsage {
                    id: profile.id.0.clone(),
                    shared: !first,
                    limits,
                }
            })
            .collect()
    }
    /// Added work per coordinator tick is bounded by declared sources (two).
    /// One worker per source, no queued reads; one publish only on a transition.
    pub(crate) fn read_if_due(&mut self, activity: UsageActivity) -> Option<Vec<AgentUsage>> {
        let now = Instant::now();
        let opened = activity.popover_open_generation > self.last_seen_popover_generation;
        self.last_seen_popover_generation = activity.popover_open_generation;
        for read in &mut self.reads {
            let required = activity.required.iter().any(|agent| agent == read.agent);
            let visible = !activity.hidden.iter().any(|agent| agent == read.agent);
            let enabled = visible || required;
            let active = (visible && activity.window_visible) || required;
            read.poll(
                now,
                enabled && now.duration_since(self.started_at) >= INITIAL_DELAY,
                active,
                opened && visible,
            );
        }
        self.follow_profiles(&activity.profiles);
        // A profile's account is read only while a window shows its agent's
        // row, its first read included: no one else reads it (Memory and the
        // limit ask for the default account).
        for read in self.accounts.values_mut() {
            let shown = activity.shown && !activity.hidden.iter().any(|agent| agent == read.agent);
            read.poll(
                now,
                shown && now.duration_since(self.started_at) >= INITIAL_DELAY,
                shown,
                opened && shown,
            );
        }
        let now_ms = unix_ms();
        let mut next = AgentUsage::initial_rows();
        for row in &mut next {
            for read in self.reads.iter().filter(|read| read.agent == row.agent) {
                row.limits = merge(row.limits.clone(), read.project(now_ms));
            }
            row.profiles = self.profile_rows(&activity.profiles, &row.agent, now_ms);
        }
        if next == self.published {
            return None;
        }
        self.published = next.clone();
        Some(next)
    }
}
impl Drop for UsageDriver {
    fn drop(&mut self) {
        self.cancel.cancel();
        for read in self.reads.iter_mut().chain(self.accounts.values_mut()) {
            read.worker.join_pending();
        }
    }
}

impl UsagePaths {
    /// The folder a read of `source` runs in: Claude's working folder, or
    /// the default Codex home.
    fn root(&self, source: LimitSource) -> Option<PathBuf> {
        match source {
            LimitSource::ClaudeUsage => self.claude_cwd.clone(),
            LimitSource::CodexUsage => self.codex_home.clone(),
            // agy's `/usage` reads no workspace; it runs in Hide's own
            // folder as Claude's does.
            LimitSource::AntigravityUsage => self.claude_cwd.clone(),
        }
    }
}
fn merge(current: Cell<LimitReading>, next: Cell<LimitReading>) -> Cell<LimitReading> {
    match (current, next) {
        (
            Cell::Reading {
                mut value,
                freshness,
                observed_at_ms,
            },
            Cell::Reading {
                value: incoming,
                freshness: incoming_freshness,
                observed_at_ms: incoming_at,
            },
        ) => {
            value.windows.extend(incoming.windows);
            Cell::Reading {
                value,
                freshness: if freshness == Freshness::Fresh {
                    incoming_freshness
                } else {
                    freshness
                },
                observed_at_ms: observed_at_ms.min(incoming_at),
            }
        }
        (_, next) => next,
    }
}
fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> SourceRead {
        SourceRead {
            agent: "claude",
            profile: None,
            source: LimitSource::ClaudeUsage,
            worker: BackgroundRead::on_change(Duration::ZERO, |_: &u64| unreachable!()),
            attempt: 0,
            pending: false,
            last_attempt: None,
            checked_at_ms: Some(2000),
            success: Some(Success {
                value: LimitReading {
                    windows: vec![WindowReading {
                        kind: WindowKind::Weekly,
                        scope: Scope::All,
                        outcome: WindowOutcome::Open {
                            used_percent: 12.0,
                            resets_at_s: 9999,
                        },
                    }],
                },
                observed_at_ms: 1000,
            }),
            fallback: None,
            failure: None,
            retry_at: None,
            backoff_index: 0,
        }
    }
    #[test]
    fn schema_failure_drops_retained_values_while_transient_failure_keeps_recent_values() {
        let mut source = source();
        source.failure = Some(Failure {
            cause: Cause::Offline,
            kind: "transport".into(),
            status: None,
            retry_after: None,
        });
        assert!(matches!(
            source.project(2000),
            Cell::Reading {
                freshness: Freshness::Stale,
                ..
            }
        ));
        assert!(matches!(
            source.project(STALE_LIMIT_MS + 1001),
            Cell::Unavailable {
                cause: Cause::Offline,
                ..
            }
        ));
        source.failure = Some(Failure::parse(Cause::FormatChanged));
        assert!(matches!(
            source.project(2000),
            Cell::Unavailable {
                cause: Cause::FormatChanged,
                ..
            }
        ));
    }
    #[test]
    fn hidden_ticks_popover_age_and_backoff_preserve_the_schedule() {
        let now = Instant::now();
        let mut source = source();
        source.last_attempt = Some(now - REFRESH_INTERVAL);
        assert!(!source.due(now, false, false));
        assert!(source.due(now, true, false));
        source.last_attempt = Some(now - POPOVER_REFRESH_AGE);
        assert!(!source.due(now, false, true));
        assert!(source.due(now + Duration::from_secs(1), false, true));
        source.retry_at = Some(now + Duration::from_secs(90));
        assert!(!source.due(now + Duration::from_secs(89), true, true));
        assert!(source.due(now + Duration::from_secs(90), true, true));
    }
    #[test]
    fn reopening_during_a_read_cannot_queue_work_after_hide_or_bypass_backoff() {
        use std::sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(2));
        let mut source = source();
        let counted = calls.clone();
        let worker_barrier = barrier.clone();
        source.worker = BackgroundRead::on_change(Duration::ZERO, move |_: &u64| {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                worker_barrier.wait();
                worker_barrier.wait();
            }
            Answer {
                checked_at_ms: 1000,
                result: Err(Failure {
                    cause: Cause::RateLimited,
                    kind: "rate_limit".into(),
                    status: Some(429),
                    retry_after: Some(Duration::from_secs(300)),
                }),
                fallback: None,
            }
        });
        let start = Instant::now();
        source.poll(start, true, true, false);
        barrier.wait();
        let reopened = start + Duration::from_secs(61);
        source.poll(reopened, true, false, true);
        barrier.wait();
        source.worker.join_pending();
        source.poll(reopened, false, false, false);
        source.worker.join_pending();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "Hide must not start a queued read"
        );
        source.poll(reopened + Duration::from_secs(299), true, true, true);
        source.worker.join_pending();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "Retry-After still governs a reopened popover"
        );
        source.poll(reopened + Duration::from_secs(300), true, true, true);
        source.worker.join_pending();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    /// A node that answers nothing: these reads are never due.
    struct Silent;
    impl NodeLink for Silent {
        fn call(
            &self,
            _: Call,
            _: Duration,
        ) -> Result<hide_node_link::LinkAnswer, crate::node_access::LinkError> {
            unreachable!("no read is due")
        }
    }

    #[test]
    fn each_profile_account_is_read_once_and_a_profile_sharing_one_shows_a_dash() {
        use hide_agent_adapter::{AccountKey, ProfileId};
        let mut driver = UsageDriver::new(
            UsagePaths {
                home: None,
                claude_cwd: Some("/hide".into()),
                codex_home: Some("/codex".into()),
            },
            Arc::new(Silent),
        );
        let profile = |id: &str, agent: &str, account: AccountKey| UsageProfile {
            agent: agent.into(),
            id: ProfileId(id.into()),
            account,
        };
        let folder = |path: &str| AccountKey::Folder(path.into());
        let profiles = [
            profile("work", "claude", folder("/accounts/work")),
            // A wrapper in the default account, and a second profile of the
            // work account: their figure is another row's.
            profile("gateway", "claude", AccountKey::Default),
            profile("work-too", "claude", folder("/accounts/work")),
            profile("team", "codex", folder("/accounts/team")),
        ];
        driver.follow_profiles(&profiles);
        assert_eq!(
            driver.accounts.keys().cloned().collect::<Vec<_>>(),
            [
                ("claude", LimitSource::ClaudeUsage, folder("/accounts/work")),
                ("codex", LimitSource::CodexUsage, folder("/accounts/team")),
            ]
        );
        let rows = driver.profile_rows(&profiles, "claude", 0);
        assert_eq!(
            rows.iter()
                .map(|row| (row.id.as_str(), row.shared, row.limits.clone()))
                .collect::<Vec<_>>(),
            [
                ("work", false, Some(Cell::Loading)),
                ("gateway", true, None),
                ("work-too", true, None),
            ]
        );
        // A profile that goes takes its account's read with it.
        driver.follow_profiles(&profiles[..2]);
        assert_eq!(driver.accounts.len(), 1);
        // With no profile, nothing is read and the row carries none.
        driver.follow_profiles(&[]);
        assert!(driver.accounts.is_empty());
        assert!(driver.profile_rows(&[], "claude", 0).is_empty());
    }

    /// A node that answers no read, keeping the profile each read named.
    #[derive(Default)]
    struct Reads(std::sync::Mutex<Vec<Option<hide_agent_adapter::ProfileId>>>);
    impl NodeLink for Reads {
        fn call(
            &self,
            call: Call,
            _: Duration,
        ) -> Result<hide_node_link::LinkAnswer, crate::node_access::LinkError> {
            if let Call::UsageLimitFetch { profile, .. } = call {
                self.0.lock().unwrap().push(profile);
            }
            Err(crate::node_access::LinkError::Busy)
        }
    }

    /// Memory or the usage limit with no window reads the default account
    /// alone; a profile account is read only while a window shows its row,
    /// its first read included (B23).
    #[test]
    fn a_profile_account_is_read_only_while_a_window_shows_its_row() {
        use hide_agent_adapter::{AccountKey, ProfileId};
        let reads = Arc::new(Reads::default());
        let mut driver = UsageDriver::new(
            UsagePaths {
                home: None,
                claude_cwd: Some("/hide".into()),
                codex_home: Some("/codex".into()),
            },
            reads.clone(),
        );
        driver.started_at = Instant::now() - INITIAL_DELAY;
        let activity = |shown: bool| UsageActivity {
            window_visible: true,
            shown,
            required: vec!["claude".into()],
            profiles: vec![UsageProfile {
                agent: "claude".into(),
                id: ProfileId("work".into()),
                account: AccountKey::Folder("/accounts/work".into()),
            }],
            ..Default::default()
        };
        let settle = |driver: &mut UsageDriver| {
            for read in driver.reads.iter_mut().chain(driver.accounts.values_mut()) {
                read.worker.join_pending();
            }
        };
        driver.read_if_due(activity(false));
        settle(&mut driver);
        let read = |reads: &Reads| reads.0.lock().unwrap().clone();
        assert!(!read(&reads).is_empty(), "the default account is read");
        assert!(
            read(&reads).iter().all(Option::is_none),
            "{:?}",
            read(&reads)
        );
        driver.read_if_due(activity(true));
        settle(&mut driver);
        assert!(read(&reads).contains(&Some(ProfileId("work".into()))));
    }

    /// With no profile, an agent's usage row is on the wire as it was
    /// before profiles (B1).
    #[test]
    fn a_usage_row_without_profiles_is_written_as_before() {
        for row in AgentUsage::initial_rows() {
            let written = serde_json::to_value(&row).unwrap();
            assert_eq!(
                written.as_object().unwrap().keys().collect::<Vec<_>>(),
                ["agent", "limits", "consumption"],
                "{}",
                row.agent
            );
        }
    }

    #[test]
    fn registry_has_all_agents_but_only_three_distinct_limit_reads() {
        let rows = AgentUsage::initial_rows();
        assert_eq!(rows.len(), 7);
        assert_eq!(
            rows.iter()
                .filter(|r| matches!(r.limits, Cell::Loading))
                .count(),
            3
        );
        assert!(rows.iter().all(|row| row.agent != "cursor"));
        for row in &rows {
            assert!(matches!(row.consumption, Cell::Loading), "{}", row.agent);
        }
        assert_eq!(
            hide_agent_adapter::ADAPTERS
                .iter()
                .map(|r| r.usage.limit_sources().count())
                .sum::<usize>(),
            3
        );
    }
}
