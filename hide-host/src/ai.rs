//! The node's background AI backends (`hide_node_link::ai`): one real
//! provider backend per core backend instance, built on first use with the
//! node's own logins and dropped when the core releases it or the node goes
//! away, so its resident process ends with its owner.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use hide_agent_adapter::ProfileId;
use hide_ai::{AiBackend, AiLogEvent, AiLogSink, CancelToken, ItemRun, ProviderId};
use hide_node_link::ai::{AiError, AiRequest, AiResponse, BackendSpec, Logged};

use crate::error::{ErrorCode, HostError, HostResult};

/// The most backends one node keeps. A core holds a few routers (Settings,
/// Project Memory, a labels worker per machine), each with one backend per
/// registered provider, and a rebuilt router overlaps the one it replaces.
const MAX_BACKENDS: usize = 64;
/// The most log events kept between two answers; older ones are counted.
const LOG_LIMIT: usize = 256;

/// The backends this node answers for, by core instance.
#[derive(Default)]
pub struct Backends {
    entries: Mutex<HashMap<u64, Entry>>,
}

impl std::fmt::Debug for Backends {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.entries.lock().map_or(0, |entries| entries.len());
        formatter
            .debug_struct("Backends")
            .field("count", &count)
            .finish()
    }
}

struct Entry {
    provider: ProviderId,
    model: String,
    profile: Option<ProfileId>,
    backend: Arc<dyn AiBackend>,
    log: Arc<Buffered>,
}

/// How the node runs a backend's profile (`crate::profiles::ai_run`), asked
/// only when the backend is built.
pub(crate) type RunOf<'a> = &'a dyn Fn(&BackendSpec) -> HostResult<ItemRun>;

impl Backends {
    /// The backend for `spec`, built on its first use as the profile it
    /// names runs here (`run`). An instance asked for with another provider,
    /// model or profile than it was built with is refused, as is a profile
    /// this node cannot run.
    fn get(
        &self,
        spec: &BackendSpec,
        run: RunOf<'_>,
    ) -> HostResult<(Arc<dyn AiBackend>, Arc<Buffered>)> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = entries.get(&spec.instance) {
            if entry.provider != spec.provider
                || entry.model != spec.model
                || entry.profile != spec.profile
            {
                return Err(HostError::new(
                    ErrorCode::InvalidPath,
                    format!(
                        "AI backend {} was built for {} {} {}, not {} {} {}",
                        spec.instance,
                        entry.provider,
                        entry.model,
                        profile_name(entry.profile.as_ref()),
                        spec.provider,
                        spec.model,
                        profile_name(spec.profile.as_ref()),
                    ),
                ));
            }
            return Ok((Arc::clone(&entry.backend), Arc::clone(&entry.log)));
        }
        if entries.len() >= MAX_BACKENDS {
            return Err(HostError::new(
                ErrorCode::Unsupported,
                format!("This node already keeps {MAX_BACKENDS} AI backends"),
            ));
        }
        // Read before anything is kept, so a profile that cannot run here
        // holds no place; the profile file is small and read once a build.
        let run = run(spec)?;
        let log = Arc::new(Buffered::default());
        let backend = hide_ai::build_backend(
            spec.provider,
            &spec.model,
            run,
            Arc::clone(&log) as Arc<dyn AiLogSink>,
        );
        entries.insert(
            spec.instance,
            Entry {
                provider: spec.provider,
                model: spec.model.clone(),
                profile: spec.profile.clone(),
                backend: Arc::clone(&backend),
                log: Arc::clone(&log),
            },
        );
        Ok((backend, log))
    }

    /// Drops the backend of `instance`, ending its resident process once no
    /// request still uses it.
    pub fn release(&self, instance: u64) {
        let removed = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&instance);
        drop(removed);
    }

    pub fn availability(
        &self,
        spec: &BackendSpec,
        run: RunOf<'_>,
    ) -> HostResult<Logged<hide_ai::Availability>> {
        let (backend, log) = self.get(spec, run)?;
        let value = backend.availability();
        Ok(Logged {
            value,
            log: log.drain(),
        })
    }

    pub fn models(
        &self,
        spec: &BackendSpec,
        run: RunOf<'_>,
    ) -> HostResult<Logged<hide_ai::ModelCatalog>> {
        let (backend, log) = self.get(spec, run)?;
        let value = backend.models();
        Ok(Logged {
            value,
            log: log.drain(),
        })
    }

    /// Runs `request`, asking `report` once a second whether to go on; a
    /// false answer cancels it, which the backend answers as `Cancelled`.
    pub fn execute(
        &self,
        spec: &BackendSpec,
        run: RunOf<'_>,
        request: AiRequest,
        report: &mut dyn FnMut() -> bool,
    ) -> HostResult<Logged<Result<AiResponse, AiError>>> {
        let (backend, log) = self.get(spec, run)?;
        let cancel = CancelToken::new();
        let running = cancel.clone();
        let value = crate::reporting::run_reporting(
            "hide-node-ai-request",
            move || backend.execute(&request, &running),
            &|| cancel.cancel(),
            report,
        )
        .unwrap_or_else(|reason| Err(AiError::CompletionUnknown(reason)));
        Ok(Logged {
            value,
            log: log.drain(),
        })
    }

    pub fn measurement(
        &self,
        spec: &BackendSpec,
        run: RunOf<'_>,
    ) -> HostResult<hide_ai::ProcessMeasurement> {
        Ok(self.get(spec, run)?.0.last_measurement())
    }

    pub fn restart(&self, spec: &BackendSpec, run: RunOf<'_>) -> HostResult<()> {
        self.get(spec, run)?.0.restart();
        Ok(())
    }
}

/// A backend's profile as an error names it: the built-in default by that
/// name.
fn profile_name(profile: Option<&ProfileId>) -> &str {
    profile.map_or("default", |profile| profile.0.as_str())
}

/// A backend's log events, kept until the next answer carries them.
#[derive(Default)]
struct Buffered {
    events: Mutex<(Vec<AiLogEvent>, usize)>,
}

impl Buffered {
    fn drain(&self) -> Vec<AiLogEvent> {
        let mut guard = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (events, dropped) = &mut *guard;
        let mut drained = std::mem::take(events);
        if *dropped > 0 {
            let mut lost = AiLogEvent::new("ai.log.dropped");
            lost.detail = Some(format!("count={dropped}"));
            drained.push(lost);
            *dropped = 0;
        }
        drained
    }
}

impl AiLogSink for Buffered {
    fn log(&self, event: AiLogEvent) {
        let mut guard = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (events, dropped) = &mut *guard;
        if events.len() >= LOG_LIMIT {
            events.remove(0);
            *dropped += 1;
        }
        events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude(instance: u64, model: &str) -> BackendSpec {
        BackendSpec {
            instance,
            provider: ProviderId::CLAUDE,
            model: model.to_owned(),
            profile: None,
        }
    }

    fn default_run(_: &BackendSpec) -> HostResult<ItemRun> {
        Ok(ItemRun::default())
    }

    #[test]
    fn an_instance_keeps_what_it_was_built_with_and_a_release_frees_its_place() {
        let backends = Backends::default();
        assert!(backends.get(&claude(1, "sonnet"), &default_run).is_ok());
        assert!(backends.get(&claude(1, "opus"), &default_run).is_err());
        let profiled = BackendSpec {
            profile: Some(ProfileId("p-work".into())),
            ..claude(1, "sonnet")
        };
        assert!(
            backends.get(&profiled, &default_run).is_err(),
            "another profile is another backend"
        );
        for instance in 2..=MAX_BACKENDS as u64 {
            backends
                .get(&claude(instance, "sonnet"), &default_run)
                .unwrap();
        }
        assert!(
            backends.get(&claude(99, "sonnet"), &default_run).is_err(),
            "past the cap"
        );
        backends.release(1);
        assert!(backends.get(&claude(99, "sonnet"), &default_run).is_ok());
    }

    /// A profile the node cannot run is refused before it takes a place, so
    /// a list of gone profiles never fills the node.
    #[test]
    fn a_profile_the_node_cannot_run_is_refused_and_holds_no_place() {
        let backends = Backends::default();
        let gone = |_: &BackendSpec| -> HostResult<ItemRun> {
            Err(HostError::new(
                ErrorCode::NotFound,
                "This machine has no profile p-gone",
            ))
        };
        let spec = BackendSpec {
            profile: Some(ProfileId("p-gone".into())),
            ..claude(1, "sonnet")
        };
        let refused = backends.models(&spec, &gone).unwrap_err();
        assert!(refused.message.contains("p-gone"), "{refused:?}");
        assert_eq!(format!("{backends:?}"), "Backends { count: 0 }");
    }

    #[test]
    fn log_events_wait_for_the_next_answer_and_an_overflow_is_counted() {
        let log = Buffered::default();
        for _ in 0..LOG_LIMIT + 3 {
            log.log(AiLogEvent::new("ai.attempt"));
        }
        let drained = log.drain();
        assert_eq!(drained.len(), LOG_LIMIT + 1);
        let lost = drained.last().unwrap();
        assert_eq!(lost.event, "ai.log.dropped");
        assert_eq!(lost.detail.as_deref(), Some("count=3"));
        assert!(log.drain().is_empty());
    }
}
