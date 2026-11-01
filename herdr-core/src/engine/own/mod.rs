//! The core's own engine (PRD engine-bundle-3-core-owns D-3): the second
//! implementation of [`Engine`], which runs a machine's panes without Herdr.
//! It holds its session's topology ([`topology::Topology`]) under a lock of
//! its own, never the runtime's, and asks the machine's node to start, end,
//! read and type into each pane's holder through [`PaneHolders`], always
//! outside that lock. A change to the topology wakes whoever watches it
//! ([`Engine::changes`]), and the watcher reads the whole session again
//! ([`Engine::session`]); there is no event stream to replay.
//!
//! This is the skeleton the later bundle 3 pull requests fill (D-25): it
//! makes workspaces, tabs and panes, splits and closes them, keeps focus,
//! names and Hide's tokens, and reads, types into and lists the processes of
//! a pane. What a later pull request brings (zoom, resize, swap and move,
//! saved layouts, agent starts and their state, pane identity, worktrees)
//! answers a refusal under [`UNSUPPORTED`] that names it, never a silent
//! default. Only a daemon with the development switch `HIDE_DEV_PANE_HOLDER`
//! on builds this engine (D-5).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use hide_node_link::holders::{HolderError, HolderStart, PaneHolders};

use super::keys::PaneKey;
use super::{
    AgentLaunch, AgentState, Answer, Changes, CreatedWorktree, DeliveryAgent, Engine, EngineError,
    Env, EventStream, Gate, Notified, OpenedWorktree, PaneJoinResult, PanePlace, PaneProcessGroup,
    PaneProof, PaneSession, PaneText, PreparedStart, ServerPong, StartedAgent, Wait, Wake, Zoom,
};
use crate::model::PaneLayoutDirection;
use crate::recent_closed::{ClosedLayout, ClosedLayoutNode, ClosedSplitDirection};
use crate::session_sync::{ProjectedAgent, ProjectionState};
use crate::sidebar::{SessionLayoutPayload, SessionSnapshotPayload};

mod ids;
pub(crate) mod layout;
mod topology;

use topology::{Closed, Pane, Topology};

/// The refusal code of a call this engine does not answer yet.
pub(crate) const UNSUPPORTED: &str = "own_engine_unsupported";

/// The size a pane starts at before a screen draws it; the first screen
/// that attaches it resizes it to the rectangle drawn for it.
const START_ROWS: u16 = 24;
const START_COLS: u16 = 80;

/// The most of a pane's history a read asks its holder for.
const READ_LINES_CAP: u32 = u16::MAX as u32;

/// The engine and whatever it holds, shared by every gated view of it.
struct Shared {
    topology: Mutex<Topology>,
    holders: Arc<dyn PaneHolders>,
    wake: Mutex<Option<Wake>>,
    stopped: AtomicBool,
}

/// The own engine, with every call first passing `gates`.
pub(crate) struct OwnEngine<'g> {
    shared: Arc<Shared>,
    gates: Vec<Gate<'g>>,
}

impl OwnEngine<'static> {
    /// An engine with no workspace, whose panes `holders` run.
    pub(crate) fn new(holders: Arc<dyn PaneHolders>) -> Self {
        Self {
            shared: Arc::new(Shared {
                topology: Mutex::default(),
                holders,
                wake: Mutex::default(),
                stopped: AtomicBool::new(false),
            }),
            gates: Vec::new(),
        }
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

    fn topology(&self) -> MutexGuard<'_, Topology> {
        self.shared
            .topology
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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

    /// Tells the watcher the session changed, once per change.
    fn changed(&self) {
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

    /// Starts `pane`'s holder running `program`, or the machine's shell.
    fn start_pane(
        &self,
        pane: &str,
        program: Option<&[String]>,
        cwd: &str,
        env: Env,
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
            })
            .map_err(holder_error)
    }

    /// Ends the holders of `panes`, every one asked even when one fails:
    /// the first failure, if any. A holder already gone is ended.
    fn end_panes(&self, panes: &[String]) -> Result<(), EngineError> {
        let mut first = None;
        for pane in panes {
            match self.shared.holders.terminate(pane) {
                Ok(()) | Err(HolderError::NoPane(_)) => {}
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
        self.start_pane(pane, program, cwd, env)?;
        let added = self.topology().add_tab(
            workspace,
            (tab, pane),
            label,
            Pane::new(cwd, pane_label),
            focus,
        );
        if let Err(error) = added {
            // The workspace closed while the pane started.
            let _ = self.end_panes(std::slice::from_ref(pane));
            return Err(error);
        }
        self.changed();
        if let Some(replace) = replace {
            self.close_tab_now(replace)?;
        }
        Ok(reserved)
    }

    /// A new pane beside `target`: its id.
    fn open_split(
        &self,
        target: &str,
        direction: PaneLayoutDirection,
        cwd: Option<&str>,
        env: Env,
        focus: bool,
    ) -> Answer<String> {
        let (pane, cwd) = {
            let mut topology = self.topology();
            let cwd = match cwd.filter(|cwd| !cwd.trim().is_empty()) {
                Some(cwd) => cwd.to_owned(),
                None => topology.pane(target)?.cwd.clone(),
            };
            let (workspace, _) = topology.pane_place(target)?;
            (topology.reserve_pane(&workspace)?, cwd)
        };
        self.start_pane(&pane, None, &cwd, env)?;
        let added =
            self.topology()
                .add_split(target, direction, &pane, Pane::new(&cwd, None), focus);
        if let Err(error) = added {
            // The pane beside it closed while this one started.
            let _ = self.end_panes(std::slice::from_ref(&pane));
            return Err(error);
        }
        self.changed();
        Ok(pane)
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

    /// The pane, refused when the engine holds no such pane.
    fn known(&self, pane: &str) -> Result<(), EngineError> {
        self.topology().pane(pane).map(|_| ())
    }

    fn type_bytes(&self, pane: &str, bytes: Vec<u8>) -> Answer<()> {
        self.known(pane)?;
        self.shared.holders.input(pane, bytes).map_err(holder_error)
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
        HolderError::NoPane(message) => EngineError::Refused {
            code: "pane_not_found".to_owned(),
            message,
        },
        HolderError::Refused { code, message } => EngineError::Refused {
            code: code.to_owned(),
            message,
        },
        HolderError::Unreachable(message) => EngineError::Transport(message),
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

const LAYOUT_PR: &str = "layout pull request";
const AGENTS_PR: &str = "agent start pull request";
const IDENTITY_PR: &str = "pane identity pull request";

/// The bytes a key sends to a program that has asked for no key mode.
fn key_bytes(key: PaneKey) -> &'static [u8] {
    match key {
        PaneKey::Esc => b"\x1b",
        PaneKey::Enter => b"\r",
        PaneKey::Tab => b"\t",
        PaneKey::ShiftTab => b"\x1b[Z",
        PaneKey::Up => b"\x1b[A",
        PaneKey::Down => b"\x1b[B",
        PaneKey::Right => b"\x1b[C",
        PaneKey::Left => b"\x1b[D",
        PaneKey::Backspace => b"\x7f",
        PaneKey::CtrlC => b"\x03",
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

    fn session(&self, _wait: Wait) -> Answer<ProjectionState> {
        self.check()?;
        self.topology().state()
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

    fn agents(&self, _wait: Wait) -> Answer<Vec<ProjectedAgent>> {
        // No agent is started on this engine before the agent start pull
        // request, so it has none to list.
        self.check()?;
        Ok(Vec::new())
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
        let reserved = self.topology().reserve_workspace();
        self.start_pane(&reserved.pane, None, cwd, env)?;
        self.topology()
            .add_workspace(&reserved, label, Pane::new(cwd, None), focus);
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

    fn export_layout(&self, _wait: Wait, _tab: &str) -> Answer<ClosedLayout> {
        self.check()?;
        unsupported("a layout export", LAYOUT_PR)
    }

    fn apply_layout(
        &self,
        _wait: Wait,
        _workspace: &str,
        _tab: Option<&str>,
        _tab_label: Option<&str>,
        _root: &ClosedLayoutNode,
    ) -> Answer<ClosedLayout> {
        self.check()?;
        unsupported("a saved layout", LAYOUT_PR)
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
        self.open_split(pane, direction, cwd, Env::new(), true)
    }

    fn split_pane_at(
        &self,
        _wait: Wait,
        _pane: &str,
        _direction: ClosedSplitDirection,
        _cwd: &str,
        _ratio: f32,
        _env: Env,
    ) -> Answer<String> {
        self.check()?;
        unsupported("a split at a ratio", LAYOUT_PR)
    }

    fn split_beside(
        &self,
        _wait: Wait,
        parent: &str,
        cwd: Option<&str>,
        env: Env,
    ) -> Answer<String> {
        self.check()?;
        self.open_split(parent, PaneLayoutDirection::Right, cwd, env, false)
    }

    fn split_replacement(&self, _wait: Wait, pane: &str, cwd: &str, env: Env) -> Answer<String> {
        self.check()?;
        self.open_split(pane, PaneLayoutDirection::Right, Some(cwd), env, false)
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

    fn toggle_zoom(&self, _wait: Wait, _pane: &str) -> Answer<bool> {
        self.check()?;
        unsupported("a zoom", LAYOUT_PR)
    }

    fn set_zoom(&self, _wait: Wait, _pane: &str, _zoom: Zoom) -> Answer<()> {
        self.check()?;
        unsupported("a zoom", LAYOUT_PR)
    }

    fn resize_pane(
        &self,
        _wait: Wait,
        _pane: &str,
        _direction: crate::live::PaneResizeDirection,
        _amount: f32,
    ) -> Answer<bool> {
        self.check()?;
        unsupported("a resize", LAYOUT_PR)
    }

    fn swap_panes(&self, _wait: Wait, _first: &str, _second: &str) -> Answer<()> {
        self.check()?;
        unsupported("a swap", LAYOUT_PR)
    }

    fn move_pane_to_new_tab(
        &self,
        _wait: Wait,
        _pane: &str,
        _workspace: &str,
        _label: &str,
    ) -> Answer<PaneJoinResult> {
        self.check()?;
        unsupported("a pane move", LAYOUT_PR)
    }

    fn move_pane_beside(
        &self,
        _wait: Wait,
        _pane: &str,
        _target_tab: &str,
        _target_pane: &str,
        _down: bool,
    ) -> Answer<PaneJoinResult> {
        self.check()?;
        unsupported("a pane move", LAYOUT_PR)
    }

    fn pane_session(
        &self,
        _wait: Wait,
        pane: &str,
    ) -> Answer<(Option<String>, Option<PaneSession>)> {
        self.check()?;
        // No agent reports a session to this engine before the agent start
        // pull request.
        Ok((Some(self.topology().pane(pane)?.cwd.clone()), None))
    }

    fn pane_terminal(&self, _wait: Wait, pane: &str) -> Answer<String> {
        self.check()?;
        // One terminal per pane until a restore starts a pane again.
        self.known(pane)?;
        Ok(pane.to_owned())
    }

    fn pane_token(&self, _wait: Wait, pane: &str, token: &str) -> Answer<Option<String>> {
        self.check()?;
        self.topology().pane_token(pane, token)
    }

    fn pane_agent_status(&self, _wait: Wait, pane: &str) -> Answer<String> {
        self.check()?;
        self.known(pane)?;
        Ok("unknown".to_owned())
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
        unsupported("a scroll", LAYOUT_PR)
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
        _pane: &str,
        _kind: &str,
        _session: &str,
        _seq: u64,
    ) -> Answer<()> {
        self.check()?;
        unsupported("an agent's session", AGENTS_PR)
    }

    fn read_pane(&self, _wait: Wait, pane: &str, _source: &str, lines: u32) -> Answer<PaneText> {
        self.check()?;
        self.known(pane)?;
        let lines = u16::try_from(lines.min(READ_LINES_CAP)).unwrap_or(u16::MAX);
        let (text, truncated) = self
            .shared
            .holders
            .read(pane, lines)
            .map_err(holder_error)?;
        Ok(PaneText { text, truncated })
    }

    fn read_screen(&self, _wait: Wait, _pane: &str) -> Answer<PaneText> {
        self.check()?;
        unsupported("a styled screen read", LAYOUT_PR)
    }

    fn read_rows(&self, _wait: Wait, _pane: &str, _lines: u32) -> Answer<PaneText> {
        self.check()?;
        unsupported("a read of drawn rows", LAYOUT_PR)
    }

    fn send_text(&self, _wait: Wait, pane: &str, text: &str) -> Answer<()> {
        self.check()?;
        self.type_bytes(pane, text.as_bytes().to_vec())
    }

    fn send_keys(&self, _wait: Wait, pane: &str, keys: &[&str]) -> Answer<()> {
        self.check()?;
        let mut bytes = Vec::new();
        for key in keys {
            bytes.extend_from_slice(key_bytes(named_key(key)?));
        }
        self.type_bytes(pane, bytes)
    }

    fn submit_text(&self, _wait: Wait, pane: &str, text: &str) -> Answer<()> {
        self.check()?;
        let mut bytes = text.as_bytes().to_vec();
        bytes.extend_from_slice(key_bytes(PaneKey::Enter));
        self.type_bytes(pane, bytes)
    }

    fn press_key(&self, _wait: Wait, pane: &str, key: &str) -> Answer<()> {
        self.check()?;
        self.type_bytes(pane, key_bytes(named_key(key)?).to_vec())
    }

    fn press_pane_keys(&self, _wait: Wait, pane: &str, keys: &[PaneKey]) -> Answer<()> {
        self.check()?;
        let bytes = keys
            .iter()
            .flat_map(|key| key_bytes(*key).iter().copied())
            .collect();
        self.type_bytes(pane, bytes)
    }

    fn paste_text(&self, _wait: Wait, _pane: &str, _text: &str) -> Answer<()> {
        self.check()?;
        unsupported("a paste", IDENTITY_PR)
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
        _pane: &str,
        _agent: hide_session::Agent,
    ) -> Answer<Option<(String, String)>> {
        self.check()?;
        unsupported("a resumed session", AGENTS_PR)
    }

    fn current_pane(&self, _wait: Wait, pane: &str) -> Answer<String> {
        self.check()?;
        self.known(pane)?;
        Ok(pane.to_owned())
    }

    fn pane_identity(&self, _wait: Wait, _pane: &str) -> Answer<PaneProof> {
        self.check()?;
        unsupported("a pane's proof", IDENTITY_PR)
    }

    fn prepare_start(&self, _launch: AgentLaunch) -> Result<PreparedStart, String> {
        Err(format!(
            "the core's own engine starts no agent yet; the {AGENTS_PR} brings it"
        ))
    }

    fn start_agent(&self, _wait: Wait, _start: &PreparedStart) -> Answer<StartedAgent> {
        self.check()?;
        unsupported("an agent start", AGENTS_PR)
    }

    fn agent_state(&self, _wait: Wait, _target: &str) -> Answer<AgentState> {
        self.check()?;
        unsupported("an agent's state", AGENTS_PR)
    }

    fn delivery_agent(&self, _wait: Wait, _target: &str) -> Answer<DeliveryAgent> {
        self.check()?;
        unsupported("a letter's agent", AGENTS_PR)
    }

    fn observed_agent(
        &self,
        _wait: Wait,
        _target: &str,
    ) -> Answer<(
        DeliveryAgent,
        Option<hide_agent_hooks::observation::ObservationNativeSession>,
    )> {
        self.check()?;
        unsupported("a letter's agent", AGENTS_PR)
    }

    fn prompt_agent(&self, _wait: Wait, _target: &str, _text: &str) -> Answer<()> {
        self.check()?;
        unsupported("an agent prompt", AGENTS_PR)
    }

    fn create_worktree(
        &self,
        _wait: Wait,
        _cwd: &str,
        _branch: &str,
        _base: Option<&str>,
        _label: Option<&str>,
        _path: Option<&str>,
        _focus: bool,
    ) -> Answer<CreatedWorktree> {
        self.check()?;
        unsupported("a worktree", LAYOUT_PR)
    }

    fn worktree_path(&self, _wait: Wait, _cwd: &str, _branch: &str) -> Answer<Option<String>> {
        self.check()?;
        unsupported("a worktree", LAYOUT_PR)
    }

    fn open_worktree(
        &self,
        _wait: Wait,
        _path: &str,
        _repository_root: &str,
        _focus: bool,
    ) -> Answer<OpenedWorktree> {
        self.check()?;
        unsupported("a worktree", LAYOUT_PR)
    }

    fn remove_worktree(&self, _wait: Wait, _workspace: &str) -> Answer<()> {
        self.check()?;
        unsupported("a worktree", LAYOUT_PR)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use hide_node_link::holders::HolderProcesses;

    use super::*;

    /// Holders that start every pane until told to refuse, as a node whose
    /// holder program cannot start refuses.
    #[derive(Default)]
    struct Holders {
        refuse: AtomicBool,
    }

    impl PaneHolders for Holders {
        fn start(&self, start: &HolderStart) -> Result<(), HolderError> {
            if self.refuse.load(Ordering::Acquire) {
                return Err(HolderError::Refused {
                    code: "holder_start_failed",
                    message: format!("pane {}: the holder program is missing", start.pane),
                });
            }
            Ok(())
        }
        fn terminate(&self, _: &str) -> Result<(), HolderError> {
            Ok(())
        }
        fn read(&self, _: &str, _: u16) -> Result<(String, bool), HolderError> {
            Ok((String::new(), false))
        }
        fn processes(&self, _: &str) -> Result<HolderProcesses, HolderError> {
            Err(HolderError::Unreachable("not asked".to_owned()))
        }
        fn input(&self, _: &str, _: Vec<u8>) -> Result<(), HolderError> {
            Ok(())
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
}
