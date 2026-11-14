//! What the coordination ledger says Hide started itself (PRD
//! hide-ai-origin-mark D-01, D-07). A spawn is Hide's own when it is a
//! handoff whose origin is Hide's code-owned identity: a Factory's worker is
//! a delegation of a Factory identity, and a pane agent's handoff has a
//! pane's identity as its origin, so neither matches. Nothing here reads a
//! branch name or a path's spelling.

use std::collections::HashMap;

use super::{SpawnMode, SpawnTask};
use crate::delivery::ledger::Ledger;

/// One live agent Hide started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartedAgent {
    /// The coordination record's id, which a home's attempt names its agent by.
    pub(crate) agent_id: String,
    /// The pane in the core's own ids, and the session digest it was
    /// registered with, which a later agent in a reused pane does not share.
    pub(crate) pane_id: String,
    pub(crate) session: Option<String>,
    pub(crate) task: Option<SpawnTask>,
}

/// A checkout Hide made for a spawn, on the machine the child lives on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartedCheckout {
    pub(crate) machine: String,
    pub(crate) path: String,
    pub(crate) task: Option<SpawnTask>,
}

/// One live agent an external tool started (PRD hide-external-control
/// D-09): the tool's connection, by id, beside the agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccessStarted {
    pub(crate) connection: String,
    pub(crate) agent: StartedAgent,
}

/// One registration an external tool's work holds (PRD hide-external-control
/// D-09): an agent the tool started, or one started under such an agent with
/// `--parent` (B27), however deep. An ended one is kept, so a mission whose
/// Conductor ended stays the tool's (D-36).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolWork {
    pub(crate) connection: String,
    /// The name the tool connected with, as its identity in the ledger says.
    pub(crate) name: String,
    pub(crate) agent_id: String,
    pub(crate) pane_id: String,
    pub(crate) session: Option<String>,
    pub(crate) ended: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HideStarted {
    pub(crate) agents: Vec<StartedAgent>,
    pub(crate) checkouts: Vec<StartedCheckout>,
    /// What external tools started, apart from Hide AI's own: a tool's work
    /// is the tool's while its connection stands at Act, which the
    /// connection list says, not this ledger.
    pub(crate) access: Vec<AccessStarted>,
    /// Every registration of the tools' work, their starts' descendants
    /// included (`ToolWork`), which decides whose a row and a mission are.
    pub(crate) tool_work: Vec<ToolWork>,
}

/// The agents and checkouts Hide started, in ledger order. Compared by value
/// with the previous answer, so a ledger change that moves neither (a letter,
/// a watch) publishes nothing.
pub(crate) fn hide_started(ledger: &Ledger) -> HideStarted {
    let mut started = HideStarted::default();
    // Hide's own identities are a few records, found once; the ledger's
    // spawns are then filtered by a parent in that short list, so a publish
    // with no Hide AI start costs one pass over the agents and one over the spawns.
    // Desk's sessions are the operator's own work, which Desk starts on the
    // operator's word, not Hide AI's (PRD Desk 1차); an external tool's are
    // that tool's (PRD hide-external-control D-09), kept apart.
    let desk = format!(
        "{}{}",
        crate::delivery::HIDE_PREFIX,
        crate::delivery::HIDE_DESK_ROLE
    );
    let origins: Vec<(&str, Option<(&str, &str)>)> = ledger
        .agents
        .iter()
        .filter(|agent| agent.actor.hide_owned() && agent.actor.pane_id != desk)
        .map(|agent| {
            let tool = agent
                .actor
                .access_connection()
                .map(|connection| (connection, agent.actor.access_name().unwrap_or(connection)));
            (agent.id.as_str(), tool)
        })
        .collect();
    if origins.is_empty() {
        return started;
    }
    // The registrations the tools started, ended or not: their work's roots.
    let mut tool_roots: HashMap<&str, (&str, &str)> = HashMap::new();
    for spawn in ledger
        .spawns
        .iter()
        .filter(|spawn| spawn.mode == SpawnMode::Handoff)
    {
        let Some(&(_, tool)) = origins
            .iter()
            .find(|(origin, _)| *origin == spawn.parent.as_str())
        else {
            continue;
        };
        let connection = tool.map(|(connection, _)| connection);
        if let (Some(tool), Some(child)) = (tool, spawn.child.as_deref()) {
            tool_roots.insert(child, tool);
        }
        let machine = super::spawn_machine(ledger, spawn);
        if connection.is_none()
            && let (Some(path), Some(machine)) = (&spawn.path, machine)
        {
            started.checkouts.push(StartedCheckout {
                machine: machine.to_owned(),
                path: path.clone(),
                task: spawn.task.clone(),
            });
        }
        let Some(child) = spawn
            .child
            .as_ref()
            .and_then(|id| ledger.agents.iter().find(|agent| &agent.id == id))
            .filter(|agent| !agent.ended)
        else {
            continue;
        };
        let agent = StartedAgent {
            agent_id: child.id.clone(),
            pane_id: child.actor.pane_id.clone(),
            session: child.actor.session.clone(),
            task: spawn.task.clone(),
        };
        match connection {
            Some(connection) => started.access.push(AccessStarted {
                connection: connection.to_owned(),
                agent,
            }),
            None => started.agents.push(agent),
        }
    }
    started.tool_work = tool_work(ledger, tool_roots);
    started
}

/// Every registration of the tools' work: the roots they started and each
/// registration whose responsibility parent (`AgentRecord::parent`, set only
/// by a delegation) is already one. A child registers after its parent, so
/// one pass finds them all; a pass repeats only while it still adds one.
fn tool_work<'a>(
    ledger: &'a Ledger,
    mut owner: HashMap<&'a str, (&'a str, &'a str)>,
) -> Vec<ToolWork> {
    if owner.is_empty() {
        return Vec::new();
    }
    loop {
        let before = owner.len();
        for agent in &ledger.agents {
            if owner.contains_key(agent.id.as_str()) {
                continue;
            }
            if let Some(&tool) = agent.parent.as_deref().and_then(|parent| owner.get(parent)) {
                owner.insert(agent.id.as_str(), tool);
            }
        }
        if owner.len() == before {
            break;
        }
    }
    ledger
        .agents
        .iter()
        .filter_map(|agent| {
            let &(connection, name) = owner.get(agent.id.as_str())?;
            Some(ToolWork {
                connection: connection.to_owned(),
                name: name.to_owned(),
                agent_id: agent.id.clone(),
                pane_id: agent.actor.pane_id.clone(),
                session: agent.actor.session.clone(),
                ended: agent.ended,
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::coordination::{AgentRecord, Mutation, apply};
    use crate::delivery::Actor;

    const NODE: &str = "node-1";

    pub(crate) fn pane_on(machine: &str, name: &str, session: &str) -> Actor {
        Actor {
            pane_id: name.into(),
            name: name.into(),
            kind: "claude".into(),
            device_id: machine.into(),
            session: crate::engine::tokens::session_digest(session),
        }
    }

    fn pane(name: &str, session: &str) -> Actor {
        pane_on(NODE, name, session)
    }

    fn record(actor: &Actor) -> AgentRecord {
        AgentRecord {
            id: String::new(),
            name: Some(actor.name.clone()),
            machine: actor.device_id.clone(),
            host_scope: actor.kind.clone(),
            native_machine: actor.kind.clone(),
            session: actor.name.clone(),
            instance: actor.pane_id.clone(),
            pane: actor.pane_id.clone(),
            parent: None,
            origin: None,
            project: None,
            actor: actor.clone(),
            ended: false,
            terminal_id: None,
            forked: false,
        }
    }

    pub(crate) fn register(ledger: &mut Ledger, actor: &Actor) -> String {
        apply(
            ledger,
            actor,
            &Mutation::Register {
                record: record(actor),
                check: false,
            },
            1,
        )
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Where a spawn's child lives: its machine, its pane and the session
    /// its pane runs.
    pub(crate) struct Child<'a> {
        pub(crate) machine: &'a str,
        pub(crate) pane: &'a str,
        pub(crate) session: &'a str,
    }

    /// A completed spawn by `caller` (registered as `parent`): a handoff, or
    /// with `delegated` a delegation of `parent`. The child's checkout is
    /// `/repo.worktrees/<branch>`.
    pub(crate) fn spawn(
        ledger: &mut Ledger,
        caller: &Actor,
        parent: &str,
        delegated: bool,
        task: Option<SpawnTask>,
        branch: &str,
        child: Child,
    ) -> Result<String, String> {
        let command = crate::coordination::Command::Spawn {
            pick: None,
            mission: None,
            plan_node: None,
            plan: None,
            parent: delegated.then(|| parent.to_owned()),
            machine: (child.machine != caller.device_id).then(|| child.machine.to_owned()),
            name: format!("child-{branch}"),
            intent: format!("intent-{branch}"),
            kind: "claude".into(),
            repo: Some("/repo".into()),
            branch: Some(branch.into()),
            path: None,
            args: Vec::new(),
            launch: Default::default(),
            brief: None,
            task,
            beside: false,
            profile: None,
        };
        let reserved = apply(
            ledger,
            caller,
            &Mutation::Reserve {
                parent: parent.to_owned(),
                command,
            },
            1,
        )?;
        let id = reserved["id"].as_str().unwrap().to_owned();
        let mut actor = pane_on(child.machine, child.pane, child.session);
        actor.name = format!("child-{branch}");
        let mut child_record = record(&actor);
        let mode = if delegated {
            SpawnMode::Delegation
        } else {
            SpawnMode::Handoff
        };
        child_record.parent = mode.responsibility(parent);
        child_record.origin = mode.origin(parent);
        child_record.project = Some(format!("/repo.worktrees/{branch}"));
        apply(
            ledger,
            caller,
            &Mutation::Advance {
                seed_tab: None,
                id: id.clone(),
                path: Some(format!("/repo.worktrees/{branch}")),
                pane: Some(actor.pane_id.clone()),
                terminal: None,
                shell: None,
                child: None,
            },
            1,
        )?;
        apply(
            ledger,
            caller,
            &Mutation::BindChild {
                id: id.clone(),
                record: child_record,
            },
            1,
        )?;
        apply(ledger, caller, &Mutation::Complete { id: id.clone() }, 1)?;
        Ok(id)
    }

    pub(crate) fn memory_task(target: &str) -> Option<SpawnTask> {
        Some(SpawnTask {
            kind: crate::coordination::TaskKind::MemoryPromote,
            target: Some(target.into()),
        })
    }

    /// A ledger in which Hide, as `node`, started one agent for a card on
    /// `child`'s machine, in the checkout `/repo.worktrees/<branch>`.
    pub(crate) fn hide_started_ledger(
        node: &str,
        branch: &str,
        task: Option<SpawnTask>,
        child: Child,
    ) -> Ledger {
        let mut ledger = Ledger::default();
        let hide = Actor::hide("memory", node);
        let origin = register(&mut ledger, &hide);
        spawn(&mut ledger, &hide, &origin, false, task, branch, child).unwrap();
        ledger
    }

    #[test]
    fn a_session_desk_started_is_not_hide_ai_work() {
        let mut ledger = Ledger::default();
        let desk = Actor::hide("desk", NODE);
        let origin = register(&mut ledger, &desk);
        spawn(
            &mut ledger,
            &desk,
            &origin,
            false,
            None,
            "desk/fix",
            here("w1:p1"),
        )
        .unwrap();
        assert_eq!(hide_started(&ledger), HideStarted::default());
    }

    #[test]
    fn a_session_a_tool_started_names_its_connection_and_is_not_hide_ai_work() {
        let mut ledger = Ledger::default();
        let tool = Actor::access("c0ffee00c0ffee00", "hermes", NODE);
        let origin = register(&mut ledger, &tool);
        spawn(
            &mut ledger,
            &tool,
            &origin,
            false,
            None,
            "hermes/fix",
            here("w1:p1"),
        )
        .unwrap();
        let started = hide_started(&ledger);
        assert!(started.agents.is_empty());
        assert!(started.checkouts.is_empty());
        assert_eq!(started.access.len(), 1);
        assert_eq!(started.access[0].connection, "c0ffee00c0ffee00");
        assert_eq!(started.access[0].agent.pane_id, "w1:p1");
        assert_eq!(started.tool_work.len(), 1);
        assert_eq!(started.tool_work[0].name, "hermes");
    }

    /// B27, D-36: what a tool's agent starts under itself with `--parent`,
    /// and what that starts in turn, is the tool's work too; a handoff the
    /// agent makes without `--parent` is the operator's own, and an ended
    /// Conductor's registration stays the tool's so its mission does.
    #[test]
    fn a_tools_work_holds_what_its_agent_delegates_however_deep_and_ends_nothing() {
        let mut ledger = Ledger::default();
        let tool = Actor::access("c0ffee00c0ffee00", "hermes", NODE);
        let origin = register(&mut ledger, &tool);
        spawn(
            &mut ledger,
            &tool,
            &origin,
            false,
            None,
            "conductor",
            here("w1:p1"),
        )
        .unwrap();
        let conductor_id = ledger.agents.last().unwrap().id.clone();
        let mut conductor = pane("w1:p1", "session");
        conductor.name = "child-conductor".into();
        let child = |pane: &'static str| Child {
            machine: NODE,
            pane,
            session: "session",
        };
        spawn(
            &mut ledger,
            &conductor,
            &conductor_id,
            true,
            None,
            "worker",
            child("w1:p2"),
        )
        .unwrap();
        let worker_id = ledger.agents.last().unwrap().id.clone();
        let mut worker = pane("w1:p2", "session");
        worker.name = "child-worker".into();
        spawn(
            &mut ledger,
            &worker,
            &worker_id,
            true,
            None,
            "reviewer",
            child("w1:p3"),
        )
        .unwrap();
        spawn(
            &mut ledger,
            &conductor,
            &conductor_id,
            false,
            None,
            "handed",
            child("w1:p4"),
        )
        .unwrap();
        let work = |ledger: &Ledger| {
            hide_started(ledger)
                .tool_work
                .into_iter()
                .map(|work| (work.pane_id, work.connection, work.ended))
                .collect::<Vec<_>>()
        };
        let held = |pane: &str, ended| (pane.to_owned(), "c0ffee00c0ffee00".to_owned(), ended);
        assert_eq!(
            work(&ledger),
            [
                held("w1:p1", false),
                held("w1:p2", false),
                held("w1:p3", false)
            ]
        );
        for agent in &mut ledger.agents {
            if agent.id == conductor_id {
                agent.ended = true;
            }
        }
        assert_eq!(
            work(&ledger),
            [
                held("w1:p1", true),
                held("w1:p2", false),
                held("w1:p3", false)
            ]
        );
        assert!(
            hide_started(&ledger).access.is_empty(),
            "the live starts the settings count leave the ended Conductor"
        );
    }

    fn here(pane: &str) -> Child<'_> {
        Child {
            machine: NODE,
            pane,
            session: "session",
        }
    }

    #[test]
    fn an_agent_and_checkout_hide_started_carry_the_task() {
        let ledger = hide_started_ledger(NODE, "hide-memory-m72", memory_task("m72"), here("p72"));
        let started = hide_started(&ledger);
        assert_eq!(started.agents.len(), 1);
        assert_eq!(started.agents[0].task, memory_task("m72"));
        assert_eq!(started.agents[0].pane_id, "p72");
        assert_eq!(
            started.checkouts,
            [StartedCheckout {
                machine: NODE.into(),
                path: "/repo.worktrees/hide-memory-m72".into(),
                task: memory_task("m72"),
            }]
        );
    }

    /// D-10, D-19, B20, B22: a known kind has its home and writes its code
    /// unchanged; a kind another build wrote loads, keeps its value, and has
    /// no home, so its agent alone stays an operator's row.
    #[test]
    fn a_ledger_with_a_kind_this_build_does_not_know_still_loads() {
        let ledger = hide_started_ledger(NODE, "hide-memory-m72", memory_task("m72"), here("p72"));
        let written = serde_json::to_string(&ledger).unwrap();
        assert!(
            written.contains("\"kind\":\"memory_promote\""),
            "the code is unchanged"
        );
        for code in ["future_kind", ""] {
            let other = written.replace(
                "\"kind\":\"memory_promote\"",
                &format!("\"kind\":\"{code}\""),
            );
            let loaded: Ledger = serde_json::from_str(&other).unwrap();
            let started = hide_started(&loaded);
            assert_eq!(started.agents.len(), 1, "{code:?}");
            let task = started.agents[0].task.as_ref().unwrap();
            assert_eq!(task.kind.as_str(), code);
            assert_eq!(task.kind.home(), None, "{code:?}");
            assert_eq!(
                serde_json::to_string(&loaded).unwrap(),
                other,
                "kept as written"
            );
        }
        let known = hide_started(&ledger);
        assert_eq!(
            known.agents[0].task.as_ref().unwrap().kind.home(),
            Some(crate::coordination::TaskHome::Suggestions)
        );
    }

    /// A new start needs a home to show its progress in.
    #[test]
    fn a_new_start_of_a_kind_with_no_home_is_refused() {
        let mut ledger = Ledger::default();
        let hide = Actor::hide("memory", NODE);
        let origin = register(&mut ledger, &hide);
        let task = Some(SpawnTask {
            kind: crate::coordination::TaskKind::from_code("future_kind"),
            target: None,
        });
        let refused = spawn(&mut ledger, &hide, &origin, false, task, "b", here("p1"));
        assert_eq!(refused.unwrap_err(), "task_kind_unknown");
    }

    #[test]
    fn a_hide_start_with_no_task_is_still_hide_ais() {
        let ledger = hide_started_ledger(NODE, "any-name", None, here("p1"));
        let started = hide_started(&ledger);
        assert_eq!(started.agents.len(), 1);
        assert_eq!(started.agents[0].task, None);
    }

    #[test]
    fn a_pane_agents_handoff_a_delegation_and_a_branch_name_are_not_hide_ais() {
        let mut ledger = Ledger::default();
        let operator = pane("operator-pane", "operator");
        let operator_id = register(&mut ledger, &operator);
        // The branch spells Hide AI's naming; the record does not say it.
        spawn(
            &mut ledger,
            &operator,
            &operator_id,
            false,
            None,
            "hide-memory-m1-x",
            here("p1"),
        )
        .unwrap();
        spawn(
            &mut ledger,
            &operator,
            &operator_id,
            true,
            None,
            "delegated",
            here("p2"),
        )
        .unwrap();
        assert_eq!(hide_started(&ledger), HideStarted::default());
    }

    #[test]
    fn an_ended_agent_leaves_its_checkout_in_place() {
        let mut ledger =
            hide_started_ledger(NODE, "hide-memory-m43", memory_task("m43"), here("p43"));
        for agent in &mut ledger.agents {
            if agent.origin.is_some() {
                agent.ended = true;
            }
        }
        let started = hide_started(&ledger);
        assert!(started.agents.is_empty());
        assert_eq!(started.checkouts.len(), 1);
    }

    #[test]
    fn only_hides_own_identity_may_give_a_task() {
        let mut ledger = Ledger::default();
        let operator = pane("operator-pane", "operator");
        let operator_id = register(&mut ledger, &operator);
        let error = spawn(
            &mut ledger,
            &operator,
            &operator_id,
            false,
            memory_task("m1"),
            "x",
            here("p1"),
        )
        .unwrap_err();
        assert_eq!(error, "task_reserved");
        let hide = Actor::hide("memory", NODE);
        let origin = register(&mut ledger, &hide);
        let error = spawn(
            &mut ledger,
            &hide,
            &origin,
            true,
            memory_task("m1"),
            "y",
            here("p2"),
        )
        .unwrap_err();
        assert_eq!(error, "task_reserved");
        // An external tool's identity is code-owned with a hide: pane id,
        // but it is not Hide itself (PRD hide-external-control B23).
        let tool = Actor::access("00000000000000a1", "nightly", NODE);
        let origin = register(&mut ledger, &tool);
        let error = spawn(
            &mut ledger,
            &tool,
            &origin,
            false,
            memory_task("m1"),
            "z",
            here("p3"),
        )
        .unwrap_err();
        assert_eq!(error, "task_reserved");
    }

    /// A start reserved by a build that kept no task, asked again by one that
    /// does, is the same spawn and takes the task up; a different task is not.
    #[test]
    fn a_spawn_reserved_without_a_task_takes_the_same_intents_task_up() {
        let mut ledger = hide_started_ledger(NODE, "hide-memory-m72", None, here("p72"));
        let hide = Actor::hide("memory", NODE);
        let origin = ledger.agents[0].id.clone();
        let again = |task| Mutation::Reserve {
            parent: origin.clone(),
            command: crate::coordination::Command::Spawn {
                pick: None,
                mission: None,
                plan_node: None,
                plan: None,
                parent: None,
                machine: None,
                name: "child-hide-memory-m72".into(),
                intent: "intent-hide-memory-m72".into(),
                kind: "claude".into(),
                repo: Some("/repo".into()),
                branch: Some("hide-memory-m72".into()),
                path: None,
                args: Vec::new(),
                launch: Default::default(),
                brief: None,
                task,
                beside: false,
                profile: None,
            },
        };
        apply(&mut ledger, &hide, &again(memory_task("m72")), 1).unwrap();
        assert_eq!(ledger.spawns.len(), 1);
        assert_eq!(ledger.spawns[0].task, memory_task("m72"));
        assert_eq!(
            apply(&mut ledger, &hide, &again(memory_task("m73")), 1).unwrap_err(),
            "intent_conflict"
        );
        assert_eq!(ledger.spawns[0].task, memory_task("m72"));
    }
}
