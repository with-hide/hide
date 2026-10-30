//! The agents that search their own conversation, and the keys that open it.
//!
//! A full-screen agent (Claude Code with `tui: fullscreen`, Codex on its
//! default alternate screen) keeps its conversation inside itself: Herdr holds
//! no history for the pane, so a search of what Herdr holds finds only the
//! rows on screen. The agent's own find covers the whole conversation and
//! scrolls to each match, so ⌘F hands such a pane to it and the agent's own
//! prompt, keys and count take over. An agent that draws inline leaves its
//! conversation in Herdr's history, and Hide's find bar searches that, so the
//! decision also asks Herdr whether it holds any (`crate::find::pane::spawn_agent_find`).
//!
//! The shared adapter selects a find dialect; this module owns its keys.
//! An agent without one keeps Hide's find bar.

/// How one agent's own search is opened, in Herdr's key names for
/// `pane.send_keys`. Every key is a mode switch in the agent: none of them
/// submits, interrupts or edits what the operator typed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AgentFind {
    /// The keys that open the search from the agent's prompt.
    pub(crate) open: &'static [&'static str],
    /// Set when the opening keys toggle a view the operator may still be in,
    /// so a second ⌘F would close it again.
    pub(crate) already_open: Option<AlreadyOpen>,
}

/// The agent's search view as its screen shows it. The agent reports its mode
/// nowhere else, so its own footer is the only witness; a footer that changed
/// in a later version sends the opening keys again, which is what a second ⌘F
/// did before this check existed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AlreadyOpen {
    /// Text only that view draws on screen.
    pub(crate) footer: &'static str,
    /// The keys that start a new search from inside that view.
    pub(crate) keys: &'static [&'static str],
}

/// `agent_kind`'s own search over its conversation, or `None` for an agent
/// Hide knows no such search for.
pub(crate) fn agent_find(agent_kind: &str) -> Option<AgentFind> {
    match hide_agent_adapter::adapter(agent_kind)?.find? {
        // Ctrl+O toggles the transcript, where `/` searches, `n` and `N` step
        // and the footer counts (Claude Code 2.1, fullscreen renderer).
        hide_agent_adapter::FindDialect::ClaudeTranscript => Some(AgentFind {
            open: &["ctrl+o", "/"],
            already_open: Some(AlreadyOpen {
                footer: "Showing detailed transcript",
                keys: &["/"],
            }),
        }),
        // F3 finds over the full transcript and stays open when pressed
        // again; Enter steps and Ctrl+P goes back (Codex 0.157).
        hide_agent_adapter::FindDialect::CodexTranscript => Some(AgentFind {
            open: &["f3"],
            already_open: None,
        }),
    }
}

impl AgentFind {
    /// The keys for a pane whose screen shows `visible`; `None` when the
    /// screen was not read because nothing here depends on it.
    pub(crate) fn keys(&self, visible: Option<&str>) -> &'static [&'static str] {
        match (self.already_open, visible) {
            (Some(view), Some(screen)) if screen.contains(view.footer) => view.keys,
            _ => self.open,
        }
    }
}
