//! The ⌘P file index (PRD S3 B12, D-05, D-08; S5.5 B5-B6).
//!
//! One index per checkout root on each device, built lazily the first time the
//! operator opens the palette there, or for each project's primary checkout
//! the first time the search across every project runs (`search`, PRD
//! desk-2-ui B38), and kept until the root leaves the catalog. The
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
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Matches the palette shows.
pub const RESULT_LIMIT: usize = 80;

/// Walks the search of every project (`search`) keeps running at once, this
/// machine's and every device's together; a project past it answers
/// `indexing` and the palette, which polls while any project does, asks again
/// once one ends. A query for one checkout (`query`) is not counted against it.
pub const SEARCH_WALKS_MAX: usize = 4;

/// Of `SEARCH_WALKS_MAX`, the walks on one device at once: its link serves
/// four machine calls at a time (`hide-host/src/serve.rs`), so two leave two
/// for the operator's own file lists, opens and saves on that device.
pub const SEARCH_DEVICE_WALKS_MAX: usize = 2;

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
    /// The walk could not be made, with the reason; a query `STALE_AFTER`
    /// after it walks again.
    Failed(String),
}

/// One index: a device and a root path on it.
pub type IndexKey = (String, String);

/// Where one index stands. A walk's end and its result are one change under
/// one lock: a query never sees a finished walk that is still building, which
/// would answer `Indexing` and start nothing.
enum State {
    /// Never walked, or a walk that ended without a result: the next query
    /// walks.
    Empty,
    /// A walk is running; `Some` is the older list it replaces, still served.
    Walking(Option<Arc<Walked>>),
    Walked {
        walked: Arc<Walked>,
        at: Instant,
    },
    /// A first walk failed: answered as failed until `STALE_AFTER` has
    /// passed, so a device that is away is asked once per that long and not
    /// on every keystroke.
    Failed {
        message: String,
        at: Instant,
    },
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
    /// Counts this walk among its device's while it runs.
    _running: Running,
}

/// The walks running now, by device; a walk holds its count until it ends,
/// whether it finished, failed, panicked or never started.
#[derive(Clone, Default)]
struct Walking(Arc<Mutex<HashMap<String, usize>>>);

impl Walking {
    fn counts(&self) -> MutexGuard<'_, HashMap<String, usize>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn start(&self, device: &str) -> Running {
        *self.counts().entry(device.to_owned()).or_default() += 1;
        Running {
            walking: self.clone(),
            device: device.to_owned(),
        }
    }

    /// Whether the search may start one more walk on `device`.
    fn room_for(&self, device: &str) -> bool {
        let counts = self.counts();
        counts.values().sum::<usize>() < SEARCH_WALKS_MAX
            && counts.get(device).copied().unwrap_or(0) < SEARCH_DEVICE_WALKS_MAX
    }
}

struct Running {
    walking: Walking,
    device: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        let mut counts = self.walking.counts();
        if let Some(count) = counts.get_mut(&self.device) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.device);
            }
        }
    }
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
            (Err(message), None) => State::Failed {
                message,
                at: Instant::now(),
            },
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
    /// The roots the catalog carried at the last `set_roots`, so one that
    /// came or went is seen.
    roots: Mutex<HashSet<IndexKey>>,
    /// One root per project, the ones `search` asks (`set_projects`).
    projects: Mutex<Vec<IndexKey>>,
    walking: Walking,
}

impl IndexService {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            roots: Mutex::new(HashSet::new()),
            projects: Mutex::new(Vec::new()),
            walking: Walking::default(),
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
            if let State::Walked { at, .. } | State::Failed { at, .. } = &mut *entry.state() {
                *at = at.checked_sub(by).expect("an instant that far back");
            }
        }
    }

    /// Keeps only the roots still in the catalog, so a removed checkout's
    /// index does not live on. A root that came or went also drops the index
    /// of every root it holds or sits in on that device: a home project's list
    /// leaves out the projects registered inside it, so a project added or
    /// removed there makes it stale, and a walk still running for it ends
    /// into an entry nothing reads (PRD home-folder-project D-05).
    pub fn set_roots(&self, roots: &[IndexKey]) {
        let now: HashSet<IndexKey> = roots.iter().cloned().collect();
        let changed: Vec<IndexKey> = {
            let mut last = self
                .roots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let changed = last.symmetric_difference(&now).cloned().collect();
            *last = now;
            changed
        };
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.retain(|key, _| {
            roots.contains(key)
                && !changed.iter().any(|(device, root)| {
                    *device == key.0
                        && key.1 != *root
                        && (Path::new(root).starts_with(&key.1)
                            || Path::new(&key.1).starts_with(root))
                })
        });
    }

    /// The root of each project the catalog carries, one per project and
    /// none twice, in the catalog's order: what `search` asks.
    pub fn set_projects(&self, projects: Vec<IndexKey>) {
        let mut seen = HashSet::new();
        let projects = projects
            .into_iter()
            .filter(|key| seen.insert(key.clone()))
            .collect();
        *self
            .projects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = projects;
    }

    /// The ranked matches for one root of one device. The first call starts
    /// `walk` on its own thread and answers `Indexing`; a list older than
    /// `STALE_AFTER` is answered as refreshing while `walk` replaces it, and
    /// stays when that walk fails (it is logged); a first walk that fails is
    /// answered as failed until `STALE_AFTER` has passed, and then walked
    /// again.
    pub fn query(
        &self,
        device: &str,
        root: &str,
        query: &str,
        walk: impl FnOnce() -> Result<Walked, String> + Send + 'static,
    ) -> IndexAnswer {
        match self.lookup(&(device.to_owned(), root.to_owned()), true, || -> WalkFn {
            Box::new(walk)
        }) {
            Lookup::Indexing => IndexAnswer::Indexing,
            Lookup::Failed(message) => IndexAnswer::Failed(message),
            Lookup::Ready { walked, refreshing } => IndexAnswer::Ready {
                entries: rank(&walked.paths, query, RESULT_LIMIT)
                    .into_iter()
                    .map(|(path, _)| path)
                    .collect(),
                truncated: walked.truncated,
                refreshing,
            },
        }
    }

    /// The palette's search across every project at once (PRD desk-2-ui
    /// B38): each project's root is asked of its own index as `query` asks
    /// it, but a walk starts only while fewer than `SEARCH_WALKS_MAX` run,
    /// and fewer than `SEARCH_DEVICE_WALKS_MAX` on its device; a project
    /// left waiting answers `indexing` and the next poll asks again, and a
    /// stale list is served until there is room to refresh it. `walk_for`
    /// makes a project's walk, and is called only for one that starts.
    /// Each project's list is ranked, then `merge` keeps `limit` of them.
    pub fn search(
        &self,
        query: &str,
        prefer: Option<&IndexKey>,
        limit: usize,
        walk_for: impl Fn(&IndexKey) -> WalkFn,
    ) -> Searched {
        let projects = self
            .projects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let mut searched = Searched::default();
        let mut lists = Vec::new();
        for key in projects {
            let room = self.walking.room_for(&key.0);
            match self.lookup(&key, room, || walk_for(&key)) {
                Lookup::Indexing => searched.indexing = true,
                Lookup::Failed(message) => searched.unavailable.push((key, message)),
                Lookup::Ready { walked, refreshing } => {
                    searched.indexing |= refreshing;
                    searched.index_truncated |= walked.truncated;
                    // One past the limit, so a project with more matches
                    // than it keeps says so.
                    lists.push((key, rank(&walked.paths, query, limit + 1)));
                }
            }
        }
        let (hits, more) = merge(lists, prefer, limit);
        searched.hits = hits;
        searched.more = more;
        searched
    }

    /// Where one root's index stands, starting its walk when `may_walk` and
    /// it needs one: never walked, stale, or failed `STALE_AFTER` ago.
    fn lookup(&self, key: &IndexKey, may_walk: bool, walk: impl FnOnce() -> WalkFn) -> Lookup {
        let entry = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Arc::clone(entries.entry(key.clone()).or_insert_with(|| {
                Arc::new(Entry {
                    state: Mutex::new(State::Empty),
                })
            }))
        };
        let mut state = entry.state();
        // Taken out and put back unless the answer moves it on.
        match std::mem::replace(&mut *state, State::Empty) {
            State::Walked { walked, at } if at.elapsed() < STALE_AFTER || !may_walk => {
                *state = State::Walked {
                    walked: Arc::clone(&walked),
                    at,
                };
                Lookup::Ready {
                    walked,
                    refreshing: false,
                }
            }
            State::Walked { walked, .. } => {
                *state = State::Walking(Some(Arc::clone(&walked)));
                drop(state);
                self.start_walk(&entry, key.clone(), Some(Arc::clone(&walked)), walk());
                Lookup::Ready {
                    walked,
                    refreshing: true,
                }
            }
            State::Walking(older) => {
                *state = State::Walking(older.clone());
                match older {
                    Some(walked) => Lookup::Ready {
                        walked,
                        refreshing: true,
                    },
                    None => Lookup::Indexing,
                }
            }
            State::Failed { message, at } if at.elapsed() < STALE_AFTER || !may_walk => {
                *state = State::Failed {
                    message: message.clone(),
                    at,
                };
                Lookup::Failed(message)
            }
            State::Failed { .. } | State::Empty if may_walk => {
                *state = State::Walking(None);
                drop(state);
                self.start_walk(&entry, key.clone(), None, walk());
                Lookup::Indexing
            }
            State::Failed { .. } | State::Empty => Lookup::Indexing,
        }
    }

    /// Starts the one walk an entry is `Walking` for, on its own thread.
    fn start_walk(
        &self,
        entry: &Arc<Entry>,
        key: IndexKey,
        older: Option<Arc<Walked>>,
        walk: WalkFn,
    ) {
        let walk_of_entry = Walk {
            entry: Some(Arc::clone(entry)),
            older,
            _running: self.walking.start(&key.0),
            key,
        };
        let _ = std::thread::Builder::new()
            .name("hided-index".to_owned())
            .spawn(move || {
                let result = walk();
                walk_of_entry.finish(result);
            });
    }
}

/// A walk of one root, run on its own thread.
pub type WalkFn = Box<dyn FnOnce() -> Result<Walked, String> + Send>;

/// Where one root's index stands for a query.
enum Lookup {
    Indexing,
    Ready {
        walked: Arc<Walked>,
        refreshing: bool,
    },
    Failed(String),
}

/// One match of the search across every project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub project: IndexKey,
    pub relative_path: String,
}

/// The answer of `IndexService::search`.
#[derive(Debug, Default)]
pub struct Searched {
    /// The preferred project's matches first, then the others by score.
    pub hits: Vec<Hit>,
    /// Some project matched more than `hits` holds.
    pub more: bool,
    /// Some project's index stopped at `hide_host::index::INDEX_CAP`.
    pub index_truncated: bool,
    /// Some project's walk has not landed, or is refreshing.
    pub indexing: bool,
    /// The projects whose walk failed, with why.
    pub unavailable: Vec<(IndexKey, String)>,
}

/// Keeps `limit` of every project's ranked matches (each best first, cut at
/// `limit + 1`): the preferred project's first, up to half the limit while
/// the others have matches to fill the rest, then the others' best by score,
/// a tie by name. Answers whether any match was left out.
pub fn merge(
    lists: Vec<(IndexKey, Vec<(String, i64)>)>,
    prefer: Option<&IndexKey>,
    limit: usize,
) -> (Vec<Hit>, bool) {
    let matched: usize = lists.iter().map(|(_, list)| list.len()).sum();
    let (preferred, others): (Vec<_>, Vec<_>) =
        lists.into_iter().partition(|(key, _)| Some(key) == prefer);
    let mut preferred: Vec<Hit> = preferred
        .into_iter()
        .flat_map(|(project, list)| {
            list.into_iter().map(move |(relative_path, _)| Hit {
                project: project.clone(),
                relative_path,
            })
        })
        .collect();
    let mut others: Vec<(i64, Hit)> = others
        .into_iter()
        .flat_map(|(project, list)| {
            list.into_iter().map(move |(relative_path, score)| {
                (
                    score,
                    Hit {
                        project: project.clone(),
                        relative_path,
                    },
                )
            })
        })
        .collect();
    others.sort_by(|left, right| {
        right.0.cmp(&left.0).then_with(|| {
            hide_host::list::natural_cmp(&left.1.relative_path, &right.1.relative_path)
        })
    });
    let first = preferred
        .len()
        .min((limit / 2).max(limit.saturating_sub(others.len())));
    preferred.truncate(first);
    others.truncate(limit - preferred.len());
    let mut hits = preferred;
    hits.extend(others.into_iter().map(|(_, hit)| hit));
    let more = matched > hits.len();
    (hits, more)
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
    fn a_project_added_or_removed_inside_a_root_drops_that_roots_list_only() {
        let service = IndexService::new();
        let key = |device: &str, root: &str| (device.to_owned(), root.to_owned());
        fn walked(name: &str) -> Walked {
            Walked {
                paths: vec![name.to_owned()],
                truncated: false,
            }
        }
        let home = key("mac", "/home/me");
        let other = key("mac", "/srv/other");
        let elsewhere = key("mini", "/home/me");
        service.set_roots(&[home.clone(), other.clone(), elsewhere.clone()]);
        for (device, root) in [&home, &other, &elsewhere] {
            let _ = service.query(device, root, "", || Ok(walked("old")));
        }
        let held = |service: &IndexService, (device, root): &IndexKey| {
            let entries = service.entries.lock().unwrap();
            entries.contains_key(&(device.clone(), root.clone()))
        };

        // A project registered inside home on the same device.
        let inside = key("mac", "/home/me/work/app");
        service.set_roots(&[
            home.clone(),
            other.clone(),
            elsewhere.clone(),
            inside.clone(),
        ]);
        assert!(!held(&service, &home));
        assert!(held(&service, &other));
        assert!(held(&service, &elsewhere));

        // The same root set again changes nothing.
        let _ = service.query("mac", "/home/me", "", || Ok(walked("new")));
        service.set_roots(&[home.clone(), other.clone(), elsewhere.clone(), inside]);
        assert!(held(&service, &home));
    }

    #[test]
    fn a_failed_walk_is_answered_until_it_is_stale_and_then_walked_again() {
        let service = IndexService::new();
        let failed = answer_after_walk(&service, Err("helper gone".to_owned()));
        assert!(matches!(failed, IndexAnswer::Failed(message) if message == "helper gone"));
        // A device that is away is not asked again on every keystroke.
        let held = service.query("device", "/root", "", || panic!("walked again at once"));
        assert!(matches!(held, IndexAnswer::Failed(message) if message == "helper gone"));
        service.age(STALE_AFTER);
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

    fn key(device: &str, root: &str) -> IndexKey {
        (device.to_owned(), root.to_owned())
    }

    fn ranked(paths: &[(&str, i64)]) -> Vec<(String, i64)> {
        paths
            .iter()
            .map(|(path, score)| ((*path).to_owned(), *score))
            .collect()
    }

    fn shown(hits: &[Hit]) -> Vec<(&str, &str)> {
        hits.iter()
            .map(|hit| (hit.project.1.as_str(), hit.relative_path.as_str()))
            .collect()
    }

    #[test]
    fn the_merge_orders_every_projects_matches_by_score_and_says_when_some_were_left_out() {
        let lists = vec![
            (key("mac", "/a"), ranked(&[("a1", 90), ("a2", 10)])),
            (key("mac", "/b"), ranked(&[("b1", 95), ("b2", 50)])),
        ];
        let (hits, more) = merge(lists.clone(), None, 4);
        assert_eq!(
            shown(&hits),
            [("/b", "b1"), ("/a", "a1"), ("/b", "b2"), ("/a", "a2")]
        );
        assert!(!more);
        let (hits, more) = merge(lists, None, 3);
        assert_eq!(shown(&hits), [("/b", "b1"), ("/a", "a1"), ("/b", "b2")]);
        assert!(more, "a match past the limit is left out and said");
        // One project cut at limit + 1 by its own ranking says more too.
        let (hits, more) = merge(
            vec![(key("mac", "/a"), ranked(&[("a1", 3), ("a2", 2), ("a3", 1)]))],
            None,
            2,
        );
        assert_eq!(hits.len(), 2);
        assert!(more);
    }

    #[test]
    fn the_preferred_project_comes_first_with_half_the_limit_while_others_match() {
        let prefer = key("mac", "/a");
        let lists = vec![
            (
                prefer.clone(),
                ranked(&[("a1", 1), ("a2", 1), ("a3", 1), ("a4", 1)]),
            ),
            (
                key("mini", "/b"),
                ranked(&[("b1", 99), ("b2", 98), ("b3", 97)]),
            ),
        ];
        let (hits, more) = merge(lists.clone(), Some(&prefer), 4);
        assert_eq!(
            shown(&hits),
            [("/a", "a1"), ("/a", "a2"), ("/b", "b1"), ("/b", "b2")]
        );
        assert!(more);
        // With nothing elsewhere the preferred project fills the limit.
        let (hits, _) = merge(vec![lists[0].clone()], Some(&prefer), 4);
        assert_eq!(hits.len(), 4);
    }

    /// Answers `search` until no project is indexing, bounded by a deadline.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn searched(service: &IndexService, walk_for: impl Fn(&IndexKey) -> WalkFn) -> Searched {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let answer = service.search("", None, RESULT_LIMIT, &walk_for);
            if !answer.indexing {
                return answer;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the walks never landed"
            );
            std::thread::sleep(std::time::Duration::from_millis(5)); // wait-ok: every project's walk landed
        }
    }

    #[test]
    fn the_search_asks_each_project_once_and_names_the_ones_whose_walk_failed() {
        let service = IndexService::new();
        service.set_projects(vec![
            key("mac", "/a"),
            key("mini", "/b"),
            key("mac", "/a"),
            key("away", "/c"),
        ]);
        let answer = searched(&service, |(device, root)| {
            let (device, root) = (device.clone(), root.clone());
            Box::new(move || match device.as_str() {
                "away" => Err("link.not_connected".to_owned()),
                _ => Ok(walked(&[&format!("{}.txt", &root[1..])])),
            })
        });
        assert_eq!(
            shown(&answer.hits),
            [("/a", "a.txt"), ("/b", "b.txt")],
            "each project once"
        );
        assert_eq!(
            answer.unavailable,
            [(key("away", "/c"), "link.not_connected".to_owned())]
        );
        assert!(!answer.more && !answer.index_truncated);
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // polls the walk counts, bounded by a deadline
    fn the_search_runs_at_most_its_walks_at_once_and_two_on_one_device() {
        let service = IndexService::new();
        let projects: Vec<IndexKey> = (0..3)
            .map(|index| key("mini", &format!("/m{index}")))
            .chain((0..4).map(|index| key("mac", &format!("/l{index}"))))
            .collect();
        service.set_projects(projects.clone());
        let (release, held) = std::sync::mpsc::channel::<()>();
        let held = Arc::new(Mutex::new(held));
        let started = Arc::new(Mutex::new(Vec::new()));
        let walk_for = |project: &IndexKey| -> WalkFn {
            started.lock().unwrap().push(project.clone());
            let held = Arc::clone(&held);
            Box::new(move || {
                let _ = held.lock().unwrap().recv();
                Ok(walked(&["x"]))
            })
        };
        let first = service.search("", None, RESULT_LIMIT, walk_for);
        assert!(first.indexing);
        let started_now = started.lock().unwrap().clone();
        assert_eq!(started_now.len(), SEARCH_WALKS_MAX);
        assert_eq!(
            started_now
                .iter()
                .filter(|(device, _)| device == "mini")
                .count(),
            SEARCH_DEVICE_WALKS_MAX
        );
        // A second query while they run starts nothing.
        service.search("", None, RESULT_LIMIT, walk_for);
        assert_eq!(started.lock().unwrap().len(), SEARCH_WALKS_MAX);
        // As walks end the rest start, and every project lands.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let _ = release.send(());
            let answer = service.search("", None, RESULT_LIMIT, walk_for);
            if !answer.indexing {
                assert_eq!(answer.hits.len(), projects.len());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the search never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(5)); // wait-ok: the released walks landed
        }
        assert_eq!(
            started.lock().unwrap().len(),
            projects.len(),
            "each project walked once"
        );
    }
}
