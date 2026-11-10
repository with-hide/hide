//! A Conductor's worker starts with the agent item its admission judged
//! (PRD agent-profiles-phase2 D-08, B13): the core picks the item once,
//! under its lock, and the start runs that same item, so a Project
//! settings change between the two cannot start what was not admitted.
//! The same spawn asked again once it has finished answers its worker
//! without judging the item again, as a retry of the same intent does
//! (engineering principle 11), while asking it with other arguments is
//! still a conflict.
//!
//! The spawn runs through the core's delivery path (`prepare_delivery`,
//! then `Prepared::run`) on this machine's fake engine, beside its caller.

use super::beside_spawn::{LOCAL_CHECKOUT, Spawner, local_spawner};
use super::*;
use crate::delivery::Command;
use crate::missions::{Candidate, MissionCommand};
use crate::runtime::delivery::tests::authority;
use serde_json::{Value, json};

/// This machine's kit with Codex on, its built-in default account's hook
/// in place, and one Codex profile, `work` (`p-work`), whose hook is in
/// place too and whose account folder the fake node answers for.
fn work_listed(runtime: &mut Runtime) {
    let node = runtime.node.to_string();
    let mut kit = runtime.kit_state(&node);
    let installed = crate::model::KitPieceSnapshot {
        state: hide_kit::ComponentState::Installed,
        why: None,
        location: None,
    };
    kit.agents = vec![crate::model::KitAgentSnapshot {
        id: "codex".into(),
        label: "Codex".into(),
        availability: hide_kit::Availability::Available,
        enabled: true,
        chosen: false,
        skill: installed.clone(),
        hook: Some(installed.clone()),
        partial: false,
        features: Vec::new(),
        sessions: None,
        update: None,
        doc_url: String::new(),
    }];
    kit.profiles = Some(crate::model::ProfilesSnapshot {
        state: crate::model::ProfilesState::Listed,
        version: 1,
        profiles: vec![crate::model::ProfileSnapshot {
            id: "p-work".into(),
            kind: "codex".into(),
            name: "work".into(),
            account: Some("/accounts/p-work".into()),
            program: None,
            env_keys: Vec::new(),
            version: 1,
            install: None,
            folder: None,
            hook: Some(installed),
            skill: None,
            login: None,
            wrapper_folder: false,
            startable: true,
            account_key: Some(hide_agent_adapter::AccountKey::Folder(
                "/accounts/p-work".into(),
            )),
            account_variables: Vec::new(),
        }],
        defaults: Default::default(),
    });
    runtime.kit_states.insert(node, kit);
}

/// The project at the caller's checkout starts Codex with `profile`.
fn project_picks(runtime: &mut Runtime, profile: &str) {
    let node = runtime.node.to_string();
    let mut record = crate::project_settings::ProjectSettings::new(&node, LOCAL_CHECKOUT);
    record
        .agent_profiles
        .insert("codex".to_owned(), profile.to_owned());
    runtime
        .snapshot
        .ui_state
        .project_settings
        .insert(crate::project_settings::key(&node, LOCAL_CHECKOUT), record);
}

/// A Codex worker candidate of the built-in default (`None`) or a profile.
fn codex(profile: Option<&str>) -> Candidate {
    Candidate {
        agent: "codex".into(),
        profile: profile.map(|id| hide_agent_adapter::ProfileId(id.into())),
        model: None,
        effort: None,
        purpose: String::new(),
    }
}

/// The caller `sender` conducts a started mission whose worker candidates
/// are `candidates`, with `work` listed on this machine.
fn conducting(candidates: Vec<Candidate>) -> Spawner {
    let spawner = local_spawner();
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        // The core's delivery path hands its spawn to this fixture's worker.
        runtime.install_delivery_client(spawner.client.clone());
        work_listed(&mut runtime);
        runtime
            .worktree_catalog
            .projects
            .push(crate::model::ProjectWorktreesSnapshot {
                root_path: LOCAL_CHECKOUT.into(),
                ..Default::default()
            });
        runtime.snapshot.ui_state.conductor_settings.candidates = candidates;
        let context = authority(&spawner.actor).context;
        for op in [
            json!({"op": "create", "title": "Review the fix"}),
            json!({"op": "plan", "plan": {"nodes": [
                {"key": "fix", "title": "Fix it", "kind": "change", "criteria": ["it works"], "delivery": "pr"},
            ]}}),
            json!({"op": "start"}),
        ] {
            let op: MissionCommand = serde_json::from_value(op).unwrap();
            runtime
                .mission_command(crate::node::TEST_NODE, "sender", &context, op)
                .unwrap();
        }
    }
    spawner
}

/// The Conductor's reviewer, beside it in its checkout, started with
/// `--profile <profile>` or none.
fn reviewer(profile: Option<&str>) -> Command {
    Command::Agents {
        command: crate::coordination::Command::Spawn {
            pick: None,
            parent: Some("here".into()),
            machine: None,
            // The name the fake engine shows its started agent under.
            name: "worker".into(),
            intent: "review".into(),
            kind: "codex".into(),
            repo: LOCAL_CHECKOUT.into(),
            branch: "topic".into(),
            path: None,
            args: Vec::new(),
            launch: Default::default(),
            task: None,
            brief: None,
            mission: None,
            plan_node: None,
            plan: None,
            beside: true,
            profile: profile.map(str::to_owned),
        },
    }
}

impl Spawner {
    /// The core admits `command` under its lock, `meanwhile` changes the
    /// runtime before the start, and the start runs off the lock.
    fn spawn_mission(
        &self,
        command: Command,
        meanwhile: impl FnOnce(&mut Runtime),
    ) -> Result<Value, String> {
        let prepared = {
            let mut runtime = self.runtime.lock().unwrap();
            let prepared = runtime.prepare_delivery(
                crate::node::TEST_NODE,
                "sender",
                &authority(&self.actor).context,
                None,
                command,
            )?;
            meanwhile(&mut runtime);
            prepared
        };
        prepared.run(Duration::from_secs(10))
    }

    /// The profile the reviewer's start ran with, as its spawn record keeps
    /// it.
    fn started_profile(&self) -> Option<String> {
        let ledger = self.ledger();
        let record = ledger
            .spawns
            .iter()
            .find(|spawn| spawn.intent == "review")
            .expect("the reviewer's spawn");
        assert!(record.completed, "the reviewer started");
        record
            .started_profile
            .as_ref()
            .map(|profile| profile.name.clone())
    }
}

/// A worker admitted on Codex's built-in default starts on it, though the
/// project chose `work` for Codex between the admission and the start: the
/// start runs the item the admission judged and does not pick again.
#[test]
fn a_worker_starts_with_the_item_its_admission_judged() {
    let spawner = conducting(vec![codex(None)]);
    spawner
        .spawn_mission(reviewer(None), |runtime| project_picks(runtime, "p-work"))
        .expect("the admitted default starts");
    assert_eq!(spawner.started_profile(), None);
    assert_eq!(spawner.count("start_agent"), 1);
}

/// The same spawn asked again after its worker started answers that worker,
/// though the profile it named was renamed since: what finished is not
/// judged again, so the name it gave no longer has to resolve. The same
/// intent asked with another profile is still a conflict.
#[test]
fn a_finished_spawn_asked_again_after_its_profile_was_renamed_answers_its_worker() {
    let spawner = conducting(vec![codex(Some("p-work"))]);
    let worker = spawner
        .spawn_mission(reviewer(Some("work")), |_| {})
        .expect("the work candidate starts");
    assert_eq!(spawner.started_profile().as_deref(), Some("work"));
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        let node = runtime.node.to_string();
        let mut kit = runtime.kit_state(&node);
        kit.profiles.as_mut().unwrap().profiles[0].name = "team".into();
        runtime.kit_states.insert(node, kit);
    }
    let again = spawner
        .spawn_mission(reviewer(Some("work")), |_| {})
        .expect("the same spawn answers its worker");
    assert_eq!(again["id"], worker["id"]);
    assert_eq!(spawner.count("start_agent"), 1, "nothing started again");
    assert_eq!(
        spawner.spawn_mission(reviewer(Some("default")), |_| {}),
        Err("intent_conflict".to_owned())
    );
}

/// The same spawn asked again after its worker started on the project's
/// choice answers that worker, though the project has since chosen an item
/// outside the candidates: the item was judged when the worker started.
#[test]
fn a_finished_spawn_asked_again_after_the_projects_choice_moved_answers_its_worker() {
    let spawner = conducting(vec![codex(None)]);
    let worker = spawner
        .spawn_mission(reviewer(None), |_| {})
        .expect("the default candidate starts");
    assert_eq!(spawner.started_profile(), None);
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        project_picks(&mut runtime, "p-work");
    }
    let again = spawner
        .spawn_mission(reviewer(None), |_| {})
        .expect("the same spawn answers its worker");
    assert_eq!(again["id"], worker["id"]);
    assert_eq!(spawner.count("start_agent"), 1, "nothing started again");
}

/// The started mission's id and the Conductor's context.
fn mission_id(spawner: &Spawner) -> String {
    let runtime = spawner.runtime.lock().unwrap();
    runtime.missions.book.missions[0].id.clone()
}

/// The Conductor replans the mission's one node with `worker`.
fn plan_worker(spawner: &Spawner, worker: Value) -> Result<Value, String> {
    let mut runtime = spawner.runtime.lock().unwrap();
    let context = authority(&spawner.actor).context;
    let op: MissionCommand = serde_json::from_value(json!({"op": "plan", "plan": {"nodes": [
        {"key": "fix", "title": "Fix it", "kind": "change", "criteria": ["it works"], "delivery": "pr", "worker": worker},
    ]}}))
    .unwrap();
    runtime
        .mission_command(crate::node::TEST_NODE, "sender", &context, op)
        .map(|(answer, _)| answer)
}

/// The node's planned worker as the book keeps it.
fn kept_worker(spawner: &Spawner) -> Value {
    let runtime = spawner.runtime.lock().unwrap();
    json!(runtime.missions.book.missions[0].nodes[0].worker)
}

/// The model a plan names for Codex when its candidate keeps the CLI's
/// default: the one `hide agent catalog` offers as `cli_default_model`.
const CODEX_DEFAULT_MODEL: &str = "gpt-5.6-luna";

/// A plan names its worker's profile by the name `--profile` takes, and the
/// book keeps that profile's id. A name the machine does not list is refused
/// with the node and the item, and so is an item none of the candidates
/// admits (PRD agent-profiles-phase2 D-08).
#[test]
fn a_plan_names_its_workers_profile_and_is_held_to_the_candidates() {
    let spawner = conducting(vec![codex(Some("p-work"))]);
    let model = CODEX_DEFAULT_MODEL;
    plan_worker(
        &spawner,
        json!({"agent": "codex", "profile": "work", "model": model}),
    )
    .expect("the work candidate is planned");
    assert_eq!(
        kept_worker(&spawner),
        json!({"agent": "codex", "profile": "p-work", "model": model, "effort": null, "effort_auto": false})
    );
    assert_eq!(
        plan_worker(
            &spawner,
            json!({"agent": "codex", "profile": "nope", "model": model}),
        ),
        Err("mission_profile_unknown\u{1f}fix: Codex · nope".to_owned())
    );
    assert_eq!(
        plan_worker(&spawner, json!({"agent": "codex", "model": model})),
        Err(format!(
            "mission_candidate_refused\u{1f}fix: Codex ({model})"
        ))
    );
    assert_eq!(
        plan_worker(
            &spawner,
            json!({"agent": "codex", "profile": "work", "model": "gpt-other"}),
        ),
        Err("mission_candidate_refused\u{1f}fix: Codex · work (gpt-other)".to_owned())
    );
}

/// A plan of the built-in default keeps no profile, so its book is the
/// bytes a build before plan profiles wrote and reads.
#[test]
fn a_plan_of_the_built_in_default_keeps_the_bytes_it_kept_before_profiles() {
    let model = CODEX_DEFAULT_MODEL;
    let spawner = conducting(vec![codex(None)]);
    plan_worker(
        &spawner,
        json!({"agent": "codex", "profile": "default", "model": model}),
    )
    .expect("the default candidate is planned");
    assert_eq!(
        serde_json::to_string(&kept_worker(&spawner)).unwrap(),
        format!(r#"{{"agent":"codex","model":"{model}","effort":null,"effort_auto":false}}"#),
        "a plan of the built-in default keeps the bytes it kept before profiles"
    );
}

/// The node's worker, started with the node's intent and its brief, with
/// `--profile <profile>` or none.
fn node_worker(mission: &str, profile: Option<&str>) -> Command {
    let mut command = reviewer(profile);
    if let Command::Agents {
        command: crate::coordination::Command::Spawn { intent, brief, .. },
    } = &mut command
    {
        *intent = format!("{mission}:fix");
        *brief = Some("Fix it.".into());
    }
    command
}

/// The profile the node's worker started with, as its spawn record keeps it.
fn node_started_profile(spawner: &Spawner, mission: &str) -> Option<String> {
    let ledger = spawner.ledger();
    let record = ledger
        .spawns
        .iter()
        .find(|spawn| spawn.intent == format!("{mission}:fix"))
        .expect("the worker's spawn");
    assert!(record.completed, "the worker started");
    record
        .started_profile
        .as_ref()
        .map(|profile| profile.name.clone())
}

/// A node's worker started with no `--profile` runs as its plan names it,
/// whatever the project chose (PRD agent-profiles-phase2 D-02).
#[test]
fn a_nodes_worker_named_by_no_profile_starts_as_its_plan_names_it() {
    let spawner = conducting(vec![codex(Some("p-work"))]);
    let mission = mission_id(&spawner);
    plan_worker(
        &spawner,
        json!({"agent": "codex", "profile": "work", "model": CODEX_DEFAULT_MODEL}),
    )
    .unwrap();
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        project_picks(&mut runtime, "default");
    }
    spawner
        .spawn_mission(node_worker(&mission, None), |_| {})
        .expect("the planned item starts");
    assert_eq!(
        node_started_profile(&spawner, &mission).as_deref(),
        Some("work")
    );
}

/// A `--profile` that differs from the plan is the spawn's own choice, held
/// to the candidates as any spawn's is.
#[test]
fn a_nodes_worker_named_by_another_profile_starts_with_it_when_a_candidate_admits_it() {
    let spawner = conducting(vec![codex(Some("p-work")), codex(None)]);
    let mission = mission_id(&spawner);
    plan_worker(
        &spawner,
        json!({"agent": "codex", "profile": "work", "model": CODEX_DEFAULT_MODEL}),
    )
    .unwrap();
    spawner
        .spawn_mission(node_worker(&mission, Some("default")), |_| {})
        .expect("the named default starts");
    assert_eq!(node_started_profile(&spawner, &mission), None);
}

/// `hide mission show` names a planned worker as its item, and a profile
/// the machine no longer lists as gone, so the Conductor reads which
/// account a node runs before it starts.
#[test]
fn mission_show_names_a_planned_workers_item() {
    let spawner = conducting(vec![codex(Some("p-work"))]);
    plan_worker(
        &spawner,
        json!({"agent": "codex", "profile": "work", "model": CODEX_DEFAULT_MODEL}),
    )
    .unwrap();
    let show = |spawner: &Spawner| {
        let mut runtime = spawner.runtime.lock().unwrap();
        let context = authority(&spawner.actor).context;
        let op: MissionCommand =
            serde_json::from_value(json!({"op": "show", "node": "fix"})).unwrap();
        runtime
            .mission_command(crate::node::TEST_NODE, "sender", &context, op)
            .unwrap()
            .0
    };
    let shown = show(&spawner);
    assert_eq!(shown["node"]["worker"]["item"], "Codex · work");
    assert!(shown["node"]["worker"]["profile_gone"].is_null());
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        let node = runtime.node.to_string();
        let mut kit = runtime.kit_state(&node);
        kit.profiles.as_mut().unwrap().profiles.clear();
        runtime.kit_states.insert(node, kit);
    }
    let shown = show(&spawner);
    assert_eq!(shown["node"]["worker"]["item"], "Codex");
    assert_eq!(shown["node"]["worker"]["profile_gone"], true);
}

/// A Conductor running `Codex · work` whose plan names its Codex worker with
/// no profile starts that worker on the built-in default, as the plan shows
/// it, rather than handing it its own profile as a spawn outside a plan
/// does (PRD agent-profiles-phase2 D-02).
#[test]
fn a_nodes_worker_planned_on_the_default_does_not_take_its_conductors_profile() {
    let spawner = conducting(vec![codex(Some("p-work")), codex(None)]);
    let mission = mission_id(&spawner);
    plan_worker(
        &spawner,
        json!({"agent": "codex", "model": CODEX_DEFAULT_MODEL}),
    )
    .unwrap();
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        let actor = spawner.actor.clone();
        runtime.snapshot.ui_state.pane_launches.insert(
            actor.pane_id.clone(),
            crate::model::PaneLaunch {
                pending: false,
                start: 0,
                kind: "codex".into(),
                name: actor.name.clone(),
                choice: Default::default(),
                session: None,
                profile: Some(crate::launch::LaunchProfile {
                    id: hide_agent_adapter::ProfileId("p-work".into()),
                    name: "work".into(),
                    account: hide_agent_adapter::AccountKey::Folder("/accounts/p-work".into()),
                }),
            },
        );
        assert_eq!(
            runtime.caller_launch_profile(&actor, "codex"),
            crate::launch::Parent::Profile(crate::launch::LaunchProfile {
                id: hide_agent_adapter::ProfileId("p-work".into()),
                name: "work".into(),
                account: hide_agent_adapter::AccountKey::Folder("/accounts/p-work".into()),
            }),
            "the Conductor runs work, which a spawn outside a plan would hand down"
        );
    }
    spawner
        .spawn_mission(node_worker(&mission, None), |_| {})
        .expect("the planned default starts");
    assert_eq!(node_started_profile(&spawner, &mission), None);
}

/// This machine's profile `p-work` now goes by `name`, or is gone for none.
fn work_becomes(spawner: &Spawner, name: Option<&str>) {
    let mut runtime = spawner.runtime.lock().unwrap();
    let node = runtime.node.to_string();
    let mut kit = runtime.kit_state(&node);
    let profiles = &mut kit.profiles.as_mut().unwrap().profiles;
    match name {
        Some(name) => profiles[0].name = name.into(),
        None => profiles.clear(),
    }
    runtime.kit_states.insert(node, kit);
}

/// A node that started keeps the profiles its worker and its reviewer were
/// planned with: the plan file it was planned from, asked again after `work`
/// was renamed and after it was removed, is taken, and the book still names
/// `p-work` for both.
#[test]
fn a_started_nodes_replan_keeps_its_profiles_after_one_is_renamed_or_removed() {
    let spawner = conducting(vec![codex(Some("p-work"))]);
    let worked = json!({"agent": "codex", "profile": "work", "model": CODEX_DEFAULT_MODEL});
    let replan = |spawner: &Spawner| {
        let mut runtime = spawner.runtime.lock().unwrap();
        let context = authority(&spawner.actor).context;
        let op: MissionCommand = serde_json::from_value(json!({"op": "plan", "plan": {"nodes": [
            {"key": "fix", "title": "Fix it", "kind": "change", "criteria": ["it works"], "delivery": "pr",
             "worker": worked, "checks": [{"as": "review", "by": [worked]}]},
        ]}}))
        .unwrap();
        runtime
            .mission_command(crate::node::TEST_NODE, "sender", &context, op)
            .map(|_| ())
    };
    let kept = |spawner: &Spawner| {
        let runtime = spawner.runtime.lock().unwrap();
        let node = &runtime.missions.book.missions[0].nodes[0];
        let reviewer = node
            .criteria
            .iter()
            .find(|line| line.key == "review")
            .and_then(|line| line.checkers.first())
            .map(|checker| checker.worker.profile.clone());
        (node.worker.as_ref().unwrap().profile.clone(), reviewer)
    };
    let work = Some(hide_agent_adapter::ProfileId("p-work".into()));
    replan(&spawner).expect("the work worker and reviewer are planned");
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        let context = authority(&spawner.actor).context;
        let op: MissionCommand = serde_json::from_value(
            json!({"op": "node", "node": "fix", "patch": {"status": "working"}}),
        )
        .unwrap();
        runtime
            .mission_command(crate::node::TEST_NODE, "sender", &context, op)
            .expect("the node starts");
    }
    work_becomes(&spawner, Some("team"));
    replan(&spawner).expect("a started node's renamed profile does not refuse the replan");
    assert_eq!(kept(&spawner), (work.clone(), Some(work.clone())));
    work_becomes(&spawner, None);
    replan(&spawner).expect("a started node's removed profile does not refuse the replan");
    assert_eq!(kept(&spawner), (work.clone(), Some(work)));
}

/// The node's worker started with no `--profile` answers that same worker
/// when the same command is asked again: after `work` was renamed, after it
/// was removed, and after the plan moved the node to the built-in default,
/// nothing starts again. Asked with a `--profile` it did not send, the same
/// intent is still a conflict.
#[test]
fn a_nodes_worker_asked_again_answers_its_worker_after_its_profile_or_plan_moved() {
    let spawner = conducting(vec![codex(Some("p-work")), codex(None)]);
    let mission = mission_id(&spawner);
    plan_worker(
        &spawner,
        json!({"agent": "codex", "profile": "work", "model": CODEX_DEFAULT_MODEL}),
    )
    .unwrap();
    let worker = spawner
        .spawn_mission(node_worker(&mission, None), |_| {})
        .expect("the planned item starts");
    assert_eq!(
        node_started_profile(&spawner, &mission).as_deref(),
        Some("work")
    );
    let again = |spawner: &Spawner| {
        spawner
            .spawn_mission(node_worker(&mission, None), |_| {})
            .map(|answer| answer["id"].clone())
    };
    work_becomes(&spawner, Some("team"));
    assert_eq!(again(&spawner), Ok(worker["id"].clone()), "after a rename");
    work_becomes(&spawner, None);
    assert_eq!(again(&spawner), Ok(worker["id"].clone()), "after a removal");
    plan_worker(
        &spawner,
        json!({"agent": "codex", "model": CODEX_DEFAULT_MODEL}),
    )
    .expect("the node moves to the built-in default");
    assert_eq!(again(&spawner), Ok(worker["id"].clone()), "after a replan");
    assert_eq!(spawner.count("start_agent"), 1, "nothing started again");
    assert_eq!(
        spawner.spawn_mission(node_worker(&mission, Some("default")), |_| {}),
        Err("intent_conflict".to_owned())
    );
}
