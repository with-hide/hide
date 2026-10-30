//! Searching a pane's text and scrolling its viewport through the engine,
//! for a local pane or a device's.

use std::sync::{Arc, Mutex, Weak};
use std::thread;

use crate::engine::{Engine, EngineError, PaneText, SharedEngine};

use crate::agent_find::AgentFind;
use crate::find::PaneFindOptions;
use crate::handle::ChangeNotifier;
use crate::live::{CONTROL_WAIT, LiveContext, RemoteControlContext, failed};
use crate::model::PaneFindRoute;
use crate::runtime::Runtime;

/// How many lines of history a pane search asks Herdr for.
///
/// Herdr caps what it keeps; this is the ceiling on what is searched, and it
/// is reported alongside the count so a truncated buffer is visible rather
/// than quietly reported as the whole thing.
const PANE_FIND_LINE_LIMIT: u32 = 10_000;

/// What a pane search needs, gathered under the runtime mutex so the worker
/// carries no reference back into runtime state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneFindRequest {
    pub term: String,
    pub options: PaneFindOptions,
    /// Which match to move to once the search lands: 0 keeps the current one,
    /// and stepping is relative so a search and a step share one path.
    pub step: i64,
    /// The index the shell is on now, so a step continues from it rather than
    /// restarting at the top every time.
    pub current_index: usize,
}

/// What a pane search found, in the shape the runtime stores.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PaneFindOutcome {
    pub term: String,
    pub total: usize,
    /// 1-based position of the current match, or 0 when there is none.
    pub index: usize,
    pub truncated: bool,
    /// The viewport move that puts the current match on screen, as a direction
    /// and a line count. `None` when it is already there.
    pub scroll: Option<(String, u16)>,
}

/// Where one pane's socket requests go: this machine's Herdr, or a device's,
/// with the id that Herdr knows the pane by beside the id the core projects.
#[derive(Clone)]
pub struct PaneApiRoute {
    pub(crate) pane_id: String,
    herdr_pane_id: String,
    engine: SharedEngine,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

impl PaneApiRoute {
    pub(crate) fn local(context: &LiveContext, pane_id: &str) -> Self {
        Self {
            pane_id: pane_id.to_owned(),
            herdr_pane_id: pane_id.to_owned(),
            engine: Arc::clone(&context.engine),
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
        }
    }

    pub(crate) fn remote(
        context: &RemoteControlContext,
        pane_id: &str,
        herdr_pane_id: &str,
    ) -> Self {
        Self {
            pane_id: pane_id.to_owned(),
            herdr_pane_id: herdr_pane_id.to_owned(),
            engine: Arc::clone(&context.engine),
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
        }
    }

    /// Runs `work` on its own thread and hands its result to `ingest` under
    /// the runtime lock, notifying when that changed the snapshot.
    pub(crate) fn spawn<T: Send + 'static>(
        self,
        name: &str,
        work: impl FnOnce(&dyn Engine, &str) -> T + Send + 'static,
        ingest: impl FnOnce(&mut Runtime, &str, T) -> bool + Send + 'static,
    ) -> Result<(), String> {
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let result = work(self.engine.as_ref(), &self.herdr_pane_id);
                let Some(runtime) = self.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => ingest(&mut guard, &self.pane_id, result),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    self.notifier.notify();
                }
            })
            .map(|_| ())
            .map_err(|error| format!("{name} worker could not be started: {error}"))
    }
}

/// Searches a pane's whole scrollback and moves the viewport to the match.
///
/// The two reads and the search happen on this thread, never under the runtime
/// mutex: the buffer is thousands of lines and every shell snapshot read blocks
/// on that mutex.
pub fn spawn_pane_find(route: PaneApiRoute, request: PaneFindRequest) -> Result<(), String> {
    route.spawn(
        "herdr-core-pane-find",
        move |engine, herdr_pane_id| run_pane_find(engine, herdr_pane_id, &request),
        |runtime, pane_id, result| runtime.ingest_pane_find(pane_id, result),
    )
}

/// Opens an agent's own search in its pane, unless Herdr holds the pane's
/// history, which Hide's find bar searches instead (`agent_find.rs`).
///
/// The requests run on this thread, never under the runtime mutex.
pub(crate) fn spawn_agent_find(
    route: PaneApiRoute,
    request_id: String,
    find: AgentFind,
    gate: Arc<crate::input_gate::PaneInputGate>,
) -> Result<(), String> {
    let core_pane_id = route.pane_id.clone();
    route.spawn(
        "herdr-core-agent-find",
        move |engine, herdr_pane_id| {
            open_agent_find(engine, herdr_pane_id, find, &gate, &core_pane_id)
        },
        move |runtime, pane_id, result| runtime.ingest_agent_find(pane_id, request_id, result),
    )
}

fn open_agent_find(
    engine: &dyn Engine,
    pane_id: &str,
    find: AgentFind,
    gate: &Arc<crate::input_gate::PaneInputGate>,
    core_pane_id: &str,
) -> Result<PaneFindRoute, String> {
    // An agent that draws inline leaves its conversation in Herdr's history,
    // where the find bar reaches all of it; its own search may not exist in
    // that mode (Claude Code's default renderer has none).
    let scroll = engine
        .pane_scroll(CONTROL_WAIT, pane_id)
        .map_err(failed("pane.get"))?;
    if scroll.is_some_and(|scroll| scroll.max_offset_from_bottom > 0) {
        return Ok(PaneFindRoute::Bar);
    }
    let visible = match find.already_open {
        Some(_) => Some(read_pane_text(engine, pane_id, "visible")?.text),
        None => None,
    };
    // The keys go into the agent's own search box; into the shell under a
    // sleeping agent they would be typed as a command.
    let _admission = gate
        .admit(core_pane_id)
        .map_err(|_| "The agent is asleep, so its own search is closed".to_owned())?;
    engine
        .send_keys(CONTROL_WAIT, pane_id, find.keys(visible.as_deref()))
        .map_err(failed("pane.send_keys"))?;
    Ok(PaneFindRoute::Agent)
}

/// Herdr's scroll position for one pane, in lines above the bottom.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneScroll {
    pub offset_from_bottom: u64,
    pub max_offset_from_bottom: u64,
}

/// Moves an observed pane's viewport by `lines` (positive shows older lines).
///
/// `pane.scroll` takes an absolute offset and any socket client may send it,
/// which is what an observer without a terminal writer has; the offset comes
/// from the metrics `pane.get` reports, and Herdr clamps an overshoot at the
/// top. Two requests off the runtime mutex; the core keeps one in flight per
/// pane and sums the wheels that arrive meanwhile.
pub fn spawn_viewport_scroll(route: PaneApiRoute, lines: i32) -> Result<(), String> {
    route.spawn(
        "herdr-core-viewport-scroll",
        move |engine, herdr_pane_id| scroll_viewport(engine, herdr_pane_id, lines),
        |runtime, pane_id, result| runtime.ingest_viewport_scroll(pane_id, result),
    )
}

/// Why an observed pane's viewport did not move.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewportScrollError {
    /// Herdr answered and refused the request (an older device Herdr without
    /// `pane.scroll`, say): this client cannot scroll the pane.
    Refused(String),
    /// Herdr was not reached or answered something unreadable; nothing is
    /// known about the pane's scrolling.
    Unreachable(String),
}

fn scroll_failure(method: &str) -> impl FnOnce(EngineError) -> ViewportScrollError + '_ {
    move |error| match error {
        EngineError::Refused { code, message } => {
            ViewportScrollError::Refused(format!("{method} was refused: {code}: {message}"))
        }
        EngineError::Request(message) | EngineError::Answer(message) => {
            ViewportScrollError::Unreachable(message)
        }
        error => ViewportScrollError::Unreachable(format!("{method} failed: {error}")),
    }
}

fn scroll_viewport(
    engine: &dyn Engine,
    pane_id: &str,
    lines: i32,
) -> Result<Option<PaneScroll>, ViewportScrollError> {
    let Some(current) = engine
        .pane_scroll(CONTROL_WAIT, pane_id)
        .map_err(scroll_failure("pane.get"))?
    else {
        return Ok(None);
    };
    let distance = u64::from(lines.unsigned_abs());
    let target = if lines > 0 {
        current.offset_from_bottom.saturating_add(distance)
    } else {
        current.offset_from_bottom.saturating_sub(distance)
    };
    if target == current.offset_from_bottom {
        return Ok(Some(current));
    }
    engine
        .scroll_pane(CONTROL_WAIT, pane_id, target)
        .map_err(scroll_failure("pane.scroll"))
}

fn run_pane_find(
    engine: &dyn Engine,
    pane_id: &str,
    request: &PaneFindRequest,
) -> Result<PaneFindOutcome, String> {
    if request.term.is_empty() {
        return Ok(PaneFindOutcome {
            term: String::new(),
            ..PaneFindOutcome::default()
        });
    }
    let buffer = read_pane_text(engine, pane_id, "recent")?;
    let matches = crate::find::find_matches(&buffer.text, &request.term, &request.options)?;
    let total = matches.len();
    if total == 0 {
        return Ok(PaneFindOutcome {
            term: request.term.clone(),
            total: 0,
            index: 0,
            truncated: buffer.truncated,
            scroll: None,
        });
    }

    // Stepping wraps, because a search that stops at the end of the buffer
    // makes the reader guess whether there is more or they have gone round.
    let count = total as i64;
    let current = request.current_index as i64;
    let next = (current - 1 + request.step).rem_euclid(count);
    let target = matches[next as usize];

    let visible = read_pane_text(engine, pane_id, "visible")?;
    let viewport_rows = visible.text.lines().count();
    let scroll = crate::find::viewport_anchor(&buffer.text, &visible.text)
        .map(|top| crate::find::scroll_delta(target.line, top, viewport_rows))
        .filter(|delta| *delta != 0)
        .and_then(|delta| {
            let lines = u16::try_from(delta.unsigned_abs()).ok()?;
            Some((if delta > 0 { "up" } else { "down" }.to_owned(), lines))
        });

    Ok(PaneFindOutcome {
        term: request.term.clone(),
        total,
        index: next as usize + 1,
        truncated: buffer.truncated,
        scroll,
    })
}

fn read_pane_text(engine: &dyn Engine, pane_id: &str, source: &str) -> Result<PaneText, String> {
    engine
        .read_pane(CONTROL_WAIT, pane_id, source, PANE_FIND_LINE_LIMIT)
        .map_err(failed("pane.read"))
}
