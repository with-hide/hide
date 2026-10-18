//! The ⌘P file index (PRD S3 B12, D-05, D-08; S5.5 B5-B6).
//!
//! One index per checkout root on each device, built lazily the first time the
//! operator opens the palette and kept until the root leaves the catalog. The
//! walk is `hide_host::index::walk`: this machine's runs in process on a
//! worker, a device's is its helper's `Call::Index`. The list is capped at
//! `hide_host::index::INDEX_CAP` and the palette says when it is truncated.
//!
//! A walked index follows the files that change by age: a query that finds the
//! list `STALE_AFTER` old answers it at once, marked refreshing, and starts one
//! new walk, so a file made after the first walk shows up on the next answer
//! without a walk per keystroke. The watcher cannot carry this: it stamps the
//! Explorer's at most 64 folders, never a whole checkout.
//!
//! Ranking is the web shell's (`web/src/search.ts`, `fuzzyScore`):
//! the query's characters must appear in order, an earlier match scores higher,
//! adjacent and word-boundary matches score higher, and a shorter path wins a
//! tie. The result limit is the palette's 80.

use hide_host::index::Walked;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Matches the palette shows.
pub const RESULT_LIMIT: usize = 80;

/// How old a walked list may get before a query starts a new walk. It bounds
/// the work to one walk per root per this long however fast the operator
/// types, and it is short enough that a file made a moment ago is there the
/// next time the palette opens.
pub const STALE_AFTER: Duration = Duration::from_secs(5);

/// The fuzzy score of `query` against `candidate`, or `None` when the query's
/// characters are not found in order. `candidate` is lowercased by the caller
/// and `query` is already trimmed and lowercased, exactly as the web shell
/// prepares them, so the two scorers agree.
pub fn fuzzy_score(candidate: &str, query: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = candidate.chars().collect();
    let mut cursor = 0usize;
    let mut score: i64 = 0;
    let mut previous: Option<usize> = None;
    for wanted in query.chars() {
        let found = chars[cursor..].iter().position(|c| *c == wanted)? + cursor;
        let offset = found;
        score += 100 - offset.min(90) as i64;
        if previous.is_some_and(|last| last + 1 == found) {
            score += 35;
        }
        if found == 0 || matches!(chars[found - 1], '/' | '_' | '-' | ' ' | '.') {
            score += 25;
        }
        previous = Some(found);
        cursor = found + 1;
    }
    score -= chars.len() as i64;
    Some(score)
}

/// The ranked matches for `query`, best first. An empty query lists the index
/// as it is (the walk's natural order), which is what the palette shows.
pub fn rank(paths: &[String], query: &str, limit: usize) -> Vec<(String, i64)> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return paths
            .iter()
            .take(limit)
            .map(|path| (path.clone(), 0))
            .collect();
    }
    let mut scored: Vec<(String, i64)> = paths
        .iter()
        .filter_map(|path| {
            fuzzy_score(&path.to_lowercase(), &needle).map(|score| (path.clone(), score))
        })
        .collect();
    scored.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| hide_host::list::natural_cmp(&left.0, &right.0))
    });
    scored.truncate(limit);
    scored
}

/// The answer to one palette query.
pub enum IndexAnswer {
    /// The first request kicked a build; the next one has the list.
    Indexing,
    /// The list. `refreshing` marks one old enough that a new walk is running
    /// for it; the palette asks again until the answer drops the mark.
    Ready {
        entries: Vec<String>,
        truncated: bool,
        refreshing: bool,
    },
    /// The walk could not be made, with the reason; the next query tries again.
    Failed(String),
}

/// One index: a device and a root path on it.
type IndexKey = (String, String);

/// Where one index stands. A walk's end and its result are one change under
/// one lock: a query never sees a finished walk that is still building, which
/// would answer `Indexing` and start nothing.
enum State {
    /// Never walked, or a failure already answered: the next query walks.
    Empty,
    /// A walk is running; `Some` is the older list it replaces, still served.
    Walking(Option<Arc<Walked>>),
    Walked {
        walked: Arc<Walked>,
        at: Instant,
    },
    Failed(String),
}

struct Entry {
    state: Mutex<State>,
}

impl Entry {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The one walk an entry is `Walking` for. Its result is stored with
/// `finish`; a walk that panics, or a thread that never started, drops it
/// unfinished, which leaves the entry `Empty` so the next query walks again
/// instead of answering `Indexing` forever.
struct Walk {
    entry: Option<Arc<Entry>>,
    /// The list a refresh replaces; it stays when the refresh does not land.
    older: Option<Arc<Walked>>,
    key: IndexKey,
}

impl Walk {
    fn finish(mut self, result: Result<Walked, String>) {
        let Some(entry) = self.entry.take() else {
            return;
        };
        *entry.state() = match (result, self.older.take()) {
            (Ok(walked), _) => State::Walked {
                walked: Arc::new(walked),
                at: Instant::now(),
            },
            // A list the palette was showing is not thrown away by a refresh
            // that failed: it is kept as if just walked, so the next refresh
            // comes after another `STALE_AFTER` rather than on every query.
            (Err(message), Some(older)) => {
                self.log("index.refresh_failed", &message);
                State::Walked {
                    walked: older,
                    at: Instant::now(),
                }
            }
            (Err(message), None) => State::Failed(message),
        };
    }

    fn log(&self, kind: &str, message: &str) {
        eprintln!(
            "{}",
            serde_json::json!({
                "component": "hided",
                "kind": kind,
                "device": self.key.0,
                "root": self.key.1,
                "message": message,
            })
        );
    }
}

impl Drop for Walk {
    fn drop(&mut self) {
        let Some(entry) = self.entry.take() else {
            return;
        };
        // A walk that panicked or never started: a refresh keeps the list it
        // replaced, a first walk leaves the entry to walk again.
        *entry.state() = match self.older.take() {
            Some(older) => {
                self.log("index.refresh_abandoned", "the walk ended without a result");
                State::Walked {
                    walked: older,
                    at: Instant::now(),
                }
            }
            None => State::Empty,
        };
    }
}

/// The daemon's index cache: one lazy index per checkout root of each device,
/// bounded to the roots the catalog currently carries (engineering 15). A root
/// that is removed drops its index on the next reconcile.
pub struct IndexService {
    entries: Mutex<HashMap<IndexKey, Arc<Entry>>>,
}

impl IndexService {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Makes every walked list `by` older, so a test crosses `STALE_AFTER`
    /// without waiting for it.
    #[cfg(test)]
    fn age(&self, by: Duration) {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for entry in entries.values() {
            if let State::Walked { at, .. } = &mut *entry.state() {
                *at = at.checked_sub(by).expect("an instant that far back");
            }
        }
    }

    /// Keeps only the roots still in the catalog, so a removed checkout's
    /// index does not live on.
    pub fn set_roots(&self, roots: &[IndexKey]) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.retain(|key, _| roots.contains(key));
    }

    /// The ranked matches for one root of one device. The first call starts
    /// `walk` on its own thread and answers `Indexing`; a list older than
    /// `STALE_AFTER` is answered as refreshing while `walk` replaces it, and
    /// stays when that walk fails (it is logged); a first walk that fails is
    /// answered once and then forgotten, so the next query walks again.
    pub fn query(
        &self,
        device: &str,
        root: &str,
        query: &str,
        walk: impl FnOnce() -> Result<Walked, String> + Send + 'static,
    ) -> IndexAnswer {
        let entry = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Arc::clone(
                entries
                    .entry((device.to_owned(), root.to_owned()))
                    .or_insert_with(|| {
                        Arc::new(Entry {
                            state: Mutex::new(State::Empty),
                        })
                    }),
            )
        };
        let (walked, refreshing) = {
            let mut state = entry.state();
            // Taken out and put back unless the answer moves it on: a failure
            // is answered once and leaves the entry `Empty`.
            match std::mem::replace(&mut *state, State::Empty) {
                State::Walked { walked, at } if at.elapsed() < STALE_AFTER => {
                    *state = State::Walked {
                        walked: Arc::clone(&walked),
                        at,
                    };
                    (walked, false)
                }
                State::Walked { walked, .. } => {
                    *state = State::Walking(Some(Arc::clone(&walked)));
                    drop(state);
                    start_walk(
                        &entry,
                        (device.to_owned(), root.to_owned()),
                        Some(Arc::clone(&walked)),
                        walk,
                    );
                    (walked, true)
                }
                State::Walking(older) => {
                    *state = State::Walking(older.clone());
                    match older {
                        Some(walked) => (walked, true),
                        None => return IndexAnswer::Indexing,
                    }
                }
                State::Failed(message) => return IndexAnswer::Failed(message),
                State::Empty => {
                    *state = State::Walking(None);
                    drop(state);
                    start_walk(&entry, (device.to_owned(), root.to_owned()), None, walk);
                    return IndexAnswer::Indexing;
                }
            }
        };
        IndexAnswer::Ready {
            entries: rank(&walked.paths, query, RESULT_LIMIT)
                .into_iter()
                .map(|(path, _)| path)
                .collect(),
            truncated: walked.truncated,
            refreshing,
        }
    }
}

/// Starts the one walk an entry is `Walking` for, on its own thread.
fn start_walk(
    entry: &Arc<Entry>,
    key: IndexKey,
    older: Option<Arc<Walked>>,
    walk: impl FnOnce() -> Result<Walked, String> + Send + 'static,
) {
    let walk_of_entry = Walk {
        entry: Some(Arc::clone(entry)),
        older,
        key,
    };
    let _ = std::thread::Builder::new()
        .name("hided-index".to_owned())
        .spawn(move || {
            let result = walk();
            walk_of_entry.finish(result);
        });
}

impl Default for IndexService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn answer_after_walk(service: &IndexService, walk: Result<Walked, String>) -> IndexAnswer {
        assert!(matches!(
            service.query("device", "/root", "", move || walk),
            IndexAnswer::Indexing
        ));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let answer = service.query("device", "/root", "", || Ok(Walked::default()));
            if !matches!(answer, IndexAnswer::Indexing) {
                return answer;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the walk never landed"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_failed_walk_is_reported_once_and_walked_again() {
        let service = IndexService::new();
        let failed = answer_after_walk(&service, Err("helper gone".to_owned()));
        assert!(matches!(failed, IndexAnswer::Failed(message) if message == "helper gone"));
        let again = answer_after_walk(
            &service,
            Ok(Walked {
                paths: vec!["a.txt".to_owned()],
                truncated: false,
            }),
        );
        assert!(matches!(again, IndexAnswer::Ready { entries, .. } if entries == ["a.txt"]));
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // polls the index state, bounded by a deadline
    fn a_walk_that_panics_leaves_the_index_to_walk_again() {
        let service = IndexService::new();
        assert!(matches!(
            service.query("device", "/root", "", || panic!("the walker crashed")),
            IndexAnswer::Indexing
        ));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let answer = service.query("device", "/root", "", || {
                Ok(Walked {
                    paths: vec!["a.txt".to_owned()],
                    truncated: false,
                })
            });
            if let IndexAnswer::Ready { entries, .. } = answer {
                assert_eq!(entries, ["a.txt"]);
                return;
            }
            assert!(
                matches!(answer, IndexAnswer::Indexing),
                "a crashed walk is not a failure to report"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "the index never walked again"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn walked(paths: &[&str]) -> Walked {
        Walked {
            paths: paths.iter().map(|path| (*path).to_owned()).collect(),
            truncated: false,
        }
    }

    /// The service with `paths` walked and the walk landed.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn walked_service(paths: &'static [&'static str]) -> IndexService {
        let service = IndexService::new();
        let answer = answer_after_walk(&service, Ok(walked(paths)));
        assert!(matches!(answer, IndexAnswer::Ready { .. }));
        service
    }

    /// Asks until the list is no longer refreshing, bounded by a deadline.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn settled(service: &IndexService, query: &str) -> Vec<String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let answer = service.query("device", "/root", query, || {
                panic!("a fresh list is not walked again")
            });
            if let IndexAnswer::Ready {
                entries,
                refreshing: false,
                ..
            } = answer
            {
                return entries;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the index never settled"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_stale_list_is_answered_at_once_and_replaced_by_a_new_walk() {
        let service = walked_service(&["app.js"]);
        service.age(STALE_AFTER);
        // A file made after the first walk: the old list is answered at once,
        // marked refreshing, and the walk it started has the new file.
        let answer = service.query("device", "/root", "", || {
            Ok(walked(&["app.js", "zqnew_after_index.txt"]))
        });
        assert!(
            matches!(&answer, IndexAnswer::Ready { entries, refreshing: true, .. } if *entries == ["app.js"]),
            "the stale list is served while it refreshes"
        );
        assert_eq!(settled(&service, "zqnew"), ["zqnew_after_index.txt"]);
    }

    #[test]
    fn a_refresh_that_fails_or_panics_keeps_the_list_it_replaced() {
        let service = walked_service(&["app.js"]);
        service.age(STALE_AFTER);
        service.query("device", "/root", "", || Err("helper gone".to_owned()));
        // The failed refresh lands as a list just walked: still served, and
        // not walked again until it is stale once more.
        assert_eq!(settled(&service, ""), ["app.js"]);
        service.age(STALE_AFTER);
        service.query("device", "/root", "", || panic!("the walker crashed"));
        assert_eq!(settled(&service, ""), ["app.js"]);
        // A refresh after that still lands.
        service.age(STALE_AFTER);
        service.query("device", "/root", "", || Ok(walked(&["app.js", "b.js"])));
        assert_eq!(settled(&service, ""), ["app.js", "b.js"]);
    }

    #[test]
    fn a_fresh_list_starts_no_walk() {
        let service = walked_service(&["app.js"]);
        let answer = service.query("device", "/root", "", || {
            panic!("a fresh list is not walked again")
        });
        assert!(
            matches!(&answer, IndexAnswer::Ready { entries, refreshing: false, .. } if *entries == ["app.js"])
        );
    }

    #[test]
    fn a_refreshing_list_runs_one_walk_however_often_it_is_asked() {
        let service = walked_service(&["a"]);
        service.age(STALE_AFTER);
        let (release, held) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&started);
        service.query("device", "/root", "", move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = held.recv();
            Ok(walked(&["a", "b"]))
        });
        for _ in 0..3 {
            let answer = service.query("device", "/root", "", || {
                panic!("a second walk started while one runs")
            });
            assert!(matches!(
                answer,
                IndexAnswer::Ready {
                    refreshing: true,
                    ..
                }
            ));
        }
        release.send(()).unwrap();
        assert_eq!(settled(&service, ""), ["a", "b"]);
        assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn the_scorer_agrees_with_the_web_shells() {
        // The characters have to appear in order.
        assert_eq!(fuzzy_score("src/main.rs", "nope"), None);
        assert!(fuzzy_score("src/main.rs", "smr").is_some());
        // An adjacent and word-boundary match beats a scattered one.
        let tight = fuzzy_score("src/main.rs", "main").unwrap();
        let loose = fuzzy_score("server/migrations/init.rs", "main").unwrap();
        assert!(tight > loose, "{tight} > {loose}");
        // A prefix match scores higher than a late one.
        assert!(
            fuzzy_score("readme.md", "r").unwrap() > fuzzy_score("docs/readme.md", "r").unwrap()
        );
    }

    #[test]
    fn ranking_orders_by_score_then_name_and_limits_the_list() {
        let paths = vec![
            "src/main.rs".to_owned(),
            "src/mnemonic.rs".to_owned(),
            "docs/readme.md".to_owned(),
        ];
        let ranked = rank(&paths, "main", 80);
        assert_eq!(ranked[0].0, "src/main.rs");
        assert!(!ranked.iter().any(|(path, _)| path == "docs/readme.md"));
        let limited = rank(&paths, "", 2);
        assert_eq!(
            limited.len(),
            2,
            "an empty query lists the index up to the limit"
        );
    }
}
