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
//! ([`restore`]), so a pane keeps its id across a restart and a reboot. What
//! a later bundle 3 pull request brings (pane identity, scrolling, paste)
//! answers a refusal under [`UNSUPPORTED`] that names it, never a silent
//! default. Only a daemon with the development
//! switch `HIDE_DEV_PANE_HOLDER` on builds this engine (D-5).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use hide_node_link::NodeLink;
use hide_node_link::holders::{
    HolderError, HolderKey, HolderStart, PaneFacts, PaneHolders, PaneInput, StyledRead,
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
mod ids;
pub(crate) mod layout;
mod restore;
mod rules;
mod shell_line;
mod store;
mod topology;
mod worktrees;

use topology::{Closed, Pane, Topology, refused};

/// The refusal code of a call this engine does not answer yet.
pub(crate) const UNSUPPORTED: &str = "own_engine_unsupported";

/// The size a pane starts at before a screen draws it; the first screen
/// that attaches it resizes it to the rectangle drawn for it.
const START_ROWS: u16 = 24;
const START_COLS: u16 = 80;

/// The most of a pane's history a read asks its holder for.
const READ_LINES_CAP: u32 = u16::MAX as u32;

/// The read sources this engine answers, each with the one read a holder
/// gives: the last rows of its screen and scrollback, soft wraps joined
/// (`hide_holder_protocol::Ask::Read`). `recent` is therefore unwrapped as
/// `recent_unwrapped` is, and `visible` reads those same last rows, the
/// screen's bottom rows among them; a read of exactly the drawn screen is
/// not answered yet: the screen's size is the attached client's, which the
/// engine does not hold.
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
    pub(crate) node: Arc<dyn NodeLink>,
    /// The agent definitions the operator's folder held when the daemon
    /// started, so a restore resumes their agents before the core's first
    /// read reaches the engine.
    pub(crate) definitions: Arc<hide_agent_adapter::definition::Catalog>,
}

/// The engine and whatever it holds, shared by every gated view of it.
struct Shared {
    topology: Arc<Mutex<Topology>>,
    holders: Arc<dyn PaneHolders>,
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
    /// Facts a holder sent before its pane entered the topology (a pane
    /// reserves its id and starts its holder first), laid on once it has.
    /// The oldest goes first past [`EARLY_FACTS_CAP`].
    early: Mutex<Vec<(String, PaneFacts)>>,
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
    done: Mutex<bool>,
    finished: Condvar,
}

impl Restored {
    pub(super) fn finish(&self) {
        *self.done.lock().unwrap_or_else(PoisonError::into_inner) = true;
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
    definitions: Arc<hide_agent_adapter::definition::Catalog>,
) -> Result<OwnEngine<'static>, String> {
    let engine_dir = hide_kit::layout::device_engine_dir(state_dir, device)?;
    OwnEngine::open(
        link_panes(Arc::clone(&node)),
        Place {
            engine_dir,
            home,
            node,
            definitions,
        },
    )
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
        let engine = Self {
            shared: Arc::new(Shared {
                topology: Arc::new(Mutex::new(topology)),
                rules: rules::RulesWorker::start(Arc::clone(&holders))?,
                holders,
                wake: Mutex::default(),
                stopped: AtomicBool::new(false),
                place,
                saver: OnceLock::new(),
                restored: Arc::new(Restored {
                    done: Mutex::new(restored),
                    finished: Condvar::new(),
                }),
                definitions: agents::Definitions::new(definitions),
                early: Mutex::default(),
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
            .wait_timeout_while(done, restore::BOUND, |done| !*done)
            .unwrap_or_else(PoisonError::into_inner);
        if *done {
            Ok(())
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
                if let Some(kind) = applied.rules_for {
                    self.give_rules(pane, kind.as_deref(), &catalog);
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
            if let Ok(applied) = self.topology().apply_facts(&pane, &facts, &catalog)
                && let Some(kind) = applied.rules_for
            {
                self.give_rules(&pane, kind.as_deref(), &catalog);
            }
        }
    }

    /// Gives `pane`'s holder the screen rules of the agent `kind`, or none.
    fn give_rules(
        &self,
        pane: &str,
        kind: Option<&str>,
        catalog: &hide_agent_adapter::definition::Catalog,
    ) {
        let rules = kind
            .and_then(|kind| agents::definition(catalog, kind))
            .and_then(agents::screen_rules);
        self.shared.rules.give(pane, rules);
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
        self.shared
            .holders
            .start(&HolderStart {
                pane: pane.to_owned(),
                program: program.map(<[String]>::to_vec),
                cwd: cwd.to_owned(),
                env: env.into_iter().collect(),
                rows: START_ROWS,
                cols: START_COLS,
                notice: notice.map(str::to_owned),
            })
            .map_err(holder_error)
    }

    /// Starts the holder of `pane`, whose id this call just reserved, once
    /// every gate's [`super::Placement`] has recorded it (PRD
    /// engine-bundle-3-core-owns D-11). The order is the locks' order and
    /// never overlaps: the topology lock was let go once the id was reserved
    /// (the caller's block ended), a placement takes the lock its record
    /// needs (the runtime's, for a spawn) and lets it go, and the holder
    /// starts outside every lock. A placement that refuses leaves the pane
    /// unmade; a holder that does not start withdraws every placement in this
    /// same call, so no record names a pane that never ran.
    fn start_placed(
        &self,
        pane: &str,
        program: Option<&[String]>,
        cwd: &str,
        env: Env,
    ) -> Result<(), EngineError> {
        let placements: Vec<_> = self
            .gates
            .iter()
            .filter_map(|gate| gate.placement.as_ref())
            .collect();
        for (at, placement) in placements.iter().enumerate() {
            if let Err(error) = placement.place(pane) {
                for placed in &placements[..at] {
                    placed.withdraw(pane);
                }
                return Err(error);
            }
        }
        if let Err(error) = self.start_pane(pane, program, cwd, env, None) {
            self.withdraw(pane);
            return Err(error);
        }
        Ok(())
    }

    /// Takes back every gate's placement of `pane`, which will not run.
    fn withdraw(&self, pane: &str) {
        for placement in self.gates.iter().filter_map(|gate| gate.placement.as_ref()) {
            placement.withdraw(pane);
        }
    }

    /// Ends the holders of `panes`, every one asked even when one fails:
    /// the first failure, if any. A holder already gone is ended.
    fn end_panes(&self, panes: &[String]) -> Result<(), EngineError> {
        let mut first = None;
        for pane in panes {
            match self.shared.holders.terminate(pane) {
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
        self.start_placed(pane, program, cwd, env.clone())?;
        let added = self.topology().add_tab(
            workspace,
            (tab, pane),
            label,
            Pane::started(cwd, pane_label, program, &env),
            focus,
        );
        if let Err(error) = added {
            // The workspace closed while the pane started.
            let _ = self.end_panes(std::slice::from_ref(pane));
            self.withdraw(pane);
            return Err(error);
        }
        self.changed();
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
        let (pane, cwd) = {
            let mut topology = self.topology();
            let cwd = match cwd.filter(|cwd| !cwd.trim().is_empty()) {
                Some(cwd) => cwd.to_owned(),
                None => topology.pane(target)?.cwd.clone(),
            };
            let (workspace, _) = topology.pane_place(target)?;
            (topology.reserve_pane(&workspace)?, cwd)
        };
        self.start_placed(&pane, None, &cwd, env.clone())?;
        let added = self.topology().add_split(
            target,
            direction,
            ratio,
            &pane,
            Pane::started(&cwd, None, None, &env),
            focus,
        );
        if let Err(error) = added {
            // The pane beside it closed while this one started.
            let _ = self.end_panes(std::slice::from_ref(&pane));
            self.withdraw(&pane);
            return Err(error);
        }
        self.changed();
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
        let panes = self.topology().tab_panes(tab)?;
        self.end_panes(&panes)?;
        match self.topology().remove_tab(tab) {
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

    fn read_styled(&self, pane: &str, read: StyledRead) -> Answer<PaneText> {
        self.known(pane)?;
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
        self.known(pane)?;
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
        HolderError::Unreachable { message } => EngineError::Transport(message),
    }
}

/// The refusal of a call a later bundle 3 pull request answers.
fn unsupported<T>(call: &str, brought_by: &str) -> Answer<T> {
    Err(EngineError::Refused {
        code: UNSUPPORTED.to_owned(),
        message: format!(
            "the core's own engine does not answer {call} yet; the {brought_by} brings it"
        ),
    })
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
            Some(agent.starting(std::time::Instant::now())),
        ),
    }
}

/// What brings a find's scroll of the screen: the holder saying and moving
/// its screen's scroll position (PRD engine-bundle-3-core-owns D-21).
const SCROLL_POSITION: &str = "holder's scroll position capability";

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
                self.shared.rules.give(&pane, now);
            }
        }
        self.changed();
    }

    fn hook_session(&self, pane: &str, kind: &str, session: &str) -> Answer<()> {
        self.check()?;
        let catalog = self.shared.definitions.get();
        let definition =
            agents::definition(&catalog, kind).ok_or_else(|| EngineError::Refused {
                code: "unknown_agent".to_owned(),
                message: format!("no agent definition names {kind}"),
            })?;
        // Only the agent the pane runs takes it: a session the operator's
        // own hook started in the pane's environment (`claude -p` from a
        // Stop hook) is another agent's, and the engine already knows the
        // pane runs no agent of that kind.
        let runs = self
            .topology()
            .pane(pane)?
            .agent
            .as_ref()
            .is_some_and(|agent| agent.kind == definition.id);
        if runs
            && self
                .topology()
                .set_agent_session(pane, &definition.id, session)?
        {
            self.changed();
        }
        Ok(())
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
        self.end_panes(&[pane.to_owned()])?;
        match self.topology().remove_pane(pane) {
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

    fn pane_scroll(
        &self,
        _wait: Wait,
        pane: &str,
    ) -> Answer<Option<crate::find::pane::PaneScroll>> {
        self.check()?;
        // The holder keeps the scroll of its screen connection and reports
        // none here.
        self.known(pane)?;
        Ok(None)
    }

    fn scroll_pane(
        &self,
        _wait: Wait,
        _pane: &str,
        _offset_from_bottom: u64,
    ) -> Answer<Option<crate::find::pane::PaneScroll>> {
        self.check()?;
        unsupported("a scroll", SCROLL_POSITION)
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
        self.known(pane)?;
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
        let keys = keys
            .iter()
            .map(|key| named_key(key).map(holder_key))
            .collect::<Answer<_>>()?;
        self.type_input(pane, PaneInput::Keys(keys))
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
        self.known(pane)?;
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
        let shell = shell_line::Shell::of(processes.shell_name()).ok_or_else(|| {
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
        {
            let mut topology = self.topology();
            if topology.name_taken(&launch.name, pane) {
                return Err(refused(
                    "agent_name_taken",
                    format!("another pane runs an agent named {}", launch.name),
                ));
            }
            topology.start_agent(
                pane,
                agents::PaneAgent::started(
                    &definition.id,
                    Some(&launch.name),
                    agents::resumed_in(definition, &argv),
                ),
            )?;
        }
        self.give_rules(pane, Some(&definition.id), &catalog);
        self.changed();
        self.type_bytes(pane, format!("{line}\r").into_bytes())?;
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
            if !agent.live.shown || agent.starting(std::time::Instant::now()) {
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Condvar, Mutex};

    use hide_node_link::holders::{HolderProcesses, HolderRecovery};

    use crate::sidebar::SessionSnapshotPayload;

    use super::*;

    /// Holders that start every pane until told to refuse, as a node whose
    /// holder program cannot start refuses, and say what they found of the
    /// last daemon's holders once the test hands them that.
    #[derive(Default)]
    struct Holders {
        refuse: AtomicBool,
        started: Mutex<Vec<HolderStart>>,
        ended: Mutex<Vec<String>>,
        found: (Mutex<Option<HolderRecovery>>, Condvar),
        ends: Mutex<Option<hide_node_link::holders::PaneEnded>>,
        facts: Mutex<Option<hide_node_link::holders::PaneFactsSink>>,
        /// The rules given to each pane, by agent id, in order.
        rules: (Mutex<Given>, Condvar),
        typed: (Mutex<Typed>, Condvar),
        /// What every pane's terminal holds; `None` answers unreachable.
        processes: Mutex<Option<HolderProcesses>>,
    }

    impl Holders {
        fn found(recovery: HolderRecovery) -> Arc<Self> {
            let holders = Arc::new(Self::default());
            holders.hand(recovery);
            holders
        }

        fn hand(&self, recovery: HolderRecovery) {
            *self.found.0.lock().unwrap() = Some(recovery);
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
        fn start(&self, start: &HolderStart) -> Result<(), HolderError> {
            if self.refuse.load(Ordering::Acquire) {
                return Err(HolderError::Refused {
                    code: "holder_start_failed".to_owned(),
                    message: format!("pane {}: the holder program is missing", start.pane),
                });
            }
            self.started.lock().unwrap().push(start.clone());
            Ok(())
        }
        fn terminate(&self, pane: &str) -> Result<(), HolderError> {
            self.ended.lock().unwrap().push(pane.to_owned());
            Ok(())
        }
        fn read(&self, _: &str, _: u16) -> Result<(String, bool), HolderError> {
            Ok(("rows".to_owned(), false))
        }
        fn read_styled(&self, _: &str, read: StyledRead) -> Result<(String, bool), HolderError> {
            Ok((format!("\u{1b}[1m{read:?}\u{1b}[0m"), false))
        }
        fn processes(&self, _: &str) -> Result<HolderProcesses, HolderError> {
            self.processes
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| HolderError::Unreachable {
                    message: "not asked".to_owned(),
                })
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
            found.clone().ok_or(HolderError::Refused {
                code: "recovery_unfinished".to_owned(),
                message: "not handed".to_owned(),
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
        for source in ["recent", "recent_unwrapped", "visible"] {
            assert_eq!(
                engine.read_pane(WAIT, &pane, source, 20).unwrap().text,
                "rows"
            );
        }
        match engine.read_pane(WAIT, &pane, "scrollback", 20) {
            Err(EngineError::Refused { code, message }) => {
                assert_eq!(code, "unknown_read_source");
                assert!(message.contains("\"scrollback\""), "{message}");
            }
            other => panic!("expected a refusal naming the source, got {other:?}"),
        }
    }

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
    fn a_pane_whose_holder_cannot_start_is_refused_with_its_reason_and_leaves_nothing() {
        let holders = Arc::new(Holders::default());
        let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);

        holders.refuse.store(true, Ordering::Release);
        refused_for_the_holder(engine.create_workspace(WAIT, "/w", "w", true, Env::new()));
        assert!(engine.pane_folders(WAIT).unwrap().is_empty());
        assert_eq!(engine.focused_workspace(WAIT).unwrap(), None);

        holders.refuse.store(false, Ordering::Release);
        let (workspace, _, pane) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        holders.refuse.store(true, Ordering::Release);
        refused_for_the_holder(engine.split_beside(WAIT, &pane, Some("/w"), Env::new()));
        refused_for_the_holder(engine.launch_tab(
            WAIT,
            &workspace,
            None,
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
            home: wire(&home),
            definitions: Arc::new(hide_agent_adapter::definition::Catalog::read(Vec::new())),
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
            closed: vec![exited.clone()],
            ended: vec![ended.clone()],
            present: vec![running.clone(), shown, unplaced.clone()],
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
            closed: Vec::new(),
            ended: Vec::new(),
            present: vec!["w2:p1".to_owned(), "w2:p3".to_owned()],
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
        assert_eq!(
            (workspace.as_str(), pane.as_str()),
            ("w3", "w3:p1"),
            "a new workspace past the recorded ones"
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
            .open_worktree(WAIT, &wire(&herdr_place), &wire(&repo), false)
            .unwrap();
        assert!(again.already_open);
        assert_eq!(again.workspace_id, opened.workspace_id);

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

        holders.say(&pane, &running("claude", ScreenState::Working));
        let working = engine.agent_state(WAIT, "worker").unwrap();
        assert_eq!(working.status, "working");
        assert_eq!(
            engine.delivery_agent(WAIT, &pane).unwrap().readiness,
            Readiness::Ready,
            "the pane id names its agent too"
        );
        holders.say(&pane, &running("claude", ScreenState::Idle));
        let done = engine.agent_state(WAIT, "worker").unwrap();
        assert_eq!(done.status, "done", "a rest after work is done");
        assert!(done.state_change_seq > working.state_change_seq);
        let listed = engine.agents(WAIT).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name.as_deref(), Some("worker"));
        assert_eq!(listed[0].agent_status.as_deref(), Some("done"));
        assert_eq!(engine.pane_agent_status(WAIT, &pane).unwrap(), "done");
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
        holders.say(
            &pane,
            &PaneFacts {
                bracketed_paste: true,
                ..running("codex", ScreenState::Idle)
            },
        );
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
