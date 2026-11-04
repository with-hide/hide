//! The core's own engine (PRD engine-bundle-3-core-owns D-3): the second
//! implementation of [`Engine`], which runs a machine's panes without Herdr.
//! It holds its session's topology ([`topology::Topology`]) under a lock of
//! its own, never the runtime's, and asks the machine's node to start, end,
//! read and type into each pane's holder through [`PaneHolders`], always
//! outside that lock. A change to the topology wakes whoever watches it
//! ([`Engine::changes`]), and the watcher reads the whole session again
//! ([`Engine::session`]); there is no event stream to replay.
//!
//! It makes workspaces, tabs and panes and lays them out by the rules of
//! [`layout`] (split, close, zoom, resize, swap, move, a saved layout given
//! back), binds workspaces to checkouts and makes worktrees ([`worktrees`]),
//! keeps focus, names and Hide's tokens, and reads (plain or styled), types
//! into and lists the processes of a pane. It starts agents by their
//! definition files and follows what each pane runs and shows from its
//! holder's facts ([`agents`]), handing each holder its agent's screen rules
//! ([`rules`]). A daemon's engine saves its session after every change
//! ([`store`]) and, as it opens, lays the saved session out again around the
//! holders that ran on, starts the rest again and resumes their agents
//! ([`restore`]), so a pane keeps its id across a restart and a reboot.
//! Only a daemon with the development
//! switch `HIDE_DEV_PANE_HOLDER` on builds this engine (D-5).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use hide_node_link::NodeLink;
use hide_node_link::holders::{
    HolderError, HolderKey, HolderScroll, HolderStart, PaneFacts, PaneHolders, PaneInput,
    StyledRead, UNSUPPORTED_HOLDER,
};

use super::keys::PaneKey;
use super::{
    AgentLaunch, AgentState, Answer, Changes, CreatedWorktree, DeliveryAgent, Engine, EngineError,
    Env, EventStream, Gate, Notified, OpenedWorktree, PaneJoinResult, PanePlace, PaneProcessGroup,
    PaneProof, PaneSession, PaneSessionKind, PaneText, PreparedStart, Readiness, ServerPong,
    StartedAgent, Wait, Wake, Zoom,
};
use crate::model::PaneLayoutDirection;
use crate::recent_closed::{ClosedLayout, ClosedLayoutNode, ClosedSplitDirection};
use crate::session_sync::{ProjectedAgent, ProjectionState};
use crate::sidebar::{SessionLayoutPayload, SessionSnapshotPayload};

mod agents;
mod creation;
pub(crate) mod herdr_session;
mod ids;
pub(crate) mod layout;
mod restore;
mod rules;
mod shell_line;
mod starts;
mod store;
mod topology;
mod worktrees;

use topology::{Closed, Pane, Topology, refused};

/// The size a pane starts at before a screen draws it; the first screen
/// that attaches it resizes it to the rectangle drawn for it.
const START_ROWS: u16 = 24;
const START_COLS: u16 = 80;

/// The most rows a read hands the node, which bounds it again to what a
/// holder answers (`Holders::read`, truncated past it).
const READ_LINES_CAP: u32 = u16::MAX as u32;

/// The read sources this engine answers, soft wraps joined: `recent` and
/// `recent_unwrapped` the last rows of the screen and its scrollback
/// (`hide_holder_protocol::Ask::Read`), so `recent` is unwrapped as
/// `recent_unwrapped` is, and `visible` the rows the screen shows now,
/// wherever it is scrolled to (`Ask::Visible`), which a find measures its
/// scroll against.
const READ_SOURCES: [&str; 3] = ["recent", "recent_unwrapped", "visible"];

/// The refusal code of a read from a source this engine does not hold.
const UNKNOWN_SOURCE: &str = "unknown_read_source";

/// Where a daemon's engine keeps its session and finds its home: the
/// engine folder on the core's machine its saved session lives in
/// (`hide_kit::layout::engine_dir`, or a device's,
/// `hide_kit::layout::device_engine_dir`), the home of the machine its panes
/// run on, in the wire spelling, which a restored pane whose folder is gone
/// opens in and new worktrees go under, and that machine's node, which reads
/// its folders and runs its Git, since the core touches no machine (PRD
/// core-host-node D-21).
#[derive(Clone)]
pub(crate) struct Place {
    pub(crate) engine_dir: PathBuf,
    pub(crate) home: String,
    /// The home whose Hide agent hook files this engine's panes write, when
    /// those files are on the machine the core runs on: the daemon's own
    /// engine. A device's engine has none; its panes' files are on the
    /// device.
    pub(crate) hooks_home: Option<PathBuf>,
    pub(crate) node: Arc<dyn NodeLink>,
    /// Whether the machine `node` answers for runs Windows: a device's
    /// system, from its node's hello, never the core's own, since a Mac core
    /// runs the panes of a Windows device and the reverse.
    pub(crate) windows: bool,
    /// The agent definitions the operator's folder held when the daemon
    /// started, so a restore resumes their agents before the core's first
    /// read reaches the engine.
    pub(crate) definitions: Arc<hide_agent_adapter::definition::Catalog>,
    /// The model and effort Hide started each pane's agent with, by this
    /// engine's pane id, as the core keeps them (`ui_state.pane_launches`):
    /// a restore resumes a saved agent with the choice whose record
    /// describes it, so a mission worker does not come back on another
    /// model (PRD engine-bundle-4-herdr-removal D-24).
    pub(crate) launches: Arc<BTreeMap<String, crate::model::PaneLaunch>>,
}

/// The engine and whatever it holds, shared by every gated view of it.
struct Shared {
    topology: Arc<Mutex<Topology>>,
    holders: Arc<dyn PaneHolders>,
    starts: Arc<starts::Starts>,
    pending: Mutex<BTreeMap<String, creation::Ticket>>,
    wake: Mutex<Option<Wake>>,
    stopped: AtomicBool,
    place: Option<Place>,
    /// The thread that saves the session, for a daemon's engine; dropped
    /// with the engine, which writes the last change.
    saver: OnceLock<store::Saver>,
    /// Whether the restore has finished: until it has, no pane is made, so
    /// a new pane never takes an id a holder still running was recorded
    /// under before the restore counted past it.
    restored: Arc<Restored>,
    /// The agent definitions agents are started, resumed and recognized by.
    definitions: agents::Definitions,
    /// Gives panes' holders their screen rules off every lock.
    rules: rules::RulesWorker,
    /// Whether the machine whose holders run the panes runs Windows: the
    /// place's, or this machine's for a test engine with no place.
    windows: bool,
    /// Facts a holder sent before its pane entered the topology (a pane
    /// reserves its id and starts its holder first), laid on once it has.
    /// The oldest goes first past [`EARLY_FACTS_CAP`].
    early: Mutex<Vec<(String, PaneFacts)>>,
    /// The panes whose agent the restore resumed after its turn was cut off,
    /// by the definition resumed there, until its hook names its session
    /// (PRD engine-bundle-4-herdr-removal D-22). Bounded by one restore's
    /// resumes.
    cut: Mutex<BTreeMap<String, String>>,
}

/// The most panes whose facts wait to enter the topology: a node's cap of
/// holders.
const EARLY_FACTS_CAP: usize = hide_kit::panes::HOLDER_CAP;

/// One hold of the topology's lock. As it ends, a change of the tab the
/// keyboard's workspace shows is recorded as a move, so the session reports
/// every move in the order the holds made them ([`Topology::note_shown`]).
struct Held<'a>(MutexGuard<'a, Topology>);

impl std::ops::Deref for Held<'_> {
    type Target = Topology;
    fn deref(&self) -> &Topology {
        &self.0
    }
}

impl std::ops::DerefMut for Held<'_> {
    fn deref_mut(&mut self) -> &mut Topology {
        &mut self.0
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.0.note_shown();
    }
}

/// Whether a restore has finished, which the restore marks once it has let
/// go of the engine, so an engine dropped after a pane was made is dropped
/// whole, its last change written.
#[derive(Default)]
pub(super) struct Restored {
    done: Mutex<Option<Result<(), EngineError>>>,
    finished: Condvar,
}

impl Restored {
    pub(super) fn finish(&self, result: Result<(), EngineError>) {
        *self.done.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
        self.finished.notify_all();
    }
}

/// The own engine, with every call first passing `gates`.
pub(crate) struct OwnEngine<'g> {
    shared: Arc<Shared>,
    gates: Vec<Gate<'g>>,
}

/// The pane holders of the machine `node` reaches, as every engine of this
/// kind asks them: through its node, whichever machine it is (PRD
/// engine-bundle-3-core-owns D-22).
pub(crate) fn link_panes(node: Arc<dyn NodeLink>) -> Arc<dyn PaneHolders> {
    Arc::new(hide_node_link::link_panes::LinkPanes::new(
        node,
        Arc::new(|record: serde_json::Value| crate::diagnostic!(record)),
    ))
}

/// A device's own engine (PRD engine-bundle-3-core-owns D-22): its session
/// kept in the core's state folder `state_dir` under the device's own
/// folder, its panes run by the holders of the device's node `node`, which
/// answers for the account whose home is `home` (in the wire spelling). One
/// is opened for each link the device's node brings and let go with it, so
/// a node that starts again is laid out again around the holders it finds.
pub(crate) fn open_device(
    device: &str,
    node: Arc<dyn NodeLink>,
    state_dir: &std::path::Path,
    home: String,
    windows: bool,
    definitions: Arc<hide_agent_adapter::definition::Catalog>,
    launches: Arc<BTreeMap<String, crate::model::PaneLaunch>>,
) -> Result<OwnEngine<'static>, String> {
    let engine_dir = hide_kit::layout::device_engine_dir(state_dir, device)?;
    OwnEngine::open(
        link_panes(Arc::clone(&node)),
        Place {
            engine_dir,
            home,
            hooks_home: None,
            node,
            windows,
            definitions,
            launches,
        },
    )
}

/// Whether the workspace, tab or pane id `id` is in a workspace this engine
/// numbered (PRD engine-bundle-4-herdr-removal D-26): the only ids a
/// machine's cleanup owns while its Herdr save cannot be read (D-25).
pub(crate) fn made_by_engine(id: &str) -> bool {
    ids::made_by_engine(id)
}

/// The workspaces the engine whose folder is `engine_dir` saved, read as its
/// open reads them, while no engine is open there.
pub(crate) fn saved_workspaces(engine_dir: &std::path::Path) -> BTreeSet<String> {
    restore::read(
        &hide_kit::layout::engine_session_in(engine_dir),
        &hide_kit::layout::engine_backups_in(engine_dir),
    )
    .to_saved()
    .workspaces
    .into_iter()
    .map(|workspace| workspace.id)
    .collect()
}

impl OwnEngine<'static> {
    /// An engine with no workspace, whose panes `holders` run, that saves
    /// nothing and restores nothing: a test's.
    #[cfg(test)]
    pub(crate) fn new(holders: Arc<dyn PaneHolders>) -> Self {
        Self::with(Topology::default(), holders, None, true).expect("a test engine starts")
    }

    /// A daemon's engine (PRD engine-bundle-3-core-owns D-6, D-7): its
    /// session read from `place`'s saved session, saved there after every
    /// change, and laid out again around what `holders` found running as
    /// soon as the node says what that is, on a thread of its own.
    pub(crate) fn open(holders: Arc<dyn PaneHolders>, place: Place) -> Result<Self, String> {
        let path = hide_kit::layout::engine_session_in(&place.engine_dir);
        let backups = hide_kit::layout::engine_backups_in(&place.engine_dir);
        let topology = restore::read(&path, &backups);
        let saved = topology.panes().map(|(id, _)| id.to_owned()).collect();
        let engine = Self::with(topology, holders, Some(place), false)?;
        let session = Arc::clone(&engine.shared.topology);
        let saver = store::Saver::start(path, move || {
            Some(
                session
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .to_saved(),
            )
        })?;
        let _ = engine.shared.saver.set(saver);
        // A pane whose program exits on its own closes, as Herdr closes it.
        // The node holds no engine: it is told the engine's shared state
        // weakly, so an engine dropped is not kept by its holders.
        let shared = Arc::downgrade(&engine.shared);
        engine
            .shared
            .holders
            .watch_ends(Arc::new(move |pane: &str| {
                if let Some(shared) = shared.upgrade() {
                    OwnEngine {
                        shared,
                        gates: Vec::new(),
                    }
                    .program_exited(pane);
                }
            }));
        restore::start(
            OwnEngine {
                shared: Arc::clone(&engine.shared),
                gates: Vec::new(),
            },
            saved,
        )?;
        Ok(engine)
    }

    fn with(
        topology: Topology,
        holders: Arc<dyn PaneHolders>,
        place: Option<Place>,
        restored: bool,
    ) -> Result<Self, String> {
        let definitions = place.as_ref().map_or_else(
            || Arc::new(hide_agent_adapter::definition::Catalog::read(Vec::new())),
            |place| Arc::clone(&place.definitions),
        );
        let windows = place.as_ref().map_or(cfg!(windows), |place| place.windows);
        let engine = Self {
            shared: Arc::new(Shared {
                windows,
                topology: Arc::new(Mutex::new(topology)),
                rules: rules::RulesWorker::start(Arc::clone(&holders), restored)?,
                holders,
                starts: Arc::default(),
                pending: Mutex::default(),
                wake: Mutex::default(),
                stopped: AtomicBool::new(false),
                place,
                saver: OnceLock::new(),
                restored: Arc::new(Restored {
                    done: Mutex::new(restored.then_some(Ok(()))),
                    finished: Condvar::new(),
                }),
                definitions: agents::Definitions::new(definitions),
                early: Mutex::default(),
                cut: Mutex::default(),
            }),
            gates: Vec::new(),
        };
        // What runs in each pane and what its screen shows, whether or not a
        // screen shows it (D-18). The node holds no engine, as for the ends
        // above.
        let shared = Arc::downgrade(&engine.shared);
        engine
            .shared
            .holders
            .watch_facts(Arc::new(move |pane: &str, facts: &PaneFacts| {
                if let Some(shared) = shared.upgrade() {
                    OwnEngine {
                        shared,
                        gates: Vec::new(),
                    }
                    .pane_facts(pane, facts);
                }
            }));
        Ok(engine)
    }
}

impl OwnEngine<'_> {
    /// Ends the engine: every later call answers that nothing runs. The
    /// panes' holders run on (they outlive the daemon, D-10); whoever started
    /// them for a test ends them.
    #[cfg(test)]
    pub(crate) fn stop(&self) {
        self.shared.stopped.store(true, Ordering::Release);
    }

    fn topology(&self) -> Held<'_> {
        Held(
            self.shared
                .topology
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Whether a call may run: the engine runs and every gate lets it.
    fn check(&self) -> Result<(), EngineError> {
        if self.shared.stopped.load(Ordering::Acquire) {
            return Err(EngineError::NotRunning(
                "the core's own engine has stopped".to_owned(),
            ));
        }
        for gate in &self.gates {
            (gate.before)()?;
            (gate.at_send)()?;
        }
        Ok(())
    }

    /// Waits for the restore to finish before a pane is made; refused when
    /// it has not within its own bound.
    fn restored(&self) -> Result<(), EngineError> {
        let restored = &self.shared.restored;
        let done = restored.done.lock().unwrap_or_else(PoisonError::into_inner);
        let (done, _) = restored
            .finished
            .wait_timeout_while(done, restore::BOUND, |done| done.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(result) = &*done {
            result.clone()
        } else {
            Err(EngineError::Refused {
                code: "restore_unfinished".to_owned(),
                message: "the core's own engine is still laying its saved session out again"
                    .to_owned(),
            })
        }
    }

    /// What marks the restore finished.
    fn restore_gate(&self) -> Arc<Restored> {
        Arc::clone(&self.shared.restored)
    }

    /// The node of this engine's machine.
    fn node(&self) -> Result<&Arc<dyn NodeLink>, EngineError> {
        self.shared
            .place
            .as_ref()
            .map(|place| &place.node)
            .ok_or_else(|| EngineError::Refused {
                code: "no_node".to_owned(),
                message: "this engine was given no node to reach its machine".to_owned(),
            })
    }

    /// The home this engine opens worktrees and lost folders under.
    fn home(&self) -> Result<String, EngineError> {
        self.shared
            .place
            .as_ref()
            .map(|place| place.home.clone())
            .ok_or_else(|| EngineError::Refused {
                code: "no_home".to_owned(),
                message: "this engine was given no home folder".to_owned(),
            })
    }

    /// Tells the watcher the session changed, once per change, and has it
    /// saved.
    fn changed(&self) {
        self.settle_early();
        if let Some(saver) = self.shared.saver.get() {
            saver.changed();
        }
        let wake = self
            .shared
            .wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(wake) = wake {
            wake();
        }
    }

    /// Lays what `pane`'s holder says of it on the pane: its folder, the
    /// agent its foreground runs and the state its screen shows. A pane not
    /// yet in the topology keeps its latest facts until it enters.
    fn pane_facts(&self, pane: &str, facts: &PaneFacts) {
        let catalog = self.shared.definitions.get();
        let applied = self.topology().apply_facts(pane, facts, &catalog);
        match applied {
            Ok(applied) => {
                if applied.rules_for.is_some() {
                    self.give_rules(pane);
                }
                if applied.changed {
                    self.changed();
                }
            }
            Err(_) => {
                let mut early = self
                    .shared
                    .early
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                early.retain(|(waiting, _)| waiting != pane);
                if early.len() >= EARLY_FACTS_CAP {
                    let (dropped, _) = early.remove(0);
                    crate::diagnostic!(serde_json::json!({
                        "component": "own_engine",
                        "kind": "pane.early_facts_dropped",
                        "pane_id": dropped,
                    }));
                }
                early.push((pane.to_owned(), facts.clone()));
            }
        }
    }

    /// Lays the facts kept for panes that have entered the topology since.
    /// A pane that never enters (its start failed) is dropped once
    /// [`EARLY_FACTS_CAP`] newer ones wait.
    fn settle_early(&self) {
        let entered = {
            let mut early = self
                .shared
                .early
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if early.is_empty() {
                return;
            }
            let topology = self.topology();
            let (entered, waiting): (Vec<_>, Vec<_>) = early
                .drain(..)
                .partition(|(pane, _)| topology.pane(pane).is_ok());
            *early = waiting;
            entered
        };
        let catalog = self.shared.definitions.get();
        for (pane, facts) in entered {
            let applied = self.topology().apply_facts(&pane, &facts, &catalog);
            if let Ok(applied) = applied
                && applied.rules_for.is_some()
            {
                self.give_rules(&pane);
            }
        }
    }

    /// Queues the pane's current decision in topology order. A caller that
    /// lost a race with a new agent must not enqueue the previous agent's
    /// rules afterwards. Only the queue insertion is under this lock; the
    /// rules worker owns the actual holder call.
    fn give_rules(&self, pane: &str) {
        loop {
            let kind = {
                let topology = self.topology();
                let Ok(held) = topology.pane(pane) else {
                    return;
                };
                held.agent.as_ref().map(|agent| agent.kind.clone())
            };
            let catalog = self.shared.definitions.get();
            let rules = kind
                .as_deref()
                .and_then(|kind| agents::definition(&catalog, kind))
                .and_then(agents::screen_rules);
            let mut topology = self.topology();
            let Ok(held) = topology.pane_mut(pane) else {
                return;
            };
            if held.agent.as_ref().map(|agent| &agent.kind) != kind.as_ref() {
                // The newer topology transition owns its publication.
                return;
            }
            if !Arc::ptr_eq(&catalog, &self.shared.definitions.get()) {
                // A catalog update queues only changed rules. It may have
                // nothing to publish for this kind, so recompute outside the
                // lock instead of dropping the pane's first decision.
                drop(topology);
                continue;
            }
            held.live.rules_sent = Some(kind.clone());
            self.shared.rules.give(pane, rules);
            return;
        }
    }

    /// Publishes a start provisionally so early hooks have their identity,
    /// then withdraws only proven non-delivery of that same attempt.
    /// An unanswered write may already have reached the program.
    fn type_agent_start(&self, pane: &str, agent: agents::PaneAgent, line: &str) -> Answer<()> {
        let start = {
            let mut topology = self.topology();
            if agent
                .name
                .as_deref()
                .is_some_and(|name| topology.name_taken(name, pane))
            {
                return Err(refused(
                    "agent_name_taken",
                    "another pane runs an agent under its name".to_owned(),
                ));
            }
            topology.start_agent(pane, agent)?
        };
        self.give_rules(pane);
        self.changed();
        let result = self.type_bytes(pane, format!("{line}\r").into_bytes());
        let not_delivered = matches!(&result, Err(EngineError::Refused { .. }));
        let withdrawn = self
            .topology()
            .finish_agent_start(pane, start, not_delivered);
        if withdrawn {
            self.give_rules(pane);
            self.changed();
        }
        result
    }

    /// Starts `pane`'s holder running `program`, or the machine's shell
    /// after `notice` when one is given.
    fn start_pane(
        &self,
        pane: &str,
        program: Option<&[String]>,
        cwd: &str,
        env: Env,
        notice: Option<&str>,
    ) -> Result<(), EngineError> {
        self.start_pane_replacing(pane, program, cwd, env, notice, None)
    }

    fn start_pane_replacing(
        &self,
        pane: &str,
        program: Option<&[String]>,
        cwd: &str,
        env: Env,
        notice: Option<&str>,
        previous: Option<u64>,
    ) -> Result<(), EngineError> {
        let ticket = self.reserve_start(
            start_request(pane, program, cwd, env, notice),
            previous,
            false,
            None,
        )?;
        if self.topology().pane(pane).is_ok() {
            ticket.publish(true, || Ok(()))?;
        }
        creation::run(ticket, &*self.shared.holders, &Arc::downgrade(&self.shared))
    }

    /// Starts the holder of `pane`, whose id this call just reserved, once
    /// every gate's [`super::Placement`] has recorded it (PRD
    /// engine-bundle-3-core-owns D-11), for a pane that enters the topology
    /// once its holder runs. A holder that does not start withdraws every
    /// placement in this same call, so no record names a pane that never
    /// ran.
    fn start_placed(
        &self,
        pane: &str,
        program: Option<&[String]>,
        cwd: &str,
        env: Env,
    ) -> Result<(), EngineError> {
        let ticket = self.reserve_start(
            start_request(pane, program, cwd, env, None),
            None,
            true,
            None,
        )?;
        if let Err(error) = ticket.check_open().and_then(|()| self.check()) {
            self.cancel_start(ticket)?;
            return Err(error);
        }
        creation::run(ticket, &*self.shared.holders, &Arc::downgrade(&self.shared))
    }

    /// Starts the holder of `pane`, which this call has placed and drawn
    /// `starting`: the screen shows a new tab or split at once and its
    /// terminal attaches once the holder runs, as a Herdr pane is answered
    /// once it forked rather than once its shell is up (perf-gate rows 22
    /// and 23 measured the holder's start, about 50 ms, in front of every
    /// new pane). A holder that does not start takes the pane out again,
    /// its tab with it when it was the tab's only pane, and withdraws every
    /// placement, so the call is refused with the holder's reason and
    /// nothing of it is left; a pane closed while its holder started has
    /// that holder ended here, since its closer left it to this call.
    fn start_drawn(
        &self,
        ticket: creation::Ticket,
        starting: Arc<starts::Starting>,
    ) -> Result<(), EngineError> {
        let pane = ticket.start.pane.clone();
        let started = creation::run(ticket, &*self.shared.holders, &Arc::downgrade(&self.shared));
        let failure = if started.as_ref().is_err_and(|error| error.reply_unknown()) {
            OnFailure::Mark
        } else {
            OnFailure::TakeOut
        };
        let settled = settle_start(
            &self.shared.topology,
            &*self.shared.holders,
            &pane,
            &starting,
            &started,
            failure,
        );
        if settled == Settled::Failed {
            self.changed();
        }
        started
    }

    /// Whether a gate records the panes this call makes: a spawn's, whose
    /// caller must hear a holder that did not start (D-11, B30).
    fn places(&self) -> bool {
        self.gates.iter().any(|gate| gate.placement.is_some())
    }

    /// Starts the holder of `pane`, drawn `starting`, after this call has
    /// answered, as Herdr answers a new tab or split once its pane forked
    /// and its shell starts after: the core shows a created tab when the
    /// answer names it, so the holder's start (three flushed writes of its
    /// record and two programs started) stood between the chord and the
    /// screen. A holder that does not start then leaves its pane where it
    /// was drawn, marked with why ([`OnFailure::Mark`]: the screen shows
    /// the pane's terminal could not start and offers to close it), and
    /// logs `pane.start_failed`; a call whose outcome hangs on the start (a
    /// spawn's placement, a tab it replaces) uses [`Self::start_drawn`].
    /// Starts share the restore's admission gate: eight run and eight
    /// wait at most, admitted before a thread is made. An admission or
    /// thread failure marks the pane just like a holder start failure.
    fn start_after_answer(&self, ticket: creation::Ticket, starting: Arc<starts::Starting>) {
        let topology = Arc::clone(&self.shared.topology);
        let holders = Arc::clone(&self.shared.holders);
        let shared = Arc::downgrade(&self.shared);
        let start = move |ticket: creation::Ticket, failure: Option<EngineError>| {
            let pane = ticket.start.pane.clone();
            let started = match failure {
                None => creation::run(ticket, &*holders, &shared),
                Some(error) => {
                    // No worker can start this accepted reservation now. It
                    // remains recoverable, with its visible failure, on disk.
                    creation::retire(&*holders, &ticket, &shared).and(Err(error))
                }
            };
            let settled = settle_start(
                &topology,
                &*holders,
                &pane,
                &starting,
                &started,
                OnFailure::Mark,
            );
            if let Err(error) = started {
                crate::diagnostic!(
                    serde_json::json!({ "component": "own_engine", "kind": "pane.start_failed", "pane_id": pane, "message": error.to_string() })
                );
            }
            if settled == Settled::Failed
                && let Some(shared) = shared.upgrade()
            {
                OwnEngine {
                    shared,
                    gates: Vec::new(),
                }
                .changed();
            }
        };
        // The slot was acquired before reserve and publication, never twice.
        let pending = Arc::new(Mutex::new(Some((start, ticket))));
        let spawned = std::thread::Builder::new()
            .name("hide-own-engine-pane-start".into())
            .spawn({
                let pending = Arc::clone(&pending);
                move || {
                    let work = pending
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take();
                    if let Some((start, ticket)) = work {
                        start(ticket, None);
                    }
                }
            });
        if let Err(error) = spawned {
            let work = pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some((start, ticket)) = work {
                start(
                    ticket,
                    Some(EngineError::Transport(format!(
                        "the holder start thread could not start: {error}"
                    ))),
                );
            }
        }
    }

    /// Ends the holders of `panes` and takes them out with `remove`, under
    /// one hold of the topology lock: a pane still `starting` is not ended
    /// here, since its holder may not be written in yet, and the call
    /// starting it ends it once it finds the pane gone; one whose start
    /// ended meanwhile is ended after the removal. A holder that does not
    /// end leaves every pane in place, as before.
    fn end_and_remove<R>(
        &self,
        select: impl FnOnce(&Topology) -> Result<Vec<String>, EngineError>,
        remove: impl FnOnce(&mut Topology) -> R,
    ) -> Result<R, EngineError> {
        self.resolve_unpublished()?;
        let panes = {
            let mut topology = self.topology();
            let panes = select(&topology)?;
            // Selection and the fence share publication's topology lock.
            // Keep the fence on an unresolved close; a later close retries
            // the same panes rather than admitting new children behind it.
            for pane in &panes {
                if let Ok(pane) = topology.pane_mut(pane) {
                    pane.closing = true;
                }
            }
            panes
        };
        self.cancel_unpublished_for(&panes)?;
        self.end_panes(&panes)?;
        let removed = remove(&mut self.topology());
        self.release_closed(&panes)?;
        Ok(removed)
    }

    /// Ends the holders of `panes`, every one asked even when one fails:
    /// the first failure, if any. A holder already gone is ended.
    fn end_panes(&self, panes: &[String]) -> Result<(), EngineError> {
        end_holders(&*self.shared.holders, panes)
    }

    /// A new tab of `workspace` whose first pane runs `program`, replacing
    /// `replace` when given: its id and its pane.
    #[allow(clippy::too_many_arguments)] // one tab's place, name, focus, folder and program
    fn open_tab(
        &self,
        workspace: &str,
        replace: Option<&str>,
        label: &str,
        pane_label: Option<&str>,
        focus: bool,
        cwd: &str,
        env: Env,
        program: Option<&[String]>,
    ) -> Answer<(String, String)> {
        self.restored()?;
        self.resolve_unpublished()?;
        let reserved = {
            let mut topology = self.topology();
            if let Some(replace) = replace
                && !topology.tab_in(workspace, replace)?
            {
                return Err(EngineError::Refused {
                    code: "tab_not_found".to_owned(),
                    message: format!("tab {replace} is not in workspace {workspace}"),
                });
            }
            topology.reserve_tab(workspace)?
        };
        let (tab, pane) = &reserved;
        let ticket = self.reserve_start(
            start_request(pane, program, cwd, env.clone(), None),
            None,
            true,
            None,
        )?;
        if let Err(error) = ticket.check_open().and_then(|()| self.check()) {
            self.cancel_start(ticket)?;
            return Err(error);
        }
        let (drawn, starting) = Pane::starting(cwd, pane_label, program, &env);
        let added = ticket.publish(replace.is_none() && !self.places(), || {
            self.topology()
                .add_tab(workspace, (tab, pane), label, drawn, focus)
        });
        if let Err(error) = added {
            // The workspace closed since the id was reserved.
            self.cancel_start(ticket)?;
            return Err(error);
        }
        self.changed();
        if replace.is_some() || self.places() {
            self.start_drawn(ticket, starting)?;
        } else {
            self.start_after_answer(ticket, starting);
        }
        if let Some(replace) = replace {
            self.close_tab_now(replace)?;
        }
        Ok(reserved)
    }

    /// A new pane beside `target`, `target` keeping `ratio` of the room: its
    /// id.
    fn open_split(
        &self,
        target: &str,
        direction: PaneLayoutDirection,
        ratio: f32,
        cwd: Option<&str>,
        env: Env,
        focus: bool,
    ) -> Answer<String> {
        self.restored()?;
        self.resolve_unpublished()?;
        let (pane, cwd) = {
            let mut topology = self.topology();
            let cwd = match cwd.filter(|cwd| !cwd.trim().is_empty()) {
                Some(cwd) => cwd.to_owned(),
                None => topology.pane(target)?.cwd.clone(),
            };
            let (workspace, _) = topology.pane_place(target)?;
            (topology.reserve_pane(&workspace)?, cwd)
        };
        let ticket = self.reserve_start(
            start_request(&pane, None, &cwd, env.clone(), None),
            None,
            true,
            Some(target),
        )?;
        if let Err(error) = ticket.check_open().and_then(|()| self.check()) {
            self.cancel_start(ticket)?;
            return Err(error);
        }
        let (drawn, starting) = Pane::starting(&cwd, None, None, &env);
        let added = ticket.publish(!self.places(), || {
            self.topology()
                .add_split(target, direction, ratio, &pane, drawn, focus)
        });
        if let Err(error) = added {
            // The pane beside it closed since the id was reserved.
            self.cancel_start(ticket)?;
            return Err(error);
        }
        self.changed();
        if self.places() {
            self.start_drawn(ticket, starting)?;
        } else {
            self.start_after_answer(ticket, starting);
        }
        Ok(pane)
    }

    /// Lays `root` out in a new tab of `workspace`, over `replace` when
    /// given: the panes `replace` runs that `root` names stay running in
    /// their places, the leaves it does not run start, and the panes of
    /// `replace` it leaves out end. Herdr makes every pane again; this
    /// engine keeps the ones it can, since ending a shell to draw it again
    /// would lose what runs there.
    fn apply_planned(
        &self,
        workspace: &str,
        replace: Option<&str>,
        label: Option<&str>,
        root: &ClosedLayoutNode,
    ) -> Answer<ClosedLayout> {
        self.restored()?;
        self.resolve_unpublished()?;
        let planned = self
            .topology()
            .plan_layout(workspace, replace, label, root)?;
        let mut started: Vec<String> = Vec::new();
        for (pane, spec) in &planned.new {
            let command = spec.command.as_deref();
            if let Err(error) = self.start_pane(pane, command, &spec.cwd, spec.env.clone(), None) {
                // Nothing of the layout is added: what started ends.
                let _ = self.end_panes(&started);
                return Err(error);
            }
            started.push(pane.clone());
        }
        let added = self.topology().add_planned(planned, replace);
        let (dropped, made) = match added {
            Ok(added) => added,
            Err(error) => {
                // The workspace or the replaced tab closed while the panes
                // started.
                let _ = self.end_panes(&started);
                return Err(error);
            }
        };
        self.changed();
        self.end_panes(&dropped)?;
        Ok(made)
    }

    fn close_tab_now(&self, tab: &str) -> Answer<()> {
        match self.end_and_remove(
            |topology| topology.tab_panes(tab),
            |topology| topology.remove_tab(tab),
        )? {
            // A pane of it closed the tab while its holders ended.
            Ok(_) | Err(EngineError::Refused { .. }) => {}
            Err(error) => return Err(error),
        }
        self.changed();
        Ok(())
    }

    /// Closes `pane`, whose program exited on its own; its holder ended
    /// with it.
    fn program_exited(&self, pane: &str) {
        // A pane the engine closed first, or never held, is left alone.
        let removed = self.topology().remove_pane(pane);
        if removed.is_ok() {
            self.changed();
        }
    }

    /// The pane, refused when the engine holds no such pane.
    fn known(&self, pane: &str) -> Result<(), EngineError> {
        self.topology().pane(pane).map(|_| ())
    }

    /// Reads wait for the start already represented by this pane, outside
    /// the topology lock. Its actual refusal is kept for later readers too.
    fn ready(&self, pane: &str) -> Result<(), EngineError> {
        let starting = self.topology().pane(pane)?.starting.clone();
        if let Some(starting) = starting {
            starting.wait()?;
        }
        self.known(pane)
    }

    fn read_styled(&self, pane: &str, read: StyledRead) -> Answer<PaneText> {
        self.ready(pane)?;
        let (text, truncated) = self
            .shared
            .holders
            .read_styled(pane, read)
            .map_err(holder_error)?;
        Ok(PaneText { text, truncated })
    }

    fn type_bytes(&self, pane: &str, bytes: Vec<u8>) -> Answer<()> {
        self.type_input(pane, PaneInput::Bytes(bytes))
    }

    /// Types into the pane through its holder, in order with every other
    /// key typed there; a paste and named keys are encoded by the holder for
    /// the program's modes when they are written (PRD
    /// engine-bundle-3-core-owns D-21).
    fn type_input(&self, pane: &str, input: PaneInput) -> Answer<()> {
        self.known(pane)?;
        self.shared.holders.input(pane, input).map_err(holder_error)
    }

    fn processes(&self, pane: &str) -> Answer<PaneProcessGroup> {
        self.ready(pane)?;
        let processes = self.shared.holders.processes(pane).map_err(holder_error)?;
        let (group, members) = match processes.foreground {
            Some(foreground) => (Some(foreground.group), foreground.members),
            None => (None, Vec::new()),
        };
        let (foreground_pids, foreground_names) = members.into_iter().unzip();
        Ok(PaneProcessGroup {
            shell_pid: Some(processes.shell),
            foreground_process_group_id: group,
            foreground_pids,
            foreground_names,
        })
    }
}

/// Ends the holders of `panes`, every one asked even when one fails: the
/// first failure, if any. A holder already gone is ended.
fn end_holders(holders: &dyn PaneHolders, panes: &[String]) -> Result<(), EngineError> {
    let mut first = None;
    for pane in panes {
        match holders.terminate(pane) {
            Ok(()) | Err(HolderError::NoPane { .. }) => {}
            Err(error) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "pane.end_failed",
                    "pane_id": pane,
                    "message": error.to_string(),
                }));
                first.get_or_insert(holder_error(error));
            }
        }
    }
    first.map_or(Ok(()), Err)
}

/// What a pane drawn `starting` becomes when its holder does not start.
#[derive(Clone, Copy)]
enum OnFailure {
    /// Taken out again, its tab with it when it was the only one: the call
    /// that drew it waits for the start and is refused with its reason.
    TakeOut,
    /// Kept where it was drawn, marked with why (`Pane::failed`), which the
    /// session gives as the pane's `restore_error`: the screen shows that
    /// its terminal could not start and offers to close it, as it does for
    /// a pane a restore could not start. The call that drew it was answered
    /// before the start ended, so the pane is where the operator learns.
    Mark,
}

/// What became of the pane a start drew `starting`.
#[derive(Debug, PartialEq, Eq)]
enum Settled {
    /// Its holder runs and the pane is in the topology.
    Running,
    /// It was closed while its holder started; a holder that started is
    /// ended.
    Closed,
    /// Its holder did not start and the pane changed as [`OnFailure`] says.
    Failed,
}

/// Settles the start of `pane`, drawn `starting`, once its holder started
/// or did not, `failed` saying why ([`OwnEngine::start_drawn`]).
fn settle_start(
    topology: &Mutex<Topology>,
    holders: &dyn PaneHolders,
    pane: &str,
    starting: &starts::Starting,
    result: &Result<(), EngineError>,
    on_failure: OnFailure,
) -> Settled {
    let failed = result.as_ref().err().map(ToString::to_string);
    let kept = {
        let mut topology = Held(topology.lock().unwrap_or_else(PoisonError::into_inner));
        starting.finish(result.clone());
        match topology.pane_mut(pane) {
            Ok(drawn) => {
                match (failed.as_deref(), on_failure) {
                    (None, _) => {}
                    (Some(why), OnFailure::Mark) => drawn.failed = Some(why.to_owned()),
                    (Some(_), OnFailure::TakeOut) => {
                        let _ = topology.remove_pane(pane);
                    }
                }
                true
            }
            Err(_) => false,
        }
    };
    match (failed, kept) {
        (None, true) => Settled::Running,
        (None, false) => {
            let _ = end_holders(holders, &[pane.to_owned()]);
            Settled::Closed
        }
        (Some(_), true) => Settled::Failed,
        (Some(_), false) => Settled::Closed,
    }
}

/// Starts `pane`'s holder running `program`, or the machine's shell after
/// `notice` when one is given.
fn start_request(
    pane: &str,
    program: Option<&[String]>,
    cwd: &str,
    env: Env,
    notice: Option<&str>,
) -> HolderStart {
    HolderStart {
        pane: pane.to_owned(),
        program: program.map(<[String]>::to_vec),
        cwd: cwd.to_owned(),
        env: env.into_iter().collect(),
        rows: START_ROWS,
        cols: START_COLS,
        notice: notice.map(str::to_owned),
    }
}

/// A holder's failure in the engine boundary's words: a pane the node does
/// not have is the refusal a pane the engine does not have is.
fn holder_error(error: HolderError) -> EngineError {
    match error {
        HolderError::NoPane { message } => EngineError::Refused {
            code: "pane_not_found".to_owned(),
            message,
        },
        HolderError::Refused { code, message } => EngineError::Refused {
            code: code.to_owned(),
            message,
        },
        HolderError::Unreachable { message } => EngineError::Unanswered(message),
    }
}

fn pane_scroll(scroll: HolderScroll) -> crate::find::pane::PaneScroll {
    crate::find::pane::PaneScroll {
        offset_from_bottom: scroll.offset_from_bottom,
        max_offset_from_bottom: scroll.max_offset_from_bottom,
    }
}

/// How long a prompt waits before its Enter, as Herdr's `agent.prompt`
/// waits.
const PROMPT_SUBMIT_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

/// The source a session this engine knows is reported under: what the
/// engine itself saw at a start or a report, never an agent's integration.
const NATIVE_SESSION_SOURCE: &str = "hide";

/// The agent kind a pane runs, in the name Herdr would give it.
fn shown_kind(catalog: &hide_agent_adapter::definition::Catalog, kind: &str) -> String {
    agents::definition(catalog, kind).map_or_else(|| kind.to_owned(), agents::kind_name)
}

/// What a letter checks of the agent in `pane` before it is typed to.
fn delivery_of(
    catalog: &hide_agent_adapter::definition::Catalog,
    pane: String,
    agent: &agents::PaneAgent,
) -> DeliveryAgent {
    DeliveryAgent {
        terminal_id: pane.clone(),
        name: agent.name.clone().unwrap_or_else(|| pane.clone()),
        pane_id: pane,
        kind: Some(shown_kind(catalog, &agent.kind)),
        session: agent
            .session
            .as_deref()
            .and_then(crate::engine::tokens::session_digest),
        status: agent.live.status.as_str().to_owned(),
        state_change_seq: agent.live.seq,
        readiness: Readiness::of(
            Some(agent.live.shown),
            Some(agent.launch_pending(std::time::Instant::now())),
        ),
    }
}

/// Hide's key as the holder names it; the holder encodes it for the
/// program's keyboard modes, as Herdr encodes a named key for its pane.
fn holder_key(key: PaneKey) -> HolderKey {
    match key {
        PaneKey::Esc => HolderKey::Escape,
        PaneKey::Enter => HolderKey::Enter,
        PaneKey::Tab => HolderKey::Tab,
        PaneKey::ShiftTab => HolderKey::ShiftTab,
        PaneKey::Up => HolderKey::Up,
        PaneKey::Down => HolderKey::Down,
        PaneKey::Left => HolderKey::Left,
        PaneKey::Right => HolderKey::Right,
        PaneKey::Backspace => HolderKey::Backspace,
        PaneKey::CtrlC => HolderKey::CtrlC,
    }
}

/// What `send_keys` types for `keys`, in their order: a named key for the
/// holder to encode, and as bytes a single character (`/`) or a Ctrl chord
/// of a letter (`ctrl+o`, its control byte), which Herdr's `send_keys` takes
/// too and an agent's own search is opened with (`crate::agent_find`).
/// Adjacent bytes go as one input; the holder writes its inputs in order.
fn typed_keys(keys: &[&str]) -> Answer<Vec<PaneInput>> {
    let mut typed: Vec<PaneInput> = Vec::new();
    for key in keys {
        if let Ok(named) = named_key(key) {
            match typed.last_mut() {
                Some(PaneInput::Keys(run)) => run.push(holder_key(named)),
                _ => typed.push(PaneInput::Keys(vec![holder_key(named)])),
            }
            continue;
        }
        let bytes = typed_bytes(key).ok_or_else(|| EngineError::Refused {
            code: "unknown_key".to_owned(),
            message: format!("the core's own engine presses no key named {key:?}"),
        })?;
        match typed.last_mut() {
            Some(PaneInput::Bytes(run)) => run.extend(bytes),
            _ => typed.push(PaneInput::Bytes(bytes)),
        }
    }
    Ok(typed)
}

/// The bytes of a single character, or the control byte of a Ctrl chord of
/// a letter (`ctrl+o` or `ctrl-o`).
fn typed_bytes(key: &str) -> Option<Vec<u8>> {
    let mut chars = key.chars();
    if let (Some(single), None) = (chars.next(), chars.next()) {
        return Some(single.to_string().into_bytes());
    }
    let lower = key.trim().to_ascii_lowercase();
    let letter = lower
        .strip_prefix("ctrl+")
        .or_else(|| lower.strip_prefix("ctrl-"))?;
    match letter.as_bytes() {
        [letter @ b'a'..=b'z'] => Some(vec![letter & 0x1f]),
        _ => None,
    }
}

/// A key a caller names, by Hide's name or Herdr's (`ctrl+c`, `escape`).
fn named_key(name: &str) -> Answer<PaneKey> {
    let spelled = name.trim().to_ascii_lowercase().replace('+', "-");
    let spelled = match spelled.as_str() {
        "escape" => "esc",
        "return" => "enter",
        other => other,
    };
    PaneKey::parse(spelled).ok_or_else(|| EngineError::Refused {
        code: "unknown_key".to_owned(),
        message: format!("the core's own engine presses no key named {name:?}"),
    })
}

impl<'g> Engine for OwnEngine<'g> {
    fn name(&self) -> &'static str {
        "own_engine"
    }

    fn gated<'a>(&'a self, gate: Gate<'a>) -> Box<dyn Engine + 'a> {
        let mut gates: Vec<Gate<'a>> = self.gates.clone();
        gates.push(gate);
        Box::new(OwnEngine {
            shared: Arc::clone(&self.shared),
            gates,
        })
    }

    fn changes(&self, wake: Wake) -> Changes {
        *self
            .shared
            .wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(wake);
        Changes::Pulled
    }

    fn set_agent_definitions(&self, catalog: Arc<hide_agent_adapter::definition::Catalog>) {
        let before = self.shared.definitions.get();
        self.shared.definitions.set(Arc::clone(&catalog));
        // A pane whose agent's rules changed is given the new ones; every
        // other holder keeps reading with what it has.
        let running = self.topology().agent_kinds();
        for (pane, kind) in running {
            let rules = |catalog| agents::definition(catalog, &kind).and_then(agents::screen_rules);
            let now = rules(&catalog);
            if rules(&before) != now {
                self.give_rules(&pane);
            }
        }
        self.changed();
    }

    fn hook_session(&self, pane: &str, kind: &str, session: &str) -> Answer<bool> {
        self.check()?;
        let catalog = self.shared.definitions.get();
        let definition =
            agents::definition(&catalog, kind).ok_or_else(|| EngineError::Refused {
                code: "unknown_agent".to_owned(),
                message: format!("no agent definition names {kind}"),
            })?;
        if self
            .topology()
            .hook_session(pane, &definition.id, session)?
        {
            self.changed();
        }
        // The first session the resumed agent names is the conversation it
        // resumed, whatever id its runtime gives it now.
        let mut cut = self
            .shared
            .cut
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(cut.get(pane) == Some(&definition.id) && cut.remove(pane).is_some())
    }

    fn session(&self, _wait: Wait) -> Answer<ProjectionState> {
        self.check()?;
        let catalog = self.shared.definitions.get();
        self.topology().state(&catalog)
    }

    fn session_payload(&self, wait: Wait) -> Answer<SessionSnapshotPayload> {
        Ok(self.session(wait)?.project())
    }

    fn pane_folders(&self, wait: Wait) -> Answer<Vec<(String, Option<String>)>> {
        Ok(self
            .session(wait)?
            .panes
            .into_iter()
            .map(|pane| (pane.pane_id, pane.cwd))
            .collect())
    }

    fn pane_places(&self, wait: Wait) -> Answer<Vec<PanePlace>> {
        Ok(self
            .session(wait)?
            .panes
            .into_iter()
            .map(|pane| PanePlace {
                pane_id: pane.pane_id,
                cwd: pane.cwd,
                foreground_cwd: None,
            })
            .collect())
    }

    fn agents(&self, wait: Wait) -> Answer<Vec<ProjectedAgent>> {
        Ok(self.session(wait)?.agents)
    }

    fn ping(&self, _wait: Wait) -> Answer<ServerPong> {
        self.check()?;
        Ok(ServerPong {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol: 0,
            live_handoff: false,
        })
    }

    fn subscribe(&self, _timeout: Duration) -> Answer<EventStream> {
        self.check()?;
        Err(EngineError::Refused {
            code: "no_event_stream".to_owned(),
            message: "the core's own engine wakes its watcher instead of streaming events"
                .to_owned(),
        })
    }

    fn create_workspace(
        &self,
        _wait: Wait,
        cwd: &str,
        label: &str,
        focus: bool,
        env: Env,
    ) -> Answer<(String, String, String)> {
        self.check()?;
        self.restored()?;
        self.resolve_unpublished()?;
        let reserved = self.topology().reserve_workspace();
        self.start_placed(&reserved.pane, None, cwd, env.clone())?;
        self.topology().add_workspace(
            &reserved,
            label,
            Pane::started(cwd, None, None, &env),
            focus,
        );
        self.changed();
        Ok((reserved.workspace, reserved.tab, reserved.pane))
    }

    fn workspace_active_tab(&self, _wait: Wait, workspace: &str) -> Answer<String> {
        self.check()?;
        self.topology().active_tab(workspace)
    }

    fn focused_workspace(&self, _wait: Wait) -> Answer<Option<String>> {
        self.check()?;
        Ok(self.topology().focused_workspace())
    }

    fn workspace_tokens(&self, _wait: Wait, token: &str) -> Answer<Vec<(String, Option<String>)>> {
        self.check()?;
        Ok(self.topology().workspace_tokens(token))
    }

    fn rename_workspace(&self, _wait: Wait, workspace: &str, label: &str) -> Answer<()> {
        self.check()?;
        self.topology().rename_workspace(workspace, label)?;
        self.changed();
        Ok(())
    }

    fn focus_workspace(&self, _wait: Wait, workspace: &str) -> Answer<()> {
        self.check()?;
        self.topology().focus_workspace(workspace)?;
        self.changed();
        Ok(())
    }

    fn report_workspace_token(
        &self,
        _wait: Wait,
        workspace: &str,
        token: &str,
        value: Option<&str>,
    ) -> Answer<()> {
        self.check()?;
        self.topology()
            .set_workspace_token(workspace, token, value)?;
        self.changed();
        Ok(())
    }

    fn create_tab(
        &self,
        _wait: Wait,
        workspace: &str,
        cwd: &str,
        label: &str,
        focus: bool,
        env: Env,
    ) -> Answer<(String, String)> {
        self.check()?;
        self.open_tab(workspace, None, label, None, focus, cwd, env, None)
    }

    fn create_replacement_tab(
        &self,
        _wait: Wait,
        workspace: &str,
        cwd: &str,
        env: Env,
    ) -> Answer<(String, String)> {
        self.check()?;
        self.open_tab(workspace, None, "", None, false, cwd, env, None)
    }

    fn launch_tab(
        &self,
        _wait: Wait,
        workspace: &str,
        replace: Option<&str>,
        label: &str,
        focus: bool,
        cwd: &str,
        env: Env,
        command: &[String],
    ) -> Answer<(String, String)> {
        self.check()?;
        self.open_tab(
            workspace,
            replace,
            label,
            None,
            focus,
            cwd,
            env,
            Some(command),
        )
    }

    fn launch_checkout_tab(
        &self,
        _wait: Wait,
        workspace: &str,
        seed_tab: Option<&str>,
        cwd: &str,
        label: &str,
        env: Env,
        command: Option<&[String]>,
        claim: &str,
    ) -> Answer<(String, String)> {
        self.check()?;
        self.open_tab(
            workspace,
            seed_tab,
            label,
            Some(claim),
            true,
            cwd,
            env,
            command,
        )
    }

    fn close_tab(&self, _wait: Wait, tab: &str) -> Answer<()> {
        self.check()?;
        self.close_tab_now(tab)
    }

    fn focus_tab(&self, _wait: Wait, tab: &str) -> Answer<()> {
        self.check()?;
        self.topology().focus_tab(tab)?;
        self.changed();
        Ok(())
    }

    fn rename_tab(&self, _wait: Wait, tab: &str, label: &str) -> Answer<()> {
        self.check()?;
        self.topology().rename_tab(tab, label)?;
        self.changed();
        Ok(())
    }

    fn move_tab(&self, _wait: Wait, tab: &str, index: usize) -> Answer<Vec<String>> {
        self.check()?;
        let order = self.topology().move_tab(tab, index)?;
        self.changed();
        Ok(order)
    }

    fn pane_layout(&self, _wait: Wait, pane: &str) -> Answer<SessionLayoutPayload> {
        self.check()?;
        self.topology().pane_layout(pane)
    }

    fn export_layout(&self, _wait: Wait, tab: &str) -> Answer<ClosedLayout> {
        self.check()?;
        self.topology().export(tab)
    }

    fn apply_layout(
        &self,
        _wait: Wait,
        workspace: &str,
        tab: Option<&str>,
        tab_label: Option<&str>,
        root: &ClosedLayoutNode,
    ) -> Answer<ClosedLayout> {
        self.check()?;
        layout::check_layout(root).map_err(|why| EngineError::Refused {
            code: "invalid_layout".to_owned(),
            message: why,
        })?;
        self.apply_planned(workspace, tab, tab_label, root)
    }

    fn split_pane(
        &self,
        _wait: Wait,
        pane: &str,
        direction: crate::live::PaneSplitDirection,
        cwd: Option<&str>,
    ) -> Answer<String> {
        self.check()?;
        let direction = match direction {
            crate::live::PaneSplitDirection::Right => PaneLayoutDirection::Right,
            crate::live::PaneSplitDirection::Down => PaneLayoutDirection::Down,
        };
        self.open_split(pane, direction, 0.5, cwd, Env::new(), true)
    }

    fn split_pane_at(
        &self,
        _wait: Wait,
        pane: &str,
        direction: ClosedSplitDirection,
        cwd: &str,
        ratio: f32,
        env: Env,
    ) -> Answer<String> {
        self.check()?;
        let direction = match direction {
            ClosedSplitDirection::Right => PaneLayoutDirection::Right,
            ClosedSplitDirection::Down => PaneLayoutDirection::Down,
        };
        self.open_split(
            pane,
            direction,
            layout::valid_ratio(ratio),
            Some(cwd),
            env,
            true,
        )
    }

    fn split_beside(
        &self,
        _wait: Wait,
        parent: &str,
        cwd: Option<&str>,
        env: Env,
    ) -> Answer<String> {
        self.check()?;
        self.open_split(parent, PaneLayoutDirection::Right, 0.5, cwd, env, false)
    }

    fn split_replacement(&self, _wait: Wait, pane: &str, cwd: &str, env: Env) -> Answer<String> {
        self.check()?;
        self.open_split(pane, PaneLayoutDirection::Right, 0.5, Some(cwd), env, false)
    }

    fn close_pane(&self, _wait: Wait, pane: &str) -> Answer<()> {
        self.check()?;
        self.known(pane)?;
        match self.end_and_remove(
            |_| Ok(vec![pane.to_owned()]),
            |topology| topology.remove_pane(pane),
        )? {
            Ok(Closed { .. }) => {}
            // Its tab closed while its holder ended.
            Err(EngineError::Refused { .. }) => {}
            Err(error) => return Err(error),
        }
        self.changed();
        Ok(())
    }

    fn focus_pane(&self, _wait: Wait, pane: &str) -> Answer<()> {
        self.check()?;
        self.topology().focus_pane(pane)?;
        self.changed();
        Ok(())
    }

    fn rename_pane(&self, _wait: Wait, pane: &str, label: Option<&str>) -> Answer<()> {
        self.check()?;
        self.topology().rename_pane(pane, label)?;
        self.changed();
        Ok(())
    }

    fn toggle_zoom(&self, _wait: Wait, pane: &str) -> Answer<bool> {
        self.check()?;
        let changed = self.topology().zoom(pane, None)?;
        if changed {
            self.changed();
        }
        Ok(changed)
    }

    fn set_zoom(&self, _wait: Wait, pane: &str, zoom: Zoom) -> Answer<()> {
        self.check()?;
        if self.topology().zoom(pane, Some(zoom == Zoom::On))? {
            self.changed();
        }
        Ok(())
    }

    fn resize_pane(
        &self,
        _wait: Wait,
        pane: &str,
        direction: crate::live::PaneResizeDirection,
        amount: f32,
    ) -> Answer<bool> {
        self.check()?;
        let changed = self.topology().resize(pane, direction, amount)?;
        if changed {
            self.changed();
        }
        Ok(changed)
    }

    fn swap_panes(&self, _wait: Wait, first: &str, second: &str) -> Answer<()> {
        self.check()?;
        self.topology().swap(first, second)?;
        self.changed();
        Ok(())
    }

    fn move_pane_to_new_tab(
        &self,
        _wait: Wait,
        pane: &str,
        workspace: &str,
        label: &str,
    ) -> Answer<PaneJoinResult> {
        self.check()?;
        let moved = self.topology().move_to_new_tab(pane, workspace, label)?;
        if moved.changed {
            self.changed();
        }
        Ok(moved)
    }

    fn move_pane_beside(
        &self,
        _wait: Wait,
        pane: &str,
        target_tab: &str,
        target_pane: &str,
        down: bool,
    ) -> Answer<PaneJoinResult> {
        self.check()?;
        let moved = self
            .topology()
            .move_beside(pane, target_tab, target_pane, down)?;
        if moved.changed {
            self.changed();
        }
        Ok(moved)
    }

    fn pane_session(
        &self,
        _wait: Wait,
        pane: &str,
    ) -> Answer<(Option<String>, Option<PaneSession>)> {
        self.check()?;
        let catalog = self.shared.definitions.get();
        let topology = self.topology();
        let held = topology.pane(pane)?;
        let session = held
            .agent
            .as_ref()
            .filter(|agent| agent.listed(std::time::Instant::now()))
            .and_then(|agent| {
                Some(PaneSession {
                    agent: agents::definition(&catalog, &agent.kind)
                        .map_or_else(|| agent.kind.clone(), agents::kind_name),
                    kind: agents::session_kind(agent.session.as_deref()?),
                    value: agent.session.clone()?,
                })
            });
        Ok((Some(held.cwd.clone()), session))
    }

    fn pane_terminal(&self, _wait: Wait, pane: &str) -> Answer<String> {
        self.check()?;
        // One terminal per pane until a restore starts a pane again.
        self.known(pane)?;
        Ok(pane.to_owned())
    }

    fn pane_token(&self, _wait: Wait, pane: &str, token: &str) -> Answer<Option<String>> {
        self.check()?;
        let topology = self.topology();
        // The shell's first prompt is marked on its screen (OSC 133;A), which
        // the holder counts, never by a token a program reports.
        if token == crate::agent_start::PROMPT_TOKEN {
            return Ok((topology.pane(pane)?.live.facts.prompts > 0).then(|| "1".to_owned()));
        }
        topology.pane_token(pane, token)
    }

    fn pane_agent_status(&self, _wait: Wait, pane: &str) -> Answer<String> {
        self.check()?;
        let now = std::time::Instant::now();
        let topology = self.topology();
        Ok(topology
            .pane(pane)?
            .agent
            .as_ref()
            .filter(|agent| agent.listed(now))
            .map_or(agents::Status::Unknown, |agent| agent.live.status)
            .as_str()
            .to_owned())
    }

    /// Where the holder's screen is in its scrollback. A holder built
    /// before scroll asks reports none, as a Herdr pane with no scroll
    /// metrics does, so a find on it opens the agent's own search.
    fn pane_scroll(
        &self,
        _wait: Wait,
        pane: &str,
    ) -> Answer<Option<crate::find::pane::PaneScroll>> {
        self.check()?;
        self.known(pane)?;
        match self.shared.holders.scroll(pane, None) {
            Ok(scroll) => Ok(Some(pane_scroll(scroll))),
            Err(HolderError::Refused { code, .. }) if code == UNSUPPORTED_HOLDER => Ok(None),
            Err(error) => Err(holder_error(error)),
        }
    }

    fn scroll_pane(
        &self,
        _wait: Wait,
        pane: &str,
        offset_from_bottom: u64,
    ) -> Answer<Option<crate::find::pane::PaneScroll>> {
        self.check()?;
        self.known(pane)?;
        let scroll = self
            .shared
            .holders
            .scroll(pane, Some(offset_from_bottom))
            .map_err(holder_error)?;
        Ok(Some(pane_scroll(scroll)))
    }

    fn report_pane_tokens(
        &self,
        _wait: Wait,
        pane: &str,
        tokens: &BTreeMap<String, Option<String>>,
    ) -> Answer<()> {
        self.check()?;
        self.topology().set_pane_tokens(pane, tokens)?;
        self.changed();
        Ok(())
    }

    fn report_agent_session(
        &self,
        _wait: Wait,
        pane: &str,
        kind: &str,
        session: &str,
        _seq: u64,
    ) -> Answer<()> {
        self.check()?;
        let catalog = self.shared.definitions.get();
        let definition =
            agents::definition(&catalog, kind).ok_or_else(|| EngineError::Refused {
                code: "unknown_agent".to_owned(),
                message: format!("no agent definition names {kind}"),
            })?;
        if self
            .topology()
            .set_agent_session(pane, &definition.id, session)?
        {
            self.changed();
        }
        Ok(())
    }

    fn read_pane(&self, _wait: Wait, pane: &str, source: &str, lines: u32) -> Answer<PaneText> {
        self.check()?;
        if !READ_SOURCES.contains(&source) {
            return Err(EngineError::Refused {
                code: UNKNOWN_SOURCE.to_owned(),
                message: format!(
                    "the core's own engine reads a pane from {}, not {source:?}",
                    READ_SOURCES.join(", ")
                ),
            });
        }
        self.ready(pane)?;
        if source == "visible" {
            let text = self
                .shared
                .holders
                .read_visible(pane)
                .map_err(holder_error)?;
            return Ok(PaneText {
                text,
                truncated: false,
            });
        }
        let lines = u16::try_from(lines.min(READ_LINES_CAP)).unwrap_or(u16::MAX);
        let (text, truncated) = self
            .shared
            .holders
            .read(pane, lines)
            .map_err(holder_error)?;
        Ok(PaneText { text, truncated })
    }

    fn read_screen(&self, _wait: Wait, pane: &str) -> Answer<PaneText> {
        self.check()?;
        self.read_styled(pane, StyledRead::Screen)
    }

    fn read_rows(&self, _wait: Wait, pane: &str, lines: u32) -> Answer<PaneText> {
        self.check()?;
        let lines = u16::try_from(lines.min(READ_LINES_CAP)).unwrap_or(u16::MAX);
        self.read_styled(pane, StyledRead::Rows(lines))
    }

    fn send_text(&self, _wait: Wait, pane: &str, text: &str) -> Answer<()> {
        self.check()?;
        self.type_bytes(pane, text.as_bytes().to_vec())
    }

    fn send_keys(&self, _wait: Wait, pane: &str, keys: &[&str]) -> Answer<()> {
        self.check()?;
        for input in typed_keys(keys)? {
            self.type_input(pane, input)?;
        }
        Ok(())
    }

    /// The text as it is, then Enter encoded for the program's keyboard
    /// modes, as Herdr submits text.
    fn submit_text(&self, _wait: Wait, pane: &str, text: &str) -> Answer<()> {
        self.check()?;
        self.type_bytes(pane, text.as_bytes().to_vec())?;
        self.type_input(pane, PaneInput::Keys(vec![HolderKey::Enter]))
    }

    fn press_key(&self, _wait: Wait, pane: &str, key: &str) -> Answer<()> {
        self.check()?;
        let key = holder_key(named_key(key)?);
        self.type_input(pane, PaneInput::Keys(vec![key]))
    }

    fn press_pane_keys(&self, _wait: Wait, pane: &str, keys: &[PaneKey]) -> Answer<()> {
        self.check()?;
        let keys = keys.iter().copied().map(holder_key).collect();
        self.type_input(pane, PaneInput::Keys(keys))
    }

    fn paste_text(&self, _wait: Wait, pane: &str, text: &str) -> Answer<()> {
        self.check()?;
        self.type_input(pane, PaneInput::Paste(text.as_bytes().to_vec()))
    }

    fn notify(&self, _wait: Wait, _title: &str, _body: &str) -> Answer<Notified> {
        self.check()?;
        Ok(Err("the core's own engine shows no notification"))
    }

    fn pane_processes(&self, _wait: Wait, pane: &str) -> Answer<PaneProcessGroup> {
        self.check()?;
        self.processes(pane)
    }

    fn foreground_program(&self, _wait: Wait, pane: &str) -> Answer<Option<String>> {
        self.check()?;
        Ok(self
            .processes(pane)?
            .foreground_program()
            .map(str::to_owned))
    }

    fn resumed_session(
        &self,
        _wait: Wait,
        pane: &str,
        agent: hide_session::Agent,
    ) -> Answer<Option<(String, String)>> {
        self.check()?;
        let catalog = self.shared.definitions.get();
        let Some(definition) = agents::definition(&catalog, agent.as_str()) else {
            return Ok(None);
        };
        let topology = self.topology();
        let held = topology.pane(pane)?;
        // The program the holder sees in the foreground, with its arguments,
        // is the one asked about only when its definition names it.
        let resumed = held
            .live
            .facts
            .foreground
            .as_ref()
            .filter(|program| {
                definition
                    .executables
                    .iter()
                    .any(|executable| executable.eq_ignore_ascii_case(&program.name))
            })
            .and_then(|program| agents::resumed_in(definition, &program.argv));
        Ok(resumed.map(|session| {
            let kind = match agents::session_kind(&session) {
                PaneSessionKind::Id => "id",
                PaneSessionKind::Path => "path",
            };
            (kind.to_owned(), session)
        }))
    }

    fn current_pane(&self, _wait: Wait, pane: &str) -> Answer<String> {
        self.check()?;
        self.known(pane)?;
        Ok(pane.to_owned())
    }

    fn pane_identity(&self, _wait: Wait, pane: &str) -> Answer<PaneProof> {
        self.check()?;
        // The pane's record names the start that runs and its program, with
        // the time the system gave that program (D-9): a pid the system
        // reused since does not match it. A pane id never moves on this
        // engine, so the pane named is the pane proved.
        self.ready(pane)?;
        let proof = self.shared.holders.proof(pane).map_err(holder_error)?;
        Ok(PaneProof {
            pane_id: pane.to_owned(),
            terminal_id: pane.to_owned(),
            shell_pid: i32::try_from(proof.program).map_err(|_| {
                refused(
                    "pane_not_running",
                    format!(
                        "pane {pane}'s program pid {} is out of range",
                        proof.program
                    ),
                )
            })?,
            shell_started: Some(proof.started),
        })
    }

    fn inspect_pane(
        &self,
        wait: Wait,
        pane: &str,
    ) -> Option<Answer<hide_node_link::panes::PaneIdentity>> {
        Some(self.pane_identity(wait, pane).and_then(|proof| {
            Ok(hide_node_link::panes::PaneIdentity {
                terminal_id: proof.terminal_id,
                shell_pid: proof.shell_pid,
                shell_started: proof.shell_started.ok_or_else(|| {
                    refused("pane_not_running", format!("pane {pane} has no start"))
                })?,
            })
        }))
    }

    fn prepare_start(&self, launch: AgentLaunch) -> Result<PreparedStart, String> {
        let catalog = self.shared.definitions.get();
        let definition = agents::definition(&catalog, &launch.kind)
            .ok_or_else(|| format!("no agent definition names {}", launch.kind))?;
        let argv = agents::start_argv(
            definition,
            crate::codex_launch::start_arguments(
                &definition.id,
                launch.codex_daemon,
                launch.args.clone(),
            )?,
        )?;
        // The longest of the shells' lines, so the start is bounded as the
        // shell it meets will type it.
        let line = shell_line::longest(|shell| shell_line::command(&argv, shell))
            .ok_or_else(|| format!("the {} start cannot be typed into a shell", definition.id))?;
        Ok(PreparedStart::new(launch, line))
    }

    fn start_agent(&self, _wait: Wait, start: &PreparedStart) -> Answer<StartedAgent> {
        self.check()?;
        let launch = start.launch();
        let pane = launch.pane_id.as_str();
        let catalog = self.shared.definitions.get();
        let definition = agents::definition(&catalog, &launch.kind).ok_or_else(|| {
            refused(
                "unknown_agent",
                format!("no agent definition names {}", launch.kind),
            )
        })?;
        let args = crate::codex_launch::start_arguments(
            &definition.id,
            launch.codex_daemon,
            launch.args.clone(),
        )
        .map_err(|message| refused("agent_start_refused", message))?;
        let argv = agents::start_argv(definition, args)
            .map_err(|message| refused("agent_start_refused", message))?;
        // Herdr's condition for a start: the shell holds the terminal alone.
        let processes = self.processes(pane)?;
        if !processes.shell_holds_terminal() {
            return Err(refused(
                "agent_pane_busy",
                format!("pane {pane} runs a program; an agent starts only in an idle shell"),
            ));
        }
        let shell = shell_line::Shell::of(processes.shell_name(), self.shared.windows).ok_or_else(|| {
            refused(
                "agent_start_refused",
                format!("pane {pane} runs cmd.exe, which takes no start line; start the agent from PowerShell"),
            )
        })?;
        let line = shell_line::command(&argv, shell).ok_or_else(|| {
            refused(
                "agent_start_refused",
                format!("the {} start cannot be typed into a shell", definition.id),
            )
        })?;
        self.type_agent_start(
            pane,
            agents::PaneAgent::started(
                &definition.id,
                Some(&launch.name),
                agents::resumed_in(definition, &argv),
            ),
            &line,
        )?;
        Ok(StartedAgent {
            pane_id: pane.to_owned(),
            terminal_id: Some(pane.to_owned()),
        })
    }

    fn agent_state(&self, _wait: Wait, target: &str) -> Answer<AgentState> {
        self.check()?;
        let catalog = self.shared.definitions.get();
        let topology = self.topology();
        let (_, _, agent) = topology.agent(target)?;
        Ok(AgentState {
            agent: Some(shown_kind(&catalog, &agent.kind)),
            status: agent.live.status.as_str().to_owned(),
            state_change_seq: agent.live.seq,
        })
    }

    fn delivery_agent(&self, _wait: Wait, target: &str) -> Answer<DeliveryAgent> {
        self.check()?;
        let catalog = self.shared.definitions.get();
        let topology = self.topology();
        let (pane, _, agent) = topology.agent(target)?;
        Ok(delivery_of(&catalog, pane, agent))
    }

    fn observed_agent(
        &self,
        _wait: Wait,
        target: &str,
    ) -> Answer<(
        DeliveryAgent,
        Option<hide_agent_hooks::observation::ObservationNativeSession>,
    )> {
        self.check()?;
        let catalog = self.shared.definitions.get();
        let topology = self.topology();
        let (pane, _, agent) = topology.agent(target)?;
        let native = agent.session.as_ref().map(|session| {
            hide_agent_hooks::observation::ObservationNativeSession {
                source: NATIVE_SESSION_SOURCE.to_owned(),
                agent: shown_kind(&catalog, &agent.kind),
                kind: match agents::session_kind(session) {
                    PaneSessionKind::Id => "id",
                    PaneSessionKind::Path => "path",
                }
                .to_owned(),
                value: session.clone(),
            }
        });
        Ok((delivery_of(&catalog, pane, agent), native))
    }

    #[allow(clippy::disallowed_methods)] // Herdr's submit delay, a production wait
    fn prompt_agent(&self, _wait: Wait, target: &str, text: &str) -> Answer<()> {
        self.check()?;
        if text.is_empty() {
            return Err(refused(
                "empty_agent_prompt",
                "agent prompt must not be empty".to_owned(),
            ));
        }
        let (pane, paste) = {
            let topology = self.topology();
            let (pane, held, agent) = topology.agent(target)?;
            if agent.live.status == agents::Status::Blocked {
                return Err(refused(
                    "agent_blocked",
                    format!("agent {target} is blocked and requires interactive input"),
                ));
            }
            if !agent.live.shown || agent.launch_pending(std::time::Instant::now()) {
                return Err(refused(
                    "agent_not_ready",
                    format!("agent {target} is not ready for input"),
                ));
            }
            (pane, held.live.facts.bracketed_paste)
        };
        // Herdr's `agent.prompt`: the text as one paste when the program
        // takes bracketed paste, then Enter a moment later, so a program that
        // reads a fast Enter as part of the paste still submits. Typed as a
        // letter, so the operator's keys typed meanwhile wait until the
        // Enter was handed over instead of landing inside it (D-13).
        let body = if paste {
            PaneInput::Paste(text.as_bytes().to_vec())
        } else {
            PaneInput::Bytes(text.as_bytes().to_vec())
        };
        self.known(&pane)?;
        self.shared
            .holders
            .letter(
                &pane,
                body,
                PaneInput::Keys(vec![HolderKey::Enter]),
                PROMPT_SUBMIT_DELAY,
            )
            .map_err(holder_error)
    }

    fn create_worktree(
        &self,
        _wait: Wait,
        cwd: &str,
        branch: &str,
        base: Option<&str>,
        label: Option<&str>,
        path: Option<&str>,
        focus: bool,
    ) -> Answer<CreatedWorktree> {
        self.check()?;
        self.make_worktree(cwd, branch, base, label, path, focus)
    }

    fn worktree_path(&self, _wait: Wait, cwd: &str, branch: &str) -> Answer<Option<String>> {
        self.check()?;
        self.listed_worktree(cwd, branch)
    }

    fn open_worktree(
        &self,
        _wait: Wait,
        path: &str,
        repository_root: &str,
        focus: bool,
    ) -> Answer<OpenedWorktree> {
        self.check()?;
        self.open_checkout(path, repository_root, focus)
    }

    fn remove_worktree(&self, _wait: Wait, workspace: &str) -> Answer<()> {
        self.check()?;
        self.drop_worktree(workspace)
    }
}

#[cfg(test)]
mod tests {
    // An agent's own search opens with `ctrl+o` and `/` (`crate::agent_find`):
    // the own engine types them as the bytes Herdr would, in order with the
    // named keys around them.
    #[test]
    fn send_keys_types_a_character_and_a_ctrl_letter_in_order_with_named_keys() {
        use super::{HolderKey, PaneInput, typed_keys};
        assert_eq!(
            typed_keys(&["ctrl+o", "/"]).unwrap(),
            vec![PaneInput::Bytes(vec![0x0f, b'/'])]
        );
        assert_eq!(
            typed_keys(&["escape", "x", "ctrl-c", "enter"]).unwrap(),
            vec![
                PaneInput::Keys(vec![HolderKey::Escape]),
                PaneInput::Bytes(b"x".to_vec()),
                PaneInput::Keys(vec![HolderKey::CtrlC, HolderKey::Enter]),
            ]
        );
        assert!(typed_keys(&["f13"]).is_err());
        assert!(typed_keys(&["ctrl+1"]).is_err());
    }

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Condvar, Mutex};

    use hide_node_link::holders::{HolderProcesses, HolderRecovery};

    use crate::sidebar::SessionSnapshotPayload;

    use super::*;

    type TerminateHold = (
        String,
        std::sync::mpsc::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    );
    type InputHold = (
        std::sync::mpsc::Sender<()>,
        std::sync::mpsc::Receiver<Result<(), HolderError>>,
    );

    /// Holders that start every pane until told to refuse, as a node whose
    /// holder program cannot start refuses, and say what they found of the
    /// last daemon's holders once the test hands them that.
    #[derive(Default)]
    struct Holders {
        refuse: AtomicBool,
        /// One pane whose holder does not start, every other one starting.
        refuse_pane: Mutex<Option<String>>,
        /// Starts under way now and the most that ever were at once.
        starting: (
            std::sync::atomic::AtomicUsize,
            std::sync::atomic::AtomicUsize,
        ),
        /// The next start says which pane it starts on the first and waits
        /// on the second before it answers.
        hold: Mutex<
            Option<(
                std::sync::mpsc::Sender<String>,
                std::sync::mpsc::Receiver<()>,
            )>,
        >,
        started: Mutex<Vec<HolderStart>>,
        reserved: Mutex<Vec<HolderStart>>,
        lose_reserve: AtomicBool,
        reserve_hold: Mutex<
            Option<(
                std::sync::mpsc::Sender<String>,
                std::sync::mpsc::Receiver<()>,
            )>,
        >,
        lose_cancel: AtomicBool,
        cancel_notice: Mutex<Option<std::sync::mpsc::Sender<String>>>,
        terminate_hold: Mutex<Option<TerminateHold>>,
        lose_retire: AtomicBool,
        issued: std::sync::atomic::AtomicU64,
        retired: Mutex<Vec<u64>>,
        ended: Mutex<Vec<String>>,
        found: (Mutex<Option<Result<HolderRecovery, HolderError>>>, Condvar),
        ends: Mutex<Option<hide_node_link::holders::PaneEnded>>,
        facts: Mutex<Option<hide_node_link::holders::PaneFactsSink>>,
        /// The rules given to each pane, by agent id, in order.
        rules: (Mutex<Given>, Condvar),
        rules_hold: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
        typed: (Mutex<Typed>, Condvar),
        /// The next input announces admission and waits for its outcome.
        input_hold: Mutex<Option<InputHold>>,
        /// The next holder reports facts before its pane is published.
        start_facts: Mutex<Option<PaneFacts>>,
        /// What every pane's terminal holds; `None` answers unreachable.
        processes: Mutex<Option<HolderProcesses>>,
        /// Panes whose terminal runs another program in its foreground.
        busy: Mutex<BTreeSet<String>>,
    }

    impl Holders {
        fn hold_input(
            &self,
        ) -> (
            std::sync::mpsc::Receiver<()>,
            std::sync::mpsc::Sender<Result<(), HolderError>>,
        ) {
            let (began, admitted) = std::sync::mpsc::channel();
            let (outcome, finish) = std::sync::mpsc::channel();
            *self.input_hold.lock().unwrap() = Some((began, finish));
            (admitted, outcome)
        }

        /// A start as the node answers it, once counted.
        /// A start, counted while it runs, so a test sees how many ran at once.
        fn counted_start(&self, start: &HolderStart) -> Result<(), HolderError> {
            let now = self.starting.0.fetch_add(1, Ordering::AcqRel) + 1;
            self.starting.1.fetch_max(now, Ordering::AcqRel);
            let answer = self.start_now(start);
            self.starting.0.fetch_sub(1, Ordering::AcqRel);
            answer
        }

        fn start_now(&self, start: &HolderStart) -> Result<(), HolderError> {
            let hold = self.hold.lock().unwrap().take();
            if let Some((began, go)) = hold {
                began.send(start.pane.clone()).unwrap();
                go.recv().unwrap();
            }
            let refused = self.ended.lock().unwrap().contains(&start.pane)
                || self.refuse.load(Ordering::Acquire)
                || self.refuse_pane.lock().unwrap().as_deref() == Some(start.pane.as_str());
            if refused {
                return Err(HolderError::Refused {
                    code: "holder_start_failed".to_owned(),
                    message: format!("pane {}: the holder program is missing", start.pane),
                });
            }
            self.started.lock().unwrap().push(start.clone());
            let facts = self.start_facts.lock().unwrap().take();
            if let Some(facts) = facts {
                self.say(&start.pane, &facts);
            }
            Ok(())
        }

        fn found(recovery: HolderRecovery) -> Arc<Self> {
            let holders = Arc::new(Self::default());
            holders.hand(recovery);
            holders
        }

        fn hand(&self, recovery: HolderRecovery) {
            *self.found.0.lock().unwrap() = Some(Ok(recovery));
            self.found.1.notify_all();
        }

        fn started(&self) -> Vec<HolderStart> {
            self.started.lock().unwrap().clone()
        }

        /// Says `facts` of `pane` as its holder would.
        fn say(&self, pane: &str, facts: &PaneFacts) {
            let sink = self
                .facts
                .lock()
                .unwrap()
                .clone()
                .expect("the engine watches facts");
            sink(pane, facts);
        }

        fn typed(&self, pane: &str) -> String {
            typed_into(&self.typed.0.lock().unwrap(), pane)
        }

        /// Waits for something to be typed into `pane`, and answers it.
        fn wait_typed(&self, pane: &str) -> String {
            let typed = self.typed.0.lock().unwrap();
            let (typed, _) = self
                .typed
                .1
                .wait_timeout_while(typed, Duration::from_secs(10), |typed| {
                    typed_into(typed, pane).is_empty()
                })
                .unwrap();
            typed_into(&typed, pane)
        }

        /// Waits for the rules worker to have given `count` rule sets.
        fn rules_given(&self, count: usize) -> Given {
            let given = self.rules.0.lock().unwrap();
            let (given, _) = self
                .rules
                .1
                .wait_timeout_while(given, Duration::from_secs(5), |given| given.len() < count)
                .unwrap();
            given.clone()
        }

        /// Every pane's terminal held by its shell `pid` alone, named `name`.
        fn idle_shell(&self, pid: u32, name: &str) {
            *self.processes.lock().unwrap() = Some(HolderProcesses {
                shell: pid,
                foreground: Some(hide_node_link::holders::HolderForeground {
                    group: pid,
                    members: vec![(pid, name.to_owned())],
                }),
            });
        }
    }

    impl PaneHolders for Holders {
        fn request(&self) -> Result<hide_node_link::holders::HolderRequest, HolderError> {
            Ok(hide_node_link::holders::HolderRequest {
                owner: 1,
                number: self.issued.fetch_add(1, Ordering::AcqRel) + 1,
            })
        }
        fn retire(
            &self,
            request: hide_node_link::holders::HolderRequest,
        ) -> Result<(), HolderError> {
            self.retired.lock().unwrap().push(request.number);
            if self.lose_retire.swap(false, Ordering::AcqRel) {
                Err(HolderError::Unreachable {
                    message: "retire answer lost".to_owned(),
                })
            } else {
                Ok(())
            }
        }
        fn reserve(
            &self,
            start: &HolderStart,
            previous: Option<u64>,
            _: hide_node_link::holders::HolderRequest,
        ) -> Result<hide_node_link::holders::HolderReservation, HolderError> {
            if previous.is_some() {
                self.ended
                    .lock()
                    .unwrap()
                    .retain(|pane| pane != &start.pane);
            }
            let mut reserved = self.reserved.lock().unwrap();
            let incarnation =
                if let Some(at) = reserved.iter().position(|held| held.pane == start.pane) {
                    at + 1
                } else {
                    reserved.push(start.clone());
                    reserved.len()
                } as u64;
            drop(reserved);
            let hold = self.reserve_hold.lock().unwrap().take();
            if let Some((began, go)) = hold {
                began.send(start.pane.clone()).unwrap();
                go.recv().unwrap();
            }
            if self.lose_reserve.swap(false, Ordering::AcqRel) {
                Err(HolderError::Unreachable {
                    message: "reserve response lost".to_owned(),
                })
            } else {
                Ok(hide_node_link::holders::HolderReservation { incarnation })
            }
        }

        fn start_reserved(
            &self,
            start: &HolderStart,
            _: hide_node_link::holders::HolderReservation,
            _: hide_node_link::holders::HolderRequest,
        ) -> Result<(), HolderError> {
            assert!(
                self.reserved
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|reserved| reserved == start),
                "a start must have its original reservation"
            );
            self.counted_start(start)
        }

        fn cancel(
            &self,
            intent: &hide_node_link::holders::HolderIntent,
            _: hide_node_link::holders::HolderRequest,
        ) -> Result<(), HolderError> {
            self.terminate(&intent.pane)?;
            if let Some(notice) = self.cancel_notice.lock().unwrap().take() {
                notice.send(intent.pane.clone()).unwrap();
            }
            if self.lose_cancel.swap(false, Ordering::AcqRel) {
                Err(HolderError::Unreachable {
                    message: "cancel response lost".to_owned(),
                })
            } else {
                Ok(())
            }
        }

        fn terminate(&self, pane: &str) -> Result<(), HolderError> {
            let hold = {
                let mut hold = self.terminate_hold.lock().unwrap();
                if hold.as_ref().is_some_and(|(target, _, _)| target == pane) {
                    hold.take()
                } else {
                    None
                }
            };
            if let Some((_, began, go)) = hold {
                began.send(()).unwrap();
                go.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            let mut ended = self.ended.lock().unwrap();
            if !ended.iter().any(|held| held == pane) {
                ended.push(pane.to_owned());
            }
            Ok(())
        }
        fn read(&self, _: &str, _: u16) -> Result<(String, bool), HolderError> {
            Ok(("rows".to_owned(), false))
        }
        fn read_styled(&self, _: &str, read: StyledRead) -> Result<(String, bool), HolderError> {
            Ok((format!("\u{1b}[1m{read:?}\u{1b}[0m"), false))
        }
        fn read_visible(&self, _: &str) -> Result<String, HolderError> {
            Ok("screen".to_owned())
        }
        fn scroll(&self, _: &str, _: Option<u64>) -> Result<HolderScroll, HolderError> {
            Err(HolderError::Unreachable {
                message: "not asked".to_owned(),
            })
        }
        fn processes(&self, pane: &str) -> Result<HolderProcesses, HolderError> {
            let mut processes =
                self.processes
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or_else(|| HolderError::Unreachable {
                        message: "not asked".to_owned(),
                    })?;
            if self.busy.lock().unwrap().contains(pane) {
                processes.foreground = Some(hide_node_link::holders::HolderForeground {
                    group: processes.shell + 8,
                    members: vec![(processes.shell + 8, "vim".to_owned())],
                });
            }
            Ok(processes)
        }
        fn proof(&self, _: &str) -> Result<hide_node_link::holders::HolderProof, HolderError> {
            // Every pane's program is this test's own process.
            let program = std::process::id();
            Ok(hide_node_link::holders::HolderProof {
                incarnation: 1,
                program,
                started: hide_platform::process::start_time(program).map_err(|error| {
                    HolderError::Unreachable {
                        message: error.to_string(),
                    }
                })?,
            })
        }
        fn input(&self, pane: &str, input: PaneInput) -> Result<(), HolderError> {
            let hold = self.input_hold.lock().unwrap().take();
            if let Some((began, finish)) = hold {
                began.send(()).unwrap();
                finish.recv_timeout(Duration::from_secs(10)).unwrap()?;
            }
            self.typed
                .0
                .lock()
                .unwrap()
                .push((pane.to_owned(), shown(input)));
            self.typed.1.notify_all();
            Ok(())
        }
        fn letter(
            &self,
            pane: &str,
            body: PaneInput,
            submit: PaneInput,
            _gap: Duration,
        ) -> Result<(), HolderError> {
            let mut letter = b"<letter>".to_vec();
            letter.extend(shown(body));
            letter.extend(shown(submit));
            letter.extend(b"</letter>");
            self.typed.0.lock().unwrap().push((pane.to_owned(), letter));
            self.typed.1.notify_all();
            Ok(())
        }
        fn watch_ends(&self, ended: hide_node_link::holders::PaneEnded) {
            *self.ends.lock().unwrap() = Some(ended);
        }
        fn watch_facts(&self, sink: hide_node_link::holders::PaneFactsSink) {
            *self.facts.lock().unwrap() = Some(sink);
        }
        fn set_rules(
            &self,
            pane: &str,
            rules: Option<hide_node_link::holders::ScreenRules>,
        ) -> Result<(), HolderError> {
            if let Some((began, go)) = self.rules_hold.lock().unwrap().take() {
                began.send(()).unwrap();
                go.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            self.rules
                .0
                .lock()
                .unwrap()
                .push((pane.to_owned(), rules.map(|rules| rules.agent)));
            self.rules.1.notify_all();
            Ok(())
        }
        fn recovered(&self, timeout: Duration) -> Result<HolderRecovery, HolderError> {
            let found = self.found.0.lock().unwrap();
            let (found, _) = self
                .found
                .1
                .wait_timeout_while(found, timeout, |found| found.is_none())
                .unwrap();
            found.clone().unwrap_or_else(|| {
                Err(HolderError::Refused {
                    code: "recovery_unfinished".to_owned(),
                    message: "not handed".to_owned(),
                })
            })
        }
    }

    const WAIT: Wait = Wait::Each(Duration::from_secs(1));

    fn refused_for_the_holder<T: std::fmt::Debug>(answer: Answer<T>) {
        match answer {
            Err(EngineError::Refused { code, message }) => {
                assert_eq!(code, "holder_start_failed");
                assert!(
                    message.contains("the holder program is missing"),
                    "the refusal carries the holder's reason: {message}"
                );
            }
            other => panic!("expected the holder's refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_read_answers_the_sources_this_engine_holds_and_refuses_any_other_by_name() {
        let engine = OwnEngine::new(Arc::new(Holders::default()) as Arc<dyn PaneHolders>);
        let (_, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        for source in ["recent", "recent_unwrapped"] {
            assert_eq!(
                engine.read_pane(WAIT, &pane, source, 20).unwrap().text,
                "rows"
            );
        }
        assert_eq!(
            engine.read_pane(WAIT, &pane, "visible", 20).unwrap().text,
            "screen"
        );
        match engine.read_pane(WAIT, &pane, "scrollback", 20) {
            Err(EngineError::Refused { code, message }) => {
                assert_eq!(code, "unknown_read_source");
                assert!(message.contains("\"scrollback\""), "{message}");
            }
            other => panic!("expected a refusal naming the source, got {other:?}"),
        }
    }

    mod creation;
    mod rules_recovery;

    /// What a placement saw: each call, in order, with whether the pane's
    /// holder had been asked to start by then and whether the topology could
    /// be read meanwhile.
    struct Placed {
        engine: Arc<OwnEngine<'static>>,
        holders: Arc<Holders>,
        refuse: bool,
        seen: Arc<Mutex<Vec<(String, bool, bool)>>>,
    }

    impl super::super::Placement for Placed {
        fn place(&self, pane: &str) -> Result<(), EngineError> {
            let started = self
                .holders
                .started
                .lock()
                .unwrap()
                .iter()
                .any(|start| start.pane == pane);
            // Another thread reads the topology while the placement runs: a
            // placement made under the topology lock would hold it up.
            let (sent, read) = std::sync::mpsc::channel();
            let engine = Arc::clone(&self.engine);
            std::thread::spawn(move || {
                let _ = sent.send(engine.focused_workspace(WAIT).is_ok());
            });
            let topology_free = read.recv_timeout(Duration::from_secs(2)).unwrap_or(false);
            self.seen
                .lock()
                .unwrap()
                .push((format!("place {pane}"), started, topology_free));
            if self.refuse {
                return Err(refused(
                    "spawn_refused",
                    "the record took no pane".to_owned(),
                ));
            }
            Ok(())
        }

        fn withdraw(&self, pane: &str) {
            self.seen
                .lock()
                .unwrap()
                .push((format!("withdraw {pane}"), false, true));
        }
    }

    /// PRD engine-bundle-3-core-owns D-11, B30: a pane a gated call makes is
    /// placed after its id is reserved and the topology lock let go, and
    /// before its holder starts; a holder that does not start withdraws it
    /// in the same call, and a placement refused starts nothing.
    #[test]
    fn a_new_pane_is_placed_off_the_topology_lock_before_its_holder_starts() {
        let holders = Arc::new(Holders::default());
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, first) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let handle = Arc::new(OwnEngine {
            shared: Arc::clone(&engine.shared),
            gates: Vec::new(),
        });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let placing = |refuse| {
            engine.gated(Gate::placing(Placed {
                engine: Arc::clone(&handle),
                holders: Arc::clone(&holders),
                refuse,
                seen: Arc::clone(&seen),
            }))
        };

        let split = placing(false)
            .split_beside(WAIT, &first, None, Env::new())
            .expect("the split is made");
        assert_eq!(
            seen.lock().unwrap().drain(..).collect::<Vec<_>>(),
            [(format!("place {split}"), false, true)],
            "placed before its holder started, with the topology free"
        );
        assert!(
            holders
                .started
                .lock()
                .unwrap()
                .iter()
                .any(|start| start.pane == split)
        );

        holders.refuse.store(true, Ordering::Release);
        refused_for_the_holder(placing(false).split_beside(WAIT, &first, None, Env::new()));
        let seen_now: Vec<_> = seen.lock().unwrap().drain(..).collect();
        let [(placed, false, true), (withdrawn, _, _)] = seen_now.as_slice() else {
            panic!("expected a placement and its withdrawal, got {seen_now:?}");
        };
        assert_eq!(
            placed.trim_start_matches("place "),
            withdrawn.trim_start_matches("withdraw "),
            "the pane that did not start is the one withdrawn"
        );
        holders.refuse.store(false, Ordering::Release);

        let before = holders.started.lock().unwrap().len();
        match placing(true).split_beside(WAIT, &first, None, Env::new()) {
            Err(EngineError::Refused { code, .. }) => assert_eq!(code, "spawn_refused"),
            other => panic!("expected the placement's refusal, got {other:?}"),
        }
        assert_eq!(
            holders.started.lock().unwrap().len(),
            before,
            "nothing starts in a pane the record refused"
        );
        assert_eq!(engine.pane_folders(WAIT).unwrap().len(), 2);
    }

    #[test]
    fn an_unanswered_reservation_keeps_placement_until_cancellation_and_retirement_confirm() {
        let holders = Arc::new(Holders::default());
        let engine = Arc::new(OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>));
        let (_, _, first) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let placed = engine.gated(Gate::placing(Placed {
            engine: Arc::clone(&engine),
            holders: Arc::clone(&holders),
            refuse: false,
            seen: Arc::clone(&seen),
        }));
        let before = holders.started().len();
        holders.lose_reserve.store(true, Ordering::Release);
        assert!(
            placed
                .split_beside(WAIT, &first, None, Env::new())
                .unwrap_err()
                .reply_unknown()
        );
        let pane = engine
            .shared
            .pending
            .lock()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        assert_eq!(engine.pane_folders(WAIT).unwrap().len(), 1);
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "unknown does not withdraw a placement"
        );
        holders.lose_cancel.store(true, Ordering::Release);
        assert!(engine.resolve_unpublished().unwrap_err().reply_unknown());
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(engine.shared.pending.lock().unwrap().contains_key(&pane));
        holders.lose_retire.store(true, Ordering::Release);
        assert!(engine.resolve_unpublished().unwrap_err().reply_unknown());
        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "confirmed cancellation withdraws once"
        );
        assert!(engine.shared.pending.lock().unwrap().contains_key(&pane));
        let retired = *holders.retired.lock().unwrap().last().unwrap();
        engine.resolve_unpublished().unwrap();
        assert!(engine.shared.pending.lock().unwrap().is_empty());
        assert_eq!(*holders.retired.lock().unwrap().last().unwrap(), retired);
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert_eq!(
            holders.started().len(),
            before,
            "an unknown reservation never starts a replacement"
        );
    }

    #[test]
    fn authority_lost_during_reservation_cancels_before_publication() {
        let holders = Arc::new(Holders::default());
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, first) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let allowed = AtomicBool::new(true);
        let gated = engine.gated(Gate::both(|| {
            if allowed.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(refused(
                    "authority_lost",
                    "the caller was removed".to_owned(),
                ))
            }
        }));
        let (began, entered) = std::sync::mpsc::channel();
        let (go, released) = std::sync::mpsc::channel();
        *holders.reserve_hold.lock().unwrap() = Some((began, released));
        std::thread::scope(|scope| {
            let work = scope.spawn(|| gated.split_beside(WAIT, &first, None, Env::new()));
            let pane = entered.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(engine.pane_folders(WAIT).unwrap().len(), 1);
            allowed.store(false, Ordering::Release);
            go.send(()).unwrap();
            assert_eq!(
                work.join().unwrap().unwrap_err().code(),
                Some("authority_lost")
            );
            assert!(holders.ended.lock().unwrap().contains(&pane));
            assert!(!holders.started().iter().any(|start| start.pane == pane));
        });
        assert_eq!(engine.pane_folders(WAIT).unwrap().len(), 1);
        assert!(engine.shared.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn completed_creations_return_capacity_on_the_same_connection() {
        let holders = Arc::new(Holders::default());
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        for n in 1..=384 {
            engine
                .start_placed(&format!("w1:p{n}"), None, "/w", Env::new())
                .unwrap();
        }
        assert_eq!(holders.started().len(), 384);
        assert_eq!(holders.retired.lock().unwrap().len(), 384);
        assert!(engine.shared.pending.lock().unwrap().is_empty());
    }

    /// Waits for `done`, which a holder start on its own thread settles.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn settles(what: &str, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_new_tab_or_split_is_answered_and_shown_before_its_holder_starts() {
        let holders = Arc::new(Holders::default());
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (workspace, _, first) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let shown = |pane: &str| {
            engine
                .pane_folders(WAIT)
                .unwrap()
                .iter()
                .any(|(id, _)| id == pane)
        };
        let held = || {
            let (began, starting) = std::sync::mpsc::channel();
            let (go, wait) = std::sync::mpsc::channel();
            *holders.hold.lock().unwrap() = Some((began, wait));
            (starting, go)
        };
        let ended = |pane: &str| holders.ended.lock().unwrap().iter().any(|id| id == pane);

        // A split is answered and in the session while its holder starts,
        // as a Herdr pane is once it forked.
        let (starting, go) = held();
        let split = engine
            .split_beside(WAIT, &first, None, Env::new())
            .expect("the split is answered before its holder");
        assert_eq!(starting.recv().unwrap(), split);
        assert!(shown(&split), "the split is drawn while its holder starts");
        go.send(()).unwrap();
        settles("the split's holder started", || {
            holders
                .started
                .lock()
                .unwrap()
                .iter()
                .any(|start| start.pane == split)
        });

        // Close fences the already published reservation before removing
        // the tab. The delayed start must observe that terminal decision.
        let (starting, go) = held();
        let (tab, pane) = engine
            .create_tab(WAIT, &workspace, "/w", "t", false, Env::new())
            .unwrap();
        assert_eq!(starting.recv().unwrap(), pane);
        let outcome = engine
            .topology()
            .pane(&pane)
            .unwrap()
            .starting
            .clone()
            .unwrap();
        engine.close_tab(WAIT, &tab).unwrap();
        assert!(!shown(&pane), "the closed tab's pane is gone at once");
        assert!(
            ended(&pane),
            "close confirms termination before removing the published pane"
        );
        go.send(()).unwrap();
        assert!(
            outcome.wait().is_err(),
            "the delayed start cannot reopen it"
        );

        // A holder that does not start leaves its pane where it was drawn,
        // marked with the holder's reason, which the session gives as the
        // pane's `restore_error`: the screen shows the pane could not start
        // and offers to close it. Closing it then ends nothing, since no
        // holder runs.
        holders.refuse.store(true, Ordering::Release);
        let split = engine.split_beside(WAIT, &first, None, Env::new()).unwrap();
        let (_, launched) = engine
            .launch_tab(
                WAIT,
                &workspace,
                None,
                "t",
                false,
                "/w",
                Env::new(),
                &["sh".to_owned()],
            )
            .unwrap();
        let failed = |pane: &str| {
            engine
                .topology()
                .pane(pane)
                .ok()
                .and_then(|pane| pane.failed.clone())
        };
        settles("the panes that did not start are marked", || {
            failed(&split).is_some() && failed(&launched).is_some()
        });
        assert!(
            failed(&split)
                .unwrap()
                .contains("the holder program is missing"),
            "{:?}",
            failed(&split)
        );
        let session = engine.session(WAIT).unwrap();
        let projected = |pane: &str| {
            session
                .panes
                .iter()
                .find(|projected| projected.pane_id == pane)
                .and_then(|projected| projected.restore_error.clone())
        };
        assert_eq!(projected(&split), failed(&split));
        assert_eq!(projected(&launched), failed(&launched));
        let ended_before = holders.ended.lock().unwrap().len();
        engine.close_pane(WAIT, &split).unwrap();
        assert!(!shown(&split));
        assert_eq!(holders.ended.lock().unwrap().len(), ended_before + 1);
        assert!(shown(&first));
    }

    #[test]
    fn new_panes_share_the_start_limit_and_report_overflow_and_start_failures() {
        struct HeldStarts(Vec<std::sync::mpsc::Sender<()>>);
        impl Drop for HeldStarts {
            fn drop(&mut self) {
                for go in self.0.drain(..) {
                    let _ = go.send(());
                }
            }
        }
        let holders = Arc::new(Holders::default());
        holders.idle_shell(42, "sh");
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (workspace, _, first) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let mut held = HeldStarts(Vec::new());
        let mut panes = Vec::new();
        for _ in 0..8 {
            let (began, starting) = std::sync::mpsc::channel();
            let (go, wait) = std::sync::mpsc::channel();
            *holders.hold.lock().unwrap() = Some((began, wait));
            held.0.push(go);
            let (_, pane) = engine
                .create_tab(WAIT, &workspace, "/w", "held", false, Env::new())
                .unwrap();
            assert_eq!(
                starting.recv_timeout(Duration::from_secs(10)).unwrap(),
                pane
            );
            panes.push(pane);
        }
        std::thread::scope(|scope| {
            let queued: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        engine.create_tab(WAIT, &workspace, "/w", "queued", false, Env::new())
                    })
                })
                .collect();
            settles(
                "eight creation callers admitted to the waiting bound",
                || engine.shared.starts.waiting() == 8,
            );
            let before = engine.pane_folders(WAIT).unwrap().len();
            let error = engine
                .split_beside(WAIT, &first, None, Env::new())
                .unwrap_err();
            assert_eq!(error.code(), Some("holder_start_queue_full"));
            assert_eq!(
                engine.pane_folders(WAIT).unwrap().len(),
                before,
                "refused admission publishes no pane"
            );
            let error = engine
                .create_workspace(WAIT, "/other", "other", false, Env::new())
                .unwrap_err();
            assert_eq!(error.code(), Some("holder_start_queue_full"));
            drop(held);
            for call in queued {
                panes.push(call.join().unwrap().unwrap().1);
            }
        });
        for pane in panes {
            assert_eq!(
                engine.pane_processes(WAIT, &pane).unwrap().shell_pid,
                Some(42)
            );
        }
        assert_eq!(holders.starting.1.load(Ordering::Acquire), 8);

        holders.refuse.store(true, Ordering::Release);
        let failed = engine.split_beside(WAIT, &first, None, Env::new()).unwrap();
        let expected = EngineError::Refused {
            code: "holder_start_failed".to_owned(),
            message: format!("pane {failed}: the holder program is missing"),
        };
        assert_eq!(engine.pane_processes(WAIT, &failed).unwrap_err(), expected);
        assert_eq!(
            engine.read_pane(WAIT, &failed, "recent", 10).unwrap_err(),
            expected
        );
        assert_eq!(engine.read_screen(WAIT, &failed).unwrap_err(), expected);
        assert_eq!(engine.pane_identity(WAIT, &failed).unwrap_err(), expected);
        assert_eq!(
            engine.foreground_program(WAIT, &failed).unwrap_err(),
            expected
        );
    }

    #[test]
    fn a_pane_whose_holder_cannot_start_is_refused_with_its_reason_and_leaves_nothing() {
        let holders = Arc::new(Holders::default());
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);

        holders.refuse.store(true, Ordering::Release);
        refused_for_the_holder(engine.create_workspace(WAIT, "/w", "w", true, Env::new()));
        assert!(engine.pane_folders(WAIT).unwrap().is_empty());
        assert_eq!(engine.focused_workspace(WAIT).unwrap(), None);

        holders.refuse.store(false, Ordering::Release);
        let (workspace, tab, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        holders.refuse.store(true, Ordering::Release);
        // A tab that replaces another hangs on its start: refused, and the
        // replaced tab stays.
        refused_for_the_holder(engine.launch_tab(
            WAIT,
            &workspace,
            Some(&tab),
            "t",
            false,
            "/w",
            Env::new(),
            &["sh".to_owned()],
        ));
        let panes: Vec<String> = engine
            .pane_folders(WAIT)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(panes, vec![pane]);
    }

    /// A reboot starts the saved panes again, at most [`starts::STARTS_AT_ONCE`]
    /// at a time: one whose holder does not start keeps its place marked
    /// with why, which the session gives as its `restore_error` (the screen's
    /// card that its terminal could not start, with a close), and every
    /// other pane starts all the same.
    #[test]
    fn a_reboot_starts_every_saved_pane_but_marks_the_one_whose_holder_failed() {
        let root = tempfile::tempdir().unwrap();
        let place = place(root.path());
        let kept = root.path().join("kept");
        for dir in [&home(&place), &kept] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let first =
            OwnEngine::open(Holders::found(HolderRecovery::default()), place.clone()).unwrap();
        let (workspace, _, pane) = first
            .create_workspace(WAIT, &wire(&kept), "w", true, Env::new())
            .unwrap();
        let mut panes = vec![pane.clone()];
        for _ in 0..127 {
            let (_, tab) = first
                .create_tab(WAIT, &workspace, &wire(&kept), "t", false, Env::new())
                .unwrap();
            first.read_pane(WAIT, &tab, "recent", 1).unwrap();
            panes.push(tab);
        }
        drop(first);

        // After the reboot nothing of them runs; one pane's holder refuses.
        let refused = panes[5].clone();
        let holders = Arc::new(Holders::default());
        *holders.refuse_pane.lock().unwrap() = Some(refused.clone());
        let next =
            OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place.clone()).unwrap();
        holders.hand(HolderRecovery::default());
        next.restored().unwrap();

        let started: std::collections::BTreeSet<String> = holders
            .started()
            .into_iter()
            .map(|start| start.pane)
            .collect();
        let others: std::collections::BTreeSet<String> =
            panes.iter().filter(|id| **id != refused).cloned().collect();
        assert_eq!(started, others, "every other pane started");
        assert!(holders.starting.1.load(Ordering::Acquire) <= starts::STARTS_AT_ONCE);
        let session = next.session(WAIT).unwrap();
        let error = |id: &str| {
            session
                .panes
                .iter()
                .find(|projected| projected.pane_id == id)
                .map(|projected| projected.restore_error.clone())
        };
        let why = error(&refused).expect("the refused pane keeps its place");
        assert!(
            why.as_deref()
                .is_some_and(|why| why.contains("the holder program is missing")),
            "{why:?}"
        );
        for id in &others {
            assert_eq!(error(id), Some(None), "{id} runs");
        }
    }

    fn moves(engine: &OwnEngine<'_>) -> (Vec<String>, Option<(String, bool)>) {
        let session = engine.session(WAIT).unwrap();
        (
            session.tab_moves.unwrap().recent.into_iter().collect(),
            session
                .tab_focus
                .map(|focus| (focus.tab_id, focus.creation)),
        )
    }

    /// letter-19988: the core settles a tab move it asked for when a
    /// session reports the move (`runtime::tab_focus`), and a session of the
    /// core's own engine reports every move it made, a creation's included,
    /// so nothing waits out the deadline for a confirmation Herdr would send.
    #[test]
    fn every_move_of_the_shown_tab_is_reported_with_the_session_and_none_for_a_tab_already_shown() {
        let engine = OwnEngine::new(Arc::new(Holders::default()));
        let (workspace, first, _) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let (second, _) = engine
            .create_tab(WAIT, &workspace, "/w", "two", true, Env::new())
            .unwrap();
        assert_eq!(
            moves(&engine),
            (
                vec![first.clone(), second.clone()],
                Some((second.clone(), true))
            ),
            "the creation's move is reported as one"
        );

        engine.focus_tab(WAIT, &second).unwrap();
        engine.focus_tab(WAIT, &first).unwrap();
        let (_, quiet) = engine
            .create_tab(WAIT, &workspace, "/w", "quiet", false, Env::new())
            .unwrap();
        assert_eq!(
            moves(&engine),
            (
                vec![first.clone(), second.clone(), first.clone()],
                Some((first.clone(), false))
            ),
            "a focus of the shown tab and a tab made without focus move nothing"
        );
        let _ = quiet;
    }

    /// A daemon's place under `root`, its machine read by a real node.
    fn place(root: &std::path::Path) -> Place {
        let home = root.join("home");
        Place {
            engine_dir: hide_kit::layout::engine_dir(&root.join("state")),
            node: Arc::new(hide_node::Local::new(Some(home.clone()))),
            windows: cfg!(windows),
            home: wire(&home),
            hooks_home: Some(home.clone()),
            definitions: Arc::new(hide_agent_adapter::definition::Catalog::read(Vec::new())),
            launches: Arc::default(),
        }
    }

    /// The home a test's place names, on this machine.
    fn home(place: &Place) -> std::path::PathBuf {
        hide_platform::path::from_wire(&place.home).unwrap()
    }

    fn wire(path: &std::path::Path) -> String {
        hide_platform::path::to_wire(path).unwrap()
    }

    fn layout_of(session: &SessionSnapshotPayload, tab: &str) -> Vec<String> {
        session
            .layouts
            .iter()
            .find(|layout| layout.tab_id == tab)
            .map(|layout| {
                layout
                    .panes
                    .iter()
                    .map(|pane| pane.pane_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// D-7 and letter-19977: a daemon opened again lays its saved session
    /// out under the same ids around the holders that ran on, starting none
    /// of them again; a pane whose holder ended gets a new shell under its
    /// id, in the home folder with a line saying why when its folder is
    /// gone; the tab shown before is shown again; a pane whose program
    /// exited while no daemon ran closes, with the tab it was the last pane
    /// of, and no shell takes it again; a
    /// running holder the save did not place is gathered into a new tab of
    /// its workspace; and a new pane takes an id past every one the records
    /// hold.
    #[test]
    fn a_daemon_opened_again_lays_its_session_out_around_the_panes_that_ran_on() {
        let root = tempfile::tempdir().unwrap();
        let place = place(root.path());
        let (kept, gone) = (root.path().join("kept"), root.path().join("gone"));
        for dir in [&home(&place), &kept, &gone] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let first =
            OwnEngine::open(Holders::found(HolderRecovery::default()), place.clone()).unwrap();
        let (workspace, tab, running) = first
            .create_workspace(WAIT, &wire(&kept), "w", true, Env::new())
            .unwrap();
        let ended = first
            .split_pane_at(
                WAIT,
                &running,
                ClosedSplitDirection::Down,
                &wire(&gone),
                0.3,
                Env::new(),
            )
            .unwrap();
        let (exited_tab, exited) = first
            .create_tab(WAIT, &workspace, &wire(&kept), "exited", false, Env::new())
            .unwrap();
        let (shown_tab, shown) = first
            .create_tab(WAIT, &workspace, &wire(&kept), "shown", true, Env::new())
            .unwrap();
        let before = first.session_payload(WAIT).unwrap();
        drop(first);
        std::fs::remove_dir(&gone).unwrap();

        // The node of the next daemon found the first pane's holder running,
        // the second's ended under it, the third's program exited, and one
        // more the save never placed.
        let unplaced = format!("{workspace}:p7");
        let holders = Arc::new(Holders::default());
        let next =
            OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place.clone()).unwrap();
        assert_eq!(
            layout_of(&next.session_payload(WAIT).unwrap(), &tab),
            layout_of(&before, &tab),
            "the saved layout is there before the node answers"
        );
        holders.hand(HolderRecovery {
            owner: 1,
            closed: vec![exited.clone()],
            ended: vec![ended.clone()],
            present: vec![running.clone(), shown, unplaced.clone()],
            recoverable: Vec::new(),
        });
        next.restored().unwrap();
        let active = |engine: &OwnEngine<'_>| {
            engine
                .session(WAIT)
                .unwrap()
                .workspaces
                .iter()
                .find(|candidate| candidate.workspace_id == workspace)
                .map(|candidate| candidate.active_tab_id.clone())
        };
        assert_eq!(
            active(&next),
            Some(shown_tab.clone()),
            "the tab shown before is the one shown again"
        );
        let split = next
            .split_pane(WAIT, &running, crate::live::PaneSplitDirection::Right, None)
            .unwrap();

        // The split's holder starts after its answer.
        settles("the split's holder started", || {
            holders.started().len() == 2
        });
        let started = holders.started();
        assert_eq!(started.len(), 2, "{started:?}");
        assert_eq!(started[0].pane, ended);
        assert_eq!(started[0].cwd, place.home.clone());
        assert!(
            started[0]
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains(&wire(&gone))),
            "{:?}",
            started[0].notice
        );
        assert_eq!(started[1].pane, split);
        assert_eq!(split, format!("{workspace}:p8"), "past every recorded id");
        assert!(holders.ended.lock().unwrap().is_empty(), "nothing ended");
        let session = next.session_payload(WAIT).unwrap();
        assert_eq!(
            layout_of(&session, &tab),
            [running.clone(), split, ended.clone()],
            "the same tab under the same ids"
        );
        let gathered = session
            .tabs
            .iter()
            .find(|candidate| candidate.label == "restored")
            .expect("the unplaced holder's tab");
        assert_eq!(gathered.workspace_id, workspace);
        assert_eq!(layout_of(&session, &gathered.tab_id), [unplaced]);
        assert!(
            started.iter().all(|start| start.pane != exited),
            "no shell takes the pane whose program exited"
        );
        assert!(
            session
                .tabs
                .iter()
                .all(|candidate| candidate.tab_id != exited_tab),
            "its tab closed with its last pane"
        );
        assert!(session.panes.iter().all(|pane| pane.pane_id != exited));
    }

    /// D-6 and B15: a save that cannot be read is kept in the backups, and
    /// the holders that ran on are gathered into a new tab of the workspace
    /// their ids name, none started again and none ended.
    #[test]
    fn an_unreadable_save_is_kept_and_the_running_panes_gathered_without_ending_one() {
        let root = tempfile::tempdir().unwrap();
        let place = place(root.path());
        std::fs::create_dir_all(home(&place)).unwrap();
        let path = hide_kit::layout::engine_session_in(&place.engine_dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not a session").unwrap();

        let holders = Holders::found(HolderRecovery {
            owner: 1,
            closed: Vec::new(),
            ended: Vec::new(),
            present: vec!["w2:p1".to_owned(), "w2:p3".to_owned()],
            recoverable: Vec::new(),
        });
        let engine =
            OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place.clone()).unwrap();
        let (workspace, _, pane) = engine
            .create_workspace(WAIT, &place.home.clone(), "new", false, Env::new())
            .unwrap();

        let backups = hide_kit::layout::engine_backups_in(&place.engine_dir);
        let kept: Vec<String> = std::fs::read_dir(&backups)
            .unwrap()
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        assert_eq!(kept, ["{ not a session"]);
        let session = engine.session_payload(WAIT).unwrap();
        let gathered = session
            .tabs
            .iter()
            .find(|tab| tab.workspace_id == "w2")
            .expect("the running panes' tab");
        assert_eq!(gathered.label, "restored");
        assert_eq!(layout_of(&session, &gathered.tab_id), ["w2:p1", "w2:p3"]);
        // The engine numbers its own workspaces apart from the gathered
        // Herdr ones (PRD engine-bundle-4-herdr-removal D-26).
        assert_eq!(
            (workspace.as_str(), pane.as_str()),
            ("h1", "h1:p1"),
            "a new workspace numbered apart from the gathered ones"
        );
        assert_eq!(holders.started().len(), 1, "only the new pane started");
        assert!(holders.ended.lock().unwrap().is_empty(), "nothing ended");
    }

    /// A pane whose program exits on its own closes as a close would, its
    /// holder already gone, and its tab with it when it was the last.
    #[test]
    fn a_pane_whose_program_exits_closes_and_takes_its_last_tab_with_it() {
        let root = tempfile::tempdir().unwrap();
        let place = place(root.path());
        std::fs::create_dir_all(home(&place)).unwrap();
        let holders = Holders::found(HolderRecovery::default());
        let engine =
            OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place.clone()).unwrap();
        let home = place.home.clone();
        let (workspace, tab, first) = engine
            .create_workspace(WAIT, &home, "w", true, Env::new())
            .unwrap();
        let second = engine.split_beside(WAIT, &first, None, Env::new()).unwrap();
        let (lone_tab, lone) = engine
            .create_tab(WAIT, &workspace, &home, "lone", false, Env::new())
            .unwrap();
        // A program exits only once its holder runs, and these start after
        // their answers.
        for pane in [&first, &second, &lone] {
            engine.read_pane(WAIT, pane, "recent", 1).unwrap();
        }
        let exited = holders
            .ends
            .lock()
            .unwrap()
            .clone()
            .expect("the engine watches ends");

        exited(&second);
        exited(&lone);
        exited("w9:p1");

        let session = engine.session_payload(WAIT).unwrap();
        assert_eq!(layout_of(&session, &tab), [first]);
        assert!(session.tabs.iter().all(|found| found.tab_id != lone_tab));
        assert!(
            holders.ended.lock().unwrap().is_empty(),
            "no holder was asked to end"
        );
    }

    fn run_git(dir: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn same_folder(left: &str, right: &std::path::Path) -> bool {
        std::fs::canonicalize(left).ok() == std::fs::canonicalize(right).ok()
    }

    /// D-28 and B8: a worktree the engine makes goes under Hide's own
    /// folder and opens in a workspace bound to it; a checkout under Herdr's
    /// folder opens where it is, once; removing a worktree removes its
    /// checkout and its workspace.
    #[test]
    fn a_new_worktree_goes_under_hides_folder_and_a_herdr_one_opens_where_it_is() {
        let root = tempfile::tempdir().unwrap();
        let place = place(root.path());
        let repo = home(&place).join("repo");
        std::fs::create_dir_all(home(&place)).unwrap();
        run_git(&home(&place), &["init", "-q", "repo"]);
        run_git(
            &repo,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        let herdr_place = home(&place).join(".herdr/worktrees/repo/old");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "old",
                &herdr_place.to_string_lossy(),
            ],
        );
        let engine =
            OwnEngine::open(Holders::found(HolderRecovery::default()), place.clone()).unwrap();

        let made = engine
            .create_worktree(WAIT, &wire(&repo), "feat/Layout", None, None, None, true)
            .expect("a worktree is made");
        let expected = hide_platform::path::from_wire(
            &hide_kit::layout::engine_worktree(&place.home, "repo", "feat-layout").unwrap(),
        )
        .unwrap();
        assert!(
            same_folder(&made.path, &expected),
            "{} is not {}",
            made.path,
            expected.display()
        );
        let session = engine.session(WAIT).unwrap();
        let bound = session
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == made.workspace_id)
            .and_then(|workspace| workspace.worktree.clone())
            .expect("the workspace is bound to its worktree");
        assert!(bound.is_linked_worktree);
        assert_eq!(bound.repo_name, "repo");
        assert!(same_folder(&bound.repo_root, &repo));

        let opened = engine
            .open_worktree(WAIT, &wire(&herdr_place), &wire(&repo), false)
            .expect("Herdr's worktree opens");
        assert!(!opened.already_open);
        let session = engine.session(WAIT).unwrap();
        let in_place = session
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == opened.workspace_id)
            .and_then(|workspace| workspace.worktree.clone())
            .expect("the workspace is bound to Herdr's checkout");
        assert!(
            same_folder(&in_place.checkout_path, &herdr_place),
            "opened where it is"
        );
        let again = engine
            .open_worktree(WAIT, &wire(&herdr_place), &wire(&repo), true)
            .unwrap();
        assert!(again.already_open);
        assert_eq!(again.workspace_id, opened.workspace_id);
        assert_eq!(
            engine.focused_workspace(WAIT).unwrap().as_deref(),
            Some(opened.workspace_id.as_str())
        );

        engine
            .remove_worktree(WAIT, &made.workspace_id)
            .expect("the worktree is removed");
        assert!(!expected.exists(), "its checkout is gone");
        assert!(
            engine
                .session(WAIT)
                .unwrap()
                .workspaces
                .iter()
                .all(|workspace| workspace.workspace_id != made.workspace_id)
        );
        assert!(herdr_place.exists(), "Herdr's checkout stays where it is");
    }

    fn launch(pane: &str, name: &str, kind: &str, args: &[&str]) -> AgentLaunch {
        AgentLaunch {
            pane_id: pane.to_owned(),
            name: name.to_owned(),
            kind: kind.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            codex_daemon: crate::codex_launch::CodexDaemon::Unsupported,
        }
    }

    /// Moves the start of the agent in `pane` back by the settle delay, as
    /// if it had been typed that long ago.
    fn settle_start(engine: &OwnEngine, pane: &str) {
        let mut topology = engine.topology();
        let agent = topology.pane_mut(pane).unwrap().agent.as_mut().unwrap();
        let started = agent.live.started.as_mut().unwrap();
        *started -= agents::START_SETTLE;
    }

    fn running(program: &str, screen: hide_node_link::holders::ScreenState) -> PaneFacts {
        PaneFacts {
            foreground: Some(hide_node_link::holders::ForegroundProgram {
                pid: 77,
                name: program.to_owned(),
                argv: vec![program.to_owned()],
            }),
            screen: hide_node_link::holders::ScreenRead::Read {
                state: screen,
                rule: None,
            },
            ..PaneFacts::default()
        }
    }

    /// The rules given to each pane, by agent id, in order.
    type Given = Vec<(String, Option<String>)>;

    /// Every pane's typed bytes, in order.
    type Typed = Vec<(String, Vec<u8>)>;

    /// Input as the double records it: bytes as they are, a paste and named
    /// keys by name, since the holder encodes those.
    fn shown(input: PaneInput) -> Vec<u8> {
        match input {
            PaneInput::Bytes(bytes) => bytes,
            PaneInput::Paste(text) => [b"<paste>".as_slice(), &text, b"</paste>"].concat(),
            PaneInput::Keys(keys) => keys
                .iter()
                .flat_map(|key| format!("<{key:?}>").into_bytes())
                .collect(),
        }
    }

    /// What was typed into `pane`, in order.
    fn typed_into(typed: &[(String, Vec<u8>)], pane: &str) -> String {
        typed
            .iter()
            .filter(|(into, _)| into == pane)
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .collect()
    }

    /// A device's engine types the line its pane's machine reads, never the
    /// core's: a Mac core starts an agent in a Windows device's pane with
    /// PowerShell's line when the holder could not name the shell, and a
    /// Windows core in a Linux device's with a POSIX one.
    #[test]
    fn a_start_takes_the_panes_machine_shell_when_its_name_is_unknown() {
        for (windows, line) in [(true, "& codex\r"), (false, "codex\r")] {
            let root = tempfile::tempdir().unwrap();
            let mut place = place(root.path());
            place.windows = windows;
            let folder = home(&place);
            std::fs::create_dir_all(&folder).unwrap();
            let holders = Holders::found(HolderRecovery::default());
            let engine =
                OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place).unwrap();
            let (_, _, pane) = engine
                .create_workspace(WAIT, &wire(&folder), "w", true, Env::new())
                .unwrap();
            // The holder could not read the shell's program name.
            holders.idle_shell(41, "");
            let start = engine
                .prepare_start(launch(&pane, "worker", "codex", &[]))
                .unwrap();
            engine.start_agent(WAIT, &start).unwrap();
            assert_eq!(holders.typed(&pane), line, "windows: {windows}");
        }
    }

    /// D-19, B17, B18: a start types its definition's line into the idle
    /// shell and hands the holder the agent's screen rules; the holder's
    /// facts then say what the agent does, through working to done.
    #[test]
    fn a_started_agent_is_typed_into_its_shell_and_follows_what_its_screen_shows() {
        use hide_node_link::holders::ScreenState;
        let holders = Arc::new(Holders::default());
        holders.idle_shell(41, "zsh");
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let start = engine
            .prepare_start(launch(&pane, "worker", "claude", &["--model", "it's"]))
            .unwrap();
        engine.start_agent(WAIT, &start).unwrap();
        assert_eq!(holders.typed(&pane), "claude --model 'it'\\''s'\r");
        assert_eq!(
            holders.rules_given(1),
            [(pane.clone(), Some("claude-code".to_owned()))]
        );
        let starting = engine.delivery_agent(WAIT, "worker").unwrap();
        assert_eq!(starting.readiness, Readiness::Launching);
        assert_eq!(starting.kind.as_deref(), Some("claude"));
        assert_eq!(starting.status, "unknown");

        settle_start(&engine, &pane);
        holders.say(&pane, &running("claude", ScreenState::Working));
        let working = engine.agent_state(WAIT, "worker").unwrap();
        assert_eq!(working.status, "working");
        assert_eq!(
            engine.delivery_agent(WAIT, &pane).unwrap().readiness,
            Readiness::Launching,
            "the pane id names its agent too, still starting until it rests"
        );
        holders.say(&pane, &running("claude", ScreenState::Idle));
        let done = engine.agent_state(WAIT, "worker").unwrap();
        assert_eq!(done.status, "done", "a rest after work is done");
        assert!(done.state_change_seq > working.state_change_seq);
        assert_eq!(
            engine.delivery_agent(WAIT, &pane).unwrap().readiness,
            Readiness::Ready
        );
        let listed = engine.agents(WAIT).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name.as_deref(), Some("worker"));
        assert_eq!(listed[0].agent_status.as_deref(), Some("done"));
        assert_eq!(engine.pane_agent_status(WAIT, &pane).unwrap(), "done");
    }

    fn undelivered_input() -> HolderError {
        HolderError::Refused {
            code: "pane_input_expired".to_owned(),
            message: "the queued input expired before writing".to_owned(),
        }
    }

    #[test]
    fn an_agents_early_facts_and_rules_enter_with_its_pane() {
        let holders = Arc::new(Holders::default());
        *holders.start_facts.lock().unwrap() = Some(running(
            "claude",
            hide_node_link::holders::ScreenState::Working,
        ));
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let agents = engine.agents(WAIT).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].pane_id, pane);
        assert_eq!(agents[0].agent_status.as_deref(), Some("working"));
        assert_eq!(
            holders.rules_given(1),
            [(pane, Some("claude-code".to_owned()))]
        );
    }

    /// A refused launch leaves neither a listed agent nor a held name,
    /// and does not erase a hook that preceded the provisional launch.
    #[test]
    fn an_undelivered_agent_start_releases_its_name_and_restores_the_pending_session() {
        let holders = Arc::new(Holders::default());
        holders.idle_shell(41, "zsh");
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let (_, _, other) = engine
            .create_workspace(WAIT, "/other", "other", false, Env::new())
            .unwrap();
        engine
            .hook_session(&pane, "claude", "before-start")
            .unwrap();
        let start = engine
            .prepare_start(launch(&pane, "worker", "claude", &[]))
            .unwrap();
        let (admitted, finish) = holders.hold_input();
        std::thread::scope(|scope| {
            let starting = scope.spawn(|| engine.start_agent(WAIT, &start));
            admitted.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(holders.rules_given(1)[0].1.as_deref(), Some("claude-code"));
            finish.send(Err(undelivered_input())).unwrap();
            assert!(
                matches!(starting.join().unwrap(), Err(EngineError::Refused { code, .. }) if code == "pane_input_expired")
            );
        });
        assert!(engine.agents(WAIT).unwrap().is_empty());
        assert!(holders.typed(&pane).is_empty());
        assert_eq!(holders.rules_given(2)[1], (pane.clone(), None));
        let reused = engine
            .prepare_start(launch(&other, "worker", "claude", &[]))
            .unwrap();
        engine.start_agent(WAIT, &reused).unwrap();
        holders.say(
            &pane,
            &running("claude", hide_node_link::holders::ScreenState::Working),
        );
        let agents = engine.agents(WAIT).unwrap();
        let restored = agents.iter().find(|agent| agent.pane_id == pane).unwrap();
        assert_eq!(
            restored.agent_session.as_ref().unwrap().value,
            "before-start"
        );
    }

    #[test]
    fn an_undelivered_agent_start_restores_the_preceding_agent() {
        let holders = Arc::new(Holders::default());
        holders.idle_shell(41, "zsh");
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let old = engine
            .prepare_start(launch(&pane, "old", "claude", &[]))
            .unwrap();
        engine.start_agent(WAIT, &old).unwrap();
        engine.hook_session(&pane, "claude", "old-session").unwrap();
        holders.say(
            &pane,
            &running("claude", hide_node_link::holders::ScreenState::Working),
        );
        // The processes read says the shell is idle before the next facts
        // arrive. A refused start must preserve the last observed identity.
        let new = engine
            .prepare_start(launch(&pane, "new", "codex", &[]))
            .unwrap();
        let (_admitted, finish) = holders.hold_input();
        finish.send(Err(undelivered_input())).unwrap();
        assert!(engine.start_agent(WAIT, &new).is_err());
        let agents = engine.agents(WAIT).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name.as_deref(), Some("old"));
        assert_eq!(
            agents[0].agent_session.as_ref().unwrap().value,
            "old-session"
        );
        assert_eq!(agents[0].agent_status.as_deref(), Some("working"));
        assert!(!holders.typed(&pane).contains("codex"));
    }

    /// A rollback cannot restore a name another pane was allowed to take.
    /// The backup reserves it only while restoration is still possible.
    #[test]
    fn a_start_reserves_its_previous_name_until_rollback_is_no_longer_possible() {
        for outcome in ["refused", "written", "unanswered", "hook"] {
            let holders = Arc::new(Holders::default());
            holders.idle_shell(41, "zsh");
            let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
            let (_, _, pane) = engine
                .create_workspace(WAIT, "/w", "w", true, Env::new())
                .unwrap();
            let (_, _, other) = engine
                .create_workspace(WAIT, "/other", "other", false, Env::new())
                .unwrap();
            let old = engine
                .prepare_start(launch(&pane, "old", "claude", &[]))
                .unwrap();
            engine.start_agent(WAIT, &old).unwrap();
            holders.say(
                &pane,
                &running("claude", hide_node_link::holders::ScreenState::Working),
            );
            let new = engine
                .prepare_start(launch(&pane, "new", "codex", &[]))
                .unwrap();
            let reuse = engine
                .prepare_start(launch(&other, "old", "claude", &[]))
                .unwrap();
            let (admitted, finish) = holders.hold_input();
            std::thread::scope(|scope| {
                let starting = scope.spawn(|| engine.start_agent(WAIT, &new));
                admitted.recv_timeout(Duration::from_secs(5)).unwrap();
                assert!(
                    matches!(engine.start_agent(WAIT, &reuse), Err(EngineError::Refused { code, .. }) if code == "agent_name_taken")
                );
                if outcome == "hook" {
                    engine.hook_session(&pane, "codex", "confirmed").unwrap();
                    engine.start_agent(WAIT, &reuse).unwrap();
                }
                finish
                    .send(match outcome {
                        "written" => Ok(()),
                        "unanswered" => Err(HolderError::Unreachable {
                            message: "write outcome unknown".to_owned(),
                        }),
                        _ => Err(undelivered_input()),
                    })
                    .unwrap();
                let result = starting.join().unwrap();
                assert_eq!(result.is_ok(), outcome == "written");
            });
            if outcome == "refused" {
                assert!(
                    matches!(engine.start_agent(WAIT, &reuse), Err(EngineError::Refused { code, .. }) if code == "agent_name_taken")
                );
            } else if outcome != "hook" {
                engine.start_agent(WAIT, &reuse).unwrap();
            }
            let agents = engine.agents(WAIT).unwrap();
            let old = agents
                .iter()
                .filter(|agent| agent.name.as_deref() == Some("old"))
                .collect::<Vec<_>>();
            assert_eq!(old.len(), 1, "{outcome}");
            assert_eq!(
                old[0].pane_id,
                if outcome == "refused" { &pane } else { &other }.as_str()
            );
        }
    }

    /// A write without an answer may already have reached the shell. It is
    /// not undone or resent, and another pane cannot claim its name.
    #[test]
    fn an_unanswered_agent_start_keeps_its_identity() {
        let holders = Arc::new(Holders::default());
        holders.idle_shell(41, "zsh");
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let (_, _, other) = engine
            .create_workspace(WAIT, "/other", "other", false, Env::new())
            .unwrap();
        let start = engine
            .prepare_start(launch(&pane, "worker", "claude", &[]))
            .unwrap();
        let (_admitted, finish) = holders.hold_input();
        finish
            .send(Err(HolderError::Unreachable {
                message: "the writer stopped answering".to_owned(),
            }))
            .unwrap();
        assert!(matches!(
            engine.start_agent(WAIT, &start),
            Err(EngineError::Unanswered(_))
        ));
        assert_eq!(
            engine.delivery_agent(WAIT, "worker").unwrap().readiness,
            Readiness::Launching
        );
        let duplicate = engine
            .prepare_start(launch(&other, "worker", "claude", &[]))
            .unwrap();
        assert!(
            matches!(engine.start_agent(WAIT, &duplicate), Err(EngineError::Refused { code, .. }) if code == "agent_name_taken")
        );
    }

    /// A session report, even at the value the launch already carried, or
    /// a real foreground observation is newer authority than input refusal.
    #[test]
    fn an_undelivered_start_cannot_erase_a_hook_or_foreground_observation() {
        for observation in ["hook", "claude", "wrapper", "gemini"] {
            let holders = Arc::new(Holders::default());
            holders.idle_shell(41, "zsh");
            let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
            let (_, _, pane) = engine
                .create_workspace(WAIT, "/w", "w", true, Env::new())
                .unwrap();
            let start = engine
                .prepare_start(launch(
                    &pane,
                    "worker",
                    "claude",
                    &["--resume", "same-session"],
                ))
                .unwrap();
            let (admitted, finish) = holders.hold_input();
            std::thread::scope(|scope| {
                let starting = scope.spawn(|| engine.start_agent(WAIT, &start));
                admitted.recv_timeout(Duration::from_secs(5)).unwrap();
                if observation == "hook" {
                    engine
                        .hook_session(&pane, "claude", "same-session")
                        .unwrap();
                    let duplicate = engine
                        .prepare_start(launch(&pane, "second", "codex", &[]))
                        .unwrap();
                    assert!(
                        matches!(engine.start_agent(WAIT, &duplicate), Err(EngineError::Refused { code, .. }) if code == "agent_pane_busy")
                    );
                } else {
                    holders.say(
                        &pane,
                        &running(observation, hide_node_link::holders::ScreenState::Working),
                    );
                }
                finish.send(Err(undelivered_input())).unwrap();
                assert!(matches!(
                    starting.join().unwrap(),
                    Err(EngineError::Refused { .. })
                ));
            });
            let agents = engine.agents(WAIT).unwrap();
            assert_eq!(agents.len(), 1, "{observation}");
            assert_eq!(
                agents[0].agent.as_deref(),
                Some(if observation == "gemini" {
                    "gemini"
                } else {
                    "claude"
                })
            );
            if observation == "hook" {
                assert_eq!(
                    agents[0].agent_session.as_ref().unwrap().value,
                    "same-session"
                );
            }
        }
    }

    /// The old response can arrive after the original start deadline and
    /// a new start. Neither it nor the new refusal revives the expired one.
    #[test]
    fn an_expired_agent_start_cannot_withdraw_or_return_after_a_new_attempt() {
        for new_refused in [false, true] {
            let holders = Arc::new(Holders::default());
            holders.idle_shell(41, "zsh");
            let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
            let (_, _, pane) = engine
                .create_workspace(WAIT, "/w", "w", true, Env::new())
                .unwrap();
            let old = engine
                .prepare_start(launch(&pane, "old", "claude", &[]))
                .unwrap();
            let (admitted, finish) = holders.hold_input();
            std::thread::scope(|scope| {
                let starting = scope.spawn(|| engine.start_agent(WAIT, &old));
                admitted.recv_timeout(Duration::from_secs(5)).unwrap();
                engine
                    .topology()
                    .pane_mut(&pane)
                    .unwrap()
                    .agent
                    .as_mut()
                    .unwrap()
                    .live
                    .started = Some(
                    std::time::Instant::now() - agents::START_DEADLINE - Duration::from_secs(1),
                );
                let new = engine
                    .prepare_start(launch(&pane, "new", "codex", &[]))
                    .unwrap();
                if new_refused {
                    let (_admitted, reject) = holders.hold_input();
                    reject.send(Err(undelivered_input())).unwrap();
                    assert!(engine.start_agent(WAIT, &new).is_err());
                } else {
                    engine.start_agent(WAIT, &new).unwrap();
                }
                finish.send(Err(undelivered_input())).unwrap();
                assert!(starting.join().unwrap().is_err());
            });
            let agents = engine.agents(WAIT).unwrap();
            if new_refused {
                assert!(
                    agents.is_empty(),
                    "neither expired nor refused starts survive"
                );
            } else {
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0].name.as_deref(), Some("new"));
                assert_eq!(agents[0].agent.as_deref(), Some("codex"));
                assert!(!holders.typed(&pane).contains("claude"));
            }
        }
    }

    /// Rules in flight for an old start may finish, but the queued decision
    /// after refusal and a new launch belongs to the new agent.
    #[test]
    fn a_refused_start_and_its_successor_leave_the_new_agents_rules() {
        let holders = Arc::new(Holders::default());
        holders.idle_shell(41, "zsh");
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let (began, waiting) = std::sync::mpsc::channel();
        let (finish, go) = std::sync::mpsc::channel();
        *holders.rules_hold.lock().unwrap() = Some((began, go));
        let start = engine
            .prepare_start(launch(&pane, "old", "claude", &[]))
            .unwrap();
        let (admitted, reject) = holders.hold_input();
        std::thread::scope(|scope| {
            let starting = scope.spawn(|| engine.start_agent(WAIT, &start));
            admitted.recv_timeout(Duration::from_secs(5)).unwrap();
            waiting.recv_timeout(Duration::from_secs(5)).unwrap();
            reject.send(Err(undelivered_input())).unwrap();
            assert!(starting.join().unwrap().is_err());
            let new = engine
                .prepare_start(launch(&pane, "new", "codex", &[]))
                .unwrap();
            engine.start_agent(WAIT, &new).unwrap();
            finish.send(()).unwrap();
        });
        assert_eq!(
            holders.rules_given(2).last().unwrap().1.as_deref(),
            Some("codex")
        );
        let agents = engine.agents(WAIT).unwrap();
        assert_eq!(agents[0].name.as_deref(), Some("new"));
    }

    /// A typed agent can report its first session before the holder has
    /// recognized its program. Neither ordering loses the native identity.
    #[test]
    fn a_hook_session_waits_for_its_agent_without_claiming_another_agent() {
        use hide_node_link::holders::ScreenState;
        for report_first in [true, false] {
            let holders = Arc::new(Holders::default());
            let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
            let (_, _, pane) = engine
                .create_workspace(WAIT, "/w", "w", true, Env::new())
                .unwrap();
            if report_first {
                engine.hook_session(&pane, "claude-code", "first").unwrap();
                engine.hook_session(&pane, "claude-code", "latest").unwrap();
                assert!(engine.agents(WAIT).unwrap().is_empty());
            }
            holders.say(&pane, &running("claude", ScreenState::Working));
            if !report_first {
                engine.hook_session(&pane, "claude-code", "latest").unwrap();
            }
            let session = || engine.agents(WAIT).unwrap()[0].agent_session.clone();
            assert_eq!(session().unwrap().value, "latest");
            engine.hook_session(&pane, "gemini", "nested").unwrap();
            assert_eq!(session().unwrap().value, "latest");
            holders.say(&pane, &PaneFacts::default());
            holders.say(&pane, &running("claude", ScreenState::Working));
            assert!(
                session().is_none(),
                "an ended agent's report is not replayed"
            );
            holders.say(&pane, &PaneFacts::default());
            engine
                .hook_session(&pane, "gemini", "early-other-kind")
                .unwrap();
            holders.say(&pane, &running("claude", ScreenState::Working));
            assert!(
                session().is_none(),
                "detection discards a different kind's report"
            );
            holders.say(&pane, &PaneFacts::default());
            holders.say(&pane, &running("gemini", ScreenState::Working));
            assert!(
                session().is_none(),
                "a discarded report cannot reach a later agent"
            );
            holders.say(&pane, &PaneFacts::default());
            engine.hook_session(&pane, "claude", "closing").unwrap();
            engine.close_pane(WAIT, &pane).unwrap();
            assert!(engine.hook_session(&pane, "claude", "too-late").is_err());
        }
    }

    /// B19: the folder, title and first prompt the holder reads follow the
    /// pane, an agent typed into a shell is recognized by its program, and
    /// it leaves the list when its program does.
    #[test]
    fn a_panes_folder_title_prompt_and_typed_agent_follow_its_holders_facts() {
        use hide_node_link::holders::ScreenState;
        let holders = Arc::new(Holders::default());
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        assert_eq!(engine.pane_token(WAIT, &pane, "hide_prompt").unwrap(), None);
        holders.say(
            &pane,
            &PaneFacts {
                cwd: Some("/w/sub".to_owned()),
                title: Some("✳ Fix the tests".to_owned()),
                prompts: 1,
                ..running("gemini", ScreenState::Working)
            },
        );
        let session = engine.session(WAIT).unwrap();
        let shown = session.panes.iter().find(|p| p.pane_id == pane).unwrap();
        assert_eq!(shown.cwd.as_deref(), Some("/w/sub"));
        assert_eq!(shown.terminal_title.as_deref(), Some("✳ Fix the tests"));
        assert_eq!(
            shown.terminal_title_stripped.as_deref(),
            Some("Fix the tests")
        );
        assert_eq!(shown.agent_status, "working");
        assert_eq!(shown.foreground_process.as_deref(), Some("gemini"));
        assert_eq!(session.agents[0].agent.as_deref(), Some("gemini"));
        assert_eq!(
            engine
                .pane_token(WAIT, &pane, "hide_prompt")
                .unwrap()
                .as_deref(),
            Some("1")
        );
        assert_eq!(
            holders.rules_given(1),
            [(pane.clone(), Some("gemini".to_owned()))]
        );

        holders.say(&pane, &PaneFacts::default());
        assert!(engine.agents(WAIT).unwrap().is_empty());
        assert_eq!(holders.rules_given(2)[1], (pane, None));
    }

    /// Herdr's `agent.start` and `agent.prompt` refusals: a busy pane takes
    /// no start, and a prompt waits for the agent to show and not to ask.
    #[test]
    fn a_busy_pane_takes_no_start_and_a_prompt_is_pasted_only_into_a_ready_agent() {
        use hide_node_link::holders::ScreenState;
        let holders = Arc::new(Holders::default());
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        *holders.processes.lock().unwrap() = Some(HolderProcesses {
            shell: 41,
            foreground: Some(hide_node_link::holders::HolderForeground {
                group: 90,
                members: vec![(90, "vim".to_owned())],
            }),
        });
        let start = engine
            .prepare_start(launch(&pane, "worker", "codex", &[]))
            .unwrap();
        match engine.start_agent(WAIT, &start) {
            Err(EngineError::Refused { code, .. }) => assert_eq!(code, "agent_pane_busy"),
            other => panic!("expected a busy pane, got {other:?}"),
        }
        assert_eq!(holders.typed(&pane), "");

        holders.idle_shell(41, "bash");
        engine.start_agent(WAIT, &start).unwrap();
        assert_eq!(holders.typed(&pane), "codex\r");
        let refused = |answer: Answer<()>| match answer {
            Err(EngineError::Refused { code, .. }) => code,
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert_eq!(
            refused(engine.prompt_agent(WAIT, "worker", "hi")),
            "agent_not_ready"
        );
        let idle = PaneFacts {
            bracketed_paste: true,
            ..running("codex", ScreenState::Idle)
        };
        holders.say(&pane, &idle);
        assert_eq!(
            refused(engine.prompt_agent(WAIT, "worker", "hi")),
            "agent_not_ready",
            "a screen drawn within the start's settle"
        );
        settle_start(&engine, &pane);
        engine.prompt_agent(WAIT, "worker", "a\nb").unwrap();
        assert!(
            holders
                .typed(&pane)
                .ends_with("<letter><paste>a\nb</paste><Enter></letter>"),
            "{:?}",
            holders.typed(&pane)
        );
        holders.say(&pane, &running("codex", ScreenState::Blocked));
        assert_eq!(
            refused(engine.prompt_agent(WAIT, "worker", "hi")),
            "agent_blocked"
        );
    }

    /// A brief typed while the agent's program still leaves the terminal
    /// editing lines sits in its line with its Enter read as a line break:
    /// Pi 1.0.4 on a CI runner had not turned it off when its start's
    /// settle passed (nightly 41789009557). The engine types nothing then,
    /// whatever the time, and types the brief once the program reads keys.
    #[test]
    fn a_prompt_waits_for_the_agents_program_to_read_keys_past_the_settle() {
        use hide_node_link::holders::ScreenState;
        let holders = Arc::new(Holders::default());
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
        let (_, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        holders.idle_shell(41, "bash");
        let start = engine
            .prepare_start(launch(&pane, "worker", "pi", &[]))
            .unwrap();
        engine.start_agent(WAIT, &start).unwrap();
        let before = holders.typed(&pane);
        let at = |line_editing| PaneFacts {
            line_editing,
            ..running("pi", ScreenState::Idle)
        };
        holders.say(&pane, &at(Some(true)));
        settle_start(&engine, &pane);
        match engine.prompt_agent(WAIT, "worker", "brief") {
            Err(EngineError::Refused { code, .. }) => assert_eq!(code, "agent_not_ready"),
            other => panic!("expected the agent not ready, got {other:?}"),
        }
        assert_eq!(holders.typed(&pane), before, "nothing typed");
        let readiness = engine.delivery_agent(WAIT, "worker").unwrap().readiness;
        assert!(!readiness.accepts_input(), "{readiness:?}");

        holders.say(&pane, &at(Some(false)));
        settle_start(&engine, &pane);
        assert!(
            engine
                .delivery_agent(WAIT, "worker")
                .unwrap()
                .readiness
                .accepts_input()
        );
        engine.prompt_agent(WAIT, "worker", "brief").unwrap();
        assert!(
            holders.typed(&pane).ends_with("brief<Enter></letter>"),
            "{:?}",
            holders.typed(&pane)
        );
    }

    /// D-7, B13: after a reboot each agent pane resumes its conversation
    /// once, by its session or by its last one, in its new shell; a second
    /// pane holding the same conversation opens a shell that says why.
    #[test]
    fn a_reboot_resumes_each_conversation_once_and_says_why_a_pane_stayed_a_shell() {
        let root = tempfile::tempdir().unwrap();
        let place = place(root.path());
        let folder = root.path().join("repo");
        for dir in [&home(&place), &folder] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let first_holders = Holders::found(HolderRecovery::default());
        first_holders.idle_shell(41, "zsh");
        let first = OwnEngine::open(
            Arc::clone(&first_holders) as Arc<dyn PaneHolders>,
            place.clone(),
        )
        .unwrap();
        let (_, _, claude) = first
            .create_workspace(WAIT, &wire(&folder), "w", true, Env::new())
            .unwrap();
        let split = |at: &str| {
            first
                .split_pane_at(
                    WAIT,
                    at,
                    ClosedSplitDirection::Right,
                    &wire(&folder),
                    0.5,
                    Env::new(),
                )
                .unwrap()
        };
        let twin = split(&claude);
        let gemini = split(&twin);
        first
            .report_agent_session(WAIT, &claude, "claude", "s-1", 1)
            .unwrap();
        first
            .report_agent_session(WAIT, &twin, "claude-code", "s-1", 1)
            .unwrap();
        let start = first
            .prepare_start(launch(&gemini, "reader", "gemini", &[]))
            .unwrap();
        first.start_agent(WAIT, &start).unwrap();
        drop(first);

        let holders = Arc::new(Holders::default());
        holders.idle_shell(52, "zsh");
        let next =
            OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place.clone()).unwrap();
        holders.hand(HolderRecovery {
            owner: 1,
            ended: vec![claude.clone(), twin.clone(), gemini.clone()],
            ..HolderRecovery::default()
        });
        next.restored().unwrap();

        assert_eq!(
            holders.wait_typed(&claude),
            "claude --resume s-1 || printf 'hide: %s ended with a failure after its resume; this pane is a shell now.\\n' 'Claude Code'\r"
        );
        assert!(
            holders
                .wait_typed(&gemini)
                .starts_with("gemini --resume latest || printf"),
            "the last conversation of an agent whose session is not known"
        );
        assert_eq!(holders.typed(&twin), "", "one conversation, one pane");
        let notice = holders
            .started()
            .into_iter()
            .find(|start| start.pane == twin)
            .and_then(|start| start.notice)
            .unwrap_or_default();
        assert!(
            notice.contains("Claude Code's conversation is open in another pane"),
            "{notice}"
        );
        let reader = next.delivery_agent(WAIT, "reader").unwrap();
        assert_eq!(reader.pane_id, gemini, "the resumed agent keeps its name");
    }

    /// PRD engine-bundle-4-herdr-removal D-24, B37: a rebooted agent resumes
    /// with the model and effort Hide started it with when its pane's launch
    /// record describes it; a record that names another agent is not read,
    /// so that pane resumes on its CLI's defaults.
    #[test]
    fn a_reboot_resumes_an_agent_with_the_model_and_effort_it_was_started_with() {
        let root = tempfile::tempdir().unwrap();
        let place = place(root.path());
        let folder = root.path().join("repo");
        for dir in [&home(&place), &folder] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let first_holders = Holders::found(HolderRecovery::default());
        first_holders.idle_shell(41, "zsh");
        let first = OwnEngine::open(
            Arc::clone(&first_holders) as Arc<dyn PaneHolders>,
            place.clone(),
        )
        .unwrap();
        let (_, _, worker) = first
            .create_workspace(WAIT, &wire(&folder), "w", true, Env::new())
            .unwrap();
        let other = first
            .split_pane_at(
                WAIT,
                &worker,
                ClosedSplitDirection::Right,
                &wire(&folder),
                0.5,
                Env::new(),
            )
            .unwrap();
        for (pane, name, session) in [(&worker, "worker", "s-1"), (&other, "other", "s-2")] {
            let start = first
                .prepare_start(launch(pane, name, "claude", &[]))
                .unwrap();
            first.start_agent(WAIT, &start).unwrap();
            first
                .report_agent_session(WAIT, pane, "claude", session, 1)
                .unwrap();
        }
        drop(first);

        let chosen = hide_agent_adapter::LaunchChoice {
            model: Some("opus".to_owned()),
            effort: Some("high".to_owned()),
        };
        let record = |name: &str| crate::model::PaneLaunch {
            kind: "claude".to_owned(),
            name: name.to_owned(),
            choice: chosen.clone(),
            session: None,
        };
        let place = Place {
            launches: Arc::new(BTreeMap::from([
                (worker.clone(), record("worker")),
                // The other pane's record names an agent it no longer runs.
                (other.clone(), record("someone-else")),
            ])),
            ..place
        };
        let holders = Arc::new(Holders::default());
        holders.idle_shell(52, "zsh");
        let next = OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place).unwrap();
        holders.hand(HolderRecovery {
            owner: 1,
            ended: vec![worker.clone(), other.clone()],
            ..HolderRecovery::default()
        });
        next.restored().unwrap();

        assert!(
            holders
                .wait_typed(&worker)
                .starts_with("claude --resume s-1 --model opus --effort high || printf"),
            "{:?}",
            holders.typed(&worker)
        );
        assert!(
            holders
                .wait_typed(&other)
                .starts_with("claude --resume s-2 || printf"),
            "{:?}",
            holders.typed(&other)
        );
    }

    /// PRD engine-bundle-4-herdr-removal D-22, B9: an agent whose hook last
    /// said its turn was running when the machine went down is resumed and,
    /// once its hook names its session, said to be owed its letter, once; an
    /// agent that had finished its turn is owed none.
    #[test]
    fn a_resumed_agent_whose_turn_was_cut_off_is_owed_one_letter_and_a_resting_one_none() {
        use hide_agent_hooks::counters::{Change, Reporter, Said, apply};
        use hide_agent_hooks::report::HookTurn;
        let root = tempfile::tempdir().unwrap();
        let place = place(root.path());
        let folder = root.path().join("repo");
        for dir in [&home(&place), &folder] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let first_holders = Holders::found(HolderRecovery::default());
        first_holders.idle_shell(41, "zsh");
        let first = OwnEngine::open(
            Arc::clone(&first_holders) as Arc<dyn PaneHolders>,
            place.clone(),
        )
        .unwrap();
        let (_, _, working) = first
            .create_workspace(WAIT, &wire(&folder), "w", true, Env::new())
            .unwrap();
        let resting = first
            .split_pane_at(
                WAIT,
                &working,
                ClosedSplitDirection::Right,
                &wire(&folder),
                0.5,
                Env::new(),
            )
            .unwrap();
        for (pane, session, turn) in [
            (&working, "s-1", HookTurn::Working),
            (&resting, "s-2", HookTurn::Done),
        ] {
            first
                .report_agent_session(WAIT, pane, "claude", session, 1)
                .unwrap();
            // What the agent's hook wrote last in the pane's file.
            apply(
                &home(&place),
                pane,
                Change::None,
                Reporter {
                    agent: Some("claude"),
                    session: Some(session),
                },
                Said::Turn(turn),
                None,
            )
            .unwrap();
        }
        drop(first);

        let holders = Arc::new(Holders::default());
        holders.idle_shell(52, "zsh");
        let next =
            OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place.clone()).unwrap();
        holders.hand(HolderRecovery {
            owner: 1,
            ended: vec![working.clone(), resting.clone()],
            ..HolderRecovery::default()
        });
        next.restored().unwrap();
        for pane in [&working, &resting] {
            assert!(
                holders.wait_typed(pane).starts_with("claude --resume"),
                "{:?}",
                holders.typed(pane)
            );
        }

        assert!(next.hook_session(&working, "claude", "s-1").unwrap());
        assert!(
            !next.hook_session(&working, "claude", "s-1").unwrap(),
            "owed once"
        );
        assert!(!next.hook_session(&resting, "claude", "s-2").unwrap());
    }

    /// D-8, B15: a conversation an import gave back once its Herdr stopped,
    /// whose pane runs another command, resumes in a new tab of the pane's
    /// workspace; nothing is typed over the program, and the give-back is
    /// settled, so nothing waits for the next import.
    #[test]
    fn a_given_back_conversation_whose_pane_is_busy_resumes_in_a_new_tab_of_its_workspace() {
        let root = tempfile::tempdir().unwrap();
        let place = place(root.path());
        std::fs::create_dir_all(home(&place)).unwrap();
        let save = hide_host::herdr_save::fetch(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/herdr-session/herdr-0.9.5.json"),
        );
        // Imported beside its running Herdr, then again once it stopped.
        for resume in [false, true] {
            crate::herdr_import::import(
                &place.engine_dir,
                &save,
                crate::herdr_import::Asked { resume },
            )
            .unwrap();
        }
        let busy = "w1:p1".to_owned();
        assert_eq!(
            crate::herdr_import::given_back(&place.engine_dir),
            BTreeSet::from([busy.clone()])
        );
        let saved: BTreeSet<String> = restore::read(
            &hide_kit::layout::engine_session_in(&place.engine_dir),
            &hide_kit::layout::engine_backups_in(&place.engine_dir),
        )
        .panes()
        .map(|(id, _)| id.to_owned())
        .collect();

        let holders = Arc::new(Holders::default());
        holders.idle_shell(52, "zsh");
        holders.busy.lock().unwrap().insert(busy.clone());
        let engine =
            OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place.clone()).unwrap();
        holders.hand(HolderRecovery {
            owner: 1,
            present: vec![busy.clone()],
            ..HolderRecovery::default()
        });
        engine.restored().unwrap();
        settles("the conversation's new tab started", || {
            holders
                .started()
                .iter()
                .any(|start| !saved.contains(&start.pane))
        });
        let tab = holders
            .started()
            .into_iter()
            .find(|start| !saved.contains(&start.pane))
            .unwrap()
            .pane;
        assert!(
            holders
                .wait_typed(&tab)
                .starts_with("claude --resume 0b6a3f1e-claude-session"),
            "{:?}",
            holders.typed(&tab)
        );
        assert_eq!(engine.topology().pane_place(&tab).unwrap().0, "w1");
        assert_eq!(
            holders.typed(&busy),
            "",
            "nothing is typed over its program"
        );
        assert!(crate::herdr_import::given_back(&place.engine_dir).is_empty());
        assert!(crate::herdr_import::waiting(&place.engine_dir).is_empty());
    }

    /// D-7: a pane whose holder outlived the daemon keeps its program, but
    /// the resume the daemon typed may never have reached its shell (the
    /// daemon ended first). Its shell at its prompt resumes the conversation
    /// again, once; a pane whose agent runs is left as it is.
    #[test]
    fn an_adopted_pane_whose_shell_is_at_its_prompt_resumes_its_agent_again() {
        let root = tempfile::tempdir().unwrap();
        let place = place(root.path());
        let folder = root.path().join("repo");
        for dir in [&home(&place), &folder] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let first_holders = Holders::found(HolderRecovery::default());
        first_holders.idle_shell(41, "zsh");
        let first = OwnEngine::open(
            Arc::clone(&first_holders) as Arc<dyn PaneHolders>,
            place.clone(),
        )
        .unwrap();
        let (_, _, claude) = first
            .create_workspace(WAIT, &wire(&folder), "w", true, Env::new())
            .unwrap();
        first
            .report_agent_session(WAIT, &claude, "claude", "s-1", 1)
            .unwrap();
        drop(first);
        let reopen = |processes: HolderProcesses| {
            let holders = Arc::new(Holders::default());
            *holders.processes.lock().unwrap() = Some(processes);
            let engine =
                OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place.clone())
                    .unwrap();
            holders.hand(HolderRecovery {
                owner: 1,
                present: vec![claude.clone()],
                ..HolderRecovery::default()
            });
            engine.restored().unwrap();
            (holders, engine)
        };

        // The agent runs in its terminal's foreground: nothing is typed.
        let (running, engine) = reopen(HolderProcesses {
            shell: 52,
            foreground: Some(hide_node_link::holders::HolderForeground {
                group: 60,
                members: vec![(60, "claude".to_owned())],
            }),
        });
        assert_eq!(running.typed(&claude), "");
        drop(engine);

        let (idle, _engine) = reopen(HolderProcesses {
            shell: 52,
            foreground: Some(hide_node_link::holders::HolderForeground {
                group: 52,
                members: vec![(52, "zsh".to_owned())],
            }),
        });
        assert!(
            idle.wait_typed(&claude)
                .starts_with("claude --resume s-1 || printf"),
            "{:?}",
            idle.typed(&claude)
        );
        assert!(
            idle.started().is_empty(),
            "the adopted pane keeps its shell"
        );
    }
}
