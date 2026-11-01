//! Each pane's facts as its holder reports them, kept for a lane that asks
//! what a real agent's screen read (`rules`, `facts`, `wait_screen`): the
//! node gives a pane the built-in screen rules of an agent definition, as
//! the core's own engine does, and records every facts message the holder
//! sends while it runs.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use hide_node_link::holders::{PaneFacts, RuleSet, ScreenRead, ScreenRules, ScreenState};
use serde_json::{Value, json};

/// The most facts kept for one pane; the oldest go first.
const KEPT: usize = 4096;

/// Every pane's facts, in the order they came, and when.
#[derive(Default)]
pub struct Facts {
    started: Option<Instant>,
    panes: Mutex<BTreeMap<String, Vec<(u128, PaneFacts)>>>,
    changed: Condvar,
}

impl Facts {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            started: Some(Instant::now()),
            ..Self::default()
        })
    }

    /// Records `facts` of `pane`.
    pub fn told(&self, pane: &str, facts: &PaneFacts) {
        let at = self
            .started
            .map_or(0, |started| started.elapsed().as_millis());
        let mut panes = self.panes.lock().unwrap_or_else(PoisonError::into_inner);
        let kept = panes.entry(pane.to_owned()).or_default();
        if kept.len() >= KEPT {
            kept.remove(0);
        }
        kept.push((at, facts.clone()));
        self.changed.notify_all();
    }

    /// The pane's latest facts, and each change of what its screen read with
    /// when it came.
    pub fn summary(&self, pane: &str) -> Value {
        let panes = self.panes.lock().unwrap_or_else(PoisonError::into_inner);
        let kept = panes.get(pane).map(Vec::as_slice).unwrap_or_default();
        let mut screens: Vec<Value> = Vec::new();
        for (at, facts) in kept {
            let read = screen(&facts.screen);
            if screens.last().map(|last| &last["screen"]) != Some(&read) {
                screens.push(json!({"at_ms": at, "screen": read}));
            }
        }
        json!({
            "count": kept.len(),
            "latest": kept.last().map(|(_, facts)| latest(facts)),
            "screens": screens,
        })
    }

    /// Waits up to `limit` for the pane's latest facts to read `state`;
    /// whether they did.
    pub fn wait_screen(&self, pane: &str, state: &str, limit: Duration) -> bool {
        let panes = self.panes.lock().unwrap_or_else(PoisonError::into_inner);
        let (panes, _) = self
            .changed
            .wait_timeout_while(panes, limit, |panes| {
                panes
                    .get(pane)
                    .and_then(|kept| kept.last())
                    .is_none_or(|(_, facts)| screen(&facts.screen)["state"] != state)
            })
            .unwrap_or_else(PoisonError::into_inner);
        panes
            .get(pane)
            .and_then(|kept| kept.last())
            .is_some_and(|(_, facts)| screen(&facts.screen)["state"] == state)
    }
}

/// The built-in screen rules of the agent definition `agent`, as the core's
/// own engine hands them to a pane.
pub fn rules(agent: &str) -> Result<ScreenRules, String> {
    let definition = hide_agent_adapter::definition::builtin(agent)
        .ok_or_else(|| format!("no built-in agent definition is named {agent}"))?;
    let screen = definition
        .screen
        .as_ref()
        .ok_or_else(|| format!("the {agent} definition has no screen rules"))?;
    let manifest = serde_json::to_string(screen.rules.manifest())
        .map_err(|error| format!("the {agent} rules do not serialize: {error}"))?;
    Ok(ScreenRules {
        agent: definition.id.clone(),
        sets: vec![RuleSet {
            source: "built-in".to_owned(),
            version: screen.rules.version().to_string(),
            min_engine_version: screen.rules.min_engine_version(),
            manifest,
        }],
    })
}

fn screen(read: &ScreenRead) -> Value {
    match read {
        ScreenRead::NoRules => json!({"state": "no_rules"}),
        ScreenRead::Read { state, rule } => json!({
            "state": match state {
                ScreenState::Idle => "idle",
                ScreenState::Working => "working",
                ScreenState::Blocked => "blocked",
                ScreenState::Unknown => "unknown",
            },
            "rule": rule,
        }),
        ScreenRead::Unknown { reason } => json!({"state": "unreadable", "reason": reason}),
    }
}

fn latest(facts: &PaneFacts) -> Value {
    json!({
        "cwd": facts.cwd,
        "title": facts.title,
        "bells": facts.bells,
        "prompts": facts.prompts,
        "bracketed_paste": facts.bracketed_paste,
        "foreground": facts.foreground.as_ref().map(|program| json!({
            "pid": program.pid,
            "name": program.name,
            "argv": program.argv,
        })),
        "screen": screen(&facts.screen),
    })
}
