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
