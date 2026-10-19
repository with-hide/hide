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
    pub window_visible: bool,
    pub popover_open_generation: u64,
    pub hidden: Vec<String>,
    pub required: Vec<String>,
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
        node: Arc<dyn NodeLink>,
        cancel: CancelToken,
    ) -> Self {
        let worker = BackgroundRead::on_change(Duration::ZERO, move |_: &u64| {
            let checked_at_ms = unix_ms();
            let fetch = match &root {
                Some(root) => call_as_with_progress::<LimitFetch, Value>(
                    node.as_ref(),
                    Call::UsageLimitFetch {
                        source,
                        root: root.to_string_lossy().into_owned(),
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
                        let root = match source {
                            LimitSource::ClaudeUsageText => paths.claude_cwd.clone(),
                            LimitSource::CodexUsage => paths.codex_home.clone(),
                            // agy's `/usage` reads no workspace; it runs in
                            // Hide's own folder as Claude's does.
                            LimitSource::AntigravityUsage => paths.claude_cwd.clone(),
                        };
                        SourceRead::new(
                            row.herdr.name,
                            source,
                            root,
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
            published: AgentUsage::initial_rows(),
            cancel,
        }
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
        let mut next = AgentUsage::initial_rows();
        for row in &mut next {
            for read in self.reads.iter().filter(|read| read.agent == row.agent) {
                row.limits = merge(row.limits.clone(), read.project(unix_ms()));
            }
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
        for read in &mut self.reads {
            read.worker.join_pending();
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
            source: LimitSource::ClaudeUsageText,
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
