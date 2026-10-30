//! Receipt-proven names, separate from spawn success and agent lifecycle.
//! The existing lineage writer owns reconnect repair; there is no new poller.
use super::SpawnRecord;
use crate::delivery::{
    ledger::Ledger,
    worker::{Client, Effect},
};
use crate::engine::{Engine, Wait};
use crate::session_sync::ProjectionState;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Pending,
    Applied,
    Preserved,
    Failed,
}
impl Outcome {
    fn finished(&self) -> bool {
        matches!(self, Self::Applied | Self::Preserved)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Names {
    pub workspace: Outcome,
    pub tab: Outcome,
}
impl Names {
    pub(crate) fn finished(&self) -> bool {
        self.workspace.finished() && self.tab.finished()
    }
}

/// The first tab's label as returned by a successful workspace creation.
/// Old receipts cannot manufacture this proof from a current default label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedTab {
    pub id: String,
    pub label: String,
}

/// Only the immutable durable placement is carried to the external worker.
#[derive(Clone, Debug)]
pub(crate) struct Work {
    pub id: String,
    path: String,
    pane: String,
    terminal: String,
    tab_name: String,
    branch: String,
    names: Names,
    seed_tab: Option<SeedTab>,
}
impl Work {
    pub(crate) fn of(record: &SpawnRecord) -> Option<Self> {
        let names = record.names.clone().unwrap_or_default();
        if names.finished() {
            return None;
        }
        Some(Self {
            id: record.id.clone(),
            path: record.path.clone()?,
            pane: record.pane.clone()?,
            terminal: record.terminal.clone()?,
            tab_name: record.name.clone(),
            branch: record.branch.clone(),
            names,
            seed_tab: record.seed_tab.clone(),
        })
    }
}

fn snapshot(engine: &dyn Engine) -> Result<ProjectionState, String> {
    engine
        .session(Wait::Each(Duration::from_secs(1)))
        .map_err(String::from)
}

/// No marker, no unique current identity, or another terminal means no rename.
fn target(state: &ProjectionState, work: &Work, workspace: bool) -> Option<(String, String, bool)> {
    let panes = state
        .panes
        .iter()
        .filter(|pane| pane.pane_id == work.pane)
        .collect::<Vec<_>>();
    let [pane] = panes.as_slice() else {
        return None;
    };
    if pane.terminal_id != work.terminal {
        return None;
    }
    let owners = state
        .workspaces
        .iter()
        .filter(|owner| owner.workspace_id == pane.workspace_id)
        .collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return None;
    };
    let checkout = owner.worktree.as_ref()?;
    if checkout.checkout_path != work.path {
        return None;
    }
    let marker = format!("hide:{}", work.id);
    if workspace {
        let name = if checkout.is_linked_worktree {
            work.branch.clone()
        } else {
            checkout.repo_name.clone()
        };
        if owner.label != marker {
            return Some((
                owner.workspace_id.clone(),
                name.clone(),
                owner.label == name,
            ));
        }
        if state
            .workspaces
            .iter()
            .filter(|owner| owner.label == marker)
            .count()
            != 1
        {
            return None;
        }
        Some((owner.workspace_id.clone(), name, false))
    } else {
        let tabs = state
            .tabs
            .iter()
            .filter(|tab| tab.tab_id == pane.tab_id && tab.workspace_id == pane.workspace_id)
            .collect::<Vec<_>>();
        let [tab] = tabs.as_slice() else {
            return None;
        };
        if tab.label != marker
            && !work
                .seed_tab
                .as_ref()
                .is_some_and(|seed| seed.id == tab.tab_id && seed.label == tab.label)
        {
            return Some((
                tab.tab_id.clone(),
                work.tab_name.clone(),
                tab.label == work.tab_name,
            ));
        }
        if tab.label == marker && state.tabs.iter().filter(|tab| tab.label == marker).count() != 1 {
            return None;
        }
        Some((tab.tab_id.clone(), work.tab_name.clone(), false))
    }
}

fn rename(engine: &dyn Engine, work: &Work, workspace: bool) -> Result<Outcome, String> {
    // Fresh read immediately before each target, including the second after a
    // partial success. Herdr has no rename CAS, so a check-to-write race remains.
    let state = snapshot(engine)?;
    let Some((id, name, already_named)) = target(&state, work, workspace) else {
        return Ok(Outcome::Preserved);
    };
    if already_named {
        return Ok(Outcome::Applied);
    }
    let marker = format!("hide:{}", work.id);
    let still_marked = if workspace {
        state
            .workspaces
            .iter()
            .any(|item| item.workspace_id == id && item.label == marker)
    } else {
        state.tabs.iter().any(|item| {
            item.tab_id == id
                && (item.label == marker
                    || work
                        .seed_tab
                        .as_ref()
                        .is_some_and(|seed| seed.id == item.tab_id && seed.label == item.label))
        })
    };
    if !still_marked {
        return Ok(Outcome::Preserved);
    }
    let wait = Wait::Each(Duration::from_secs(1));
    if workspace {
        engine.rename_workspace(wait, &id, &name)
    } else {
        engine.rename_tab(wait, &id, &name)
    }
    .map_err(String::from)?;
    Ok(Outcome::Applied)
}

pub(crate) fn run(engine: &dyn Engine, work: &Work) -> Names {
    let mut names = work.names.clone();
    for (workspace, result) in [(true, &mut names.workspace), (false, &mut names.tab)] {
        if result.finished() {
            continue;
        }
        *result = rename(engine, work, workspace).unwrap_or_else(|error| {
            crate::diagnostic!(serde_json::json!({"component":"spawn_naming", "kind":"rename_failed", "spawn_id":work.id, "target":if workspace {"workspace"} else {"tab"}, "detail":error}));
            Outcome::Failed
        });
    }
    names
}

pub(crate) fn save(client: &Client, work: Work, names: Names) -> Result<(), String> {
    client
        .submit(
            Effect::SpawnNames {
                work: Box::new(work),
                names,
            },
            Duration::from_secs(5),
        )
        .map(|_| ())
}

pub(crate) fn apply(ledger: &mut Ledger, work: &Work, names: &Names) -> Result<(), String> {
    let record = ledger
        .spawns
        .iter_mut()
        .find(|record| record.id == work.id)
        .ok_or("spawn_unavailable")?;
    if record.path.as_ref() != Some(&work.path)
        || record.pane.as_ref() != Some(&work.pane)
        || record.terminal.as_ref() != Some(&work.terminal)
    {
        return Err("spawn_identity_changed".into());
    }
    let previous = record.names.get_or_insert_with(Names::default);
    // A late failed attempt can never invalidate a durable successful target.
    if !previous.workspace.finished() {
        previous.workspace = names.workspace.clone();
    }
    if !previous.tab.finished() {
        previous.tab = names.tab.clone();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_herdr::FakeHerdr;
    use serde_json::{Value, json};

    fn work() -> Work {
        Work {
            id: "spawn-1".into(),
            path: "/fixture/task".into(),
            pane: "w1:p1".into(),
            terminal: "original".into(),
            tab_name: "검토 작업".into(),
            branch: "task-branch".into(),
            names: Names::default(),
            seed_tab: None,
        }
    }
    fn state() -> Value {
        json!({"type":"session_snapshot","snapshot":{
            "version":"fixture", "protocol":crate::engine::herdr::contract::HERDR_PROTOCOL_REVISION,
            "workspaces":[{"workspace_id":"w1","number":1,"label":"hide:spawn-1","focused":false,"pane_count":1,"tab_count":1,"active_tab_id":"w1:t1","agent_status":"idle",
                "worktree":{"repo_key":"fixture","repo_name":"project","repo_root":"/fixture","checkout_path":"/fixture/task","is_linked_worktree":true}}],
            "tabs":[{"workspace_id":"w1","tab_id":"w1:t1","number":1,"label":"hide:spawn-1","focused":false,"pane_count":1,"agent_status":"idle"}],
            "panes":[{"workspace_id":"w1","tab_id":"w1:t1","pane_id":"w1:p1","terminal_id":"original","focused":false,"agent_status":"idle","revision":1}],
            "layouts":[],"agents":[]
        }})
    }

    /// B8/B11: one refused name does not undo the other; the next attempt
    /// touches only the unfinished target and keeps external user names.
    #[test]
    fn partial_naming_retries_only_the_failed_target() {
        let mut state = state();
        let mut refused = false;
        let herdr =
            FakeHerdr::start_with_errors("names-partial", move |method, params| match method {
                "session.snapshot" => Ok(state.clone()),
                "workspace.rename" => {
                    state["snapshot"]["workspaces"][0]["label"] = params["label"].clone();
                    Ok(json!({"type":"ok"}))
                }
                "tab.rename" if !refused => {
                    refused = true;
                    Err(("fixture_refused".into(), "rename refused".into()))
                }
                "tab.rename" => {
                    state["snapshot"]["tabs"][0]["label"] = params["label"].clone();
                    Ok(json!({"type":"ok"}))
                }
                other => panic!("unexpected request {other}"),
            });
        let mut work = work();
        work.names = run(herdr.engine().as_ref(), &work);
        assert_eq!(
            work.names,
            Names {
                workspace: Outcome::Applied,
                tab: Outcome::Failed
            }
        );
        work.names = run(herdr.engine().as_ref(), &work);
        assert!(work.names.finished());
        let observed = snapshot(herdr.engine().as_ref()).unwrap();
        assert_eq!(observed.workspaces[0].label, "task-branch");
        assert_eq!(observed.tabs[0].label, "검토 작업");
        assert_eq!(
            herdr
                .methods()
                .iter()
                .filter(|method| *method == "workspace.rename")
                .count(),
            1
        );
        let calls = herdr.methods().len();
        assert_eq!(run(herdr.engine().as_ref(), &work), work.names);
        assert_eq!(
            herdr.methods().len(),
            calls,
            "finished targets perform no IO"
        );
        // The process can die after rename ACK and before its checkpoint.
        // The next connection observes the names and records completion
        // without repeating either mutation.
        work.names = Names::default();
        work.names = run(herdr.engine().as_ref(), &work);
        assert_eq!(
            work.names,
            Names {
                workspace: Outcome::Applied,
                tab: Outcome::Applied
            }
        );
        assert_eq!(
            &herdr.methods()[calls..],
            &["session.snapshot", "session.snapshot"]
        );
    }

    #[test]
    fn seed_tab_proof_names_only_the_unchanged_created_tab() {
        for (label, tab_id, terminal, allowed) in [
            ("1", "w1:t1", "original", true),
            ("User title", "w1:t1", "original", false),
            ("1", "w1:other", "original", false),
            ("1", "w1:t1", "replacement", false),
        ] {
            let mut state = state();
            state["snapshot"]["tabs"][0]["label"] = json!(label);
            state["snapshot"]["panes"][0]["terminal_id"] = json!(terminal);
            let herdr = FakeHerdr::start("seed-label", move |method, _| match method {
                "session.snapshot" => state.clone(),
                "tab.rename" => json!({"type":"ok"}),
                other => panic!("unexpected {other}"),
            });
            let mut work = work();
            work.names.workspace = Outcome::Applied;
            work.seed_tab = Some(SeedTab {
                id: tab_id.into(),
                label: "1".into(),
            });
            assert_eq!(
                run(herdr.engine().as_ref(), &work).tab,
                if allowed {
                    Outcome::Applied
                } else {
                    Outcome::Preserved
                }
            );
            assert_eq!(herdr.methods().contains(&"tab.rename".into()), allowed);
        }
    }

    /// B10: exact marker text is not authority without the saved terminal,
    /// checkout and unique live target. A user's name always survives.
    #[test]
    fn insufficient_or_changed_provenance_never_renames() {
        for change in ["terminal", "checkout", "user", "duplicate"] {
            let mut state = state();
            match change {
                "terminal" => state["snapshot"]["panes"][0]["terminal_id"] = json!("replacement"),
                "checkout" => {
                    state["snapshot"]["workspaces"][0]["worktree"]["checkout_path"] =
                        json!("/another")
                }
                "user" => {
                    state["snapshot"]["workspaces"][0]["label"] = json!("my project");
                    state["snapshot"]["tabs"][0]["label"] = json!("my tab");
                }
                "duplicate" => {
                    let mut other = state["snapshot"]["workspaces"][0].clone();
                    other["workspace_id"] = json!("w2");
                    state["snapshot"]["workspaces"]
                        .as_array_mut()
                        .unwrap()
                        .push(other);
                    let mut other = state["snapshot"]["tabs"][0].clone();
                    other["tab_id"] = json!("w1:t2");
                    state["snapshot"]["tabs"]
                        .as_array_mut()
                        .unwrap()
                        .push(other);
                }
                _ => unreachable!(),
            }
            let herdr = FakeHerdr::start_with_errors("names-preserved", move |method, _| {
                assert_eq!(
                    method, "session.snapshot",
                    "unproven target must never be renamed"
                );
                Ok(state.clone())
            });
            assert_eq!(
                run(herdr.engine().as_ref(), &work()),
                Names {
                    workspace: Outcome::Preserved,
                    tab: Outcome::Preserved
                },
                "{change}"
            );
        }
    }
}
