//! Each holder pane's facts, followed for the core's own engine (PRD
//! engine-bundle-3-core-owns D-18, D-21): one watch connection per pane the
//! node started or found running, beside its screen connection and never
//! counted against the attach cap, so a pane no screen shows is followed as
//! one that is shown.
//!
//! A watch is read on a thread of its own, which tells the engine's sink the
//! pane's facts each time the holder sends them. The node keeps each pane's
//! screen rules and gives its holder the first rule set its evaluator can
//! read; when none fits, the pane's screen reads [`NO_RULES_FOR_HOLDER`]. A
//! holder answers each rule set with its facts, so the sink hears the pane
//! read by its new rules from the watch's own thread, in order with every
//! other facts of the pane. A holder built before watches refuses the
//! opening, and its pane is told once that it reads [`UNSUPPORTED_HOLDER`].
//! A watch that ends any other way than with its pane (another watch took
//! it, the connection broke, the holder sent what a watch never carries)
//! tells the pane once that its screen reads [`WATCH_LOST`], until a later
//! start of it follows it again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::{
    Capability, CloseReason, Facts, HolderMessage, LineEditing, OpenError, ScreenRead as WireRead,
    ScreenState as WireState, WatchMessage, Watching, read_watch_holder, write_watch,
};
use hide_node_link::holders::{
    ForegroundProgram, HolderError, NO_RULES_FOR_HOLDER, PaneFacts, PaneFactsSink, ScreenRead,
    ScreenRules, ScreenState, UNSUPPORTED_HOLDER, WATCH_LOST,
};
use hide_platform::ipc::{LocalStream, ShutdownHandle};
use serde_json::json;

/// How long opening a watch may take, its connect and its answer: as long
/// as any connection to a holder.
use super::OPEN_LIMIT;

/// How long one rule set may take to reach a holder.
const WRITE_LIMIT: Duration = Duration::from_secs(5);

/// The watches of a node's holder panes, one entry per pane started, found
/// running or found too old, so no more than the holders the node keeps.
#[derive(Default)]
pub(super) struct Watches {
    panes: Mutex<BTreeMap<String, Watched>>,
    sink: Mutex<Option<PaneFactsSink>>,
    /// Held across each call of the sink, so the sink hears a pane's facts
    /// in the order they were read. [`Watches::set_rules`] never takes it,
    /// so a sink may give rules from inside its call.
    order: Mutex<()>,
    /// The last generation given a watch, of any pane: one is never given
    /// twice, so a watch of a pane forgotten and watched again, or retired
    /// for a renewal, changes nothing when it ends after the next one began.
    generations: Mutex<u64>,
}

/// One pane as its watch knows it.
#[derive(Default)]
struct Watched {
    /// No core decision yet, or its explicit rules/removal. Reconnecting
    /// must not turn an undecided value into a removal on a live holder.
    rules: Option<Option<ScreenRules>>,
    /// A thread follows the pane: opening its watch or reading it.
    following: bool,
    /// The watch open now, if one is.
    open: Option<Open>,
    /// The generation of the pane's latest watch, never one given before
    /// ([`Watches::generations`]), so the thread of an earlier one changes
    /// nothing.
    generation: u64,
    /// The facts the holder last sent, as it sent them, which a sink set
    /// later is told again.
    last: Option<PaneFacts>,
    /// No rule set fits the holder's evaluator.
    no_fit: bool,
}

struct Open {
    writer: Arc<Mutex<LocalStream>>,
    shutdown: ShutdownHandle,
    /// The holder's screen rule evaluator.
    engine: u32,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Watches {
    /// Tells `sink` each pane's facts from now on, and at once the last
    /// facts of every pane heard from before it.
    pub(super) fn set_sink(&self, sink: PaneFactsSink) {
        let _order = locked(&self.order);
        *locked(&self.sink) = Some(Arc::clone(&sink));
        let known: Vec<(String, PaneFacts)> = locked(&self.panes)
            .iter()
            .filter_map(|(pane, watched)| Some((pane.clone(), watched.told()?)))
            .collect();
        for (pane, facts) in known {
            sink(&pane, &facts);
        }
    }

    /// Changes what the node knows of `pane` by `change`, under the lock of
    /// its panes, and tells the sink the facts `change` answers, in order
    /// with every other pane's.
    fn deliver(
        &self,
        pane: &str,
        change: impl FnOnce(&mut BTreeMap<String, Watched>) -> Option<PaneFacts>,
    ) {
        let _order = locked(&self.order);
        let facts = change(&mut locked(&self.panes));
        let sink = locked(&self.sink).clone();
        if let (Some(sink), Some(facts)) = (sink, facts) {
            sink(pane, &facts);
        }
    }

    /// Follows `pane`'s holder at `socket` on a thread of its own, unless a
    /// thread already follows it.
    pub(super) fn watch(self: &Arc<Self>, pane: &str, socket: PathBuf) {
        let Some(generation) = self.begin(pane) else {
            return;
        };
        let watches = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("hide-node-pane-facts".into())
            .spawn({
                let pane = pane.to_owned();
                move || watches.follow(&pane, &socket, generation)
            });
        if let Err(error) = spawned {
            if let Some(watched) = locked(&self.panes).get_mut(pane) {
                watched.following = false;
            }
            crate::diagnostic!(json!({
                "component": "pane_holder",
                "kind": "pane_holder.watch_not_started",
                "pane_id": pane,
                "message": error.to_string(),
            }));
        }
    }

    /// The pane's holder, of `version`, sends no facts: a node too new to
    /// speak to it at all found it (`watch` none), or its watch was refused
    /// (`watch` the generation of that watch, still the pane's). Its pane is
    /// told so once.
    pub(super) fn unsupported(&self, pane: &str, watch: Option<u64>, version: u16) {
        crate::diagnostic!(json!({
            "component": "pane_holder",
            "kind": "pane_holder.facts_unsupported",
            "pane_id": pane,
            "holder_version": version,
        }));
        self.deliver(pane, |panes| {
            let watched = match watch {
                None => panes.entry(pane.to_owned()).or_default(),
                Some(generation) => panes
                    .get_mut(pane)
                    .filter(|watched| watched.generation == generation)?,
            };
            watched.no_fit = false;
            watched.last = Some(unsupported());
            watched.told()
        });
    }

    /// A thread is to follow `pane`: the generation of its watch, or `None`
    /// when a thread follows it already.
    fn begin(&self, pane: &str) -> Option<u64> {
        let mut panes = locked(&self.panes);
        let watched = panes.entry(pane.to_owned()).or_default();
        if watched.following {
            return None;
        }
        watched.following = true;
        watched.generation = self.next_generation();
        Some(watched.generation)
    }

    fn next_generation(&self) -> u64 {
        let mut last = locked(&self.generations);
        *last += 1;
        *last
    }

    /// The pane's holder is ended to be replaced (a renewal): its watch
    /// ends, what that watch's end says changes nothing of the pane, and its
    /// rules and last facts are kept for the next watch of it.
    pub(super) fn retire(&self, pane: &str) {
        let open = {
            let mut panes = locked(&self.panes);
            let Some(watched) = panes.get_mut(pane) else {
                return;
            };
            watched.following = false;
            watched.generation = self.next_generation();
            watched.open.take()
        };
        if let Some(open) = open {
            open.shutdown.shutdown();
        }
    }

    /// The pane ended or was forgotten: its watch ends and nothing of it is
    /// kept.
    pub(super) fn forget(&self, pane: &str) {
        let removed = locked(&self.panes).remove(pane);
        if let Some(open) = removed.and_then(|watched| watched.open) {
            open.shutdown.shutdown();
        }
    }

    /// Keeps `rules` for `pane` and gives its holder the first set it can
    /// read, when a watch is open; the holder answers with its facts, read
    /// by them. Never calls the sink.
    pub(super) fn set_rules(
        &self,
        pane: &str,
        rules: Option<ScreenRules>,
    ) -> Result<(), HolderError> {
        let send = {
            let mut panes = locked(&self.panes);
            let Some(watched) = panes.get_mut(pane) else {
                return Err(HolderError::NoPane {
                    message: format!("pane {pane} has no holder on this machine"),
                });
            };
            watched.rules = Some(rules);
            match &watched.open {
                Some(open) => {
                    let (manifest, no_fit) =
                        choose(watched.rules.as_ref().and_then(Option::as_ref), open.engine);
                    watched.no_fit = no_fit;
                    Some((Arc::clone(&open.writer), manifest))
                }
                None => None,
            }
        };
        let Some((writer, manifest)) = send else {
            return Ok(());
        };
        send_rules(&writer, manifest).map_err(|error| HolderError::Unreachable {
            message: format!("pane {pane}'s holder could not be given its screen rules: {error}"),
        })
    }

    /// One watch's thread: opens it, gives the holder its rules, and tells
    /// the sink each facts the holder sends until the watch ends, then what
    /// its end means for the pane.
    fn follow(&self, pane: &str, socket: &Path, generation: u64) {
        let ended = self.read_watch(pane, socket, generation);
        self.ended(pane, generation, ended);
    }

    /// What the end of the pane's watch of `generation` means for the pane;
    /// nothing, when a later watch of it began since.
    fn ended(&self, pane: &str, generation: u64, ended: Ended) {
        let current = locked(&self.panes)
            .get(pane)
            .is_some_and(|watched| watched.generation == generation);
        if let Ended::Lost(why) = &ended
            && current
        {
            crate::diagnostic!(json!({
                "component": "pane_holder",
                "kind": "pane_holder.watch_lost",
                "pane_id": pane,
                "message": why,
            }));
        }
        self.deliver(pane, |panes| {
            let watched = panes
                .get_mut(pane)
                .filter(|watched| watched.generation == generation)?;
            watched.following = false;
            watched.open = None;
            match ended {
                // A pane whose program ended is gone.
                Ended::Gone => {
                    panes.remove(pane);
                    None
                }
                // What the holder last said may have moved since: the pane
                // stays known, its screen read as unknown with why, and the
                // next start of it follows it again.
                Ended::Lost(_) => {
                    let mut last = watched.last.take().unwrap_or_default();
                    last.screen = ScreenRead::Unknown {
                        reason: WATCH_LOST.to_owned(),
                    };
                    watched.last = Some(last);
                    watched.no_fit = false;
                    watched.told()
                }
                Ended::Told => None,
            }
        });
    }

    /// Opens the watch and reads it until it ends: how it ended.
    fn read_watch(&self, pane: &str, socket: &Path, generation: u64) -> Ended {
        let opened = hide_holder_protocol::watch(
            socket,
            vec![
                Capability::Facts,
                Capability::ProgramPackage,
                Capability::LineEditing,
                Capability::ForegroundUnread,
                Capability::AlternateScreen,
            ],
            Instant::now() + OPEN_LIMIT,
        );
        if let Some(version) = sends_no_facts(&opened) {
            self.unsupported(pane, Some(generation), version);
            return Ended::Told;
        }
        let (mut stream, watching) = match opened {
            Ok(opened) => opened,
            Err(OpenError::Refused(closed)) => return Ended::of(&closed.reason),
            Err(error) => return Ended::Lost(format!("the watch did not open: {error}")),
        };
        let writer = Arc::new(Mutex::new(stream.duplicate()));
        // Reserve the first write before publishing Open. A newer decision
        // may then wait on this writer, but cannot be overwritten by the
        // initial decision. No panes lock is held across the write.
        let mut initial = locked(&writer);
        let first = {
            let mut panes = locked(&self.panes);
            let Some(watched) = panes
                .get_mut(pane)
                .filter(|watched| watched.generation == generation)
            else {
                return Ended::Told;
            };
            let first = watched
                .rules
                .as_ref()
                .map(|rules| choose(rules.as_ref(), watching.rules_engine));
            watched.no_fit = first.as_ref().is_some_and(|(_, no_fit)| *no_fit);
            watched.open = Some(Open {
                writer: Arc::clone(&writer),
                shutdown: stream.shutdown_handle(),
                engine: watching.rules_engine,
            });
            first.map(|(manifest, _)| manifest)
        };
        if let Some(first) = first
            && let Err(error) = write_rules(&mut initial, first)
        {
            crate::diagnostic!(json!({
                "component": "pane_holder",
                "kind": "pane_holder.rules_unsent",
                "pane_id": pane,
                "message": error.to_string(),
            }));
        }
        drop(initial);
        loop {
            match read_watch_holder(&mut stream, &watching.sends) {
                Ok(HolderMessage::Facts(facts)) => {
                    let facts = pane_facts(pane, facts);
                    let mut current = true;
                    self.deliver(pane, |panes| {
                        let Some(watched) = panes
                            .get_mut(pane)
                            .filter(|watched| watched.generation == generation)
                        else {
                            current = false;
                            return None;
                        };
                        watched.last = Some(facts);
                        watched.told()
                    });
                    if !current {
                        return Ended::Told;
                    }
                }
                Ok(HolderMessage::Closed(closed)) => return Ended::of(&closed.reason),
                Ok(other) => {
                    return Ended::Lost(format!("a watch was sent {}", message_name(&other)));
                }
                Err(error) => return Ended::Lost(format!("the watch broke: {error}")),
            }
        }
    }
}

/// How a pane's watch ended.
enum Ended {
    /// With the pane: its program exited or was ended.
    Gone,
    /// The pane was told what it reads (its holder sends no facts), or the
    /// watch is no longer the pane's: nothing more is said.
    Told,
    /// Any other way, with why: the pane's facts are no longer followed.
    Lost(String),
}

impl Ended {
    fn of(reason: &CloseReason) -> Self {
        match reason {
            CloseReason::ProgramExited | CloseReason::Terminated => Self::Gone,
            other => Self::Lost(format!("the holder closed the watch: {}", other.name())),
        }
    }
}

/// The version of a holder built before what a node opened a connection for,
/// as its refusal says: it read the opening, a watch or an ask, as a first
/// message it does not know. The one place a node judges that.
pub(in crate::terminal) fn too_old(error: &OpenError) -> Option<u16> {
    match error {
        OpenError::Refused(closed)
            if matches!(
                closed.reason,
                CloseReason::HelloInvalid | CloseReason::ProtocolError
            ) =>
        {
            Some(closed.version)
        }
        _ => None,
    }
}

/// The version of a holder too old to send facts, as its answer to a watch
/// says: [`too_old`] for the opening, or a watch opened without facts.
fn sends_no_facts(opened: &Result<(LocalStream, Watching), OpenError>) -> Option<u16> {
    match opened {
        Err(error) => too_old(error),
        Ok((_, watching)) if !watching.sends.contains(&Capability::Facts) => Some(watching.version),
        Ok(_) => None,
    }
}

impl Watched {
    /// The facts the sink is told: the holder's, with the screen read as no
    /// rules for the holder while none fits.
    fn told(&self) -> Option<PaneFacts> {
        let mut facts = self.last.clone()?;
        if self.no_fit {
            facts.screen = ScreenRead::Unknown {
                reason: NO_RULES_FOR_HOLDER.to_owned(),
            };
        }
        Some(facts)
    }
}

/// The rule set a holder whose evaluator is `engine` is given, and whether
/// no set fits it: the first, by priority, it can read.
fn choose(rules: Option<&ScreenRules>, engine: u32) -> (Option<String>, bool) {
    let Some(rules) = rules else {
        return (None, false);
    };
    match rules
        .sets
        .iter()
        .find(|set| set.min_engine_version <= engine)
    {
        Some(set) => (Some(set.manifest.clone()), false),
        None => (None, !rules.sets.is_empty()),
    }
}

fn send_rules(writer: &Mutex<LocalStream>, manifest: Option<String>) -> std::io::Result<()> {
    let mut writer = locked(writer);
    write_rules(&mut writer, manifest)
}

fn write_rules(writer: &mut LocalStream, manifest: Option<String>) -> std::io::Result<()> {
    writer.set_deadline(Instant::now() + WRITE_LIMIT);
    write_watch(writer, &WatchMessage::Rules(manifest))
}

fn message_name(message: &HolderMessage) -> &'static str {
    match message {
        HolderMessage::Welcome(_) => "a welcome",
        HolderMessage::Answer(_) => "an answer",
        HolderMessage::Watching(_) => "a second watching",
        _ => "a screen message",
    }
}

fn unsupported() -> PaneFacts {
    PaneFacts {
        screen: ScreenRead::Unknown {
            reason: UNSUPPORTED_HOLDER.to_owned(),
        },
        ..PaneFacts::default()
    }
}

/// The holder's facts in the engine's words: its folder in the wire
/// spelling, which a folder that is not a whole path has none of.
fn pane_facts(pane: &str, facts: Facts) -> PaneFacts {
    let cwd = facts
        .cwd
        .and_then(|cwd| match hide_platform::path::to_wire(Path::new(&cwd)) {
            Ok(wire) => Some(wire),
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": "pane_holder",
                    "kind": "pane_holder.cwd_unusable",
                    "pane_id": pane,
                    "message": error.to_string(),
                }));
                None
            }
        });
    PaneFacts {
        cwd,
        title: facts.title,
        bells: facts.bells,
        prompts: facts.prompts,
        bracketed_paste: facts.bracketed_paste,
        line_editing: facts.line_editing.and_then(|editing| match editing {
            LineEditing::Edits => Some(true),
            LineEditing::ReadsKeys => Some(false),
            LineEditing::Unknown => None,
        }),
        foreground_unread: facts.foreground_unread == Some(true),
        alternate_screen: facts.alternate_screen.unwrap_or(false),
        foreground: facts.foreground.map(|program| ForegroundProgram {
            pid: program.pid,
            name: program.name,
            package: program.package,
            argv: program.argv,
        }),
        screen: match facts.screen {
            WireRead::NoRules => ScreenRead::NoRules,
            WireRead::Read { state, rule } => ScreenRead::Read {
                state: match state {
                    WireState::Idle => ScreenState::Idle,
                    WireState::Working => ScreenState::Working,
                    WireState::Blocked => ScreenState::Blocked,
                    WireState::Unknown => ScreenState::Unknown,
                },
                rule,
            },
            WireRead::Unknown { reason } => ScreenRead::Unknown { reason },
        },
    }
}

#[cfg(test)]
mod tests {
    use hide_node_link::holders::{MAX_FOREGROUND_ARGS, MAX_FOREGROUND_ARGV_BYTES, RuleSet};

    use super::*;

    fn set(min_engine_version: u32, manifest: &str) -> RuleSet {
        RuleSet {
            source: "built-in".to_owned(),
            version: "1".to_owned(),
            min_engine_version,
            manifest: manifest.to_owned(),
        }
    }

    /// A renewal retires a pane's watch and its next watch begins before the
    /// earlier watch's thread hears its holder end; a pane forgotten and
    /// watched again is the same. That late end leaves the pane known, with
    /// its rules, and tells the sink nothing.
    #[test]
    fn an_earlier_watch_that_ends_late_leaves_the_next_watch_of_its_pane_alone() {
        let watches = Watches::default();
        let told = Arc::new(Mutex::new(Vec::<String>::new()));
        watches.set_sink(Arc::new({
            let told = Arc::clone(&told);
            move |pane: &str, _: &PaneFacts| locked(&told).push(pane.to_owned())
        }));
        let rules = ScreenRules {
            agent: "claude".to_owned(),
            sets: vec![set(1, "rules")],
        };

        let earlier = watches.begin("w1:p1").expect("a first watch begins");
        watches
            .set_rules("w1:p1", Some(rules.clone()))
            .expect("a watched pane takes rules");
        watches.retire("w1:p1");
        let next = watches
            .begin("w1:p1")
            .expect("a renewed pane is watched again");
        watches.ended("w1:p1", earlier, Ended::Gone);
        {
            let panes = locked(&watches.panes);
            let watched = panes.get("w1:p1").expect("the pane is still known");
            assert!(watched.following);
            assert_eq!(watched.generation, next);
            assert_eq!(watched.rules, Some(Some(rules)));
        }

        watches.forget("w1:p1");
        let again = watches
            .begin("w1:p1")
            .expect("a forgotten pane is watched again");
        watches.ended("w1:p1", next, Ended::Gone);
        assert!(
            locked(&watches.panes)
                .get("w1:p1")
                .is_some_and(|watched| watched.generation == again)
        );
        assert!(locked(&told).is_empty());

        watches.ended("w1:p1", again, Ended::Gone);
        assert!(locked(&watches.panes).get("w1:p1").is_none());
    }

    #[test]
    fn a_holder_gets_the_first_rule_set_its_evaluator_reads_and_none_when_none_fits() {
        let rules = ScreenRules {
            agent: "x".to_owned(),
            sets: vec![set(5, "user"), set(3, "built-in")],
        };
        assert_eq!(choose(Some(&rules), 5), (Some("user".to_owned()), false));
        assert_eq!(
            choose(Some(&rules), 3),
            (Some("built-in".to_owned()), false)
        );
        assert_eq!(choose(Some(&rules), 2), (None, true));
        assert_eq!(choose(None, 2), (None, false));
    }

    #[test]
    fn the_holders_bounds_are_the_engines() {
        assert_eq!(hide_holder_protocol::MAX_ARGS, MAX_FOREGROUND_ARGS);
        assert_eq!(
            hide_holder_protocol::MAX_ARGV_BYTES,
            MAX_FOREGROUND_ARGV_BYTES
        );
    }
}
